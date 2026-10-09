use crate::model::alert::AlertAsset;
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::model::watchlist::{WatchlistInstrument, new_id, now_ms};
use crate::model::watchlist_text::ImportReport;
use crate::model::{
    ChartColumn, ChartConfig, ChartLayout, DataSource, Drawing, DrawingCommand, IndicatorConfig,
    InstrumentId, Kline, LayoutSnapshot, ReplayEngine, ReplayInterval, ReplayStatus, SavedView,
    Timeframe, Tool,
};
use crate::paper_host::{PaperHost, PaperPanel, PaperPanelAction};
use crate::service::live::BinanceLiveFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use crate::ui::paper_chart_trading::PaperOverlayAction;
use crate::ui::replay_bar::{self, ReplayBarAction, ReplayBarInput};
use crate::ui::replay_date_sheet::{self, ReplayDateSheetState};
use crate::ui::watchlist_sidebar::{
    self, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState,
};
use crate::ui::{self, ChartCard, ChartCardAction, CrosshairInfo, DrawingFrame, ReplayChartFrame};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CANDLE_LIMIT: u32 = 500;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

type SearchInbox = Receiver<Vec<(DataSource, Result<Vec<SearchResult>, String>)>>;

const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
];
const STOCK_SOURCES: [DataSource; 1] = [DataSource::Alpaca];
const PREDICTION_SOURCES: [DataSource; 2] = [DataSource::Polymarket, DataSource::Kalshi];

#[derive(Clone, Copy, Eq, PartialEq)]
enum SearchTab {
    Crypto,
    Stocks,
    Predictions,
    CoinMarketCap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceAction {
    OpenPortfolio,
    /// Open the Script Manager tab.
    OpenPineScripts,
    OpenAlerts,
    /// The chart header's bell: open the price-alert editor on this market.
    NewAlert(AlertAsset),
}

/// Debounced search state for one tab of the Add Chart sheet.
struct SearchSession {
    query: String,
    loading: bool,
    inbox: Option<SearchInbox>,
    results: Vec<(DataSource, Vec<SearchResult>)>,
    selected: Option<SearchResult>,
    debounce: Option<Instant>,
    last_query: String,
    error: Option<String>,
}

impl SearchSession {
    fn new() -> Self {
        Self {
            query: String::new(),
            loading: false,
            inbox: None,
            results: Vec::new(),
            selected: None,
            debounce: None,
            last_query: String::new(),
            error: None,
        }
    }

    fn start(&mut self, sources: &[DataSource], ctx: &egui::Context) {
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            self.results.clear();
            self.last_query = query;
            return;
        }
        if query == self.last_query {
            return;
        }
        self.last_query = query.clone();
        self.loading = true;
        self.results.clear();
        self.selected = None;
        self.error = None;

        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let sources = sources.to_vec();
        std::thread::spawn(move || {
            let results = sources
                .iter()
                .map(|source| (*source, service::search(*source, &query)))
                .collect();
            let _ = sender.send(results);
            repaint.request_repaint();
        });
        self.inbox = Some(receiver);
    }

    fn receive(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok(results) = receiver.try_recv() else {
            return;
        };
        self.inbox = None;
        self.loading = false;
        let mut error = None;
        let mut collected = Vec::new();
        for (source, result) in results {
            match result {
                Ok(list) => collected.push((source, list)),
                Err(message) if error.is_none() => error = Some(message),
                Err(_) => {}
            }
        }
        self.results = collected;
        self.error = error;
    }
}

/// What a saved workspace holds for one chart (defined in `model`; re-exported so the store and
/// importers can keep their `crate::workspace::PersistedChart` path).
pub use crate::model::PersistedChart;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedWorkspace {
    pub id: u64,
    pub timeframe: Timeframe,
    pub watchlist_visible: bool,
    pub charts: Vec<PersistedChart>,
    pub columns: Vec<ChartColumn>,
    pub focused: Option<u64>,
}

impl crate::store::Document for PersistedWorkspace {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

/// The focused chart's identity, read by the Pine integration (alert subscription + tick loop).
pub struct PineChartTarget {
    pub chart_id: u64,
    pub instrument: InstrumentId,
    pub timeframe: Timeframe,
}

/// One chart's runtime state: market data, fetch worker, and view.
struct Chart {
    id: u64,
    symbol: String,
    source: DataSource,
    candles: Vec<Kline>,
    loading: bool,
    error: Option<String>,
    last_fetch: Instant,
    inbox: Option<Receiver<Result<Vec<Kline>, String>>>,
    chart: ChartCard,
    log_scale: bool,
    indicators: IndicatorConfig,
    drawings: Vec<Drawing>,
    next_drawing_id: u64,
    last_paper_quote: Option<(f64, Instant)>,
}

impl Chart {
    fn new(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
        timeframe: Timeframe,
        ctx: &egui::Context,
    ) -> Self {
        let mut chart = Self {
            id,
            symbol: symbol.into(),
            source,
            candles: Vec::new(),
            loading: false,
            error: None,
            last_fetch: Instant::now(),
            inbox: None,
            chart: ChartCard::default(),
            log_scale: false,
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            next_drawing_id: 1,
            last_paper_quote: None,
        };
        chart.fetch(timeframe, ctx);
        chart
    }

    fn restored(saved: PersistedChart, timeframe: Timeframe, ctx: &egui::Context) -> Self {
        let mut chart = Self::new(saved.id, saved.symbol, saved.source, timeframe, ctx);
        chart.log_scale = saved.log_scale;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.chart.set_pine_source(saved.pine_source);
        chart.chart.set_visible_candles(saved.candle_count as usize);
        chart.indicators = saved.indicators;
        chart.drawings = saved.drawings;
        chart.next_drawing_id = chart
            .drawings
            .iter()
            .map(Drawing::id)
            .max()
            .map_or(1, |max| max + 1);
        chart
    }

    fn fetch(&mut self, timeframe: Timeframe, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let symbol = self.symbol.clone();
        let source = self.source;
        let interval = timeframe.binance_interval();
        let repaint = ctx.clone();

        std::thread::spawn(move || {
            let result = service::fetch_klines(source, &symbol, interval, CANDLE_LIMIT);
            let _ = sender.send(result);
            repaint.request_repaint();
        });

        self.inbox = Some(receiver);
        self.loading = true;
        self.last_fetch = Instant::now();
    }

    fn receive_fetch(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(candles) => {
                self.candles = candles;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

/// A deferred mutation the grid collects while drawing, applied after the pass.
enum ChartCommand {
    Close(u64),
    Focus(u64),
    Settings(u64),
    Move {
        chart_id: u64,
        to_column: u64,
        before: Option<u64>,
    },
    NewColumn(u64),
}

/// A reversible drawing change: the state of one chart's drawings before a batch
/// of commands ran, so undo can restore it.
struct UndoEntry {
    chart_id: u64,
    drawings: Vec<Drawing>,
}

/// Rulers are transient measurements: never persisted, never in undo history.
fn is_ruler(drawing: &Drawing) -> bool {
    matches!(drawing, Drawing::Ruler { .. })
}

fn displayed_candles(candles: &[Kline], replay_ms: Option<i64>) -> &[Kline] {
    let Some(replay_ms) = replay_ms else {
        return candles;
    };
    let end = candles.partition_point(|candle| candle.open_time <= replay_ms);
    &candles[..end]
}

pub struct ChartWorkspace {
    id: u64,
    timeframe: Timeframe,
    charts: Vec<Chart>,
    columns: Vec<ChartColumn>,
    focused: Option<u64>,
    next_id: u64,
    watchlist_visible: bool,
    baseline: Option<LayoutSnapshot>,
    show_search: bool,
    search_tab: SearchTab,
    crypto_search: SearchSession,
    stock_search: SearchSession,
    prediction_search: SearchSession,
    cmc_selected: Option<SearchResult>,
    live_feed: Option<BinanceLiveFeed>,
    settings_chart: Option<u64>,
    crosshair: Option<CrosshairInfo>,
    active_tool: Tool,
    selected_drawing: Option<(u64, u64)>,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    replay: ReplayEngine,
    replay_last_step: Instant,
    watchlist: Arc<Mutex<WatchlistStore>>,
    watchlist_sidebar: WatchlistSidebarState,
    date_sheet: ReplayDateSheetState,
    quote_book: WatchlistQuoteBook,
    quote_coordinator: WatchlistQuoteCoordinator,
    import_report: Option<ImportReport>,
    watchlist_error: Option<String>,
    paper_panel_visible: bool,
    /// A chart's bell was clicked this frame; `ui` returns it as `WorkspaceAction::NewAlert`.
    new_alert_request: Option<AlertAsset>,
    paper_panel: PaperPanel,
    /// Saved-layout persistence + the tab's relation to the layout library.
    store: SharedStore,
    active_view_id: Option<u64>,
    active_view_name: Option<String>,
    save_prompt_open: bool,
    save_prompt_name: String,
    rename_prompt_open: bool,
    pending_open_id: Option<u64>,
}

impl ChartWorkspace {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u64,
        symbol: impl Into<String>,
        timeframe: Timeframe,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        let mut workspace = Self {
            id,
            timeframe,
            charts: Vec::new(),
            columns: Vec::new(),
            focused: None,
            next_id: 1,
            watchlist_visible: true,
            baseline: None,
            show_search: false,
            search_tab: SearchTab::Crypto,
            crypto_search: SearchSession::new(),
            stock_search: SearchSession::new(),
            prediction_search: SearchSession::new(),
            cmc_selected: None,
            live_feed: None,
            settings_chart: None,
            crosshair: None,
            active_tool: Tool::Cursor,
            selected_drawing: None,
            undo: Vec::new(),
            redo: Vec::new(),
            replay: ReplayEngine::default(),
            replay_last_step: Instant::now(),
            watchlist,
            watchlist_sidebar: WatchlistSidebarState::default(),
            date_sheet: ReplayDateSheetState::default(),
            quote_book,
            quote_coordinator,
            import_report: None,
            watchlist_error: None,
            paper_panel_visible: false,
            new_alert_request: None,
            paper_panel: PaperPanel::default(),
            store,
            active_view_id: None,
            active_view_name: None,
            save_prompt_open: false,
            save_prompt_name: String::new(),
        };
        let chart_id = workspace.fresh_id();
        workspace.charts.push(Chart::new(
            chart_id,
            symbol,
            DataSource::Binance,
            timeframe,
            ctx,
        ));
        let chart_ids = [chart_id];
        workspace.columns = ChartColumn::resolved(None, &chart_ids, &mut workspace.next_id);
        workspace.focused = Some(chart_id);
        workspace.baseline = Some(workspace.layout_snapshot());
        workspace
    }

    pub fn restored(
        saved: PersistedWorkspace,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        let PersistedWorkspace {
            id,
            timeframe,
            watchlist_visible,
            charts,
            columns,
            focused,
        } = saved;

        let mut workspace = Self {
            id,
            timeframe,
            charts: charts
                .into_iter()
                .map(|saved| Chart::restored(saved, timeframe, ctx))
                .collect(),
            columns: Vec::new(),
            focused,
            next_id: 1,
            watchlist_visible,
            baseline: None,
            show_search: false,
            search_tab: SearchTab::Crypto,
            crypto_search: SearchSession::new(),
            stock_search: SearchSession::new(),
            prediction_search: SearchSession::new(),
            cmc_selected: None,
            live_feed: None,
            settings_chart: None,
            crosshair: None,
            active_tool: Tool::Cursor,
            selected_drawing: None,
            undo: Vec::new(),
            redo: Vec::new(),
            replay: ReplayEngine::default(),
            replay_last_step: Instant::now(),
            watchlist,
            watchlist_sidebar: WatchlistSidebarState::default(),
            date_sheet: ReplayDateSheetState::default(),
            quote_book,
            quote_coordinator,
            import_report: None,
            watchlist_error: None,
            paper_panel_visible: false,
            new_alert_request: None,
            paper_panel: PaperPanel::default(),
            store,
            active_view_id: None,
            active_view_name: None,
            save_prompt_open: false,
            save_prompt_name: String::new(),
        };
        workspace.next_id = workspace
            .charts
            .iter()
            .map(|chart| chart.id)
            .max()
            .map_or(1, |max| max + 1);
        let chart_ids: Vec<u64> = workspace.charts.iter().map(|chart| chart.id).collect();
        workspace.columns =
            ChartColumn::resolved(Some(&columns), &chart_ids, &mut workspace.next_id);
        if workspace
            .focused
            .is_none_or(|id| !workspace.charts.iter().any(|c| c.id == id))
        {
            workspace.focused = workspace.charts.first().map(|chart| chart.id);
        }
        workspace.baseline = Some(workspace.layout_snapshot());
        workspace
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn resolved_focused_id(&self) -> Option<u64> {
        if let Some(id) = self.focused
            && self.charts.iter().any(|chart| chart.id == id)
        {
            return Some(id);
        }
        self.charts.first().map(|chart| chart.id)
    }

    pub fn title(&self) -> String {
        let symbol = self
            .resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| chart.source.display_symbol(&chart.symbol))
            .unwrap_or_else(|| "—".to_owned());
        format!("DegenView — {symbol} · {}", self.timeframe.label())
    }

    pub fn persisted(&self) -> PersistedWorkspace {
        PersistedWorkspace {
            id: self.id,
            timeframe: self.timeframe,
            watchlist_visible: self.watchlist_visible,
            charts: self
                .charts
                .iter()
                .map(|chart| PersistedChart {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    log_scale: chart.log_scale,
                    y_zoom: chart.chart.y_zoom(),
                    indicators: chart.indicators,
                    drawings: chart
                        .drawings
                        .iter()
                        .filter(|drawing| !is_ruler(drawing))
                        .cloned()
                        .collect(),
                    pine_source: chart.chart.pine_source.clone(),
                    candle_count: chart.chart.visible_candles() as i64,
                })
                .collect(),
            columns: self.columns.clone(),
            focused: self.focused,
        }
    }

    pub fn layout_snapshot(&self) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: self.timeframe,
            configs: self
                .charts
                .iter()
                .map(|chart| ChartConfig {
                    id: chart.id,
                    symbol: chart.symbol.clone(),
                    source: chart.source,
                    log_scale: chart.log_scale,
                })
                .collect(),
            columns: self
                .columns
                .iter()
                .map(|column| column.chart_ids.clone())
                .collect(),
        }
    }

    /// Whether the layout differs from the last saved fingerprint. Zoom, focus,
    /// and watchlist edits do not count.
    pub fn is_dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|baseline| *baseline != self.layout_snapshot())
    }

    pub fn mark_clean(&mut self) {
        self.baseline = Some(self.layout_snapshot());
    }

    /// Applies a Pine script source to the focused chart (the Script Manager's "Add to Chart").
    pub fn set_pine_on_focused(&mut self, source: String) {
        if let Some(id) = self.resolved_focused_id()
            && let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == id)
        {
            chart.chart.set_pine_source(Some(source));
        }
    }

    /// The focused chart's identity for the Pine integration (alert subscription creation).
    pub fn focused_pine_target(&self) -> Option<PineChartTarget> {
        let id = self.resolved_focused_id()?;
        let chart = self.charts.iter().find(|chart| chart.id == id)?;
        Some(PineChartTarget {
            chart_id: chart.id,
            instrument: InstrumentId::new(chart.source, &chart.symbol),
            timeframe: self.timeframe,
        })
    }

    /// Every chart's `(symbol_key, candles)`, so the Pine subscription tick loop fires on any open
    /// chart, not just the focused one.
    pub fn charts_candles(&self) -> Vec<(String, Vec<Kline>)> {
        self.charts
            .iter()
            .map(|chart| {
                (
                    InstrumentId::new(chart.source, &chart.symbol).key(),
                    chart.candles.clone(),
                )
            })
            .collect()
    }

    // MARK: Saved layouts

    fn load_saved_views(&self) -> Vec<SavedView> {
        self.store
            .lock()
            .map(|store| store.saved_views())
            .unwrap_or_default()
    }

    /// The toolbar menu's "Save": save in place, or ask for a name the first time.
    fn request_save(&mut self, ctx: &egui::Context) {
        if self.active_view_id.is_some() {
            self.save_active();
        } else {
            self.save_prompt_name = self.active_view_name.clone().unwrap_or_default();
            self.save_prompt_open = true;
            ctx.request_repaint();
        }
    }

    /// The ⌘S shortcut and any app-level save.
    pub fn save_layout(&mut self, ctx: &egui::Context) {
        self.request_save(ctx);
    }

    fn save_active(&mut self) {
        let Some(id) = self.active_view_id else {
            return;
        };
        let created_at_ms = self
            .load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
            .map_or_else(now_ms, |view| view.created_at_ms);
        let view = self.build_view(
            id,
            self.active_view_name.clone().unwrap_or_default(),
            created_at_ms,
            self.last_opened_at_ms(id),
        );
        self.upsert_view(view);
        self.mark_clean();
    }

    fn save_named(&mut self, name: String) {
        let view = self.build_view(new_id(), name, now_ms(), None);
        self.active_view_id = Some(view.id);
        self.active_view_name = Some(view.name.clone());
        self.upsert_view(view);
        self.mark_clean();
    }

    /// Captures the live layout into a `SavedView` (without persisting it).
    fn build_view(
        &self,
        id: u64,
        name: String,
        created_at_ms: i64,
        last_opened_at_ms: Option<i64>,
    ) -> SavedView {
        let persisted = self.persisted();
        SavedView {
            id,
            name,
            timeframe: persisted.timeframe,
            created_at_ms,
            charts: persisted.charts,
            columns: Some(persisted.columns),
            candle_count: self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map_or(0, |chart| chart.chart.visible_candles() as i64),
            last_opened_at_ms,
            autosave: None,
        }
    }

    fn last_opened_at_ms(&self, id: u64) -> Option<i64> {
        self.load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
            .and_then(|view| view.last_opened_at_ms)
    }

    fn upsert_view(&self, view: SavedView) {
        if let Ok(store) = self.store.lock() {
            let mut views = store.saved_views();
            views.retain(|existing| existing.id != view.id);
            views.push(view);
            let _ = store.replace_saved_views(&views);
        }
    }

    fn open_saved_view(&mut self, id: u64, ctx: &egui::Context) {
        let Some(view) = self
            .load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
        else {
            return;
        };
        self.apply_saved_view(&view, ctx);
    }

    fn delete_saved_view(&mut self, id: u64) {
        if let Ok(store) = self.store.lock() {
            let mut views = store.saved_views();
            views.retain(|view| view.id != id);
            let _ = store.replace_saved_views(&views);
        }
        if self.active_view_id == Some(id) {
            self.active_view_id = None;
            self.active_view_name = None;
        }
    }

    /// Rebuilds the workspace from a saved layout; the one place a layout is applied.
    fn apply_saved_view(&mut self, view: &SavedView, ctx: &egui::Context) {
        self.charts.clear();
        for saved in &view.charts {
            self.charts
                .push(Chart::restored(saved.clone(), view.timeframe, ctx));
        }
        self.timeframe = view.timeframe;
        self.next_id = self
            .charts
            .iter()
            .map(|chart| chart.id)
            .max()
            .map_or(1, |max| max + 1);
        let chart_ids: Vec<u64> = self.charts.iter().map(|chart| chart.id).collect();
        self.columns =
            ChartColumn::resolved(view.columns.as_deref(), &chart_ids, &mut self.next_id);
        self.focused = self.charts.first().map(|chart| chart.id);
        self.drop_rulers();
        self.undo.clear();
        self.redo.clear();
        self.active_view_id = Some(view.id);
        self.active_view_name = Some(view.name.clone());
        self.baseline = Some(self.layout_snapshot());
        if let Ok(store) = self.store.lock() {
            let mut views = store.saved_views();
            if let Some(existing) = views.iter_mut().find(|existing| existing.id == view.id) {
                existing.last_opened_at_ms = Some(now_ms());
                let _ = store.replace_saved_views(&views);
            }
        }
    }

    // MARK: Chart lifecycle

    fn add_chart_with_symbol(&mut self, symbol: &str, source: DataSource, ctx: &egui::Context) {
        let id = self.fresh_id();
        self.charts
            .push(Chart::new(id, symbol, source, self.timeframe, ctx));
        self.place_in_grid(id);
        self.focused = Some(id);
    }

    fn add_from_search(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let instrument = InstrumentId::new(result.source, &result.full_symbol);
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(&instrument)
        }) {
            self.focus_chart(self.charts[existing].id);
            return;
        }
        self.add_chart_with_symbol(&result.full_symbol, result.source, ctx);
    }

    // MARK: Search sheet

    fn show_search_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_search;
        let mut close_requested = false;
        egui::Window::new("Add Chart")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([620.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.search_tab, SearchTab::Crypto, "Crypto");
                    ui.selectable_value(&mut self.search_tab, SearchTab::Stocks, "Stocks");
                    ui.selectable_value(
                        &mut self.search_tab,
                        SearchTab::Predictions,
                        "Predictions",
                    );
                    ui.selectable_value(
                        &mut self.search_tab,
                        SearchTab::CoinMarketCap,
                        "CoinMarketCap",
                    );
                });
                ui.separator();

                if self.search_tab == SearchTab::CoinMarketCap {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (key, title) in crate::service::cmc::CHART_TYPES {
                            let selected = self
                                .cmc_selected
                                .as_ref()
                                .is_some_and(|selected| selected.full_symbol == key);
                            if ui.selectable_label(selected, title).clicked() {
                                self.cmc_selected = Some(SearchResult {
                                    symbol: title.to_owned(),
                                    full_symbol: key.to_owned(),
                                    source: DataSource::CoinMarketCap,
                                    price: None,
                                });
                            }
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close_requested = true;
                        }
                        let can_add = self.cmc_selected.is_some();
                        if ui.add_enabled(can_add, egui::Button::new("Add")).clicked()
                            && let Some(selected) = self.cmc_selected.clone()
                        {
                            self.add_from_search(&selected, ctx);
                            close_requested = true;
                        }
                    });
                } else {
                    let hint = match self.search_tab {
                        SearchTab::Crypto => "Ticker symbol (e.g. BTC or PEPE)",
                        SearchTab::Stocks => "US stock symbol or company name (e.g. AAPL)",
                        SearchTab::Predictions => "Search prediction markets (e.g. fed, bitcoin)",
                        SearchTab::CoinMarketCap => unreachable!(),
                    };
                    let session = match self.search_tab {
                        SearchTab::Crypto => &mut self.crypto_search,
                        SearchTab::Stocks => &mut self.stock_search,
                        SearchTab::Predictions => &mut self.prediction_search,
                        SearchTab::CoinMarketCap => unreachable!(),
                    };

                    let response = ui.add(
                        egui::TextEdit::singleline(&mut session.query)
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        session.debounce = Some(Instant::now());
                    }

                    if let Some(error) = &session.error {
                        ui.label(egui::RichText::new(error).color(ui::theme::DOWN).small());
                    }

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (source, list) in &session.results {
                            ui.label(
                                egui::RichText::new(source.display_name())
                                    .strong()
                                    .small()
                                    .weak(),
                            );
                            for result in list {
                                let label = match result.price {
                                    Some(price) => format!(
                                        "{}    {}",
                                        result.symbol,
                                        crate::format::price(price)
                                    ),
                                    None => result.symbol.clone(),
                                };
                                let selected = session.selected.as_ref().is_some_and(|selected| {
                                    selected.source == result.source
                                        && selected.full_symbol == result.full_symbol
                                });
                                if ui.selectable_label(selected, label).clicked() {
                                    session.selected = Some(result.clone());
                                }
                            }
                        }
                        if session.loading {
                            ui.spinner();
                        }
                    });

                    let (can_add, selected_to_add) =
                        (session.selected.is_some(), session.selected.clone());

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close_requested = true;
                        }
                        if ui.add_enabled(can_add, egui::Button::new("Add")).clicked()
                            && let Some(selected) = selected_to_add
                        {
                            self.add_from_search(&selected, ctx);
                            close_requested = true;
                        }
                    });
                }
            });
        self.show_search = open && !close_requested;
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let Some(chart_id) = self.settings_chart else {
            return;
        };
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            self.settings_chart = None;
            return;
        };
        let mut open = true;
        egui::Window::new("Chart Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([280.0, 280.0])
            .show(ctx, |ui| {
                ui.checkbox(&mut chart.indicators.show_volume, "Volume Bars");
                ui.checkbox(&mut chart.indicators.show_rsi, "RSI");
                ui.checkbox(&mut chart.indicators.show_ema, "EMA");
                if chart.indicators.show_ema {
                    egui::ComboBox::from_id_salt("ema_period")
                        .selected_text(chart.indicators.ema_period.to_string())
                        .show_ui(ui, |ui| {
                            for period in crate::model::EMA_PERIODS {
                                ui.selectable_value(
                                    &mut chart.indicators.ema_period,
                                    period,
                                    period.to_string(),
                                );
                            }
                        });
                }
                ui.checkbox(&mut chart.indicators.show_bollinger, "Bollinger Bands");
                ui.checkbox(&mut chart.indicators.show_trend_flips, "Supertrend Flips");
            });
        if !open {
            self.settings_chart = None;
        }
    }

    fn place_in_grid(&mut self, chart_id: u64) {
        if self.columns.is_empty() || self.columns.len() < 2 {
            let column_id = self.fresh_id();
            self.columns
                .push(ChartColumn::new(column_id, vec![chart_id]));
        } else if let Some(shortest) = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .enumerate()
            .min_by_key(|(index, len)| (*len, *index))
            .map(|(index, _)| index)
        {
            self.columns[shortest].chart_ids.push(chart_id);
        }
    }

    fn remove_chart(&mut self, chart_id: u64) {
        self.charts.retain(|chart| chart.id != chart_id);
        if self.focused == Some(chart_id) {
            self.focused = None;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
    }

    fn focus_chart(&mut self, chart_id: u64) {
        if self.focused == Some(chart_id) {
            return;
        }
        if self.charts.iter().any(|chart| chart.id == chart_id) {
            self.focused = Some(chart_id);
        }
    }

    // MARK: Watchlist navigation

    fn switch_chart(
        &mut self,
        chart_id: u64,
        symbol: &str,
        source: DataSource,
        ctx: &egui::Context,
    ) {
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        let already_showing = chart.source == source
            && InstrumentId::new(chart.source, &chart.symbol)
                .is_same_market(&InstrumentId::new(source, symbol));
        if already_showing {
            self.focus_chart(chart_id);
            return;
        }
        chart.symbol = symbol.to_uppercase();
        chart.source = source;
        chart.last_paper_quote = None;
        chart.chart.reset_view();
        chart.candles.clear();
        chart.fetch(self.timeframe, ctx);
        self.focus_chart(chart_id);
    }

    // MARK: Live feed

    fn update_live_feed(&mut self, ctx: &egui::Context) {
        let symbols: Vec<String> = self
            .charts
            .iter()
            .filter(|chart| chart.source == DataSource::Binance)
            .map(|chart| chart.symbol.to_uppercase())
            .collect();
        let interval = self.timeframe.binance_interval().to_owned();
        // Binance streams no quarterly or yearly klines; those fold monthly candles.
        let monthly_fold = matches!(self.timeframe, Timeframe::Quarter | Timeframe::Year);
        let signature = (symbols, interval);

        let should_connect = !monthly_fold && !signature.0.is_empty();
        let needs_change = match &self.live_feed {
            Some(feed) => feed.signature() != &signature || !should_connect,
            None => should_connect,
        };
        if needs_change {
            self.live_feed = None;
            if should_connect {
                self.live_feed = Some(BinanceLiveFeed::connect(
                    signature.0.clone(),
                    signature.1.clone(),
                    ctx.clone(),
                ));
            }
        }

        let mut updates = Vec::new();
        if let Some(feed) = &self.live_feed {
            while let Ok((symbol, kline)) = feed.receiver.try_recv() {
                updates.push((symbol, kline));
            }
        }
        for (symbol, kline) in updates {
            self.apply_live_kline(&symbol, kline);
        }
    }

    fn apply_live_kline(&mut self, symbol: &str, kline: crate::model::Kline) {
        let Some(chart) = self.charts.iter_mut().find(|chart| {
            chart.source == DataSource::Binance && chart.symbol.eq_ignore_ascii_case(symbol)
        }) else {
            return;
        };
        let replaces_last = chart
            .candles
            .last()
            .is_some_and(|last| last.open_time == kline.open_time);
        if replaces_last {
            if let Some(last) = chart.candles.last_mut() {
                *last = kline;
            }
        } else {
            chart.candles.push(kline);
        }
    }

    // MARK: Drawings

    fn apply_drawing_commands(&mut self, chart_id: u64, commands: Vec<DrawingCommand>) {
        if commands.is_empty() {
            return;
        }
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            return;
        };
        self.undo.push(UndoEntry {
            chart_id,
            drawings: chart
                .drawings
                .iter()
                .filter(|drawing| !is_ruler(drawing))
                .cloned()
                .collect(),
        });
        self.redo.clear();
        for command in commands {
            match command {
                DrawingCommand::Add(drawing) => chart.drawings.push(drawing),
                DrawingCommand::Replace { id, drawing } => {
                    if let Some(slot) = chart.drawings.iter_mut().find(|d| d.id() == id) {
                        *slot = drawing;
                    }
                }
                DrawingCommand::Remove(id) => chart.drawings.retain(|d| d.id() != id),
            }
        }
    }

    fn undo(&mut self) {
        let Some(entry) = self.undo.pop() else {
            return;
        };
        let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        else {
            return;
        };
        let current = std::mem::replace(&mut chart.drawings, entry.drawings);
        self.redo.push(UndoEntry {
            chart_id: entry.chart_id,
            drawings: current.into_iter().filter(|d| !is_ruler(d)).collect(),
        });
    }

    fn redo(&mut self) {
        let Some(entry) = self.redo.pop() else {
            return;
        };
        let Some(chart) = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == entry.chart_id)
        else {
            return;
        };
        let current = std::mem::replace(&mut chart.drawings, entry.drawings);
        self.undo.push(UndoEntry {
            chart_id: entry.chart_id,
            drawings: current.into_iter().filter(|d| !is_ruler(d)).collect(),
        });
    }

    fn drop_rulers(&mut self) {
        for chart in &mut self.charts {
            chart.drawings.retain(|drawing| !is_ruler(drawing));
        }
    }

    fn handle_keyboard(&mut self, ui: &egui::Ui) {
        let (undo, redo) = ui.input(|input| {
            let command = input.modifiers.command;
            let z = input.key_pressed(egui::Key::Z);
            (
                command && z && !input.modifiers.shift,
                command && z && input.modifiers.shift,
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
    }

    // MARK: Replay

    /// Timeline the replay bar builds from the focused chart's candle opens.
    #[allow(dead_code)] // consumed by the replay bar UI (second agent)
    fn replay_timeline(&self) -> Vec<i64> {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| chart.candles.iter().map(|kline| kline.open_time).collect())
            .unwrap_or_default()
    }

    fn toggle_replay(&mut self) {
        if self.replay.is_bar_visible() {
            self.replay.stop();
        } else {
            self.replay.open();
        }
    }

    fn tick_replay(&mut self) {
        if self.replay.status != ReplayStatus::Playing {
            return;
        }
        let delay = self
            .replay
            .session
            .as_ref()
            .map_or(1_000, |session| session.speed.delay_ms());
        if self.replay_last_step.elapsed() >= Duration::from_millis(delay) {
            self.replay_last_step = Instant::now();
            self.replay.step_forward();
        }
    }

    fn focused_chart_data(&self) -> (String, Vec<i64>) {
        self.resolved_focused_id()
            .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
            .map(|chart| {
                (
                    chart.symbol.clone(),
                    chart.candles.iter().map(|kline| kline.open_time).collect(),
                )
            })
            .unwrap_or_default()
    }

    fn start_replay_first(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if let Some(first) = timeline.first().copied() {
            self.replay.start(first, &symbol, timeline);
        }
    }

    fn start_replay_random(&mut self) {
        let (symbol, timeline) = self.focused_chart_data();
        if timeline.is_empty() {
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0);
        let index = nanos % timeline.len();
        let picked = timeline[index];
        self.replay.start(picked, &symbol, timeline);
    }

    fn show_replay_bar(&mut self, ui: &mut egui::Ui) {
        let available = [ReplayInterval::Automatic, ReplayInterval::ChartBar];
        let input = ReplayBarInput {
            engine: &self.replay,
            is_preparing: false,
            notice: None,
            resolved_interval: None,
            available_intervals: &available,
        };
        let actions = replay_bar::show(ui, &input);
        self.apply_replay_actions(actions);
    }

    fn apply_replay_actions(&mut self, actions: Vec<ReplayBarAction>) {
        for action in actions {
            match action {
                ReplayBarAction::SelectOnChart => self.replay.begin_selecting(),
                ReplayBarAction::CancelSelection => self.replay.cancel_selection(),
                ReplayBarAction::ReturnToLive => self.replay.stop(),
                ReplayBarAction::ChangeStart => self.replay.begin_selecting(),
                ReplayBarAction::Restart => self.replay.restart(),
                ReplayBarAction::StepBack => {
                    self.replay.step_backward();
                }
                ReplayBarAction::StepForward => {
                    self.replay.step_forward();
                }
                ReplayBarAction::TogglePlayback => {
                    if self.replay.status == ReplayStatus::Playing {
                        self.replay.pause();
                    } else {
                        self.replay.play();
                    }
                }
                ReplayBarAction::Seek(fraction) => self.replay.seek(fraction),
                ReplayBarAction::SetSpeed(speed) => self.replay.set_speed(speed),
                ReplayBarAction::SetInterval(interval) => self.replay.set_interval(interval),
                ReplayBarAction::FirstBar => self.start_replay_first(),
                ReplayBarAction::RandomBar => self.start_replay_random(),
                ReplayBarAction::ChooseDate => {
                    let timeline = self.replay_timeline();
                    self.date_sheet.open(&timeline);
                }
                ReplayBarAction::DismissNotice => {}
            }
        }
    }

    // MARK: Drag reorder

    fn move_chart(&mut self, chart_id: u64, to_column: u64, before: Option<u64>) {
        if before == Some(chart_id) || !self.columns.iter().any(|c| c.id == to_column) {
            return;
        }
        let original = self.columns.clone();

        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns
            .retain(|column| !column.chart_ids.is_empty() || column.id == to_column);

        let Some(column_index) = self
            .columns
            .iter()
            .position(|column| column.id == to_column)
        else {
            self.columns = original;
            return;
        };
        let insertion = before
            .and_then(|target| {
                self.columns[column_index]
                    .chart_ids
                    .iter()
                    .position(|id| *id == target)
            })
            .unwrap_or(self.columns[column_index].chart_ids.len());
        self.columns[column_index]
            .chart_ids
            .insert(insertion, chart_id);
    }

    fn move_to_new_trailing_column(&mut self, chart_id: u64) {
        if !self.charts.iter().any(|chart| chart.id == chart_id) {
            return;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
        let column_id = self.fresh_id();
        self.columns
            .push(ChartColumn::new(column_id, vec![chart_id]));
    }

    // MARK: Panels

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        let active_view_id = self.active_view_id;
        let saved_views = self.load_saved_views();
        let layout_label = match &self.active_view_name {
            Some(name) if self.is_dirty() => format!("{name} *"),
            Some(name) => name.clone(),
            None if self.is_dirty() => "My Chart *".to_owned(),
            None => "My Chart".to_owned(),
        };
        let mut save_clicked = false;
        let mut open_id: Option<u64> = None;
        let mut delete_id: Option<u64> = None;
        ui.horizontal_centered(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("DV")
                    .color(ui::theme::ACCENT)
                    .strong()
                    .size(12.0),
            );
            ui.label(egui::RichText::new("DegenView").strong().size(14.0));
            ui.separator();

            let mut timeframe_changed = false;
            ui.spacing_mut().item_spacing.x = 2.0;
            for timeframe in Timeframe::ALL {
                let selected = self.timeframe == timeframe;
                if ui
                    .selectable_label(selected, timeframe.label())
                    .on_hover_text(timeframe.description())
                    .clicked()
                {
                    self.timeframe = timeframe;
                    timeframe_changed = true;
                }
            }
            if timeframe_changed {
                self.drop_rulers();
                for chart in &mut self.charts {
                    chart.chart.reset_view();
                    chart.candles.clear();
                    chart.fetch(self.timeframe, ctx);
                }
            }

            ui.separator();
            if let Some(focused) = self.resolved_focused_id()
                && let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == focused)
            {
                ui.toggle_value(&mut chart.log_scale, "Log");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+  Add Chart").clicked() {
                    self.show_search = true;
                }
                if ui
                    .button("Portfolio")
                    .on_hover_text("Open the portfolio tab")
                    .clicked()
                {
                    actions.push(WorkspaceAction::OpenPortfolio);
                }
                if ui
                    .button("Scripts")
                    .on_hover_text("Open the Script Manager")
                    .clicked()
                {
                    actions.push(WorkspaceAction::OpenPineScripts);
                }
                if ui
                    .button("Alerts")
                    .on_hover_text("Price + script alerts")
                    .clicked()
                {
                    actions.push(WorkspaceAction::OpenAlerts);
                }
                let replay_label = if self.replay.is_bar_visible() {
                    "Replay ▸"
                } else {
                    "Replay"
                };
                if ui
                    .button(replay_label)
                    .on_hover_text("Historical bar replay")
                    .clicked()
                {
                    self.toggle_replay();
                }
                ui.toggle_value(&mut self.watchlist_visible, "Watchlist");
                ui.menu_button(egui::RichText::new(format!("{layout_label}  ▾")), |ui| {
                    if ui.button("Save").clicked() {
                        save_clicked = true;
                        ui.close();
                    }
                    if !saved_views.is_empty() {
                        ui.separator();
                    }
                    for view in &saved_views {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(active_view_id == Some(view.id), &view.name)
                                .on_hover_text("Open this layout")
                                .clicked()
                            {
                                open_id = Some(view.id);
                                ui.close();
                            }
                            if ui
                                .small_button("🗑")
                                .on_hover_text("Delete this layout")
                                .clicked()
                            {
                                delete_id = Some(view.id);
                                ui.close();
                            }
                        });
                    }
                });
                if self.charts.iter().any(|chart| chart.loading) {
                    ui.spinner();
                }
            });
        });

        if save_clicked {
            self.request_save(ctx);
        }
        if let Some(id) = open_id {
            self.open_saved_view(id, ctx);
        }
        if let Some(id) = delete_id {
            self.delete_saved_view(id);
        }
        self.show_layout_prompt(ctx);
        actions
    }

    fn show_layout_prompt(&mut self, ctx: &egui::Context) {
        if !self.save_prompt_open {
            return;
        }
        egui::Window::new("Save Layout")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Name this layout:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.save_prompt_name).hint_text("Layout name"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        let name = self.save_prompt_name.trim().to_owned();
                        if !name.is_empty() {
                            self.save_named(name);
                            self.save_prompt_open = false;
                            self.save_prompt_name.clear();
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.save_prompt_open = false;
                        self.save_prompt_name.clear();
                    }
                });
            });
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            for (glyph, hint, tool) in [
                ("+", "Add chart", None),
                ("/", "Trend line", Some(Tool::TrendLine)),
                ("Fib", "Fibonacci retracement", Some(Tool::Fib)),
                ("Pen", "Brush", Some(Tool::Brush)),
                ("Rul", "Ruler", Some(Tool::Ruler)),
            ] {
                let armed = tool.is_some_and(|tool| self.active_tool == tool);
                let fill = if armed {
                    Some(ui::theme::SURFACE_SELECTED)
                } else {
                    None
                };
                let mut button = egui::Button::new(glyph).frame(false);
                if let Some(fill) = fill {
                    button = button.fill(fill);
                }
                if ui::accessibility::label(ui.add_sized([32.0, 32.0], button), hint)
                    .on_hover_text(hint)
                    .clicked()
                {
                    match tool {
                        None => self.show_search = true,
                        Some(tool) => {
                            self.active_tool = if self.active_tool == tool {
                                Tool::Cursor
                            } else {
                                tool
                            };
                            self.drop_rulers();
                        }
                    }
                }
                ui.add_space(4.0);
            }
            ui.separator();
            let paper_button =
                egui::Button::new("⇄")
                    .frame(false)
                    .fill(if self.paper_panel_visible {
                        ui::theme::SURFACE_SELECTED
                    } else {
                        egui::Color32::TRANSPARENT
                    });
            if ui::accessibility::label(ui.add_sized([32.0, 32.0], paper_button), "Paper Trading")
                .on_hover_text(if self.paper_panel_visible {
                    "Hide Paper Trading"
                } else {
                    "Paper Trading"
                })
                .clicked()
            {
                self.paper_panel_visible = !self.paper_panel_visible;
            }
        });
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (actions, instruments) = {
            let watchlist = Arc::clone(&self.watchlist);
            let store = watchlist.lock().expect("watchlist lock poisoned");
            let lists = store.lists();
            let flags = store.flags();
            let instruments: Vec<InstrumentId> = lists
                .iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let quote = |id: &InstrumentId| self.quote_book.get(id);
            let focused_market = self
                .resolved_focused_id()
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| InstrumentId::new(chart.source, &chart.symbol));
            let input = WatchlistSidebarInput {
                lists,
                flags,
                quote: &quote,
                focused_market: focused_market.as_ref(),
                load_failed: store.load_failed(),
                error: self.watchlist_error.as_deref(),
                import_report: self.import_report.as_ref(),
            };
            let actions = watchlist_sidebar::show(ui, &mut self.watchlist_sidebar, &input);
            (actions, instruments)
        };
        self.quote_coordinator
            .set_demand(self.id, instruments, self.watchlist_visible);
        self.apply_watchlist_actions(actions, ctx);
    }

    fn apply_watchlist_actions(&mut self, actions: Vec<WatchlistAction>, ctx: &egui::Context) {
        let mut opens: Vec<(bool, WatchlistInstrument)> = Vec::new();
        {
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            for action in actions {
                match action {
                    WatchlistAction::Open(item) => opens.push((true, item)),
                    WatchlistAction::AddChart(item) => opens.push((false, item)),
                    WatchlistAction::OpenInNewTab(_) | WatchlistAction::AddSymbol { .. } => {}
                    WatchlistAction::CreateAlert(_) => {}
                    WatchlistAction::CreateList { name, with } => {
                        store.create_list(&name, with);
                    }
                    WatchlistAction::RenameList { id, name } => store.rename_list(id, &name),
                    WatchlistAction::DuplicateList { id, name } => {
                        store.duplicate_list(id, &name);
                    }
                    WatchlistAction::DeleteList(id) => store.delete_list(id),
                    WatchlistAction::AddToList { list, item } => store.add_to_list(list, item),
                    WatchlistAction::RemoveFromList { list, instrument } => {
                        store.remove_from_list(list, &instrument);
                    }
                    WatchlistAction::RemoveEntry { list, entry } => {
                        store.remove_entry(list, entry);
                    }
                    WatchlistAction::MoveToSection {
                        list,
                        entry,
                        section,
                    } => store.move_instruments(list, &[entry], section),
                    WatchlistAction::Move { list, mv } => {
                        store.move_entries(list, &mv.ids, mv.before);
                    }
                    WatchlistAction::MoveToSectionByDrop { list, mv } => {
                        store.move_instruments(list, &mv.ids, mv.section);
                    }
                    WatchlistAction::AddSection { list, title } => store.add_section(list, &title),
                    WatchlistAction::RenameSection {
                        list,
                        section,
                        title,
                    } => store.rename_section(list, section, &title),
                    WatchlistAction::DeleteSection { list, section } => {
                        store.delete_section(list, section);
                    }
                    WatchlistAction::SetSectionCollapsed {
                        list,
                        section,
                        collapsed,
                    } => store.set_section_collapsed(list, section, collapsed),
                    WatchlistAction::SetSort { list, sort } => store.set_sort(list, sort),
                    WatchlistAction::SetColumn {
                        list,
                        column,
                        visible,
                    } => store.set_column(list, column, visible),
                    WatchlistAction::SetShowsDescription { list, shows } => {
                        store.set_shows_description(list, shows);
                    }
                    WatchlistAction::SetFlag { instrument, flag } => {
                        store.set_flag(&instrument, flag);
                    }
                    WatchlistAction::Import { list, text } => {
                        match store.import_into(list, &text) {
                            Some(report) => {
                                self.import_report = Some(report);
                                self.watchlist_error = None;
                            }
                            None => {
                                self.watchlist_error = Some("Import failed".to_owned());
                            }
                        }
                    }
                    WatchlistAction::Export { list } => {
                        if let Some(text) = store.export(list) {
                            ctx.copy_text(text);
                        }
                    }
                    WatchlistAction::CopyText(text) => ctx.copy_text(text),
                    WatchlistAction::DismissError => self.watchlist_error = None,
                }
            }
        }
        for (open, item) in opens {
            if open {
                self.open_watchlist_instrument(&item, ctx);
            } else {
                self.add_chart_from_instrument(&item, ctx);
            }
        }
    }

    fn open_watchlist_instrument(&mut self, item: &WatchlistInstrument, ctx: &egui::Context) {
        let instrument = &item.instrument;
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(instrument)
        }) {
            self.focus_chart(self.charts[existing].id);
            return;
        }
        if let Some(focused) = self.resolved_focused_id() {
            self.switch_chart(focused, &instrument.symbol, instrument.source, ctx);
        } else {
            self.add_chart_with_symbol(&instrument.symbol, instrument.source, ctx);
        }
    }

    fn add_chart_from_instrument(&mut self, item: &WatchlistInstrument, ctx: &egui::Context) {
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(&item.instrument)
        }) {
            self.focus_chart(self.charts[existing].id);
            return;
        }
        self.add_chart_with_symbol(&item.instrument.symbol, item.instrument.source, ctx);
    }

    fn sync_paper_quotes(&mut self, host: &mut PaperHost) {
        if !self.paper_panel_visible {
            return;
        }
        let _ = host.ensure_default_account();
        let now = Instant::now();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64);
        for chart in &mut self.charts {
            if !Self::supports_paper_trading(chart.source) {
                continue;
            }
            let Some(last) = chart.candles.last().map(|candle| candle.close) else {
                continue;
            };
            if !last.is_finite() || last <= 0.0 {
                continue;
            }
            let should_send = chart.last_paper_quote.is_none_or(|(price, sent_at)| {
                let elapsed = sent_at.elapsed();
                (price != last && elapsed >= PAPER_QUOTE_FLUSH_INTERVAL)
                    || elapsed >= REFRESH_INTERVAL
            });
            if !should_send {
                continue;
            }
            let instrument = Self::paper_instrument(chart);
            let _ = host.process_quote(PaperQuote {
                instrument_key: instrument.key,
                bid: None,
                ask: None,
                last: Some(last),
                timestamp,
                is_market_open: true,
            });
            chart.last_paper_quote = Some((last, now));
        }
    }

    fn configure_paper_cards(&mut self, host: &PaperHost) {
        let account_id = host.snapshot().selected_account_id;
        let account = account_id.and_then(|id| {
            host.snapshot()
                .accounts
                .iter()
                .find(|account| account.id == id)
        });
        let enabled = self.paper_panel_visible
            && self.paper_panel.show_trading_on_charts()
            && account.is_some();
        let currency = account.map_or(PaperCurrency::Usd, |account| account.base_currency);
        for chart in &mut self.charts {
            let key = Self::paper_instrument(chart).key;
            let chart_enabled = enabled && Self::supports_paper_trading(chart.source);
            chart.chart.paper.trading_enabled = chart_enabled;
            chart.chart.paper.show_markers = chart_enabled;
            chart.chart.paper.currency = currency;
            chart.chart.paper.positions = host
                .snapshot()
                .positions
                .iter()
                .filter(|position| {
                    account.is_some_and(|account| account.settings.show_positions_on_chart)
                        && Some(position.account_id) == account_id
                        && position.instrument.key == key
                })
                .map(|position| (position.clone(), host.unrealized_pnl(position)))
                .collect();
            chart.chart.paper.orders = host
                .snapshot()
                .orders
                .iter()
                .filter(|order| {
                    account.is_some_and(|account| account.settings.show_orders_on_chart)
                        && Some(order.account_id) == account_id
                        && order.instrument.key == key
                        && order.status.is_working()
                })
                .cloned()
                .collect();
        }
    }

    /// The market a chart's bell starts an alert on: its identity, the ticker and a name.
    fn alert_asset(chart: &Chart) -> AlertAsset {
        Self::alert_asset_for(&chart.symbol, chart.source)
    }

    fn alert_asset_for(symbol: &str, source: DataSource) -> AlertAsset {
        let display = source.display_symbol(symbol);
        let ticker = display.split('/').next().unwrap_or(&display).to_uppercase();
        AlertAsset::new(
            crate::model::InstrumentId::new(source, symbol),
            ticker.clone(),
            ticker,
        )
    }

    fn paper_instrument(chart: &Chart) -> PaperInstrument {
        PaperInstrument::chart(
            &chart.symbol,
            chart.source.display_symbol(&chart.symbol),
            chart.source,
        )
    }

    fn supports_paper_trading(source: DataSource) -> bool {
        source != DataSource::CoinMarketCap
    }

    fn apply_paper_overlay_actions(host: &mut PaperHost, actions: Vec<PaperOverlayAction>) {
        for action in actions {
            match action {
                PaperOverlayAction::ClosePosition {
                    account_id,
                    instrument_key,
                } => {
                    let _ = host.close_position(account_id, &instrument_key);
                }
                PaperOverlayAction::ModifyOrder { order_id, price } => {
                    let _ = host.move_order(order_id, price);
                }
                PaperOverlayAction::CancelOrder(id) => {
                    let _ = host.cancel(id);
                }
            }
        }
    }

    // MARK: Grid

    fn render_grid(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) {
        if self.charts.is_empty() {
            ui.centered_and_justified(|ui| {
                if ui.button("+  Add Chart").clicked() {
                    self.show_search = true;
                }
            });
            return;
        }

        let timeframe = self.timeframe;
        let available = ui.available_size();
        let column_count = self.columns.len().max(1);
        let max_rows = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .max()
            .unwrap_or(0);
        let sizing = max_rows * column_count;
        let card_height = ChartLayout::grid_card_height(available.y, sizing, column_count);

        let plan = self.columns.clone();
        let index_map: HashMap<u64, usize> = self
            .charts
            .iter()
            .enumerate()
            .map(|(index, chart)| (chart.id, index))
            .collect();
        let focused = self.focused;
        let multi = self.charts.len() > 1;
        let crosshair = self.crosshair;
        let mut crosshair_update: Option<CrosshairInfo> = None;
        let active_tool = self.active_tool;
        let selected_drawing = self.selected_drawing;
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        let replay_selecting = self.replay.status == ReplayStatus::SelectingStart;
        let replay_active = self.replay.is_active();
        let mut replay_picked: Option<(u64, i64)> = None;

        let mut drawing_commands: Vec<(u64, Vec<DrawingCommand>)> = Vec::new();
        let mut drawing_select: Option<(u64, Option<u64>)> = None;
        let mut drawing_escape = false;
        let mut paper_actions = Vec::new();
        let mut quick_trade = None;
        let mut new_alert: Option<AlertAsset> = None;

        let mut commands: Vec<ChartCommand> = Vec::new();
        let can_add_column = ChartLayout::can_add_column(available.x, column_count);
        let strip_width = if can_add_column { 24.0 } else { 0.0 };
        let cols_width = (ui.available_width() - strip_width).max(0.0);

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(cols_width, available.y),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.columns(column_count, |uis| {
                        for (column_index, column_ui) in uis.iter_mut().enumerate() {
                            let column = &plan[column_index];
                            for &chart_id in &column.chart_ids {
                                let Some(&index) = index_map.get(&chart_id) else {
                                    continue;
                                };
                                let is_focused = multi && focused == Some(chart_id);
                                let card = egui::Frame::new()
                                    .fill(ui::theme::SURFACE)
                                    .corner_radius(8.0)
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if is_focused {
                                            ui::theme::ACCENT
                                        } else {
                                            ui::theme::BORDER
                                        },
                                    ));

                                let drawn = column_ui.allocate_ui_with_layout(
                                    egui::vec2(column_ui.available_width(), card_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let chart = &mut self.charts[index];
                                        let candles = displayed_candles(&chart.candles, replay_ms);
                                        card.show(ui, |ui| {
                                            let mut action = ChartCardAction::default();
                                            chart.chart.header(
                                                ui,
                                                &chart.symbol,
                                                chart.source,
                                                timeframe,
                                                candles,
                                                chart.loading,
                                                &mut action,
                                                chart_id,
                                            );
                                            ui.separator();
                                            let selected = selected_drawing
                                                .filter(|(chart, _)| *chart == chart_id)
                                                .map(|(_, drawing)| drawing);
                                            let mut frame = DrawingFrame::new(
                                                &chart.drawings,
                                                active_tool,
                                                selected,
                                                chart.next_drawing_id,
                                            );
                                            let mut replay_frame = ReplayChartFrame {
                                                selecting: replay_selecting,
                                                active: replay_active,
                                                picked_ms: None,
                                            };
                                            let pointer = chart.chart.body(
                                                ui,
                                                &chart.symbol,
                                                candles,
                                                chart.loading,
                                                chart.error.as_deref(),
                                                chart.log_scale,
                                                chart.source,
                                                &chart.indicators,
                                                crosshair.as_ref(),
                                                chart_id,
                                                &mut frame,
                                                &mut replay_frame,
                                            );
                                            if let Some(picked_ms) = replay_frame.picked_ms {
                                                replay_picked = Some((chart_id, picked_ms));
                                            }
                                            chart.next_drawing_id = frame.next_id;
                                            if let Some(select) = frame.select {
                                                drawing_select = Some((chart_id, select));
                                            }
                                            drawing_escape |= frame.escape_consumed;
                                            let frame_commands =
                                                std::mem::take(&mut frame.commands);
                                            if !frame_commands.is_empty() {
                                                drawing_commands.push((chart_id, frame_commands));
                                            }
                                            if pointer.clicked {
                                                action.focus_requested = true;
                                            }
                                            if let (Some(x_fraction), Some(price)) =
                                                (pointer.hover_x_fraction, pointer.hover_price)
                                            {
                                                crosshair_update = Some(CrosshairInfo {
                                                    owner_id: chart_id,
                                                    x_fraction,
                                                    price,
                                                });
                                            }
                                            action
                                        })
                                    },
                                );

                                if drawn.inner.inner.focus_requested {
                                    commands.push(ChartCommand::Focus(chart_id));
                                }
                                if drawn.inner.inner.close_requested {
                                    commands.push(ChartCommand::Close(chart_id));
                                }
                                if drawn.inner.inner.settings_requested {
                                    commands.push(ChartCommand::Settings(chart_id));
                                }
                                if drawn.inner.inner.new_alert {
                                    new_alert = Some(Self::alert_asset(&self.charts[index]));
                                }
                                if let Some(side) = drawn.inner.inner.quick_trade {
                                    let reference_price =
                                        displayed_candles(&self.charts[index].candles, replay_ms)
                                            .last()
                                            .map(|candle| candle.close);
                                    quick_trade = Some((chart_id, side, reference_price));
                                }
                                paper_actions.append(&mut self.charts[index].chart.paper.actions);
                                if let Some(dropped) =
                                    drawn.inner.response.dnd_release_payload::<u64>()
                                {
                                    commands.push(ChartCommand::Move {
                                        chart_id: *dropped,
                                        to_column: column.id,
                                        before: Some(chart_id),
                                    });
                                }
                            }

                            let (_, dropped) =
                                column_ui.dnd_drop_zone::<u64, _>(egui::Frame::new(), |ui| {
                                    ui.allocate_space(egui::vec2(ui.available_width(), 6.0));
                                });
                            if let Some(dropped) = dropped {
                                commands.push(ChartCommand::Move {
                                    chart_id: *dropped,
                                    to_column: column.id,
                                    before: None,
                                });
                            }
                        }
                    });
                },
            );

            if can_add_column {
                let (_, dropped) =
                    ui.dnd_drop_zone::<u64, _>(egui::Frame::new().fill(ui::theme::CHROME), |ui| {
                        ui.allocate_space(egui::vec2(ui.available_width(), available.y));
                    });
                if let Some(dropped) = dropped {
                    commands.push(ChartCommand::NewColumn(*dropped));
                }
            }
        });

        for command in commands {
            match command {
                ChartCommand::Close(chart_id) => self.remove_chart(chart_id),
                ChartCommand::Focus(chart_id) => self.focus_chart(chart_id),
                ChartCommand::Settings(chart_id) => self.settings_chart = Some(chart_id),
                ChartCommand::Move {
                    chart_id,
                    to_column,
                    before,
                } => self.move_chart(chart_id, to_column, before),
                ChartCommand::NewColumn(chart_id) => self.move_to_new_trailing_column(chart_id),
            }
        }

        self.crosshair = crosshair_update;

        if let Some((chart_id, picked_ms)) = replay_picked {
            let (symbol, timeline) = self
                .charts
                .iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| {
                    (
                        chart.symbol.clone(),
                        chart.candles.iter().map(|kline| kline.open_time).collect(),
                    )
                })
                .unwrap_or_default();
            self.replay.start(picked_ms, &symbol, timeline);
        }

        for (chart_id, drawing) in drawing_commands {
            self.apply_drawing_commands(chart_id, drawing);
        }
        Self::apply_paper_overlay_actions(paper_host, paper_actions);
        if new_alert.is_some() {
            self.new_alert_request = new_alert;
        }
        if let Some((chart_id, side, reference_price)) = quick_trade
            && let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id)
        {
            let instrument = Self::paper_instrument(chart);
            self.paper_panel
                .open_ticket(paper_host, instrument, reference_price, side);
        }
        if let Some((chart_id, select)) = drawing_select {
            self.selected_drawing = select.map(|drawing| (chart_id, drawing));
        }
        let esc_pressed = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if esc_pressed && !drawing_escape && !self.date_sheet.is_open() {
            if self.replay.status == ReplayStatus::SelectingStart {
                self.replay.cancel_selection();
            } else if self.active_tool != Tool::Cursor {
                self.active_tool = Tool::Cursor;
            } else {
                self.selected_drawing = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) -> Vec<WorkspaceAction> {
        let ctx = ui.ctx().clone();
        let timeframe = self.timeframe;
        for chart in &mut self.charts {
            chart.receive_fetch();
            if chart.inbox.is_none() && chart.last_fetch.elapsed() >= REFRESH_INTERVAL {
                chart.fetch(timeframe, &ctx);
            }
        }

        self.update_live_feed(&ctx);
        self.sync_paper_quotes(paper_host);
        self.configure_paper_cards(paper_host);
        self.handle_keyboard(ui);
        self.tick_replay();

        self.crypto_search.receive();
        self.stock_search.receive();
        self.prediction_search.receive();
        if self.show_search
            && let Some((session, sources)) = match self.search_tab {
                SearchTab::Crypto => Some((&mut self.crypto_search, CRYPTO_SOURCES.as_slice())),
                SearchTab::Stocks => Some((&mut self.stock_search, STOCK_SOURCES.as_slice())),
                SearchTab::Predictions => {
                    Some((&mut self.prediction_search, PREDICTION_SOURCES.as_slice()))
                }
                SearchTab::CoinMarketCap => None,
            }
            && session.inbox.is_none()
            && session
                .debounce
                .is_some_and(|debounce| debounce.elapsed() >= Duration::from_millis(300))
        {
            session.start(sources, &ctx);
        }

        ctx.request_repaint_after(Duration::from_millis(250));

        let mut actions = egui::Panel::top("workspace_toolbar")
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
            )
            .show(ui, |ui| self.show_toolbar(ui, &ctx))
            .inner;

        if self.replay.is_bar_visible() {
            egui::Panel::top("replay_bar")
                .exact_size(48.0)
                .frame(
                    egui::Frame::new()
                        .fill(replay_bar::tint(self.replay.status, false))
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| self.show_replay_bar(ui));
        }

        egui::Panel::left("tool_rail")
            .exact_size(48.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::CHROME)
                    .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
            )
            .show(ui, |ui| self.show_tool_rail(ui, &ctx));

        if self.watchlist_visible {
            egui::Panel::right("watchlist")
                .default_size(260.0)
                .min_size(220.0)
                .max_size(420.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::CHROME)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
                )
                .show(ui, |ui| self.show_watchlist(ui, &ctx));
        }

        if self.paper_panel_visible {
            let panel_actions = egui::Panel::bottom(egui::Id::new(("paper_panel", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .show(ui, |ui| self.paper_panel.ui(ui, paper_host))
                .inner;
            for action in panel_actions {
                match action {
                    PaperPanelAction::Close => self.paper_panel_visible = false,
                    // File dialogs/export plumbing is a follow-up; the panel keeps the action
                    // explicit instead of silently exporting to an arbitrary directory.
                    PaperPanelAction::Export => {}
                }
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::BACKGROUND)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| self.render_grid(ui, paper_host));
        if let Some(asset) = self.new_alert_request.take() {
            actions.push(WorkspaceAction::NewAlert(asset));
        }

        if self.show_search {
            self.show_search_window(&ctx);
        }
        if self.settings_chart.is_some() {
            self.show_settings_window(&ctx);
        }
        if self.date_sheet.is_open() {
            let timeline = self.replay_timeline();
            if let Some(replay_date_sheet::ReplayDateSheetResult::Start(ms)) =
                replay_date_sheet::show(&ctx, &mut self.date_sheet, &timeline)
            {
                let (symbol, timeline) = self.focused_chart_data();
                self.replay.start(ms, &symbol, timeline);
            }
        }
        actions
    }
}

impl Drop for ChartWorkspace {
    fn drop(&mut self) {
        self.quote_coordinator.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(open_time: i64, close: f64) -> Kline {
        Kline {
            open_time,
            close_time: open_time + 59_999,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn persisted_workspace_round_trips() {
        let saved = PersistedWorkspace {
            id: 7,
            timeframe: Timeframe::Hour,
            watchlist_visible: false,
            charts: vec![
                PersistedChart {
                    id: 1,
                    symbol: "BTCUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                    y_zoom: 1.0,
                    indicators: IndicatorConfig::default(),
                    drawings: Vec::new(),
                    pine_source: None,
                    candle_count: 0,
                },
                PersistedChart {
                    id: 2,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Coinbase,
                    log_scale: true,
                    y_zoom: 1.5,
                    indicators: IndicatorConfig {
                        show_rsi: true,
                        ..IndicatorConfig::default()
                    },
                    drawings: Vec::new(),
                    pine_source: Some("plot(close)".into()),
                    candle_count: 0,
                },
            ],
            columns: vec![ChartColumn::new(10, vec![1]), ChartColumn::new(11, vec![2])],
            focused: Some(2),
        };
        let encoded = serde_json::to_string(&saved).expect("serialize workspace");
        let decoded: PersistedWorkspace =
            serde_json::from_str(&encoded).expect("deserialize workspace");
        assert_eq!(decoded, saved);
    }

    #[test]
    fn an_old_chart_row_without_pine_source_decodes_as_none() {
        let encoded = serde_json::to_string(&PersistedChart {
            id: 1,
            symbol: "BTCUSDT".into(),
            source: DataSource::Binance,
            log_scale: false,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            pine_source: None,
            candle_count: 0,
        })
        .expect("serialize chart");
        let mut row: serde_json::Value = serde_json::from_str(&encoded).expect("json");
        row.as_object_mut().unwrap().remove("pine_source");
        let decoded: PersistedChart = serde_json::from_value(row).expect("deserialize old row");
        assert_eq!(decoded.pine_source, None);
    }

    #[test]
    fn coinmarketcap_index_cards_are_not_paper_markets() {
        assert!(!ChartWorkspace::supports_paper_trading(
            DataSource::CoinMarketCap
        ));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Binance));
        assert!(ChartWorkspace::supports_paper_trading(DataSource::Alpaca));
        assert!(ChartWorkspace::supports_paper_trading(
            DataSource::Polymarket
        ));
    }

    #[test]
    fn paper_ticket_reference_follows_the_replay_slice() {
        let candles = vec![
            candle(1_000, 10.0),
            candle(2_000, 20.0),
            candle(3_000, 30.0),
        ];
        assert_eq!(
            displayed_candles(&candles, None).last().unwrap().close,
            30.0
        );
        assert_eq!(
            displayed_candles(&candles, Some(2_500))
                .last()
                .unwrap()
                .close,
            20.0
        );
        assert!(displayed_candles(&candles, Some(500)).is_empty());
    }

    #[test]
    fn the_bell_starts_an_alert_on_the_charts_market() {
        let btc = ChartWorkspace::alert_asset_for("BTCUSDT", DataSource::Binance);
        assert_eq!(btc.symbol, "BTC");
        assert_eq!(btc.instrument.symbol, "BTCUSDT");
        assert_eq!(btc.key(), "BINANCE:BTCUSDT");
        let coinbase = ChartWorkspace::alert_asset_for("ETH-USD", DataSource::Coinbase);
        assert_eq!(coinbase.symbol, "ETH");
        assert!(ui::supports_alerts(DataSource::Binance));
        assert!(ui::supports_alerts(DataSource::Alpaca));
        assert!(!ui::supports_alerts(DataSource::CoinMarketCap));
        assert!(!ui::supports_alerts(DataSource::Polymarket));
        assert!(!ui::supports_alerts(DataSource::Kalshi));
    }
}
