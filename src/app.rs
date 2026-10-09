use crate::alert_host::AlertHost;
use crate::model::Timeframe;
use crate::model::alert::{AlertRuntimeCommand, AlertRuntimeCommandPayload};
use crate::native_bridge::{self, NativeCommand};
use crate::paper_host::PaperHost;
use crate::portfolio_host::{PortfolioHost, PortfolioTab};
use crate::service::fx_rates::FxRateService;
use crate::service::portfolio_quotes::PortfolioQuoteFeed;
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use crate::store::SharedStore;
use crate::store::watchlist::WatchlistStore;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppAction {
    OpenPortfolio,
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
    portfolio: Arc<Mutex<PortfolioHost>>,
    paper: Arc<Mutex<PaperHost>>,
    webhook_host: Arc<Mutex<WebhookHost>>,
    alert_host: Arc<Mutex<AlertHost>>,
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
            fx_rates,
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
        let actions = Arc::new(Mutex::new(Vec::new()));

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
                        &creation.egui_ctx,
                    ),
                )))],
                2,
            )
        } else {
            let mut has_portfolio = false;
            let tabs = saved_tabs
                .into_iter()
                .filter_map(|saved| match saved {
                    PersistedTab::Chart(workspace) => Some(TabSlot::new(TabContent::Chart(
                        Box::new(ChartWorkspace::restored(
                            workspace,
                            Arc::clone(&watchlist),
                            quote_book.clone(),
                            quote_coordinator.clone(),
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
            portfolio,
            paper,
            webhook_host,
            alert_host,
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
            }
        }
        self.tabs.retain(|tab| tab.open.load(Ordering::Acquire));
    }

    fn open_portfolio(&mut self, ctx: &egui::Context) {
        let existing = self.tabs.iter().find_map(|tab| {
            let content = tab.content.lock().ok()?;
            match &*content {
                TabContent::Portfolio(portfolio) => Some(portfolio.id()),
                TabContent::Chart(_) => None,
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

    fn viewport_id(id: u64) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("degenview-tab", id))
    }

    fn drain_alert_commands(&mut self) {
        let store = self.store.lock().expect("store lock poisoned");
        if let Ok(mut host) = self.alert_host.lock() {
            host.drain_commands(&store, now_ms());
            host.save_if_dirty(&store);
        }
    }

    fn sync_viewports(&self, ctx: &egui::Context) {
        for tab in &self.tabs {
            if !tab.open.load(Ordering::Acquire) {
                continue;
            }
            let content = Arc::clone(&tab.content);
            let open = Arc::clone(&tab.open);
            let portfolio_host = Arc::clone(&self.portfolio);
            let paper_host = Arc::clone(&self.paper);
            let app_actions = Arc::clone(&self.actions);
            let (id, title) = {
                let content = content.lock().expect("tab lock poisoned");
                match &*content {
                    TabContent::Chart(workspace) => (workspace.id(), workspace.title()),
                    TabContent::Portfolio(portfolio) => {
                        (portfolio.id(), portfolio.title().to_owned())
                    }
                }
            };
            let builder = egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1440.0, 900.0])
                .with_min_inner_size([960.0, 620.0]);

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
                    }
                }
                native_bridge::reconcile_tabs();
            });
        }
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
        self.drain_alert_commands();
        native_bridge::reconcile_tabs();
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_commands(&ctx);
        self.sync_viewports(&ctx);
        self.sync_settings_viewport(&ctx);
        self.drain_alert_commands();
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
