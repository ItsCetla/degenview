//! The Script Manager tab: an in-memory list of Pine scripts plus B's `ui::pine_editor`.
//!
//! This mirrors `PortfolioTab`'s role — it is a dedicated `TabContent` that owns per-window UI
//! state while the compile work stays here (there is no script store yet, so the list is
//! in-memory for now). `Save` marks the editor clean; `AddToChart` returns a
//! `PineTabAction::AddToChart(source)` the app routes to the focused chart's `set_pine_source`.
//! `CreateAlert` is left for the Pine subscription host (the Lead's slice).

use crate::pine::alert_call;
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::token::{Diagnostic, Limits};
use crate::ui::pine_editor::{self, PineEditorAction, PineEditorInput, PineEditorState};
use crate::ui::theme;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};

/// The result of compiling a script in the background.
struct CompileResult {
    diagnostics: Vec<Diagnostic>,
    has_alerts: bool,
}

/// One script in the in-memory list: the editor state plus the last compile result.
struct Script {
    editor: PineEditorState,
    diagnostics: Vec<Diagnostic>,
    has_alerts: bool,
    compiling: bool,
    inbox: Option<Receiver<CompileResult>>,
}

/// What the tab asks the app to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PineTabAction {
    /// Apply the script source to the focused chart.
    AddToChart(String),
}

pub struct PineTab {
    id: u64,
    scripts: Vec<Script>,
    selected: usize,
    next_script_id: u64,
}

impl PineTab {
    pub fn new(id: u64) -> Self {
        let mut tab = Self {
            id,
            scripts: Vec::new(),
            selected: 0,
            next_script_id: 1,
        };
        let script = tab.new_script("Untitled", "");
        tab.scripts.push(script);
        tab
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn title(&self) -> &'static str {
        "DegenView — Scripts"
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, has_chart: bool) -> Vec<PineTabAction> {
        let ctx = ui.ctx().clone();
        self.receive_compiles();

        egui::Panel::left(egui::Id::new(("pine-scripts", self.id)))
            .exact_size(220.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 10))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .show(ui, |ui| self.sidebar(ui));

        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                actions = self.editor(ui, has_chart, &ctx);
            });
        actions
    }

    fn fresh_script_id(&mut self) -> u64 {
        let id = self.next_script_id;
        self.next_script_id += 1;
        id
    }

    fn new_script(&mut self, name: &str, source: &str) -> Script {
        let id = self.fresh_script_id();
        Script {
            editor: PineEditorState::new(id, name, source),
            diagnostics: Vec::new(),
            has_alerts: false,
            compiling: false,
            inbox: None,
        }
    }

    fn receive_compiles(&mut self) {
        for script in &mut self.scripts {
            if let Some(inbox) = &script.inbox
                && let Ok(result) = inbox.try_recv()
            {
                script.diagnostics = result.diagnostics;
                script.has_alerts = result.has_alerts;
                script.compiling = false;
                script.inbox = None;
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut select: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.heading("Scripts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+").on_hover_text("New script").clicked() {
                    select = Some(self.scripts.len());
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, script) in self.scripts.iter().enumerate() {
                let label = if script.editor.is_dirty() {
                    format!("{} ●", script.editor.name)
                } else {
                    script.editor.name.clone()
                };
                if ui.selectable_label(index == self.selected, label).clicked() {
                    select = Some(index);
                }
            }
        });
        if let Some(index) = select {
            if index == self.scripts.len() {
                let script = self.new_script("Untitled", "");
                self.scripts.push(script);
            }
            self.selected = index;
        }
    }

    fn editor(
        &mut self,
        ui: &mut egui::Ui,
        has_chart: bool,
        ctx: &egui::Context,
    ) -> Vec<PineTabAction> {
        let mut actions = Vec::new();
        let script = &mut self.scripts[self.selected];
        let input = PineEditorInput {
            diagnostics: &script.diagnostics,
            has_chart,
            has_alerts: script.has_alerts,
            is_compiling: script.compiling,
        };
        for action in pine_editor::show(ui, &mut script.editor, &input) {
            match action {
                PineEditorAction::Save => script.editor.mark_saved(),
                PineEditorAction::AddToChart => {
                    actions.push(PineTabAction::AddToChart(script.editor.source.clone()));
                }
                PineEditorAction::CompileRequested => {
                    let source = script.editor.source.clone();
                    script.compiling = true;
                    let (sender, receiver) = mpsc::channel();
                    let repaint = ctx.clone();
                    std::thread::spawn(move || {
                        let program = compiler::compile(&source, Limits::default(), &NoLibraries);
                        let has_alerts = program.is_valid()
                            && !alert_call::call_sites(&program.statements).is_empty();
                        let _ = sender.send(CompileResult {
                            diagnostics: program.diagnostics,
                            has_alerts,
                        });
                        repaint.request_repaint();
                    });
                    script.inbox = Some(receiver);
                }
                // The Pine subscription host (Lead's slice) owns creating the subscription.
                PineEditorAction::CreateAlert => {}
            }
        }
        actions
    }
}
