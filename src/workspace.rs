use crate::model::{
    ChartColumn, ChartConfig, ChartLayout, DataSource, IndicatorConfig, InstrumentId, Kline,
    LayoutSnapshot, Timeframe,
};
use crate::service::live::BinanceLiveFeed;
use crate::service::{self, SearchResult};
use crate::ui::{self, ChartCard, ChartCardAction, CrosshairInfo};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CANDLE_LIMIT: u32 = 500;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

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

/// What a saved workspace holds for one chart.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedChart {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub log_scale: bool,
    pub y_zoom: f64,
    pub indicators: IndicatorConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedWorkspace {
    pub id: u64,
    pub timeframe: Timeframe,
    pub watchlist_visible: bool,
    pub charts: Vec<PersistedChart>,
    pub columns: Vec<ChartColumn>,
    pub focused: Option<u64>,
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
        };
        chart.fetch(timeframe, ctx);
        chart
    }

    fn restored(saved: PersistedChart, timeframe: Timeframe, ctx: &egui::Context) -> Self {
        let mut chart = Self::new(saved.id, saved.symbol, saved.source, timeframe, ctx);
        chart.log_scale = saved.log_scale;
        chart.chart.set_y_zoom(saved.y_zoom);
        chart.indicators = saved.indicators;
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
}

impl ChartWorkspace {
    pub fn new(
        id: u64,
        symbol: impl Into<String>,
        timeframe: Timeframe,
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

    pub fn restored(saved: PersistedWorkspace, ctx: &egui::Context) -> Self {
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

    fn open_watchlist(&mut self, symbol: &str, ctx: &egui::Context) {
        let instrument = InstrumentId::new(DataSource::Binance, symbol);
        if let Some(existing) = self.charts.iter().position(|chart| {
            InstrumentId::new(chart.source, &chart.symbol).is_same_market(&instrument)
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

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
                ui.toggle_value(&mut self.watchlist_visible, "Watchlist");
                let layout_label = if self.is_dirty() {
                    "My Chart *"
                } else {
                    "My Chart"
                };
                if ui
                    .button(format!("{layout_label}  v"))
                    .on_hover_text("Saved layout (named views arrive in Stage 6)")
                    .clicked()
                {
                    self.mark_clean();
                }
                if self.charts.iter().any(|chart| chart.loading) {
                    ui.spinner();
                }
            });
        });
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            for (glyph, hint) in [
                ("+", "Add chart"),
                ("/", "Trend line (Stage 5)"),
                ("Fib", "Fibonacci retracement (Stage 5)"),
                ("Pen", "Brush (Stage 5)"),
                ("Rul", "Ruler (Stage 5)"),
            ] {
                if ui
                    .add_sized([32.0, 32.0], egui::Button::new(glyph).frame(false))
                    .on_hover_text(hint)
                    .clicked()
                    && glyph == "+"
                {
                    self.show_search = true;
                }
                ui.add_space(4.0);
            }
        });
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("v  Crypto").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("...");
                ui.label("Search");
                ui.label("+");
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Symbol").small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Chg%").small().weak());
                ui.add_space(26.0);
                ui.label(egui::RichText::new("Last").small().weak());
            });
        });

        let btc = self.charts.iter().find(|chart| chart.symbol == "BTCUSDT");
        let latest = btc.and_then(|chart| chart.candles.last());
        let first = btc.and_then(|chart| chart.candles.first());
        let change = latest
            .zip(first)
            .map(|(latest, first)| (latest.close / first.open - 1.0) * 100.0);
        let last = latest.map_or("—".into(), |candle| crate::format::price(candle.close));
        let change_text = change.map_or("—".into(), |value| format!("{value:+.2}%"));
        let change_color = if change.unwrap_or_default() >= 0.0 {
            ui::theme::UP
        } else {
            ui::theme::DOWN
        };

        let btc_row = egui::Frame::new()
            .fill(ui::theme::SURFACE_SELECTED)
            .corner_radius(5.0)
            .inner_margin(egui::Margin::symmetric(7, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("B")
                            .color(ui::theme::BITCOIN)
                            .strong()
                            .size(17.0),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("BTC/USDT").strong().size(12.0));
                        ui.label(egui::RichText::new("Binance").weak().small());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(change_text)
                                .color(change_color)
                                .strong(),
                        );
                        ui.label(egui::RichText::new(last).monospace());
                    });
                });
            });
        if btc_row.response.interact(egui::Sense::click()).clicked() {
            self.open_watchlist("BTCUSDT", ctx);
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Meme Coins").small().strong().weak());
        for (symbol, source, price, change) in [
            ("DOGE/USDT", "Binance", "0.2478", "−1.93%"),
            ("PENGU/USDT", "Binance", "0.008525", "−0.35%"),
            ("PEPE/USDT", "Binance", "0.000010", "+0.06%"),
        ] {
            ui.add_space(4.0);
            let row = ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(symbol).strong().size(11.0));
                    ui.label(egui::RichText::new(source).weak().small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if change.starts_with('+') {
                        ui::theme::UP
                    } else {
                        ui::theme::DOWN
                    };
                    ui.label(egui::RichText::new(change).color(color).size(11.0));
                    ui.label(egui::RichText::new(price).monospace().size(11.0));
                });
            });
            if row.response.interact(egui::Sense::click()).clicked() {
                self.open_watchlist(symbol, ctx);
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.label(
                egui::RichText::new("Stage 3 · dashboard composition")
                    .small()
                    .weak(),
            );
        });
    }

    // MARK: Grid

    fn render_grid(&mut self, ui: &mut egui::Ui) {
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
                                        card.show(ui, |ui| {
                                            let mut action = ChartCardAction::default();
                                            chart.chart.header(
                                                ui,
                                                &chart.symbol,
                                                chart.source,
                                                timeframe,
                                                &chart.candles,
                                                chart.loading,
                                                &mut action,
                                                chart_id,
                                            );
                                            ui.separator();
                                            let pointer = chart.chart.body(
                                                ui,
                                                &chart.symbol,
                                                &chart.candles,
                                                chart.loading,
                                                chart.error.as_deref(),
                                                chart.log_scale,
                                                chart.source.is_line_chart(),
                                                &chart.indicators,
                                                crosshair.as_ref(),
                                                chart_id,
                                            );
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
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let timeframe = self.timeframe;
        for chart in &mut self.charts {
            chart.receive_fetch();
            if chart.inbox.is_none() && chart.last_fetch.elapsed() >= REFRESH_INTERVAL {
                chart.fetch(timeframe, &ctx);
            }
        }

        self.update_live_feed(&ctx);

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

        egui::Panel::top("workspace_toolbar")
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
            )
            .show(ui, |ui| self.show_toolbar(ui, &ctx));

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

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::BACKGROUND)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| self.render_grid(ui));

        if self.show_search {
            self.show_search_window(&ctx);
        }
        if self.settings_chart.is_some() {
            self.show_settings_window(&ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
