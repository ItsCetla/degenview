use crate::alert_host::AlertHost;
use crate::model::Timeframe;
use crate::model::alert::{
    AlertAsset, AlertCurrency, AlertRuntimeCommand, AlertRuntimeCommandPayload, AlertState,
};
use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
use crate::model::watchlist::new_id;
use crate::model::watchlist_quote::Freshness;
use crate::model::webhook::WebhookDeliveryState;
use crate::native_bridge::{self, NativeCommand, TabIcon};
use crate::paper_host::PaperHost;
use crate::pine_subscription_host::PineSubscriptionHost;
use crate::portfolio_host::{PortfolioHost, PortfolioTab};
use crate::script_manager::store::hash_source;
use crate::script_manager::{ScriptManager, ScriptManagerAction};
use crate::service::fx_rates::FxRateService;
use crate::service::macos_notifications::{self, Notifier};
use crate::service::portfolio_quotes::PortfolioQuoteFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
use crate::ui::alert_banner::{self, AlertBannerState};
use crate::ui::alert_center::{
    self, AlertCenterAction, AlertCenterInput, AlertCenterState, QuoteSummary,
};
use crate::ui::alert_editor::{self, PriceAlertEditorOutcome, PriceAlertEditorState};
use crate::ui::pine_alert_editor::{self, PineAlertWebhooksOutcome, PineAlertWebhooksState};
use crate::ui::script_alert_list::ScriptAlertRow;
use crate::ui::settings_window;
use crate::webhook_host::{WebhookHost, WebhookHostAction};
use crate::workspace::{ChartWorkspace, PersistedWorkspace, WorkspaceAction};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

enum TabContent {
    Chart(Box<ChartWorkspace>),
    Portfolio(Box<PortfolioTab>),
    Pine(Box<ScriptManager>),
}

struct TabSlot {
    content: Arc<Mutex<TabContent>>,
    open: Arc<AtomicBool>,
}

impl TabSlot {
    fn new(content: TabContent) -> Self {
        Self {
            content: Arc::new(Mutex::new(content)),
            open: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistedSpecialTabKind {
    Portfolio,
    Pine,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedSpecialTab {
    id: u64,
    kind: PersistedSpecialTabKind,
}

/// Untagged keeps every existing chart-tab row byte-compatible. Special tabs have a required
/// `kind`, so they cannot be mistaken for the older `PersistedWorkspace` shape.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
enum PersistedTab {
    Chart(PersistedWorkspace),
    Special(PersistedSpecialTab),
}

impl PersistedTab {
    fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id,
            Self::Special(tab) => tab.id,
        }
    }
}

impl crate::store::Document for PersistedTab {
    fn document_id(&self) -> String {
        self.id().to_string()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AppAction {
    OpenPortfolio,
    OpenPine,
    /// Open Settings on the Alpaca page.
    OpenAlpacaSettings,
    OpenAlerts,
    /// Open the price-alert editor on this market (the chart header's bell).
    NewAlert(AlertAsset),
    /// Apply a Pine script to the focused chart (the Script Manager's "Add to Chart").
    SetPineScript(String),
    /// Create a Pine alert subscription on the focused chart (the Script Manager's "Create Alert").
    CreatePineAlert {
        script_name: String,
        source: String,
        note: String,
        webhook_endpoint_ids: Vec<u64>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedSession {
    next_id: u64,
    tabs: Vec<PersistedTab>,
}

pub struct DegenViewApp {
    tabs: Vec<TabSlot>,
    next_id: u64,
    store: SharedStore,
    watchlist: Arc<Mutex<WatchlistStore>>,
    quote_book: WatchlistQuoteBook,
    quote_coordinator: WatchlistQuoteCoordinator,
    portfolio_quotes: PortfolioQuoteFeed,
    fx_rates: Arc<FxRateService>,
    portfolio: Arc<Mutex<PortfolioHost>>,
    paper: Arc<Mutex<PaperHost>>,
    webhook_host: Arc<Mutex<WebhookHost>>,
    alert_host: Arc<Mutex<AlertHost>>,
    pine_host: Arc<Mutex<PineSubscriptionHost>>,
    last_alert_revision: u64,
    pine_last_bar: std::collections::HashMap<String, i64>,
    alerts_open: Arc<AtomicBool>,
    alert_editor: Arc<Mutex<Option<PriceAlertEditorState>>>,
    /// The Alerts window's tab, search and dialogs, kept across frames.
    alert_center_state: Arc<Mutex<AlertCenterState>>,
    /// `chart_id → (symbol_key, timeframe)` of every open chart, refreshed each frame for the
    /// Alerts window's script-alert status.
    chart_datasets: Arc<Mutex<std::collections::HashMap<u64, (String, String)>>>,
    /// The Webhooks… sheet open on a script alert, by subscription id.
    script_webhooks: Arc<Mutex<Option<(u64, PineAlertWebhooksState)>>>,
    banner: AlertBannerState,
    notifier: Arc<dyn Notifier>,
    actions: Arc<Mutex<Vec<AppAction>>>,
}

impl DegenViewApp {
    pub fn new(creation: &eframe::CreationContext<'_>, store: SharedStore) -> Self {
        crate::ui::theme::install(&creation.egui_ctx);
        egui_extras::install_image_loaders(&creation.egui_ctx);
        native_bridge::install(&creation.egui_ctx);

        let watchlist = Arc::new(Mutex::new(WatchlistStore::new(store.clone())));
        let quote_book = WatchlistQuoteBook::default();
        let repaint_ctx = creation.egui_ctx.clone();
        let quote_coordinator = WatchlistQuoteCoordinator::new(
            quote_book.clone(),
            Arc::new(move || repaint_ctx.request_repaint()),
        );
        let application_support = std::env::var_os("HOME").map(|home| {
            std::path::PathBuf::from(home).join("Library/Application Support/com.degenview.rust")
        });
        let fx_rates = Arc::new(FxRateService::live(application_support));
        let portfolio_repaint = creation.egui_ctx.clone();
        let portfolio_quotes = PortfolioQuoteFeed::new(
            quote_book.clone(),
            quote_coordinator.clone(),
            fx_rates.clone(),
            Arc::new(move || portfolio_repaint.request_repaint()),
        );

        {
            let store = store.lock().expect("store lock poisoned");
            let summary = crate::store::import::run_once(&store, None);
            if summary.imported > 0 || summary.skipped > 0 {
                eprintln!(
                    "[import] imported {} tabs, skipped {}",
                    summary.imported, summary.skipped
                );
            }
        }

        let portfolio = Arc::new(Mutex::new(PortfolioHost::load(store.clone())));
        let paper = Arc::new(Mutex::new(PaperHost::load(store.clone())));
        let webhook_host = Arc::new(Mutex::new(WebhookHost::default()));
        let alert_host = Arc::new(Mutex::new({
            let store = store.lock().expect("store lock poisoned");
            AlertHost::load(&store, now_ms())
        }));
        let pine_host = Arc::new(Mutex::new({
            let store = store.lock().expect("store lock poisoned");
            PineSubscriptionHost::load(&store)
        }));
        let actions = Arc::new(Mutex::new(Vec::new()));
        let alerts_open = Arc::new(AtomicBool::new(false));
        let alert_editor = Arc::new(Mutex::new(None));
        let banner = AlertBannerState::default();
        let notifier = macos_notifications::default_notifier();

        let (saved_tabs, saved_next_id) = {
            let store = store.lock().expect("store lock poisoned");
            (
                store.documents::<PersistedTab>("tab"),
                store.setting::<u64>("session.next_id").unwrap_or(2),
            )
        };

        let (tabs, next_id) = if saved_tabs.is_empty() {
            (
                vec![TabSlot::new(TabContent::Chart(Box::new(
                    ChartWorkspace::new(
                        1,
                        "BTCUSDT",
                        Timeframe::Day,
                        Arc::clone(&watchlist),
                        quote_book.clone(),
                        quote_coordinator.clone(),
                        store.clone(),
                        &creation.egui_ctx,
                    ),
                )))],
                2,
            )
        } else {
            let mut has_portfolio = false;
            let mut has_pine = false;
            let tabs = saved_tabs
                .into_iter()
                .filter_map(|saved| match saved {
                    PersistedTab::Chart(workspace) => Some(TabSlot::new(TabContent::Chart(
                        Box::new(ChartWorkspace::restored(
                            workspace,
                            Arc::clone(&watchlist),
                            quote_book.clone(),
                            quote_coordinator.clone(),
                            store.clone(),
                            &creation.egui_ctx,
                        )),
                    ))),
                    PersistedTab::Special(PersistedSpecialTab {
                        id,
                        kind: PersistedSpecialTabKind::Portfolio,
                    }) if !has_portfolio => {
                        has_portfolio = true;
                        Some(TabSlot::new(TabContent::Portfolio(Box::new(
                            PortfolioTab::new(id, portfolio_quotes.clone()),
                        ))))
                    }
                    PersistedTab::Special(PersistedSpecialTab {
                        id,
                        kind: PersistedSpecialTabKind::Pine,
                    }) if !has_pine => {
                        has_pine = true;
                        Some(TabSlot::new(TabContent::Pine(Box::new(
                            ScriptManager::new(id, store.clone()),
                        ))))
                    }
                    PersistedTab::Special(_) => None,
                })
                .collect();
            (tabs, saved_next_id.max(2))
        };

        native_bridge::expect_new_windows(tabs.len());
        Self {
            tabs,
            next_id,
            store,
            watchlist,
            quote_book,
            quote_coordinator,
            portfolio_quotes,
            fx_rates,
            portfolio,
            paper,
            webhook_host,
            alert_host,
            pine_host,
            alert_center_state: Arc::new(Mutex::new(AlertCenterState::default())),
            chart_datasets: Arc::new(Mutex::new(std::collections::HashMap::new())),
            script_webhooks: Arc::new(Mutex::new(None)),
            last_alert_revision: 0,
            pine_last_bar: std::collections::HashMap::new(),
            alerts_open,
            alert_editor,
            banner,
            notifier,
            actions,
        }
    }

    fn add_tab(&mut self, ctx: &egui::Context) {
        let id = self.next_id;
        self.next_id += 1;
        let (symbol, timeframe) = if id.is_multiple_of(2) {
            ("ETHUSDT", Timeframe::Hour)
        } else {
            ("BTCUSDT", Timeframe::Day)
        };
        native_bridge::expect_new_windows(1);
        self.tabs.push(TabSlot::new(TabContent::Chart(Box::new(
            ChartWorkspace::new(
                id,
                symbol,
                timeframe,
                Arc::clone(&self.watchlist),
                self.quote_book.clone(),
                self.quote_coordinator.clone(),
                self.store.clone(),
                ctx,
            ),
        ))));
    }

    fn process_commands(&mut self, ctx: &egui::Context) {
        for command in native_bridge::take_commands() {
            match command {
                NativeCommand::NewTab => self.add_tab(ctx),
                NativeCommand::OpenSettings => {
                    if let Ok(mut host) = self.webhook_host.lock() {
                        host.set_open(true);
                    }
                }
                NativeCommand::SaveCurrentLayout => {
                    for tab in &self.tabs {
                        if let Ok(mut content) = tab.content.lock()
                            && let TabContent::Chart(workspace) = &mut *content
                        {
                            workspace.save_layout(ctx);
                            break;
                        }
                    }
                }
            }
        }
        let actions = self
            .actions
            .lock()
            .map(|mut actions| std::mem::take(&mut *actions))
            .unwrap_or_default();
        for action in actions {
            match action {
                AppAction::OpenPortfolio => self.open_portfolio(ctx),
                AppAction::OpenPine => self.open_pine(ctx),
                AppAction::OpenAlpacaSettings => {
                    if let Ok(mut host) = self.webhook_host.lock() {
                        host.open_alpaca();
                    }
                }
                AppAction::OpenAlerts => self.alerts_open.store(true, Ordering::Release),
                AppAction::NewAlert(asset) => {
                    if let Ok(mut slot) = self.alert_editor.lock() {
                        *slot = Some(PriceAlertEditorState::new(asset, None, None));
                    }
                    self.alerts_open.store(true, Ordering::Release);
                }
                AppAction::SetPineScript(source) => {
                    for tab in &self.tabs {
                        if let Ok(mut content) = tab.content.lock()
                            && let TabContent::Chart(workspace) = &mut *content
                        {
                            workspace.set_pine_on_focused(source.clone());
                            break;
                        }
                    }
                }
                AppAction::CreatePineAlert {
                    script_name,
                    source,
                    note,
                    webhook_endpoint_ids,
                } => {
                    if let Some(target) = self.focused_pine_target() {
                        let source_hash = hash_source(&source);
                        let subscription = PineAlertSubscription {
                            id: new_id(),
                            chart_id: target.chart_id,
                            instance_id: None,
                            script_id: None,
                            script_name,
                            source,
                            symbol_key: target.instrument.key(),
                            timeframe: target.timeframe.binance_interval().to_owned(),
                            source_hash,
                            note,
                            state: PineAlertState::Active,
                            created_at_ms: now_ms(),
                            webhook_endpoint_ids,
                        };
                        if let Ok(store) = self.store.lock() {
                            let mut subscriptions = store.pine_subscriptions();
                            subscriptions.push(subscription);
                            let _ = store.replace_pine_subscriptions(&subscriptions);
                            // The runtime host reads its own copy; without this a new alert
                            // would stay silent until the next launch.
                            if let Ok(mut host) = self.pine_host.lock() {
                                host.reload(&store);
                            }
                        }
                    }
                }
            }
        }
        self.tabs.retain(|tab| tab.open.load(Ordering::Acquire));
    }

    fn open_portfolio(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Portfolio(portfolio) => Some(portfolio.id()),
                TabContent::Chart(_) | TabContent::Pine(_) => None,
            }
        });
        if let Some(id) = existing {
            ctx.send_viewport_cmd_to(Self::viewport_id(id), egui::ViewportCommand::Focus);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        native_bridge::expect_new_windows(1);
        self.tabs.push(TabSlot::new(TabContent::Portfolio(Box::new(
            PortfolioTab::new(id, self.portfolio_quotes.clone()),
        ))));
    }

    fn open_pine(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Pine(pine) => Some(pine.id()),
                TabContent::Chart(_) | TabContent::Portfolio(_) => None,
            }
        });
        if let Some(id) = existing {
            ctx.send_viewport_cmd_to(Self::viewport_id(id), egui::ViewportCommand::Focus);
            return;
        }

        let id = self.next_id;
        self.next_id += 1;
        native_bridge::expect_new_windows(1);
        self.tabs.push(TabSlot::new(TabContent::Pine(Box::new(
            ScriptManager::new(id, self.store.clone()),
        ))));
    }

    fn viewport_id(id: u64) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("degenview-tab", id))
    }

    /// The first chart tab's focused chart identity, for the Pine integration.
    fn focused_pine_target(&self) -> Option<crate::workspace::PineChartTarget> {
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                return workspace.focused_pine_target();
            }
        }
        None
    }

    fn tick_alerts(&mut self) {
        let store = self.store.lock().expect("store lock poisoned");
        let now = now_ms();
        if let Ok(mut host) = self.alert_host.lock() {
            host.drain_commands(&store, now);
            let revision = self.quote_book.revision();
            if revision != self.last_alert_revision {
                self.last_alert_revision = revision;
                let events = host.evaluate_from_book(&self.quote_book, &self.fx_rates, now);
                if !events.is_empty() {
                    // TODO: run on a worker thread so a slow endpoint doesn't block the UI.
                    host.dispatch_webhooks(&events, &store, &self.store, now);
                    for event in &events {
                        self.banner.show_price(event, host.settings(), now);
                    }
                    host.dispatch_notifications(&*self.notifier, now);
                }
            }
            host.save_if_dirty(&store);
        }
    }

    /// Runs active Pine alert subscriptions over every open chart's candles and dispatches any that
    /// fire (webhooks + macOS notification via `dispatch_pine`). `pine_last_bar` gates each symbol
    /// on its last candle so `alert()` fires once per new bar, not every frame.
    fn tick_pine(&mut self) {
        let store = self.store.lock().expect("store lock poisoned");
        let now = now_ms();
        let mut fired = Vec::new();
        let mut datasets = std::collections::HashMap::new();
        for tab in &self.tabs {
            if let Ok(content) = tab.content.lock()
                && let TabContent::Chart(workspace) = &*content
            {
                for (chart_id, key, timeframe) in workspace.pine_datasets() {
                    datasets.insert(chart_id, (key, timeframe));
                }
            }
        }
        if let Ok(mut shared) = self.chart_datasets.lock() {
            *shared = datasets;
        }
        if let Ok(host) = self.pine_host.lock() {
            for tab in &self.tabs {
                if !tab.open.load(Ordering::Acquire) {
                    continue;
                }
                if let Ok(content) = tab.content.lock()
                    && let TabContent::Chart(workspace) = &*content
                {
                    for (symbol_key, candles) in workspace.charts_candles() {
                        let last_open = candles.last().map(|candle| candle.open_time);
                        if last_open == self.pine_last_bar.get(&symbol_key).copied() {
                            continue;
                        }
                        if let Some(open_time) = last_open {
                            self.pine_last_bar.insert(symbol_key.clone(), open_time);
                        }
                        fired.extend(host.run_active(&symbol_key, &candles));
                    }
                }
            }
        }
        if !fired.is_empty()
            && let Ok(host) = self.alert_host.lock()
        {
            host.dispatch_pine(&fired, &store, &self.store, &*self.notifier, now);
        }
    }

    fn sync_viewports(&self, ctx: &egui::Context) {
        // What a script alert would watch: the focused chart's market.
        let chart_info =
            self.focused_pine_target()
                .map(|target| crate::script_manager::ChartInfo {
                    chart_id: target.chart_id,
                    title: target
                        .instrument
                        .source
                        .display_symbol(&target.instrument.symbol),
                    source_name: target.instrument.source.display_name().to_owned(),
                    timeframe: target.timeframe.binance_interval().to_owned(),
                    symbol_key: target.instrument.key(),
                });
        let mut tab_icons = Vec::new();
        for tab in &self.tabs {
            if !tab.open.load(Ordering::Acquire) {
                continue;
            }
            let content = Arc::clone(&tab.content);
            let open = Arc::clone(&tab.open);
            let portfolio_host = Arc::clone(&self.portfolio);
            let paper_host = Arc::clone(&self.paper);
            let app_actions = Arc::clone(&self.actions);
            let chart_info = chart_info.clone();
            let (id, title, icon) = {
                let content = content.lock().expect("tab lock poisoned");
                match &*content {
                    TabContent::Chart(workspace) => {
                        (workspace.id(), workspace.title(), TabIcon::Charts)
                    }
                    TabContent::Portfolio(portfolio) => (
                        portfolio.id(),
                        portfolio.title().to_owned(),
                        TabIcon::Portfolio,
                    ),
                    TabContent::Pine(pine) => (pine.id(), pine.title(), TabIcon::ScriptManager),
                }
            };
            tab_icons.push((title.clone(), icon));
            let is_scripts = matches!(
                &*content.lock().expect("tab lock poisoned"),
                TabContent::Pine(_)
            );
            // The Script Manager opens at the Swift window's 900×600 and shrinks to what its
            // panes' minimums allow.
            let (size, min) = if is_scripts {
                ([900.0, 600.0], [620.0, 420.0])
            } else {
                ([1440.0, 900.0], [960.0, 620.0])
            };
            let builder = egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size(size)
                .with_min_inner_size(min);

            ctx.show_viewport_deferred(Self::viewport_id(id), builder, move |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    open.store(false, Ordering::Release);
                    return;
                }
                if let Ok(mut content) = content.lock() {
                    match &mut *content {
                        TabContent::Chart(workspace) => {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Title(workspace.title()));
                            let actions = paper_host
                                .lock()
                                .map(|mut host| workspace.ui(ui, &mut host))
                                .unwrap_or_default();
                            if !actions.is_empty()
                                && let Ok(mut pending) = app_actions.lock()
                            {
                                pending.extend(actions.into_iter().map(|action| match action {
                                    WorkspaceAction::OpenPortfolio => AppAction::OpenPortfolio,
                                    WorkspaceAction::OpenPineScripts => AppAction::OpenPine,
                                    WorkspaceAction::OpenAlerts => AppAction::OpenAlerts,
                                    WorkspaceAction::NewAlert(asset) => AppAction::NewAlert(asset),
                                }));
                            }
                        }
                        TabContent::Portfolio(portfolio) => {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
                                portfolio.title().to_owned(),
                            ));
                            if let Ok(mut host) = portfolio_host.lock() {
                                portfolio.ui(ui, &mut host);
                            }
                        }
                        TabContent::Pine(pine) => {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Title(pine.title()));
                            let pine_actions = pine.ui(ui, chart_info.clone());
                            if !pine_actions.is_empty()
                                && let Ok(mut pending) = app_actions.lock()
                            {
                                pending.extend(pine_actions.into_iter().map(
                                    |action| match action {
                                        ScriptManagerAction::AddToChart(source) => {
                                            AppAction::SetPineScript(source)
                                        }
                                        ScriptManagerAction::CreateAlert {
                                            script_name,
                                            source,
                                            note,
                                            webhook_endpoint_ids,
                                        } => AppAction::CreatePineAlert {
                                            script_name,
                                            source,
                                            note,
                                            webhook_endpoint_ids,
                                        },
                                        ScriptManagerAction::OpenAlpacaSettings => {
                                            AppAction::OpenAlpacaSettings
                                        }
                                    },
                                ));
                            }
                        }
                    }
                }
                native_bridge::reconcile_tabs();
            });
        }
        native_bridge::set_tab_icons(tab_icons);
    }

    fn sync_settings_viewport(&self, ctx: &egui::Context) {
        if !self
            .webhook_host
            .lock()
            .map(|host| host.is_open())
            .unwrap_or(false)
        {
            return;
        }
        let host = Arc::clone(&self.webhook_host);
        let alert_host = Arc::clone(&self.alert_host);
        let store = Arc::clone(&self.store);
        let builder = egui::ViewportBuilder::default()
            .with_title("Settings")
            .with_inner_size(settings_window::WINDOW_SIZE)
            .with_min_inner_size([520.0, 400.0]);
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("degenview-settings"),
            builder,
            move |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    if let Ok(mut host) = host.lock() {
                        host.set_open(false);
                    }
                    return;
                }
                let ctx = ui.ctx().clone();
                if let Ok(mut host) = host.lock() {
                    let store = store.lock().expect("store lock poisoned");
                    let (notifications, health) = {
                        let alert = alert_host.lock().expect("alert host lock poisoned");
                        (alert.settings().clone(), alert.health().clone())
                    };
                    let now = now_ms();
                    let actions = host.show(&ctx, &store, &notifications, &health, now, &|_| 0);
                    for action in actions {
                        if let WebhookHostAction::UpdateNotifications(settings) = action {
                            let command = AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::UpdateSettings(settings),
                                now,
                            );
                            let _ = store.enqueue_alert_command(&command);
                        }
                    }
                }
            },
        );
    }

    fn sync_alerts_viewport(&self, ctx: &egui::Context) {
        if !self.alerts_open.load(Ordering::Acquire) {
            return;
        }
        let alert_host = Arc::clone(&self.alert_host);
        let store = Arc::clone(&self.store);
        let quote_book = self.quote_book.clone();
        let webhook_host = Arc::clone(&self.webhook_host);
        let alerts_open = Arc::clone(&self.alerts_open);
        let alert_editor = Arc::clone(&self.alert_editor);
        let alert_center_state = Arc::clone(&self.alert_center_state);
        let pine_host = Arc::clone(&self.pine_host);
        let chart_datasets = Arc::clone(&self.chart_datasets);
        let script_webhooks = Arc::clone(&self.script_webhooks);
        let app_actions = Arc::clone(&self.actions);
        let builder = egui::ViewportBuilder::default()
            .with_title("Alerts")
            .with_inner_size([720.0, 560.0])
            .with_min_inner_size([520.0, 400.0]);
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("degenview-alerts"),
            builder,
            move |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    alerts_open.store(false, Ordering::Release);
                    return;
                }
                let store = store.lock().expect("store lock poisoned");
                let mut state = alert_center_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let script_alerts: Vec<ScriptAlertRow> = {
                    let datasets = chart_datasets
                        .lock()
                        .map(|datasets| datasets.clone())
                        .unwrap_or_default();
                    store
                        .pine_subscriptions()
                        .iter()
                        .map(|subscription| {
                            let dataset = datasets
                                .get(&subscription.chart_id)
                                .map(|(key, timeframe)| (key.as_str(), timeframe.as_str()));
                            ScriptAlertRow::from_subscription(subscription, dataset)
                        })
                        .collect()
                };
                let actions = if let Ok(host) = alert_host.lock() {
                    let snapshot = host.snapshot();
                    let input = AlertCenterInput {
                        alerts: &snapshot.alerts,
                        history: &snapshot.history,
                        health: &snapshot.health,
                        quote: &|key| {
                            let quote = quote_book.get_by_key(key)?;
                            Some(QuoteSummary {
                                price: quote.last?,
                                currency: AlertCurrency::Usd,
                                fresh: quote.freshness == Freshness::Live,
                            })
                        },
                        webhook_summary: &|event_id| {
                            let rows = store.webhook_deliveries(&[event_id]);
                            let rows = rows.get(&event_id)?;
                            let delivered = rows
                                .iter()
                                .filter(|row| row.state == WebhookDeliveryState::Delivered)
                                .count();
                            Some((delivered, rows.len()))
                        },
                        script_alerts: &script_alerts,
                        now_ms: now_ms(),
                        utc_offset_secs: 0,
                    };
                    alert_center::show(ui, &mut state, &input)
                } else {
                    Vec::new()
                };
                for action in actions {
                    match action {
                        AlertCenterAction::Delete(id) => {
                            let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::Delete(id),
                                now_ms(),
                            ));
                        }
                        AlertCenterAction::ClearHistory => {
                            let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                AlertRuntimeCommandPayload::ClearHistory,
                                now_ms(),
                            ));
                        }
                        AlertCenterAction::Toggle(id) => {
                            if let Ok(host) = alert_host.lock() {
                                let state = host
                                    .snapshot()
                                    .alerts
                                    .iter()
                                    .find(|alert| alert.id == id)
                                    .map(|alert| match alert.state {
                                        AlertState::Active => AlertState::Paused,
                                        _ => AlertState::Active,
                                    });
                                if let Some(state) = state {
                                    let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                        AlertRuntimeCommandPayload::SetState {
                                            id,
                                            state,
                                            baseline: None,
                                        },
                                        now_ms(),
                                    ));
                                }
                            }
                        }
                        AlertCenterAction::PauseScript(id)
                        | AlertCenterAction::RearmScript(id)
                        | AlertCenterAction::DeleteScript(id) => {
                            let mut subscriptions = store.pine_subscriptions();
                            match action {
                                AlertCenterAction::DeleteScript(_) => {
                                    subscriptions.retain(|s| s.id != id);
                                }
                                AlertCenterAction::PauseScript(_) => {
                                    for s in subscriptions.iter_mut().filter(|s| s.id == id) {
                                        s.state = PineAlertState::Paused;
                                    }
                                }
                                _ => {
                                    for s in subscriptions.iter_mut().filter(|s| s.id == id) {
                                        s.state = PineAlertState::Active;
                                    }
                                }
                            }
                            let _ = store.replace_pine_subscriptions(&subscriptions);
                            if let Ok(mut host) = pine_host.lock() {
                                host.reload(&store);
                            }
                        }
                        AlertCenterAction::EditScriptWebhooks(id) => {
                            if let Some(subscription) = store
                                .pine_subscriptions()
                                .into_iter()
                                .find(|subscription| subscription.id == id)
                                && let Ok(mut slot) = script_webhooks.lock()
                            {
                                *slot = Some((id, PineAlertWebhooksState::new(&subscription)));
                            }
                        }
                        AlertCenterAction::OpenScriptManager => {
                            if let Ok(mut pending) = app_actions.lock() {
                                pending.push(AppAction::OpenPine);
                            }
                        }
                        AlertCenterAction::OpenNotificationSettings => {
                            if let Ok(mut host) = webhook_host.lock() {
                                host.set_open(true);
                            }
                        }
                        AlertCenterAction::Edit(id) => {
                            if let Ok(host) = alert_host.lock()
                                && let Some(alert) =
                                    host.snapshot().alerts.iter().find(|alert| alert.id == id)
                            {
                                let editor = PriceAlertEditorState::new(
                                    alert.asset.clone(),
                                    Some(alert.clone()),
                                    Some(alert.currency),
                                );
                                if let Ok(mut slot) = alert_editor.lock() {
                                    *slot = Some(editor);
                                }
                            }
                        }
                        AlertCenterAction::Duplicate(id) => {
                            if let Ok(host) = alert_host.lock()
                                && let Some(alert) =
                                    host.snapshot().alerts.iter().find(|alert| alert.id == id)
                            {
                                // Fresh editor (new id) on the same asset.
                                let editor = PriceAlertEditorState::new(
                                    alert.asset.clone(),
                                    None,
                                    Some(alert.currency),
                                );
                                if let Ok(mut slot) = alert_editor.lock() {
                                    *slot = Some(editor);
                                }
                            }
                        }
                    }
                }
                // Webhooks… sheet of a script alert.
                if let Ok(mut slot) = script_webhooks.lock()
                    && let Some((id, sheet)) = slot.as_mut()
                {
                    let mut subscriptions = store.pine_subscriptions();
                    let outcome = subscriptions
                        .iter()
                        .find(|subscription| subscription.id == *id)
                        .and_then(|subscription| {
                            pine_alert_editor::show_webhooks(
                                ui.ctx(),
                                sheet,
                                subscription,
                                &store.webhook_endpoints(),
                            )
                        });
                    let id = *id;
                    match outcome {
                        Some(PineAlertWebhooksOutcome::Save(ids)) => {
                            for subscription in subscriptions.iter_mut().filter(|s| s.id == id) {
                                subscription.webhook_endpoint_ids = ids.clone();
                            }
                            let _ = store.replace_pine_subscriptions(&subscriptions);
                            if let Ok(mut host) = pine_host.lock() {
                                host.reload(&store);
                            }
                            *slot = None;
                        }
                        Some(PineAlertWebhooksOutcome::Cancel) => *slot = None,
                        None => {}
                    }
                }
                // Price-alert editor window.
                if let Ok(mut slot) = alert_editor.lock() {
                    let mut finished = false;
                    if let Some(state) = slot.as_mut() {
                        if let Some(_currency) = state.take_price_request() {
                            let price = quote_book
                                .get_by_key(&state.asset().key())
                                .and_then(|quote| quote.last);
                            state.set_current_price(price);
                        }
                        let endpoints = store.webhook_endpoints();
                        let now = now_ms();
                        if let Some(outcome) = alert_editor::show(ui.ctx(), state, &endpoints, now)
                        {
                            if let PriceAlertEditorOutcome::Save(alert) = outcome {
                                let baseline = state.current_price();
                                let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                                    AlertRuntimeCommandPayload::Save { alert, baseline },
                                    now,
                                ));
                            }
                            finished = true;
                        }
                    }
                    if finished {
                        *slot = None;
                    }
                }
            },
        );
    }

    fn persisted_session(&self) -> PersistedSession {
        let tabs = self
            .tabs
            .iter()
            .filter(|tab| tab.open.load(Ordering::Acquire))
            .filter_map(|tab| {
                let content = tab.content.lock().ok()?;
                Some(match &*content {
                    TabContent::Chart(workspace) => PersistedTab::Chart(workspace.persisted()),
                    TabContent::Portfolio(portfolio) => {
                        PersistedTab::Special(PersistedSpecialTab {
                            id: portfolio.id(),
                            kind: PersistedSpecialTabKind::Portfolio,
                        })
                    }
                    TabContent::Pine(pine) => PersistedTab::Special(PersistedSpecialTab {
                        id: pine.id(),
                        kind: PersistedSpecialTabKind::Pine,
                    }),
                })
            })
            .collect();
        PersistedSession {
            next_id: self.next_id,
            tabs,
        }
    }
}

impl eframe::App for DegenViewApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_commands(ctx);
        self.sync_viewports(ctx);
        self.sync_settings_viewport(ctx);
        self.sync_alerts_viewport(ctx);
        self.tick_alerts();
        // Runs once per frame (not in `ui`, which also fires for the root viewport).
        self.tick_pine();
        let now = now_ms();
        self.banner.expire(now);
        alert_banner::show(ctx, &mut self.banner, now);
        native_bridge::reconcile_tabs();
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_commands(&ctx);
        self.sync_viewports(&ctx);
        self.sync_settings_viewport(&ctx);
        self.sync_alerts_viewport(&ctx);
        self.tick_alerts();
        let now = now_ms();
        self.banner.expire(now);
        alert_banner::show(&ctx, &mut self.banner, now);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        let session = self.persisted_session();
        let store = self.store.lock().expect("store lock poisoned");
        let _ = store.replace_documents("tab", &session.tabs);
        let _ = store.set_setting("session.next_id", &session.next_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChartColumn, DataSource};
    use crate::workspace::PersistedChart;

    #[test]
    fn persisted_session_round_trips() {
        let session = PersistedSession {
            next_id: 3,
            tabs: vec![PersistedTab::Chart(PersistedWorkspace {
                id: 2,
                timeframe: Timeframe::Hour,
                watchlist_visible: true,
                charts: vec![PersistedChart {
                    id: 1,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                    y_zoom: 1.0,
                    indicators: crate::model::IndicatorConfig::default(),
                    drawings: Vec::new(),
                    pine_source: None,
                    candle_count: 0,
                }],
                columns: vec![ChartColumn::new(10, vec![1])],
                focused: Some(1),
            })],
        };
        let encoded = serde_json::to_string(&session).expect("serialize session");
        let decoded: PersistedSession =
            serde_json::from_str(&encoded).expect("deserialize session");
        assert_eq!(decoded, session);
    }

    #[test]
    fn old_chart_tab_shape_and_portfolio_tab_both_decode() {
        let old_chart = serde_json::json!({
            "id": 2,
            "timeframe": "Hour",
            "watchlist_visible": true,
            "charts": [],
            "columns": [],
            "focused": null
        });
        let portfolio = serde_json::json!({ "id": 9, "kind": "portfolio" });

        let chart: PersistedTab = serde_json::from_value(old_chart).expect("old chart tab");
        let special: PersistedTab = serde_json::from_value(portfolio).expect("portfolio tab");
        assert!(matches!(chart, PersistedTab::Chart(_)));
        assert_eq!(
            special,
            PersistedTab::Special(PersistedSpecialTab {
                id: 9,
                kind: PersistedSpecialTabKind::Portfolio,
            })
        );
    }
}
