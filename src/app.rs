use crate::model::Timeframe;
use crate::native_bridge::{self, NativeCommand};
use crate::workspace::{ChartWorkspace, PersistedWorkspace};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SESSION_KEY: &str = "degenview.tab_session.v3";

struct TabSlot {
    workspace: Arc<Mutex<ChartWorkspace>>,
    open: Arc<AtomicBool>,
}

impl TabSlot {
    fn new(workspace: ChartWorkspace) -> Self {
        Self {
            workspace: Arc::new(Mutex::new(workspace)),
            open: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct PersistedSession {
    next_id: u64,
    tabs: Vec<PersistedWorkspace>,
}

pub struct DegenViewApp {
    tabs: Vec<TabSlot>,
    next_id: u64,
}

impl DegenViewApp {
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        crate::ui::theme::install(&creation.egui_ctx);
        native_bridge::install(&creation.egui_ctx);

        let restored = creation
            .storage
            .and_then(|storage| eframe::get_value::<PersistedSession>(storage, SESSION_KEY));
        let (tabs, next_id) = match restored {
            Some(session) if !session.tabs.is_empty() => {
                let tabs = session
                    .tabs
                    .into_iter()
                    .map(|saved| TabSlot::new(ChartWorkspace::restored(saved, &creation.egui_ctx)))
                    .collect();
                (tabs, session.next_id.max(2))
            }
            _ => (
                vec![TabSlot::new(ChartWorkspace::new(
                    1,
                    "BTCUSDT",
                    Timeframe::Day,
                    &creation.egui_ctx,
                ))],
                2,
            ),
        };

        native_bridge::expect_new_windows(tabs.len());
        Self { tabs, next_id }
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
        self.tabs.push(TabSlot::new(ChartWorkspace::new(
            id, symbol, timeframe, ctx,
        )));
    }

    fn process_commands(&mut self, ctx: &egui::Context) {
        for command in native_bridge::take_commands() {
            match command {
                NativeCommand::NewTab => self.add_tab(ctx),
            }
        }
        self.tabs.retain(|tab| tab.open.load(Ordering::Acquire));
    }

    fn sync_viewports(&self, ctx: &egui::Context) {
        for tab in &self.tabs {
            if !tab.open.load(Ordering::Acquire) {
                continue;
            }
            let workspace = Arc::clone(&tab.workspace);
            let open = Arc::clone(&tab.open);
            let (id, title) = {
                let workspace = workspace.lock().expect("workspace lock poisoned");
                (workspace.id(), workspace.title())
            };
            let builder = egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([1440.0, 900.0])
                .with_min_inner_size([960.0, 620.0]);

            ctx.show_viewport_deferred(
                egui::ViewportId::from_hash_of(("chart-tab", id)),
                builder,
                move |ui, _class| {
                    if ui.input(|input| input.viewport().close_requested()) {
                        open.store(false, Ordering::Release);
                        return;
                    }
                    if let Ok(mut workspace) = workspace.lock() {
                        let title = workspace.title();
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Title(title));
                        workspace.ui(ui);
                    }
                    native_bridge::reconcile_tabs();
                },
            );
        }
    }

    fn persisted_session(&self) -> PersistedSession {
        let tabs = self
            .tabs
            .iter()
            .filter(|tab| tab.open.load(Ordering::Acquire))
            .filter_map(|tab| {
                tab.workspace
                    .lock()
                    .ok()
                    .map(|workspace| workspace.persisted())
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
        native_bridge::reconcile_tabs();
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_commands(&ctx);
        self.sync_viewports(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SESSION_KEY, &self.persisted_session());
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
            tabs: vec![PersistedWorkspace {
                id: 2,
                timeframe: Timeframe::Hour,
                watchlist_visible: true,
                charts: vec![PersistedChart {
                    id: 1,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                }],
                columns: vec![ChartColumn::new(10, vec![1])],
                focused: Some(1),
            }],
        };
        let encoded = serde_json::to_string(&session).expect("serialize session");
        let decoded: PersistedSession =
            serde_json::from_str(&encoded).expect("deserialize session");
        assert_eq!(decoded, session);
    }
}
