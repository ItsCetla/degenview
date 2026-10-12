use crate::model::alert::AlertAsset;
use crate::model::paper::{PaperCurrency, PaperInstrument, PaperQuote};
use crate::model::watchlist::{EntryId, WatchlistInstrument, new_id, now_ms};
use crate::model::watchlist_text::ImportReport;
use crate::model::{
    ChartColumn, ChartConfig, ChartLayout, DataSource, Drawing, DrawingCommand, IndicatorConfig,
    InstrumentId, Kline, LayoutSnapshot, PmSeries, ReplayEngine, ReplayInterval, ReplayStatus,
    SavedView, Timeframe, Tool,
};
use crate::paper_host::{PaperHost, PaperPanel, PaperPanelAction};
use crate::service::live::BinanceLiveFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use crate::ui::add_chart_sheet::{self, AddChartAction, AddChartSheet};
use crate::ui::chart_settings::{
    self, ChartSettingsAction, ChartSettingsContext, ChartSettingsState,
};
use crate::ui::empty_state::{self, EmptyStateAction};
use crate::ui::layout_prompts::{self, NameOutcome, NamePrompt, UnsavedChoice};
use crate::ui::paper_chart_trading::PaperOverlayAction;
use crate::ui::pine_settings_popover::{self, PineSettingsPopover};
use crate::ui::replay_bar::{self, ReplayBarAction, ReplayBarInput};
use crate::ui::replay_date_sheet::{self, ReplayDateSheetState};
use crate::ui::shortcut;
use crate::ui::tool_sidebar::{self, ToolSidebarAction};
use crate::ui::watchlist_sidebar::{
    self, WatchlistAction, WatchlistSidebarInput, WatchlistSidebarState,
};
use crate::ui::workspace_toolbar::{self, ToolbarAction};
use crate::ui::{
    self, ChartCard, ChartCardAction, CrosshairInfo, DrawingFrame, ReplayChartFrame, SeriesLine,
};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CANDLE_LIMIT: u32 = 500;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const PAPER_QUOTE_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceAction {
    OpenPortfolio,
    /// Open the Script Manager tab.
    OpenPineScripts,
    OpenAlerts,
    /// The chart header's bell: open the price-alert editor on this market.
    NewAlert(AlertAsset),
    /// Open Settings on the Alpaca page (Add Chart's "Set up Alpaca" notice).
    OpenAlpacaSettings,
    OpenWatchlistInNewTab(WatchlistInstrument),
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

/// One chart's runtime state: market data, fetch result, and view.
struct Chart {
    id: u64,
    symbol: String,
    source: DataSource,
    candles: Vec<Kline>,
    loading: bool,
    error: Option<String>,
    last_fetch: Instant,
    inbox: Option<Receiver<Result<Fetched, String>>>,
    fetch_generation: Arc<AtomicU64>,
    chart: ChartCard,
    log_scale: bool,
    indicators: IndicatorConfig,
    drawings: Vec<Drawing>,
    next_drawing_id: u64,
    last_paper_quote: Option<(f64, Instant)>,
    last_live_open_time: Option<i64>,
    /// Prediction markets: the choices charted together as lines, when there are several.
    pm_series: Vec<PmSeries>,
}

/// What one fetch brought back: the candles the chart reads, and for a multi-choice prediction
/// market each choice's own history.
struct Fetched {
    candles: Vec<Kline>,
    series: Vec<(String, Vec<Kline>)>,
}

impl Fetched {
    /// The candles come from the first choice that has any; the rest only draw their lines.
    fn from_choices(histories: Vec<(String, Vec<Kline>)>) -> Result<Self, String> {
        let candles = histories
            .iter()
            .find(|(_, history)| !history.is_empty())
            .map(|(_, history)| history.clone())
            .ok_or_else(|| "No price history for these choices".to_owned())?;
        Ok(Self {
            candles,
            series: histories,
        })
    }
}

impl Chart {
    fn new(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
        timeframe: Timeframe,
        ctx: &egui::Context,
    ) -> Self {
        let mut chart = Self::blank(id, symbol, source);
        chart.fetch(timeframe, ctx);
        chart
    }

    /// A chart that has not fetched yet, for callers that set up its prediction choices first.
    fn blank(id: u64, symbol: impl Into<String>, source: DataSource) -> Self {
        Self {
            id,
            symbol: symbol.into(),
            source,
            candles: Vec::new(),
            loading: false,
            error: None,
            last_fetch: Instant::now(),
            inbox: None,
            fetch_generation: Arc::new(AtomicU64::new(0)),
            chart: ChartCard::default(),
            log_scale: false,
            indicators: IndicatorConfig::default(),
            drawings: Vec::new(),
            next_drawing_id: 1,
            last_paper_quote: None,
            last_live_open_time: None,
            pm_series: Vec::new(),
        }
    }

    /// The name shown for the chart: a prediction market's title, else its symbol.
    fn display_title(&self) -> String {
        self.chart
            .title()
            .map_or_else(|| self.source.display_symbol(&self.symbol), str::to_owned)
    }

    /// Titles a prediction market and sets the choices it draws as lines (none for a single one).
    fn set_prediction(&mut self, title: String, series: Vec<PmSeries>) {
        self.chart.set_title(Some(title));
        self.pm_series = if series.len() > 1 { series } else { Vec::new() };
    }

    fn restored(saved: PersistedChart, timeframe: Timeframe, ctx: &egui::Context) -> Self {
        let instances = saved.instances();
        let mut chart = Self::blank(saved.id, saved.symbol, saved.source);
        chart.chart.set_title(saved.title);
        chart.pm_series = saved.pm_series;
        chart.fetch(timeframe, ctx);
        chart.log_scale = saved.log_scale;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.chart.set_appearance(saved.appearance);
        chart.chart.set_pine_instances(instances);
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
        let choices = self.pm_series.clone();
        let repaint = ctx.clone();

        let generation = self.fetch_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let current = Arc::downgrade(&self.fetch_generation);
        service::work_queue::submit(move || {
            if !current
                .upgrade()
                .is_some_and(|state| state.load(Ordering::Acquire) == generation)
            {
                return;
            }
            let result = if choices.is_empty() {
                service::fetch_klines(source, &symbol, interval, CANDLE_LIMIT).map(|candles| {
                    Fetched {
                        candles,
                        series: Vec::new(),
                    }
                })
            } else {
                Fetched::from_choices(service::fetch_choices(
                    source,
                    &choices,
                    interval,
                    CANDLE_LIMIT,
                ))
            };
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
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Chart fetch stopped before returning data".to_owned())
            }
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(fetched) => {
                self.candles =
                    merge_fetched_candles(fetched.candles, &self.candles, self.last_live_open_time);
                if !self.pm_series.is_empty() {
                    self.chart
                        .set_series(SeriesLine::build(&self.pm_series, fetched.series));
                }
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

/// A websocket update for the current or a later bar wins over an older REST snapshot.
fn merge_fetched_candles(
    mut fetched: Vec<Kline>,
    current: &[Kline],
    last_live_open_time: Option<i64>,
) -> Vec<Kline> {
    if let Some(live_open) = last_live_open_time
        && let Some(last_fetched) = fetched.last()
    {
        let newer = current
            .partition_point(|candle| candle.open_time < live_open.max(last_fetched.open_time));
        if newer < current.len() {
            let first_newer = current[newer].open_time;
            fetched.retain(|candle| candle.open_time < first_newer);
            fetched.extend_from_slice(&current[newer..]);
        }
    }
    if fetched.len() > CANDLE_LIMIT as usize {
        fetched.drain(..fetched.len() - CANDLE_LIMIT as usize);
    }
    fetched
}

/// A deferred mutation the grid collects while drawing, applied after the pass.
enum ChartCommand {
    Focus(u64),
    Settings(u64),
    Remove(u64),
    Indicator(u64, crate::ui::chart_indicator_strip::Action),
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
    add_chart: AddChartSheet,
    add_to_watchlist: Option<EntryId>,
    unseen_alerts: usize,
    live_feed: Option<BinanceLiveFeed>,
    settings_chart: Option<u64>,
    /// The Inputs and Style popover of an applied script, up beside its chip or settings row.
    script_popover: Option<PineSettingsPopover>,
    /// The chart a header ✕ asked to remove, awaiting confirmation.
    removing_chart: Option<u64>,
    settings_state: ChartSettingsState,
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
    watchlist_file_result: Option<Receiver<Result<(), String>>>,
    paper_panel_visible: bool,
    /// A chart's bell was clicked this frame; `ui` returns it as `WorkspaceAction::NewAlert`.
    new_alert_request: Option<AlertAsset>,
    open_new_tab_request: Option<WatchlistInstrument>,
    paper_panel: PaperPanel,
    /// Saved-layout persistence + the tab's relation to the layout library.
    store: SharedStore,
    active_view_id: Option<u64>,
    active_view_name: Option<String>,
    /// The name prompt that is open, if any, and the text in it.
    layout_prompt: Option<NamePrompt>,
    prompt_name: String,
    prompt_focus: bool,
    /// A layout waiting for the "save changes?" answer before it opens.
    pending_open_id: Option<u64>,
    /// Why the last layout operation failed, until the alert is dismissed.
    layout_error: Option<String>,
    autosave: AutosaveState,
    /// The chart drag in progress over the grid: where it would land, and the new-column preview.
    grid_drag: ui::grid_drop::GridDrag,
}

/// How long the layout must sit unchanged before an autosave-enabled layout is written.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// What the debounce wants done this frame.
#[derive(Debug, Eq, PartialEq)]
enum AutosaveStep {
    Nothing,
    /// Check again after this long.
    Wait(Duration),
    Save,
}

/// The debounce behind autosave. `armed` is the last layout seen dirty; `due` is when it gets
/// written, or `None` once it has been tried, so an unchanged layout is never retried in a loop.
#[derive(Default)]
struct AutosaveState {
    armed: Option<LayoutSnapshot>,
    due: Option<Instant>,
    /// The last autosave did not reach disk; the layout is still dirty.
    failed: bool,
}

impl AutosaveState {
    /// Advances the debounce: an edit restarts the countdown, and a layout that has sat unchanged
    /// for [`AUTOSAVE_DELAY`] is due. Once tried, a layout is not retried until it changes again.
    fn step(
        &mut self,
        dirty: bool,
        enabled: bool,
        snapshot: impl FnOnce() -> LayoutSnapshot,
        now: Instant,
    ) -> AutosaveStep {
        if !dirty || !enabled {
            self.armed = None;
            self.due = None;
            return AutosaveStep::Nothing;
        }
        let snapshot = snapshot();
        if self.armed.as_ref() != Some(&snapshot) {
            self.armed = Some(snapshot);
            self.due = Some(now + AUTOSAVE_DELAY);
        }
        match self.due {
            None => AutosaveStep::Nothing,
            Some(due) if now < due => AutosaveStep::Wait(due - now),
            Some(_) => {
                self.due = None;
                AutosaveStep::Save
            }
        }
    }
}

impl ChartWorkspace {
    #[allow(clippy::too_many_arguments)]
    pub fn new_for_watchlist(
        id: u64,
        item: &WatchlistInstrument,
        watchlist: Arc<Mutex<WatchlistStore>>,
        quote_book: WatchlistQuoteBook,
        quote_coordinator: WatchlistQuoteCoordinator,
        store: SharedStore,
        ctx: &egui::Context,
    ) -> Self {
        Self::new_with_source(
            id,
            &item.instrument.symbol,
            item.instrument.source,
            Timeframe::Day,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
            ctx,
        )
    }

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
        Self::new_with_source(
            id,
            symbol,
            DataSource::Binance,
            timeframe,
            watchlist,
            quote_book,
            quote_coordinator,
            store,
            ctx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_source(
        id: u64,
        symbol: impl Into<String>,
        source: DataSource,
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
            add_chart: AddChartSheet::new(store.clone()),
            add_to_watchlist: None,
            unseen_alerts: 0,
            live_feed: None,
            settings_chart: None,
            script_popover: None,
            removing_chart: None,
            settings_state: ChartSettingsState::default(),
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
            watchlist_file_result: None,
            paper_panel_visible: false,
            new_alert_request: None,
            open_new_tab_request: None,
            paper_panel: PaperPanel::default(),
            store,
            active_view_id: None,
            active_view_name: None,
            layout_prompt: None,
            prompt_name: String::new(),
            prompt_focus: false,
            pending_open_id: None,
            layout_error: None,
            autosave: AutosaveState::default(),
            grid_drag: ui::grid_drop::GridDrag::default(),
        };
        let chart_id = workspace.fresh_id();
        workspace
            .charts
            .push(Chart::new(chart_id, symbol, source, timeframe, ctx));
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
            add_chart: AddChartSheet::new(store.clone()),
            add_to_watchlist: None,
            unseen_alerts: 0,
            live_feed: None,
            settings_chart: None,
            script_popover: None,
            removing_chart: None,
            settings_state: ChartSettingsState::default(),
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
            watchlist_file_result: None,
            paper_panel_visible: false,
            new_alert_request: None,
            open_new_tab_request: None,
            paper_panel: PaperPanel::default(),
            store,
            active_view_id: None,
            active_view_name: None,
            layout_prompt: None,
            prompt_name: String::new(),
            prompt_focus: false,
            pending_open_id: None,
            layout_error: None,
            autosave: AutosaveState::default(),
            grid_drag: ui::grid_drop::GridDrag::default(),
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
            .map(Chart::display_title)
            .unwrap_or_else(|| "—".to_owned());
        format!("{symbol} · {}", self.timeframe.label())
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
                    appearance: chart.chart.appearance(),
                    drawings: chart
                        .drawings
                        .iter()
                        .filter(|drawing| !is_ruler(drawing))
                        .cloned()
                        .collect(),
                    pine_source: None,
                    pine_instances: chart.chart.pine_instances().to_vec(),
                    candle_count: chart.chart.visible_candles() as i64,
                    title: chart.chart.title().map(str::to_owned),
                    pm_series: chart.pm_series.clone(),
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
                    appearance: chart.chart.appearance(),
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

    /// Adds a Pine script to the focused chart, after the ones already there (the Script
    /// Manager's "Add to Chart").
    pub fn add_pine_on_focused(&mut self, script_id: Option<String>, name: &str, source: &str) {
        if let Some(id) = self.resolved_focused_id()
            && let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == id)
        {
            chart.chart.add_pine_instance(script_id, name, source);
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

    /// Each chart's `(chart_id, symbol_key, timeframe)`, so script alerts can say whether their
    /// chart is open and still on their market.
    pub fn pine_datasets(&self) -> Vec<(u64, String, String)> {
        self.charts
            .iter()
            .map(|chart| {
                (
                    chart.id,
                    InstrumentId::new(chart.source, &chart.symbol).key(),
                    self.timeframe.binance_interval().to_owned(),
                )
            })
            .collect()
    }

    /// Every chart's `(symbol_key, candles)`, so the Pine subscription tick loop fires on any open
    /// chart, not just the focused one.
    pub fn charts_candles(&self) -> Vec<(u64, String, String, Vec<Kline>)> {
        self.charts
            .iter()
            .map(|chart| {
                (
                    chart.id,
                    InstrumentId::new(chart.source, &chart.symbol).key(),
                    self.timeframe.binance_interval().to_owned(),
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

    fn active_view(&self) -> Option<SavedView> {
        let id = self.active_view_id?;
        self.load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
    }

    fn autosave_enabled(&self) -> bool {
        self.active_view()
            .is_some_and(|view| view.autosave == Some(true))
    }

    /// The toolbar menu's "Save": save in place, or ask for a name the first time.
    fn request_save(&mut self, ctx: &egui::Context) {
        if self.active_view_id.is_some() {
            self.save_active(true);
        } else {
            self.open_name_prompt(NamePrompt::Save { then_open: None }, ctx);
        }
    }

    /// The Save shortcut (⌘S / Ctrl+S) and any app-level save.
    pub fn save_layout(&mut self, ctx: &egui::Context) {
        self.request_save(ctx);
    }

    fn open_name_prompt(&mut self, prompt: NamePrompt, ctx: &egui::Context) {
        let base = self.active_view_name.clone();
        self.prompt_name = match prompt {
            NamePrompt::Copy => format!("{} copy", base.as_deref().unwrap_or("My Chart")),
            NamePrompt::Rename => base.unwrap_or_default(),
            NamePrompt::Save { .. } => String::new(),
        };
        self.prompt_focus = true;
        self.layout_prompt = Some(prompt);
        ctx.request_repaint();
    }

    fn close_name_prompt(&mut self) {
        self.layout_prompt = None;
        self.prompt_name.clear();
    }

    /// Writes the live layout over the open one, keeping its autosave choice. A failure leaves
    /// the layout dirty; `report` says whether to interrupt with an alert (autosave only flags it).
    fn save_active(&mut self, report: bool) -> bool {
        let Some(existing) = self.active_view() else {
            return false;
        };
        let view = self.build_view(
            existing.id,
            existing.name.clone(),
            existing.created_at_ms,
            Some(now_ms()),
            existing.autosave,
        );
        match self.upsert_view(view) {
            Ok(()) => {
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                if report {
                    self.layout_error = Some(format!("Couldn’t save “{}”: {error}", existing.name));
                }
                false
            }
        }
    }

    /// First save of an unnamed layout (or a copy): mints a new identity.
    fn save_named(&mut self, name: String, autosave: Option<bool>) -> bool {
        let view = self.build_view(new_id(), name, now_ms(), Some(now_ms()), autosave);
        let (id, name) = (view.id, view.name.clone());
        match self.upsert_view(view) {
            Ok(()) => {
                self.active_view_id = Some(id);
                self.active_view_name = Some(name);
                self.mark_clean();
                self.autosave = AutosaveState::default();
                true
            }
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t save the layout: {error}"));
                false
            }
        }
    }

    /// Captures the live layout into a `SavedView` (without persisting it).
    fn build_view(
        &self,
        id: u64,
        name: String,
        created_at_ms: i64,
        last_opened_at_ms: Option<i64>,
        autosave: Option<bool>,
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
            autosave,
        }
    }

    fn upsert_view(&self, view: SavedView) -> Result<(), String> {
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        match views.iter_mut().find(|existing| existing.id == view.id) {
            // Replace in place so the layout keeps its position.
            Some(existing) => *existing = view,
            None => views.push(view),
        }
        store
            .replace_saved_views(&views)
            .map_err(|error| error.to_string())
    }

    /// Edits the open layout's stored row.
    fn update_active_view(&mut self, edit: impl FnOnce(&mut SavedView)) -> Result<(), String> {
        let Some(id) = self.active_view_id else {
            return Ok(());
        };
        let store = self.store.lock().map_err(|error| error.to_string())?;
        let mut views = store.saved_views();
        if let Some(view) = views.iter_mut().find(|view| view.id == id) {
            edit(view);
            store
                .replace_saved_views(&views)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn set_autosave(&mut self, enabled: bool) {
        match self.update_active_view(|view| view.autosave = enabled.then_some(true)) {
            Ok(()) => self.autosave = AutosaveState::default(),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t change Autosave: {error}"));
            }
        }
    }

    /// Runs once a frame: arms the debounce while an autosave layout has unsaved edits, and
    /// writes it once it has sat unchanged for [`AUTOSAVE_DELAY`].
    fn tick_autosave(&mut self, ctx: &egui::Context, dirty: bool, enabled: bool) {
        let mut state = std::mem::take(&mut self.autosave);
        let step = state.step(dirty, enabled, || self.layout_snapshot(), Instant::now());
        self.autosave = state;
        match step {
            AutosaveStep::Nothing => {}
            AutosaveStep::Wait(remaining) => ctx.request_repaint_after(remaining),
            AutosaveStep::Save => self.autosave.failed = !self.save_active(false),
        }
    }

    /// Writes a pending autosave now, before the layout changes or the app quits. False means
    /// the layout is still dirty on disk.
    pub fn flush_autosave(&mut self) -> bool {
        self.autosave.due = None;
        if !self.is_dirty() || !self.autosave_enabled() {
            return true;
        }
        if self.save_active(true) {
            true
        } else {
            self.autosave.failed = true;
            false
        }
    }

    /// The menu's "open": write an autosave layout first, ask over unsaved changes, or apply.
    fn request_open_saved_view(&mut self, id: u64, ctx: &egui::Context) {
        if self.autosave_enabled() {
            if self.flush_autosave() {
                self.apply_saved_view_by_id(id, ctx);
            }
        } else if self.is_dirty() {
            self.pending_open_id = Some(id);
            ctx.request_repaint();
        } else {
            self.apply_saved_view_by_id(id, ctx);
        }
    }

    fn apply_saved_view_by_id(&mut self, id: u64, ctx: &egui::Context) {
        let Some(view) = self
            .load_saved_views()
            .into_iter()
            .find(|view| view.id == id)
        else {
            return;
        };
        self.apply_saved_view(&view, ctx);
    }

    /// Copies the current layout (unsaved changes included) under a new identity; the original
    /// stays as it was saved.
    fn copy_layout(&mut self, name: String) {
        let autosave = self.active_view().and_then(|view| view.autosave);
        self.save_named(name, autosave);
    }

    fn rename_layout(&mut self, name: String) {
        if self.active_view_id.is_none() {
            self.save_named(name, None);
            return;
        }
        let renamed = name.clone();
        match self.update_active_view(|view| view.name = renamed) {
            Ok(()) => self.active_view_name = Some(name),
            Err(error) => {
                self.layout_error = Some(format!("Couldn’t rename the layout: {error}"));
            }
        }
    }

    fn delete_saved_view(&mut self, id: u64) {
        let result = self
            .store
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|store| {
                let mut views = store.saved_views();
                views.retain(|view| view.id != id);
                store
                    .replace_saved_views(&views)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.layout_error = Some(format!("Couldn’t delete the layout: {error}"));
            return;
        }
        // The tab keeps its charts and becomes unnamed.
        if self.active_view_id == Some(id) {
            self.active_view_id = None;
            self.active_view_name = None;
            self.autosave = AutosaveState::default();
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
        self.autosave = AutosaveState::default();
        let _ = self.update_active_view(|opened| opened.last_opened_at_ms = Some(now_ms()));
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
        if result.source.is_prediction_market() {
            self.title_prediction(result, ctx);
        }
    }

    /// Gives the chart just added for a prediction-market pick its event title and, when several
    /// choices were ticked, their lines; the choices need a second fetch.
    fn title_prediction(&mut self, result: &SearchResult, ctx: &egui::Context) {
        let Some(chart) = self.charts.last_mut() else {
            return;
        };
        let (title, series) = match &result.event {
            Some(event) if !event.title.is_empty() => (event.title.clone(), event.series.clone()),
            Some(event) => (result.symbol.clone(), event.series.clone()),
            None => (result.symbol.clone(), Vec::new()),
        };
        chart.set_prediction(title, series);
        chart.fetch(self.timeframe, ctx);
    }

    // MARK: Search sheet

    /// The chart settings sheet over the focused window; returns what the workspace's host should
    /// do (opening the Script Manager).
    fn show_settings_window(&mut self, ctx: &egui::Context) -> Vec<WorkspaceAction> {
        let Some(chart_id) = self.settings_chart else {
            return Vec::new();
        };
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            self.settings_chart = None;
            return Vec::new();
        };
        let title = chart.display_title();
        let outcome = chart_settings::show(
            ctx,
            &mut self.settings_state,
            ChartSettingsContext {
                title: &title,
                symbol: &chart.symbol,
                source: chart.source,
                loaded_candles: chart.candles.len(),
                indicators: &mut chart.indicators,
                card: &mut chart.chart,
            },
        );
        let mut actions = Vec::new();
        match outcome {
            Some(ChartSettingsAction::Done) => {
                self.settings_chart = None;
                self.script_popover = None;
            }
            Some(ChartSettingsAction::RemoveChart) => {
                self.settings_chart = None;
                self.script_popover = None;
                self.remove_chart(chart_id);
            }
            Some(ChartSettingsAction::OpenScriptManager) => {
                self.settings_chart = None;
                self.script_popover = None;
                actions.push(WorkspaceAction::OpenPineScripts);
            }
            Some(ChartSettingsAction::ConfigureScript(id, anchor)) => {
                self.toggle_script_popover(chart_id, id, anchor);
            }
            None => {}
        }
        actions
    }

    /// Opens an applied script's Inputs and Style popover beside `anchor`, or puts it away when it
    /// is already up for that script.
    fn toggle_script_popover(&mut self, chart_id: u64, instance_id: u64, anchor: egui::Rect) {
        if self
            .script_popover
            .as_ref()
            .is_some_and(|popover| popover.is_for(chart_id, instance_id))
        {
            self.script_popover = None;
            return;
        }
        self.script_popover = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| {
                let instance = chart
                    .chart
                    .pine_instances()
                    .iter()
                    .find(|instance| instance.id == instance_id)?;
                Some(PineSettingsPopover::open(
                    chart_id,
                    instance,
                    chart.chart.pine_style_rows(instance_id),
                    anchor,
                ))
            });
    }

    /// Draws the script popover, if one is up, and applies what it returns.
    fn show_script_popover(&mut self, ctx: &egui::Context) {
        let Some(mut popover) = self.script_popover.take() else {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        };
        let (chart_id, instance_id) = (popover.chart_id(), popover.instance_id());
        let applied = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| {
                chart
                    .chart
                    .pine_instances()
                    .iter()
                    .any(|instance| instance.id == instance_id)
            });
        if !applied {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        }
        match popover.show(ctx) {
            pine_settings_popover::Outcome::Open => {
                ui::chart_indicator_strip::set_open_script_popover(
                    ctx,
                    Some((chart_id, instance_id)),
                );
                self.script_popover = Some(popover);
            }
            pine_settings_popover::Outcome::Cancel => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            }
            pine_settings_popover::Outcome::Apply { inputs, style } => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
                if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                    chart.chart.set_pine_instance_inputs(instance_id, &inputs);
                    chart.chart.set_pine_instance_style(instance_id, style);
                }
            }
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

    /// The macOS-style alert behind a header ✕.
    fn confirm_chart_removal(&mut self, ctx: &egui::Context, chart_id: u64) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            self.removing_chart = None;
            return;
        };
        let title = format!("Remove {}?", chart.display_title());
        let result = crate::ui::kit::confirm(
            ctx,
            egui::Id::new("remove-chart"),
            &crate::ui::kit::Confirm {
                icon: crate::ui::icons::Icon::Trash,
                title: &title,
                message: "The chart leaves this tab. You can add it again from Add Chart.",
                destructive: "Remove",
            },
        );
        if result == Some(crate::ui::kit::ConfirmOutcome::Confirmed) {
            self.remove_chart(chart_id);
        }
        if result.is_some() {
            self.removing_chart = None;
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
        chart.last_live_open_time = None;
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
        chart.last_live_open_time = chart.candles.last().map(|candle| candle.open_time);
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
        // Undo in the Add Chart search box edits the text, not the charts behind it.
        if self.add_chart.is_open() {
            return;
        }
        let (undo, redo) =
            ui.input(|input| (shortcut::UNDO.peek(input), shortcut::REDO.peek(input)));
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        // macOS owns Save in its menu (`NativeCommand::SaveCurrentLayout`); everywhere else the
        // shortcut is handled here, in the tab that has focus.
        if !shortcut::is_mac()
            && self.layout_prompt.is_none()
            && ui.input_mut(|input| shortcut::SAVE.consume(input))
        {
            self.save_layout(ui.ctx());
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
        // Leave the final fifth available to play through, as the Swift replay picker does.
        let eligible = ((timeline.len() as f64) * 0.8).ceil().max(1.0) as usize;
        let index = nanos % eligible;
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
        let mut save_clicked = false;
        let mut copy_clicked = false;
        let mut rename_clicked = false;
        let mut autosave_toggled = false;
        let mut open_id: Option<u64> = None;
        let mut delete_id: Option<u64> = None;
        let layouts: Vec<(u64, String)> = saved_views
            .iter()
            .map(|view| (view.id, view.name.clone()))
            .collect();
        let focused = self.resolved_focused_id();
        let dirty = self.is_dirty();
        let autosave_on = saved_views
            .iter()
            .find(|view| Some(view.id) == active_view_id)
            .is_some_and(|view| view.autosave == Some(true));
        self.tick_autosave(ctx, dirty, autosave_on);
        let input = workspace_toolbar::ToolbarInput {
            timeframe: self.timeframe,
            log_scale: focused
                .and_then(|id| self.charts.iter().find(|chart| chart.id == id))
                .map(|chart| chart.log_scale),
            layout_name: self.active_view_name.as_deref().unwrap_or("My Chart"),
            // Autosave keeps a layout clean in the user's eyes: "(unsaved)" only shows while
            // nothing is going to save it.
            unsaved: dirty && !(autosave_on && !self.autosave.failed),
            autosave: workspace_toolbar::AutosaveState {
                available: active_view_id.is_some(),
                on: autosave_on,
                failed: self.autosave.failed,
            },
            layouts: &layouts,
            active_layout: active_view_id,
            watchlist_visible: self.watchlist_visible,
        };
        for action in workspace_toolbar::show(ui, &input) {
            match action {
                ToolbarAction::SetTimeframe(timeframe) => {
                    self.timeframe = timeframe;
                    self.drop_rulers();
                    for chart in &mut self.charts {
                        chart.chart.reset_view();
                        chart.candles.clear();
                        chart.last_live_open_time = None;
                        chart.fetch(self.timeframe, ctx);
                    }
                }
                ToolbarAction::ToggleLogScale => {
                    if let Some(chart) =
                        focused.and_then(|id| self.charts.iter_mut().find(|chart| chart.id == id))
                    {
                        chart.log_scale = !chart.log_scale;
                    }
                }
                ToolbarAction::SaveLayout => save_clicked = true,
                ToolbarAction::CopyLayout => copy_clicked = true,
                ToolbarAction::RenameLayout => rename_clicked = true,
                ToolbarAction::ToggleAutosave => autosave_toggled = true,
                ToolbarAction::OpenLayout(id) => open_id = Some(id),
                ToolbarAction::DeleteLayout(id) => delete_id = Some(id),
                ToolbarAction::ToggleWatchlist => {
                    self.watchlist_visible = !self.watchlist_visible;
                }
                ToolbarAction::OpenPortfolio => actions.push(WorkspaceAction::OpenPortfolio),
                ToolbarAction::OpenScriptManager => {
                    actions.push(WorkspaceAction::OpenPineScripts);
                }
                ToolbarAction::AddChart => self.add_chart.open(),
            }
        }

        if save_clicked {
            self.request_save(ctx);
        }
        if autosave_toggled {
            self.set_autosave(!autosave_on);
        }
        if copy_clicked {
            self.open_name_prompt(NamePrompt::Copy, ctx);
        }
        if rename_clicked {
            self.open_name_prompt(NamePrompt::Rename, ctx);
        }
        if let Some(id) = open_id {
            self.request_open_saved_view(id, ctx);
        }
        if let Some(id) = delete_id {
            self.delete_saved_view(id);
        }
        self.show_layout_sheets(ctx);
        actions
    }

    /// The name prompt, the "save changes?" question and the error alert, one at a time.
    fn show_layout_sheets(&mut self, ctx: &egui::Context) {
        if let Some(message) = self.layout_error.clone() {
            if layout_prompts::show_error(ctx, &message) {
                self.layout_error = None;
            }
            return;
        }
        if let Some(prompt) = self.layout_prompt {
            let mut focus = std::mem::take(&mut self.prompt_focus);
            let mut name = std::mem::take(&mut self.prompt_name);
            let outcome = layout_prompts::show_name(ctx, prompt, &mut name, &mut focus);
            self.prompt_name = name;
            self.prompt_focus = focus;
            match outcome {
                Some(NameOutcome::Confirm(name)) => {
                    self.close_name_prompt();
                    match prompt {
                        NamePrompt::Save { then_open } => {
                            if self.save_named(name, None)
                                && let Some(id) = then_open
                            {
                                self.pending_open_id = None;
                                self.apply_saved_view_by_id(id, ctx);
                            }
                        }
                        NamePrompt::Copy => self.copy_layout(name),
                        NamePrompt::Rename => self.rename_layout(name),
                    }
                }
                Some(NameOutcome::Cancel) => {
                    self.close_name_prompt();
                    self.pending_open_id = None;
                }
                None => {}
            }
            return;
        }
        let Some(id) = self.pending_open_id else {
            return;
        };
        let name = self
            .active_view_name
            .clone()
            .unwrap_or_else(|| "Unnamed".to_owned());
        match layout_prompts::show_unsaved(ctx, &name) {
            Some(UnsavedChoice::Save) => {
                if self.active_view_id.is_none() {
                    // The first save needs a name; opening follows it.
                    self.open_name_prompt(
                        NamePrompt::Save {
                            then_open: Some(id),
                        },
                        ctx,
                    );
                } else if self.save_active(true) {
                    self.pending_open_id = None;
                    self.apply_saved_view_by_id(id, ctx);
                }
            }
            Some(UnsavedChoice::Discard) => {
                self.pending_open_id = None;
                self.apply_saved_view_by_id(id, ctx);
            }
            Some(UnsavedChoice::Cancel) => self.pending_open_id = None,
            None => {}
        }
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui) -> Vec<WorkspaceAction> {
        let input = tool_sidebar::ToolSidebarInput {
            active_tool: self.active_tool,
            paper_open: self.paper_panel_visible,
            replay_open: self.replay.is_bar_visible(),
            has_market: self
                .charts
                .iter()
                .any(|chart| chart.source != DataSource::CoinMarketCap),
            unseen_alerts: self.unseen_alerts,
        };
        let mut actions = Vec::new();
        for action in tool_sidebar::show(ui, &input) {
            match action {
                ToolSidebarAction::Select(tool) => {
                    self.active_tool = if self.active_tool == tool || tool == Tool::Cursor {
                        Tool::Cursor
                    } else {
                        tool
                    };
                    self.drop_rulers();
                }
                ToolSidebarAction::TogglePaper => {
                    self.paper_panel_visible = !self.paper_panel_visible;
                }
                ToolSidebarAction::ToggleReplay => self.toggle_replay(),
                ToolSidebarAction::OpenAlerts => actions.push(WorkspaceAction::OpenAlerts),
            }
        }
        actions
    }

    /// How many alerts have fired since the Alerts window was last looked at, for the bell.
    pub fn set_unseen_alerts(&mut self, count: usize) {
        self.unseen_alerts = count;
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(receiver) = &self.watchlist_file_result {
            match receiver.try_recv() {
                Ok(Err(error)) => {
                    self.watchlist_error = Some(error);
                    self.watchlist_file_result = None;
                }
                Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.watchlist_file_result = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
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
                error: self
                    .watchlist_error
                    .as_deref()
                    .or_else(|| store.last_error()),
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
                    WatchlistAction::OpenInNewTab(item) => {
                        // Hand off after the store lock is released.
                        self.open_new_tab_request = Some(item);
                    }
                    WatchlistAction::AddSymbol { list } => {
                        if let Some(list_id) = list
                            && let Some(name) = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list_id)
                                .map(|candidate| candidate.name.clone())
                        {
                            self.add_to_watchlist = Some(list_id);
                            self.add_chart.open_for_watchlist(&name);
                        }
                    }
                    WatchlistAction::CreateAlert(item) => {
                        self.new_alert_request = Some(AlertAsset::new(
                            item.instrument.clone(),
                            crate::ui::watchlist_rows::icon_base_symbol(&item),
                            item.name.clone(),
                        ));
                    }
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
                    WatchlistAction::BeginImport => self.import_report = None,
                    WatchlistAction::Export { list } => {
                        if let Some(text) = store.export(list) {
                            let name = store
                                .lists()
                                .iter()
                                .find(|candidate| candidate.id == list)
                                .map(|candidate| candidate.name.clone())
                                .unwrap_or_else(|| "Watchlist".to_owned());
                            let (sender, receiver) = mpsc::channel();
                            self.watchlist_file_result = Some(receiver);
                            std::thread::spawn(move || {
                                let result = crate::service::file_dialog::choose_save_path(
                                    "Export watchlist",
                                    &format!("{name}.txt"),
                                )
                                .and_then(|path| {
                                    path.map(|path| {
                                        crate::service::file_dialog::write_text(&path, &text)
                                    })
                                    .transpose()
                                })
                                .map(|_| ());
                                let _ = sender.send(result);
                            });
                        }
                    }
                    WatchlistAction::CopyText(text) => ctx.copy_text(text),
                    WatchlistAction::DismissError => {
                        self.watchlist_error = None;
                        store.clear_error();
                    }
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

    /// An empty tab: a search launcher and shortcuts back into favorites, recent markets and
    /// saved layouts.
    fn show_empty_state(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let favorites_list: Vec<WatchlistInstrument> = self
            .watchlist
            .lock()
            .map(|store| {
                let id = store.favorites();
                store
                    .lists()
                    .iter()
                    .find(|list| list.id == id)
                    .map(|list| list.instruments().cloned().collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let hidden_favorites = favorites_list.len().saturating_sub(empty_state::ITEM_LIMIT);
        let favorites: Vec<WatchlistInstrument> = favorites_list
            .into_iter()
            .take(empty_state::ITEM_LIMIT)
            .collect();
        let all_recents = self
            .store
            .lock()
            .map(|store| add_chart_sheet::load_recents(&store))
            .unwrap_or_default();
        let kept = empty_state::trim_recents(&all_recents, &favorites);
        let recents: Vec<add_chart_sheet::RecentMarket> = kept
            .iter()
            .map(|index| all_recents[*index].clone())
            .collect();
        let mut views = self.load_saved_views();
        views.sort_by_key(|view| std::cmp::Reverse(view.created_at_ms));
        let layouts: Vec<empty_state::LayoutSummary> = views
            .iter()
            .map(|view| empty_state::LayoutSummary {
                id: view.id,
                name: view.name.clone(),
                chart_count: view.charts.len(),
                timeframe: view.timeframe.label(),
                previews: view
                    .charts
                    .iter()
                    .filter(|chart| chart.source != DataSource::CoinMarketCap)
                    .map(|chart| (chart.source, chart.symbol.clone()))
                    .collect(),
            })
            .collect();
        let input = empty_state::EmptyStateInput {
            favorites: &favorites,
            hidden_favorites,
            recents: &recents,
            layouts: &layouts,
            offers_stocks: crate::service::keychain_api_keys::alpaca_credentials().is_some(),
        };
        for action in empty_state::show(ui, &input) {
            match action {
                EmptyStateAction::AddChart => self.add_chart.open(),
                EmptyStateAction::OpenFavorite(index) => {
                    self.add_chart_from_instrument(&favorites[index], &ctx);
                }
                EmptyStateAction::OpenRecent(index) => {
                    self.add_from_search(&recents[index].result(), &ctx);
                }
                EmptyStateAction::OpenLayout(id) => self.request_open_saved_view(id, &ctx),
                EmptyStateAction::OpenSuggestion(source, symbol, display) => {
                    self.add_from_search(
                        &SearchResult {
                            symbol: display,
                            full_symbol: symbol,
                            source,
                            price: None,
                            event: None,
                        },
                        &ctx,
                    );
                }
                EmptyStateAction::RemoveRecent(index) => {
                    let mut remaining = all_recents.clone();
                    remaining.remove(kept[index]);
                    if let Ok(store) = self.store.lock() {
                        add_chart_sheet::store_recents(&store, &remaining);
                    }
                }
                EmptyStateAction::ClearRecents => {
                    if let Ok(store) = self.store.lock() {
                        add_chart_sheet::store_recents(&store, &[]);
                    }
                }
                EmptyStateAction::ShowFavorites => self.watchlist_visible = true,
            }
        }
    }

    /// While a chart is dragged: finds what is under the pointer, draws its cue (an insert
    /// line, or the new-column strip and its preview) and, on release, queues the move.
    #[allow(clippy::too_many_arguments)]
    fn drive_grid_drag(
        &mut self,
        ctx: &egui::Context,
        dragged: Option<u64>,
        grid_rect: egui::Rect,
        mut zones: ui::grid_drop::Zones,
        can_add_column: bool,
        previewed: Option<u64>,
        commands: &mut Vec<ChartCommand>,
    ) {
        use ui::grid_drop::{self, DropTarget};
        let Some(dragged) = dragged else {
            return;
        };
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            egui::DragAndDrop::clear_payload(ctx);
            self.grid_drag.reset();
            return;
        }
        if can_add_column && zones.new_column.is_none() {
            zones.new_column = Some(egui::Rect::from_min_max(
                egui::pos2(
                    grid_rect.right() - grid_drop::NEW_COLUMN_STRIP_WIDTH,
                    grid_rect.top(),
                ),
                grid_rect.right_bottom(),
            ));
        }
        let target = ctx
            .pointer_latest_pos()
            .and_then(|pos| zones.target_at(pos));
        if let Some(wait) = self.grid_drag.hover(dragged, target, Instant::now()) {
            ctx.request_repaint_after(wait);
        }

        // Cues sit above the cards, on the grid's own layer.
        let painter_ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("grid-drop-cues"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("grid-drop-cues-layer"),
                ))
                .max_rect(grid_rect),
        );
        if can_add_column
            && previewed.is_none()
            && let Some(strip) = zones.new_column
        {
            grid_drop::new_column_strip(&painter_ui, strip, target == Some(DropTarget::NewColumn));
        }
        match target {
            Some(DropTarget::Before { chart, .. }) if chart != dragged => {
                if let Some((_, _, rect)) = zones.cards.iter().find(|(_, id, _)| *id == chart) {
                    grid_drop::insert_line_above(&painter_ui, *rect);
                }
            }
            Some(DropTarget::Append { column }) => {
                if let Some((_, rect)) = zones.columns.iter().find(|(id, _)| *id == column) {
                    grid_drop::insert_line_below(&painter_ui, *rect);
                }
            }
            _ => {}
        }
        if let Some(chart) = self.charts.iter().find(|chart| chart.id == dragged) {
            grid_drop::drag_label(ctx, &chart.display_title());
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);

        if ctx.input(|input| input.pointer.any_released()) {
            match target {
                Some(DropTarget::Before { column, chart }) if chart != dragged => {
                    commands.push(ChartCommand::Move {
                        chart_id: dragged,
                        to_column: column,
                        before: Some(chart),
                    });
                }
                Some(DropTarget::Append { column }) => commands.push(ChartCommand::Move {
                    chart_id: dragged,
                    to_column: column,
                    before: None,
                }),
                Some(DropTarget::NewColumn) => commands.push(ChartCommand::NewColumn(dragged)),
                _ => {}
            }
            self.grid_drag.reset();
        }
    }

    fn render_grid(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) {
        if self.charts.is_empty() {
            self.show_empty_state(ui);
            return;
        }

        let available = ui.available_size();
        let column_count = self.columns.len().max(1);
        let max_rows = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .max()
            .unwrap_or(0);
        let card_height =
            ChartLayout::grid_card_height(available.y, max_rows, ui.spacing().item_spacing.y);

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
        let (favorites, watchlist_memberships): (HashSet<InstrumentId>, Vec<_>) = {
            let store = self.watchlist.lock().expect("watchlist lock poisoned");
            let favorites = store
                .lists()
                .iter()
                .find(|list| list.is_favorites)
                .into_iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let memberships = store
                .lists()
                .iter()
                .map(|list| {
                    (
                        list.id,
                        list.name.clone(),
                        list.instruments()
                            .map(|item| item.instrument.clone())
                            .collect::<HashSet<_>>(),
                    )
                })
                .collect();
            (favorites, memberships)
        };
        let mut favorite_toggles = Vec::new();
        let mut watchlist_changes = Vec::new();

        let mut commands: Vec<ChartCommand> = Vec::new();
        let can_add_column = ChartLayout::can_add_column(available.x, column_count);
        let ctx = ui.ctx().clone();
        // Nothing about a drag shows until a chart is being dragged.
        let dragged: Option<u64> = egui::DragAndDrop::payload::<u64>(&ctx).map(|payload| *payload);
        if dragged.is_none() {
            self.grid_drag.reset();
        }
        let previewed = if can_add_column {
            self.grid_drag.previewed()
        } else {
            None
        };
        let mut zones = ui::grid_drop::Zones::default();
        let grid_rect = ui.available_rect_before_wrap();

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(available.x, available.y),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.columns(column_count + usize::from(previewed.is_some()), |uis| {
                        for (column_index, column_ui) in uis.iter_mut().enumerate() {
                            let Some(column) = plan.get(column_index) else {
                                // The previewed new column: a dashed placeholder where it would be.
                                let rect = column_ui.max_rect();
                                let card = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), card_height),
                                );
                                ui::grid_drop::new_column_placeholder(column_ui, card);
                                zones.new_column = Some(egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), available.y),
                                ));
                                continue;
                            };
                            let column_rect = column_ui.max_rect();
                            zones.columns.push((
                                column.id,
                                egui::Rect::from_min_size(
                                    column_rect.min,
                                    egui::vec2(column_rect.width(), available.y),
                                ),
                            ));
                            for &chart_id in &column.chart_ids {
                                let Some(&index) = index_map.get(&chart_id) else {
                                    continue;
                                };
                                let is_focused = multi && focused == Some(chart_id);
                                let card = egui::Frame::new()
                                    .fill(ui::theme::surface())
                                    .corner_radius(10.0)
                                    .inner_margin(egui::Margin::same(6))
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if replay_ms.is_some() {
                                            ui::theme::REPLAY.gamma_multiply(0.45)
                                        } else if is_focused {
                                            ui::theme::ACCENT
                                        } else {
                                            ui::theme::border()
                                        },
                                    ));

                                let drawn = column_ui.allocate_ui_with_layout(
                                    egui::vec2(column_ui.available_width(), card_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        if previewed == Some(chart_id) {
                                            ui.multiply_opacity(0.35);
                                        }
                                        let chart = &mut self.charts[index];
                                        chart.chart.set_series_cutoff(replay_ms);
                                        let candles = displayed_candles(&chart.candles, replay_ms);
                                        let instrument =
                                            InstrumentId::new(chart.source, &chart.symbol);
                                        let memberships: Vec<crate::ui::WatchlistMembership> =
                                            watchlist_memberships
                                                .iter()
                                                .map(|(id, name, instruments)| {
                                                    crate::ui::WatchlistMembership {
                                                        id: *id,
                                                        name: name.clone(),
                                                        contains_market: instruments
                                                            .contains(&instrument),
                                                    }
                                                })
                                                .collect();
                                        card.show(ui, |ui| {
                                            let mut action = ChartCardAction::default();
                                            chart.chart.header(
                                                ui,
                                                &chart.symbol,
                                                chart.source,
                                                &chart.indicators,
                                                candles,
                                                replay_ms.is_some(),
                                                favorites.contains(&instrument),
                                                &memberships,
                                                &mut action,
                                                chart_id,
                                            );
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
                                if drawn.inner.inner.remove_requested {
                                    commands.push(ChartCommand::Remove(chart_id));
                                }
                                if drawn.inner.inner.settings_requested {
                                    commands.push(ChartCommand::Settings(chart_id));
                                }
                                if let Some(action) = drawn.inner.inner.indicator_action {
                                    commands.push(ChartCommand::Indicator(chart_id, action));
                                }
                                if drawn.inner.inner.new_alert {
                                    new_alert = Some(Self::alert_asset(&self.charts[index]));
                                }
                                if drawn.inner.inner.toggle_favorite {
                                    favorite_toggles.push(chart_id);
                                }
                                if let Some(change) = drawn.inner.inner.watchlist_change {
                                    watchlist_changes.push((chart_id, change));
                                }
                                if let Some(side) = drawn.inner.inner.quick_trade {
                                    let reference_price =
                                        displayed_candles(&self.charts[index].candles, replay_ms)
                                            .last()
                                            .map(|candle| candle.close);
                                    quick_trade = Some((chart_id, side, reference_price));
                                }
                                paper_actions.append(&mut self.charts[index].chart.paper.actions);
                                zones.cards.push((column.id, chart_id, drawn.response.rect));
                            }
                        }
                    });
                },
            );
        });

        self.drive_grid_drag(
            &ctx,
            dragged,
            grid_rect,
            zones,
            can_add_column,
            previewed,
            &mut commands,
        );

        if !favorite_toggles.is_empty() || !watchlist_changes.is_empty() {
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            let list_id = store.favorites();
            for chart_id in favorite_toggles {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = InstrumentId::new(chart.source, &chart.symbol);
                    if favorites.contains(&instrument) {
                        store.remove_from_list(list_id, &instrument);
                    } else {
                        let item = WatchlistInstrument::from_market(
                            chart.source,
                            &chart.display_title(),
                            &chart.symbol,
                        );
                        store.add_to_list(list_id, item);
                    }
                }
            }
            for (chart_id, (list_id, included)) in watchlist_changes {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = InstrumentId::new(chart.source, &chart.symbol);
                    if included {
                        store.add_to_list(
                            list_id,
                            WatchlistInstrument::from_market(
                                chart.source,
                                &chart.display_title(),
                                &chart.symbol,
                            ),
                        );
                    } else {
                        store.remove_from_list(list_id, &instrument);
                    }
                }
            }
        }

        for command in commands {
            match command {
                ChartCommand::Focus(chart_id) => self.focus_chart(chart_id),
                ChartCommand::Remove(chart_id) => self.removing_chart = Some(chart_id),
                ChartCommand::Settings(chart_id) => {
                    self.settings_state = ChartSettingsState::default();
                    self.settings_chart = Some(chart_id);
                }
                ChartCommand::Indicator(chart_id, action) => {
                    use crate::ui::chart_indicator_strip::{Action, Kind};
                    match action {
                        Action::OpenIndicatorSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_chart = Some(chart_id);
                        }
                        Action::ConfigureScript(id, anchor) => {
                            self.toggle_script_popover(chart_id, id, anchor);
                        }
                        Action::OpenScriptSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_state.tab = chart_settings::Tab::Scripts;
                            self.settings_chart = Some(chart_id);
                        }
                        _ => {
                            if let Some(chart) = self.charts.iter_mut().find(|c| c.id == chart_id) {
                                match action {
                                    Action::Add(kind) => {
                                        kind.set_applied(&mut chart.indicators, true)
                                    }
                                    Action::Remove(kind) => {
                                        kind.set_applied(&mut chart.indicators, false)
                                    }
                                    Action::EmaPeriod(period) => {
                                        chart.indicators.ema_period = period
                                    }
                                    Action::ToggleVisible(Kind::BuiltIn(kind)) => {
                                        kind.toggle_visible(&mut chart.indicators);
                                    }
                                    Action::ToggleVisible(Kind::Script(id)) => {
                                        if let Some(instance) =
                                            chart.chart.pine_instances().iter().find(|i| i.id == id)
                                        {
                                            chart.chart.set_pine_visible(id, !instance.visible);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
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

        self.add_chart.tick(&ctx);

        ctx.request_repaint_after(Duration::from_millis(250));

        let mut actions = egui::Panel::top("workspace_toolbar")
            .exact_size(workspace_toolbar::HEIGHT)
            .frame(egui::Frame::new().fill(ui::theme::chrome()))
            .show(ui, |ui| self.show_toolbar(ui, &ctx))
            .inner;

        if self.replay.is_bar_visible() {
            egui::Panel::top("replay_bar")
                .exact_size(replay_bar::preferred_height(
                    self.replay.status,
                    ui.available_width(),
                    false,
                ))
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.show_replay_bar(ui));
        }

        let rail_actions = egui::Panel::left("tool_rail")
            .exact_size(tool_sidebar::WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(ui::theme::chrome()))
            .show(ui, |ui| self.show_tool_rail(ui))
            .inner;
        actions.extend(rail_actions);

        if self.watchlist_visible {
            let minimum_width = self
                .watchlist
                .lock()
                .ok()
                .map(|store| self.watchlist_sidebar.minimum_width(store.lists()))
                .unwrap_or(286.0);
            egui::Panel::right("watchlist")
                .default_size(minimum_width.max(270.0))
                .min_size(minimum_width)
                .max_size(640.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::chrome())
                        .inner_margin(egui::Margin::ZERO)
                        .stroke(egui::Stroke::new(1.0, ui::theme::border())),
                )
                .show(ui, |ui| self.show_watchlist(ui, &ctx));
        }

        if self.paper_panel_visible {
            let panel_actions = egui::Panel::bottom(egui::Id::new(("paper_panel", self.id)))
                .resizable(true)
                .default_size(340.0)
                .min_size(280.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::kit::window())
                        .stroke(egui::Stroke::new(1.0, ui::theme::separator())),
                )
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
                    .fill(ui::theme::background())
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| self.render_grid(ui, paper_host));
        if let Some(asset) = self.new_alert_request.take() {
            actions.push(WorkspaceAction::NewAlert(asset));
        }
        if let Some(item) = self.open_new_tab_request.take() {
            actions.push(WorkspaceAction::OpenWatchlistInNewTab(item));
        }

        match self.add_chart.show(&ctx) {
            Some(AddChartAction::Add(result)) => {
                if let Some(list_id) = self
                    .add_to_watchlist
                    .filter(|_| self.add_chart.is_watchlist_mode())
                {
                    if let Ok(mut store) = self.watchlist.lock() {
                        store.add_to_list(
                            list_id,
                            WatchlistInstrument::from_market(
                                result.source,
                                &result.symbol,
                                &result.full_symbol,
                            ),
                        );
                    }
                } else {
                    self.add_from_search(&result, &ctx);
                }
            }
            Some(AddChartAction::OpenAlpacaSettings) => {
                actions.push(WorkspaceAction::OpenAlpacaSettings);
            }
            None => {}
        }
        if !self.add_chart.is_open() {
            self.add_to_watchlist = None;
        }
        // Before the sheet, so the popover takes an Esc the sheet would otherwise act on.
        self.show_script_popover(&ctx);
        if self.settings_chart.is_some() {
            actions.extend(self.show_settings_window(&ctx));
        }
        if let Some(chart_id) = self.removing_chart {
            self.confirm_chart_removal(&ctx, chart_id);
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

    #[test]
    fn live_candle_wins_over_an_older_rest_snapshot() {
        let fetched = vec![candle(1, 10.0), candle(2, 20.0)];
        let live = vec![candle(1, 10.0), candle(2, 25.0), candle(3, 30.0)];
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, Some(2)), live);
        assert_eq!(merge_fetched_candles(fetched.clone(), &live, None), fetched);
    }

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
                    appearance: crate::model::ChartAppearance::default(),
                    drawings: Vec::new(),
                    pine_source: None,
                    pine_instances: Vec::new(),
                    candle_count: 0,
                    title: None,
                    pm_series: Vec::new(),
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
                    appearance: crate::model::ChartAppearance {
                        bullish: [12, 34, 56],
                        bearish: [210, 87, 65],
                        axis_decimals: Some(3),
                    },
                    drawings: Vec::new(),
                    pine_source: None,
                    pine_instances: vec![crate::model::pine_instance::PineInstance::new(
                        1,
                        Some("s".into()),
                        "RSI",
                        "plot(close)",
                    )],
                    candle_count: 0,
                    title: None,
                    pm_series: Vec::new(),
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
            appearance: crate::model::ChartAppearance::default(),
            drawings: Vec::new(),
            pine_source: None,
            pine_instances: Vec::new(),
            candle_count: 0,
            title: None,
            pm_series: Vec::new(),
        })
        .expect("serialize chart");
        let mut row: serde_json::Value = serde_json::from_str(&encoded).expect("json");
        row.as_object_mut().unwrap().remove("pine_source");
        row.as_object_mut().unwrap().remove("appearance");
        let decoded: PersistedChart = serde_json::from_value(row).expect("deserialize old row");
        assert_eq!(decoded.pine_source, None);
        assert_eq!(decoded.appearance, crate::model::ChartAppearance::default());
    }

    #[test]
    fn a_multi_choice_fetch_reads_candles_from_the_first_choice_with_history() {
        let bar = |time: i64| Kline {
            open_time: time,
            close_time: time + 1,
            open: 0.5,
            high: 0.5,
            low: 0.5,
            close: 0.5,
            volume: 0.0,
        };
        let fetched = Fetched::from_choices(vec![
            ("a".into(), Vec::new()),
            ("b".into(), vec![bar(1), bar(2)]),
            ("c".into(), vec![bar(3)]),
        ])
        .expect("some history");
        assert_eq!(fetched.candles.len(), 2);
        assert_eq!(fetched.series.len(), 3);
        let none = Fetched::from_choices(vec![("a".into(), Vec::new())]);
        assert!(none.is_err());
    }

    #[test]
    fn a_prediction_chart_keeps_its_title_and_choices_through_a_save() {
        let mut saved = PersistedChart {
            id: 1,
            symbol: "111".into(),
            source: DataSource::Polymarket,
            log_scale: false,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance: crate::model::ChartAppearance::default(),
            drawings: Vec::new(),
            pine_source: None,
            pine_instances: Vec::new(),
            candle_count: 0,
            title: Some("Fed decision".into()),
            pm_series: vec![
                PmSeries {
                    token_id: "111".into(),
                    label: "Cut".into(),
                },
                PmSeries {
                    token_id: "222".into(),
                    label: "Hold".into(),
                },
            ],
        };
        let encoded = serde_json::to_value(&saved).expect("serialize");
        assert_eq!(encoded["pm_series"][0]["tokenID"], "111");
        let decoded: PersistedChart = serde_json::from_value(encoded.clone()).expect("round trip");
        assert_eq!(decoded.title.as_deref(), Some("Fed decision"));
        assert_eq!(decoded.pm_series, saved.pm_series);
        // A row from before prediction titles decodes with neither.
        let mut old = encoded;
        old.as_object_mut().unwrap().remove("title");
        old.as_object_mut().unwrap().remove("pm_series");
        saved.title = None;
        saved.pm_series.clear();
        let decoded: PersistedChart = serde_json::from_value(old).expect("old row");
        assert_eq!(decoded.title, None);
        assert!(decoded.pm_series.is_empty());
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

    fn snapshot(log_scale: bool) -> LayoutSnapshot {
        LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![crate::model::ChartConfig {
                id: 1,
                symbol: "BTCUSDT".into(),
                source: DataSource::Binance,
                log_scale,
                appearance: crate::model::ChartAppearance::default(),
            }],
            columns: vec![vec![1]],
        }
    }

    #[test]
    fn autosave_waits_for_the_layout_to_sit_still_then_saves_once() {
        let mut state = AutosaveState::default();
        let start = Instant::now();
        assert_eq!(
            state.step(true, true, || snapshot(false), start),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // An edit one second in restarts the countdown.
        let edited = start + Duration::from_secs(1);
        assert_eq!(
            state.step(true, true, || snapshot(true), edited),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
        // Unchanged since, the layout is due once the delay has passed.
        let due = edited + AUTOSAVE_DELAY;
        assert_eq!(
            state.step(
                true,
                true,
                || snapshot(true),
                due - Duration::from_millis(1)
            ),
            AutosaveStep::Wait(Duration::from_millis(1))
        );
        assert_eq!(
            state.step(true, true, || snapshot(true), due),
            AutosaveStep::Save
        );
        // Tried once: a failed write is not retried every frame, only after the next edit.
        assert_eq!(
            state.step(true, true, || snapshot(true), due + Duration::from_secs(5)),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, true, || snapshot(false), due + Duration::from_secs(6)),
            AutosaveStep::Wait(AUTOSAVE_DELAY)
        );
    }

    #[test]
    fn autosave_stays_idle_for_clean_layouts_and_layouts_with_it_off() {
        let mut state = AutosaveState::default();
        let now = Instant::now();
        assert_eq!(
            state.step(false, true, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert_eq!(
            state.step(true, false, || snapshot(true), now),
            AutosaveStep::Nothing
        );
        assert!(state.armed.is_none() && state.due.is_none());
        // Going clean (saved by hand) drops a pending countdown.
        state.step(true, true, || snapshot(true), now);
        assert!(state.due.is_some());
        state.step(false, true, || snapshot(true), now);
        assert!(state.due.is_none());
    }
}
