//! The Script Manager window: file-backed Pine script library, editor, preview chart and drawer.
//! A port of DegenView's `ScriptManagerView` and its view models.
//!
//! Layout (see `workspace.rs`): a sidebar with the script list, and for the selected script a
//! split between the code editor and the preview chart, with the Inputs / Report / Problems
//! drawer under the chart.
#![allow(dead_code)] // transient: phase-2 pieces (completion, find bar) are not wired yet

mod completion;
pub mod demo;
pub mod drawer;
pub mod edit_ops;
pub mod editor;
mod find;
pub mod inputs;
pub mod model;
pub mod name;
pub mod prefs;
mod preview;
pub mod registry;
mod sheets;
mod sidebar;
mod signature;
pub mod store;
pub mod syntax;
mod workspace;

use crate::pine::compiler;
use crate::pine::token::Limits;
use crate::store::SharedStore;
use crate::ui::icons::{self, Icon};
use crate::ui::split::{self, Axis, DefaultLength, SplitSpec, SplitState};
use crate::ui::theme;
use editor::EditorState;
use eframe::egui::{self, Align, Id, Key, Layout, Margin, Stroke, Ui};
use model::{CompileStatus, LocalScript, ScriptId, ScriptType};
use prefs::{ChartPosition, DrawerTab, LayoutPrefs};
use registry::LibraryRegistry;
use sidebar::{Group, Rename, RowKey};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use store::{COMPILER_VERSION, ScriptStore, hash_source};

/// What the Script Manager asks the app to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptManagerAction {
    /// Add the script to the focused chart.
    AddToChart {
        script_id: String,
        name: String,
        source: String,
    },
    /// Create a Pine alert subscription on the focused chart.
    CreateAlert {
        script_name: String,
        source: String,
        note: String,
        webhook_endpoint_ids: Vec<u64>,
    },
    /// Open Settings on the Alpaca tab.
    OpenAlpacaSettings,
}

/// The focused chart a script alert would watch: its market and timeframe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChartInfo {
    pub chart_id: u64,
    /// "BTC/USDT".
    pub title: String,
    /// "Binance".
    pub source_name: String,
    /// "1h".
    pub timeframe: String,
    /// `"<SLUG>:<ticker>"`, what a subscription stores.
    pub symbol_key: String,
}

/// UI events collected while drawing and applied once the frame's closures have finished.
pub(crate) enum Cmd {
    Select(RowKey),
    ToggleFavorite(ScriptId),
    BeginRename(RowKey),
    CommitRename,
    CancelRename,
    Duplicate(ScriptId),
    ShowInFinder(ScriptId),
    Export(ScriptId),
    AskDelete(ScriptId),
    NewScript,
    AddExamples,
    AddToChart(ScriptId),
    CreateAlert(ScriptId),
    Save,
    Reveal(crate::pine::token::Diagnostic),
    ToggleSidebar,
    ToggleChart,
    SetChartPosition(ChartPosition),
    SetDrawerTab(DrawerTab),
    ToggleDrawer,
    OpenAlpacaSettings,
}

/// The script open in the editor.
pub(crate) struct OpenScript {
    pub id: ScriptId,
    pub name: String,
    pub script_type: ScriptType,
    pub editor: EditorState,
    pub has_alerts: bool,
}

pub(crate) enum Sheet {
    New {
        name: String,
        error: Option<String>,
        needs_focus: bool,
    },
    ConfirmDelete {
        id: ScriptId,
        name: String,
    },
    /// The Create Script Alert sheet for one script.
    Alert(Box<AlertSheet>),
}

pub(crate) struct AlertSheet {
    pub name: String,
    pub source: String,
    pub state: crate::ui::pine_alert_editor::PineAlertEditorState,
    pub call_sites: Vec<crate::pine::alert_call::AlertCallSite>,
    pub endpoints: Vec<crate::model::webhook::WebhookEndpoint>,
    pub duplicate_exists: bool,
}

pub struct ScriptManager {
    id: u64,
    shared: SharedStore,
    script_store: Option<ScriptStore>,
    pub(crate) scripts: Vec<LocalScript>,
    loaded: bool,
    last_scan: Instant,
    pub(crate) prefs: LayoutPrefs,
    saved_prefs: LayoutPrefs,
    pub(crate) search: String,
    pub(crate) collapsed: HashSet<Group>,
    pub(crate) selected_row: Option<RowKey>,
    pub(crate) renaming: Option<Rename>,
    pub(crate) last_click: Option<(RowKey, Instant)>,
    pub(crate) open: Option<OpenScript>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) chart: Option<ChartInfo>,
    pub(crate) error: Option<String>,
    pub(crate) cmds: Vec<Cmd>,
    pub(crate) preview: workspace::PreviewState,
    pub(crate) problems_copied: Option<Instant>,
    actions: Vec<ScriptManagerAction>,
    export_rx: Option<Receiver<(ScriptId, Option<PathBuf>)>>,
}

impl ScriptManager {
    pub fn new(id: u64, shared: SharedStore) -> Self {
        Self::with_script_store(id, shared, ScriptStore::default_location())
    }

    pub fn with_script_store(
        id: u64,
        shared: SharedStore,
        script_store: Option<ScriptStore>,
    ) -> Self {
        let prefs = LayoutPrefs::load(&shared);
        Self {
            id,
            preview: workspace::PreviewState::load(&shared),
            shared,
            script_store,
            scripts: Vec::new(),
            loaded: false,
            last_scan: Instant::now(),
            saved_prefs: prefs.clone(),
            prefs,
            search: String::new(),
            collapsed: HashSet::new(),
            selected_row: None,
            renaming: None,
            last_click: None,
            open: None,
            sheet: None,
            chart: None,
            error: None,
            cmds: Vec::new(),
            actions: Vec::new(),
            export_rx: None,
            problems_copied: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// The window title: the open script's name (and " — Edited" while unsaved). The
    /// "DegenView — " prefix is what groups this window into the app's tab group.
    pub fn title(&self) -> String {
        match &self.open {
            Some(open) if open.editor.is_dirty() => {
                format!("DegenView — {} — Edited", open.name)
            }
            Some(open) => format!("DegenView — {}", open.name),
            None => "DegenView — Script Manager".to_owned(),
        }
    }

    // MARK: Frame

    pub fn ui(&mut self, ui: &mut Ui, chart: Option<ChartInfo>) -> Vec<ScriptManagerAction> {
        self.chart = chart;
        let ctx = ui.ctx().clone();
        self.begin_frame(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("script-manager-toolbar")
            .exact_size(38.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .inner_margin(Margin::symmetric(10, 5))
                    .stroke(Stroke::new(1.0, theme::separator())),
            )
            .show(ui, |ui| self.toolbar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::background()))
            .show(ui, |ui| self.root(ui));

        self.sheets_ui(&ctx);
        self.apply_cmds(&ctx);
        self.end_frame(&ctx);
        std::mem::take(&mut self.actions)
    }

    fn begin_frame(&mut self, ctx: &egui::Context) {
        if !self.loaded {
            self.first_load();
        } else if self.last_scan.elapsed() > Duration::from_millis(1500) {
            // Files edited, added or renamed outside the app show up.
            self.refresh();
        }
        ctx.request_repaint_after(Duration::from_millis(1500));
        if let Some(rx) = &self.export_rx
            && let Ok((id, destination)) = rx.try_recv()
        {
            self.export_rx = None;
            if let Some(path) = destination {
                self.with_store(|store| store.export(&id, &path));
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (save, sidebar, chart) = ctx.input(|i| {
            let m = i.modifiers;
            (
                m.command && !m.ctrl && !m.alt && !m.shift && i.key_pressed(Key::S),
                m.command && m.ctrl && i.key_pressed(Key::S),
                m.command && m.alt && !m.ctrl && i.key_pressed(Key::P),
            )
        });
        if save && self.open.is_some() {
            self.cmds.push(Cmd::Save);
        }
        if sidebar {
            self.cmds.push(Cmd::ToggleSidebar);
        }
        if chart {
            self.cmds.push(Cmd::ToggleChart);
        }
    }

    fn end_frame(&mut self, ctx: &egui::Context) {
        if let Some(open) = self.open.as_mut()
            && open.editor.take_draft_due(Instant::now())
        {
            let draft = model::ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: model::now_ms(),
                based_on_revision_id: None,
            };
            self.with_store(|store| store.save_draft(&draft));
        }
        self.preview.tick(ctx);
        if self.prefs != self.saved_prefs {
            self.prefs.save(&self.shared);
            self.saved_prefs = self.prefs.clone();
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let tip = if self.prefs.sidebar_visible {
                "Hide the script list (⌃⌘S)"
            } else {
                "Show the script list (⌃⌘S)"
            };
            if icons::button(ui, Icon::SidebarLeft, 28.0, theme::text_primary(), tip).clicked() {
                self.cmds.push(Cmd::ToggleSidebar);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (icon, tip) = if self.prefs.chart_visible {
                    (Icon::ChartLine, "Hide the preview chart (⌥⌘P)")
                } else {
                    (Icon::ChartLineFlat, "Show the preview chart (⌥⌘P)")
                };
                if icons::button(ui, icon, 28.0, theme::text_primary(), tip).clicked() {
                    self.cmds.push(Cmd::ToggleChart);
                }
                let position_icon = match self.prefs.chart_position {
                    ChartPosition::Left => Icon::RectLeft,
                    ChartPosition::Top => Icon::RectTop,
                    ChartPosition::Bottom => Icon::RectBottom,
                };
                let button = icons::button(
                    ui,
                    position_icon,
                    28.0,
                    theme::text_primary(),
                    "Where the preview chart sits next to the code",
                );
                egui::Popup::menu(&button).show(|ui| {
                    ui.set_min_width(140.0);
                    for position in ChartPosition::ALL {
                        let selected = position == self.prefs.chart_position;
                        let label = if selected {
                            format!("✓  {}", position.title())
                        } else {
                            format!("    {}", position.title())
                        };
                        if ui.button(label).clicked() {
                            self.cmds.push(Cmd::SetChartPosition(position));
                            ui.close();
                        }
                    }
                });
            });
        });
    }

    fn root(&mut self, ui: &mut Ui) {
        let spec = SplitSpec {
            axis: Axis::Horizontal,
            secondary_first: true,
            min_primary: 420.0,
            min_secondary: 200.0,
            max_secondary: 320.0,
            default: DefaultLength::Points(240.0),
        };
        let mut state = SplitState {
            length: self.prefs.sidebar_width,
            visible: self.prefs.sidebar_visible,
        };
        let id = Id::new(("script-manager-root", self.id));
        let this = std::cell::RefCell::new(&mut *self);
        split::show(
            ui,
            id,
            spec,
            &mut state,
            |ui| this.borrow_mut().detail_ui(ui),
            |ui| this.borrow_mut().sidebar_ui(ui),
        );
        let _ = this;
        self.prefs.sidebar_width = state.length;
    }

    // MARK: Store

    /// Runs a store operation; a failure becomes the alert.
    fn with_store<T>(
        &mut self,
        operation: impl FnOnce(&mut ScriptStore) -> Result<T, store::ScriptStoreError>,
    ) -> Option<T> {
        let store = self.script_store.as_mut()?;
        match operation(store) {
            Ok(value) => Some(value),
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn first_load(&mut self) {
        self.loaded = true;
        let seeded: bool = self
            .shared
            .lock()
            .ok()
            .and_then(|s| s.setting(demo::SEEDED_KEY))
            .unwrap_or(false);
        if !seeded
            && self
                .with_store(|store| {
                    // A user who already has scripts is not offered the examples again; one
                    // coming from the Swift app first gets their own scripts.
                    if store.all_scripts()?.is_empty() {
                        let brought = match swift_scripts_dir() {
                            Some(folder) if folder.is_dir() => store.import_folder(&folder)?,
                            _ => 0,
                        };
                        if brought == 0 {
                            demo::add_missing(store)?;
                        }
                    }
                    Ok(())
                })
                .is_some()
            && let Ok(shared) = self.shared.lock()
        {
            let _ = shared.set_setting(demo::SEEDED_KEY, &true);
        }
        self.refresh();
        // Reopen where the user left off.
        if let Some(id) = self.prefs.selected_script_id.clone()
            && let Some(script) = self.scripts.iter().find(|s| s.id == id)
        {
            let group = group_of(script);
            self.selected_row = Some(RowKey {
                group,
                id: id.clone(),
            });
            self.open_script(&id);
        }
    }

    pub(crate) fn refresh(&mut self) {
        self.last_scan = Instant::now();
        let Some(scripts) = self.with_store(|store| store.all_scripts()) else {
            return;
        };
        self.scripts = scripts;
        // The open script vanished (deleted in Finder): close it.
        if let Some(open) = &self.open
            && !self.scripts.iter().any(|s| s.id == open.id)
        {
            self.close_script();
        } else if let Some(open) = self.open.as_mut()
            && let Some(script) = self.scripts.iter().find(|s| s.id == open.id)
        {
            open.name = script.name.clone();
            open.script_type = script.script_type;
        }
    }

    pub(crate) fn open_script(&mut self, id: &str) {
        self.flush_draft();
        let Some((script, draft)) = self.with_store(|store| {
            let script = store.script(id)?;
            let draft = store.draft(id)?;
            Ok((script, draft))
        }) else {
            return;
        };
        let Some(script) = script else {
            return;
        };
        let source = draft
            .filter(|d| d.source != script.source)
            .map(|d| d.source)
            .unwrap_or_else(|| script.source.clone());
        let mut editor = EditorState::new(&script.id, &source, &script.source);
        // A cached compile counts only when the compiler hasn't changed and the text matches.
        let cached = script.compile_record.as_ref().filter(|r| {
            r.compiler_version == COMPILER_VERSION && r.source_hash == hash_source(&source)
        });
        match cached {
            Some(record) => {
                editor.status = record.status;
                editor.diagnostics = record.diagnostics.clone();
            }
            None => compile_into(&mut editor),
        }
        self.preview.script_changed(&script.id);
        self.open = Some(OpenScript {
            id: script.id.clone(),
            name: script.name.clone(),
            script_type: script.script_type,
            editor,
            has_alerts: false,
        });
        self.prefs.selected_script_id = Some(script.id);
    }

    fn close_script(&mut self) {
        self.flush_draft();
        self.open = None;
        self.selected_row = None;
        self.renaming = None;
        self.prefs.selected_script_id = None;
    }

    /// Writes the pending draft now (switching scripts or closing).
    fn flush_draft(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.editor.has_pending_draft() && open.editor.is_dirty() {
            let draft = model::ScriptDraft {
                script_id: open.id.clone(),
                source: open.editor.source.clone(),
                modified_at_ms: model::now_ms(),
                based_on_revision_id: None,
            };
            self.with_store(|store| store.save_draft(&draft));
        }
    }

    /// Recompiles the open script after an edit, feeding the diagnostics back to the editor.
    pub(crate) fn editor_changed(&mut self) {
        if let Some(open) = self.open.as_mut() {
            compile_into(&mut open.editor);
            let program =
                compiler::compile(&open.editor.source, Limits::default(), &LibraryRegistry);
            open.has_alerts = program.is_valid()
                && !crate::pine::alert_call::call_sites(&program.statements).is_empty();
        }
    }

    pub(crate) fn script_named(&self, id: &str) -> Option<&LocalScript> {
        self.scripts.iter().find(|s| s.id == id)
    }

    // MARK: Commands

    fn apply_cmds(&mut self, ctx: &egui::Context) {
        let cmds = std::mem::take(&mut self.cmds);
        for cmd in cmds {
            self.run(cmd, ctx);
        }
    }

    fn run(&mut self, cmd: Cmd, ctx: &egui::Context) {
        match cmd {
            Cmd::Select(key) => {
                self.renaming = None;
                let id = key.id.clone();
                self.selected_row = Some(key);
                if self.open.as_ref().map(|o| o.id.as_str()) != Some(id.as_str()) {
                    self.open_script(&id);
                }
            }
            Cmd::ToggleFavorite(id) => {
                let favorite = self.script_named(&id).is_some_and(|s| s.is_favorite);
                self.with_store(|store| store.set_favorite(&id, !favorite));
                self.refresh();
            }
            Cmd::BeginRename(key) => {
                if let Some(script) = self.script_named(&key.id) {
                    self.renaming = Some(Rename {
                        row: key.clone(),
                        text: script.name.clone(),
                        error: None,
                        needs_focus: true,
                    });
                    self.selected_row = Some(key);
                }
            }
            Cmd::CommitRename => self.commit_rename(),
            Cmd::CancelRename => self.renaming = None,
            Cmd::Duplicate(id) => {
                if let Some(copy) = self.with_store(|store| store.duplicate(&id)) {
                    self.refresh();
                    let group = group_of(&copy);
                    self.cmds.push(Cmd::Select(RowKey { group, id: copy.id }));
                }
            }
            Cmd::ShowInFinder(id) => {
                if let Some(Some(path)) = self.with_store(|store| store.file_path(&id)) {
                    let _ = std::process::Command::new("open")
                        .arg("-R")
                        .arg(path)
                        .spawn();
                }
            }
            Cmd::Export(id) => self.begin_export(&id),
            Cmd::AskDelete(id) => {
                if let Some(script) = self.script_named(&id) {
                    self.sheet = Some(Sheet::ConfirmDelete {
                        name: script.name.clone(),
                        id,
                    });
                }
            }
            Cmd::NewScript => {
                self.sheet = Some(Sheet::New {
                    name: String::new(),
                    error: None,
                    needs_focus: true,
                });
            }
            Cmd::AddExamples => {
                self.with_store(demo::add_missing);
                self.refresh();
            }
            Cmd::AddToChart(id) => {
                if let Some(script) = self.script_named(&id) {
                    self.actions.push(ScriptManagerAction::AddToChart {
                        script_id: script.id.clone(),
                        name: script.name.clone(),
                        source: script.source.clone(),
                    });
                }
            }
            Cmd::CreateAlert(id) => self.open_alert_sheet(&id),
            Cmd::Save => self.save_open(),
            Cmd::Reveal(diagnostic) => {
                if let Some(open) = self.open.as_mut() {
                    open.editor.reveal(&diagnostic);
                }
            }
            Cmd::ToggleSidebar => self.prefs.sidebar_visible = !self.prefs.sidebar_visible,
            Cmd::ToggleChart => self.prefs.chart_visible = !self.prefs.chart_visible,
            Cmd::SetChartPosition(position) => self.prefs.chart_position = position,
            Cmd::SetDrawerTab(tab) => {
                self.prefs.drawer_tab = tab;
                self.prefs.drawer_open = true;
            }
            Cmd::ToggleDrawer => self.prefs.drawer_open = !self.prefs.drawer_open,
            Cmd::OpenAlpacaSettings => self.actions.push(ScriptManagerAction::OpenAlpacaSettings),
        }
        let _ = ctx;
    }

    /// Opens the Create Script Alert sheet for a script, against the focused chart's market.
    fn open_alert_sheet(&mut self, id: &str) {
        let Some(script) = self.script_named(id).cloned() else {
            return;
        };
        let Some(chart) = self.chart.clone() else {
            self.error = Some(
                "Open a chart first: a script alert watches one chart's market and timeframe."
                    .to_owned(),
            );
            return;
        };
        let program = compiler::compile(&script.source, Limits::default(), &LibraryRegistry);
        let call_sites = if program.is_valid() {
            crate::pine::alert_call::call_sites(&program.statements)
        } else {
            Vec::new()
        };
        let hash = hash_source(&script.source);
        let (endpoints, duplicate_exists) = self
            .shared
            .lock()
            .map(|store| {
                let duplicate = store.pine_subscriptions().iter().any(|s| {
                    s.is_active()
                        && s.chart_id == chart.chart_id
                        && s.source_hash == hash
                        && s.symbol_key == chart.symbol_key
                        && s.timeframe == chart.timeframe
                });
                (store.webhook_endpoints(), duplicate)
            })
            .unwrap_or_default();
        self.sheet = Some(Sheet::Alert(Box::new(AlertSheet {
            name: script.name,
            source: script.source,
            state: Default::default(),
            call_sites,
            endpoints,
            duplicate_exists,
        })));
    }

    fn commit_rename(&mut self) {
        let Some(rename) = self.renaming.take() else {
            return;
        };
        let current = self.script_named(&rename.row.id).map(|s| s.name.clone());
        if current.as_deref() == Some(rename.text.trim()) {
            return;
        }
        let id = rename.row.id.clone();
        let Some(store) = self.script_store.as_mut() else {
            return;
        };
        match store.rename(&id, &rename.text) {
            Ok(_) => self.refresh(),
            Err(error) => {
                // An invalid commit shows the alert and stays in edit mode.
                self.error = Some(error.to_string());
                self.renaming = Some(Rename {
                    error: Some(error.to_string()),
                    needs_focus: true,
                    ..rename
                });
            }
        }
    }

    fn save_open(&mut self) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let (id, source) = (open.id.clone(), open.editor.source.clone());
        if let Some(script) = self.with_store(|store| store.save(&id, &source)) {
            if let Some(open) = self.open.as_mut() {
                open.editor.mark_saved();
                open.script_type = script.script_type;
            }
            self.refresh();
        }
    }

    pub(crate) fn delete_script(&mut self, id: &str) {
        if self.open.as_ref().map(|o| o.id.as_str()) == Some(id) {
            self.close_script();
        }
        self.with_store(|store| store.delete(id));
        self.refresh();
    }

    pub(crate) fn create_script(&mut self, name: &str) -> Result<(), String> {
        let Some(store) = self.script_store.as_mut() else {
            return Err("The scripts folder isn't available.".to_owned());
        };
        match store.create(name, ScriptType::Indicator, None, false) {
            Ok(script) => {
                self.refresh();
                let group = group_of(&script);
                self.cmds.push(Cmd::Select(RowKey {
                    group,
                    id: script.id,
                }));
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// The save panel runs off the UI thread; its result comes back through `export_rx`.
    fn begin_export(&mut self, id: &str) {
        let Some(script) = self.script_named(id) else {
            return;
        };
        let default = format!("{}.pine", script.name);
        let (sender, receiver) = mpsc::channel();
        self.export_rx = Some(receiver);
        let id = id.to_owned();
        std::thread::spawn(move || {
            let script = format!(
                "POSIX path of (choose file name with prompt \"Export Script\" default name \"{}\")",
                default.replace('\\', "\\\\").replace('"', "\\\"")
            );
            let output = std::process::Command::new("osascript")
                .args(["-e", &script])
                .output();
            let path = output
                .ok()
                .filter(|o| o.status.success())
                .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_owned()))
                .filter(|p| !p.as_os_str().is_empty());
            let _ = sender.send((id, path));
        });
    }

    pub(crate) fn name_problem(&self, raw: &str) -> Option<String> {
        match name::validate(raw) {
            Err(error) => (!raw.trim().is_empty()).then(|| error.to_string()),
            Ok(clean) => self
                .scripts
                .iter()
                .any(|s| s.name.to_lowercase() == clean.to_lowercase())
                .then(|| store::ScriptStoreError::NameConflict.to_string()),
        }
    }
}

/// Where the Swift app keeps its scripts, to bring them across on first use.
fn swift_scripts_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/DegenView/Scripts"))
}

fn group_of(script: &LocalScript) -> Group {
    match script.script_type {
        ScriptType::Indicator => Group::Indicators,
        ScriptType::Strategy => Group::Strategies,
        ScriptType::Library => Group::Libraries,
    }
}

/// Compiles the editor's text and stores the diagnostics and status the chip and Problems read.
fn compile_into(editor: &mut EditorState) {
    let program = compiler::compile(&editor.source, Limits::default(), &LibraryRegistry);
    editor.status = CompileStatus::of(&program.diagnostics);
    editor.diagnostics = program.diagnostics;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn manager(tag: &str) -> (ScriptManager, PathBuf) {
        let root = std::env::temp_dir().join(format!("degenview-sm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        (
            ScriptManager::with_script_store(1, shared, Some(store)),
            root,
        )
    }

    #[test]
    fn first_load_seeds_examples_once_and_never_for_existing_libraries() {
        let (mut manager, root) = manager("seed");
        manager.first_load();
        assert_eq!(manager.scripts.len(), 9);

        // A fresh settings store over the same folder: scripts exist, so nothing is added.
        let shared: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let mut again = ScriptManager::with_script_store(2, shared, Some(store));
        again.first_load();
        assert_eq!(again.scripts.len(), 9);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn create_select_rename_delete_flow() {
        let (mut manager, root) = manager("flow");
        manager.first_load();
        manager.create_script("Mine").unwrap();
        let cmds = std::mem::take(&mut manager.cmds);
        assert_eq!(cmds.len(), 1);
        for cmd in cmds {
            manager.run(cmd, &egui::Context::default());
        }
        assert_eq!(manager.open.as_ref().unwrap().name, "Mine");
        assert_eq!(manager.title(), "DegenView — Mine");
        manager
            .open
            .as_mut()
            .unwrap()
            .editor
            .source
            .push_str("\n// edit");
        assert_eq!(manager.title(), "DegenView — Mine — Edited");
        manager.save_open();
        assert_eq!(manager.title(), "DegenView — Mine");

        assert!(
            manager.create_script("mine").is_err(),
            "case-insensitive conflict"
        );
        assert_eq!(
            manager.name_problem("MINE").as_deref(),
            Some("A script with that name already exists.")
        );
        assert_eq!(
            manager.name_problem("  "),
            None,
            "empty shows no error until it is submitted"
        );

        let id = manager.open.as_ref().unwrap().id.clone();
        manager.renaming = Some(Rename {
            row: RowKey {
                group: Group::Indicators,
                id: id.clone(),
            },
            text: "Renamed".into(),
            error: None,
            needs_focus: false,
        });
        manager.commit_rename();
        assert_eq!(manager.open.as_ref().unwrap().name, "Renamed");
        // A bad rename keeps editing and raises the alert.
        manager.renaming = Some(Rename {
            row: RowKey {
                group: Group::Indicators,
                id,
            },
            text: "a/b".into(),
            error: None,
            needs_focus: false,
        });
        manager.commit_rename();
        assert!(manager.renaming.is_some() && manager.error.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_window_draws_headlessly_through_selection_and_layout_changes() {
        let (mut manager, root) = manager("draw");
        let ctx = egui::Context::default();
        let frame = |manager: &mut ScriptManager| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                manager.ui(ui, None);
            });
            output.textures_delta.clear();
        };
        // Empty library and first load (seeds examples), then a selected script.
        frame(&mut manager);
        frame(&mut manager);
        assert_eq!(manager.scripts.len(), 9);
        let first = manager.scripts[0].clone();
        manager.cmds.push(Cmd::Select(RowKey {
            group: Group::Indicators,
            id: first.id,
        }));
        frame(&mut manager);
        frame(&mut manager);
        assert!(manager.open.is_some());
        for position in ChartPosition::ALL {
            manager.prefs.chart_position = position;
            frame(&mut manager);
        }
        for tab in [DrawerTab::Inputs, DrawerTab::Report, DrawerTab::Problems] {
            manager.cmds.push(Cmd::SetDrawerTab(tab));
            frame(&mut manager);
        }
        manager.cmds.push(Cmd::ToggleSidebar);
        manager.cmds.push(Cmd::ToggleChart);
        manager.cmds.push(Cmd::ToggleDrawer);
        manager.cmds.push(Cmd::NewScript);
        frame(&mut manager);
        frame(&mut manager);
        manager.sheet = None;
        manager.error = Some("boom".into());
        frame(&mut manager);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn create_alert_needs_a_chart_then_opens_the_sheet_for_the_script() {
        let (mut manager, root) = manager("alert");
        manager.first_load();
        let ctx = egui::Context::default();
        let demo = manager
            .scripts
            .iter()
            .find(|s| s.name == "Volume Spike Alert")
            .expect("a demo with alert() calls")
            .id
            .clone();

        manager.run(Cmd::CreateAlert(demo.clone()), &ctx);
        assert!(manager.sheet.is_none());
        assert!(
            manager
                .error
                .as_deref()
                .is_some_and(|e| e.contains("Open a chart"))
        );
        manager.error = None;

        manager.chart = Some(ChartInfo {
            chart_id: 3,
            title: "BTC/USDT".into(),
            source_name: "Binance".into(),
            timeframe: "1h".into(),
            symbol_key: "BINANCE:BTCUSDT".into(),
        });
        manager.run(Cmd::CreateAlert(demo), &ctx);
        let Some(Sheet::Alert(sheet)) = &manager.sheet else {
            panic!("the Create Script Alert sheet opens");
        };
        assert_eq!(sheet.name, "Volume Spike Alert");
        assert!(!sheet.call_sites.is_empty(), "the example raises an alert");
        assert!(!sheet.duplicate_exists);
        // And it draws.
        for _ in 0..2 {
            let chart = manager.chart.clone();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                manager.ui(ui, chart.clone());
            });
            output.textures_delta.clear();
        }
        assert!(
            matches!(manager.sheet, Some(Sheet::Alert(_))),
            "still open until chosen"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
