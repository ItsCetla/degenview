//! The watchlist panel: header (list selector, add, filter, options), column header,
//! rows with native-feeling drag reorder, and the prompts around them.
//!
//! A pure view. It reads the lists, flags and quotes it is handed and returns
//! `WatchlistAction`s; the workspace applies them to the store. Which list a window
//! shows, its filter and its highlighted row live in `WatchlistSidebarState`.

use crate::model::InstrumentId;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistError, WatchlistFlag, WatchlistInstrument,
    WatchlistSort, WatchlistSortKey,
};
use crate::model::watchlist_layout::{self, Filter, Flags, Row};
use crate::model::watchlist_quote::WatchlistQuote;
use crate::model::watchlist_quote::{self, Freshness};
use crate::model::watchlist_text::ImportReport;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, Confirm, ConfirmOutcome};
use crate::ui::sheet;
use crate::ui::source_logo;
use crate::ui::theme;
use crate::ui::watchlist_rows::{self, RowStyle};
use crate::ui::watchlist_style::{self as style, LEADING_INSET, TRAILING_INSET};
use eframe::egui::{
    self, Align, Color32, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::sync::mpsc::{self, Receiver};

/// What the workspace should do on the sidebar's behalf.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchlistAction {
    // Charts.
    /// Open in the tab's focused chart.
    Open(WatchlistInstrument),
    AddChart(WatchlistInstrument),
    OpenInNewTab(WatchlistInstrument),
    /// The "+" button: the host shows its Add Symbol sheet for the current list.
    AddSymbol {
        list: Option<EntryId>,
    },
    CreateAlert(WatchlistInstrument),
    // Lists.
    CreateList {
        name: String,
        /// Seed the new list with this market ("New Watchlist…" from a membership menu).
        with: Option<WatchlistInstrument>,
    },
    RenameList {
        id: EntryId,
        name: String,
    },
    DuplicateList {
        id: EntryId,
        name: String,
    },
    DeleteList(EntryId),
    // Contents.
    AddToList {
        list: EntryId,
        item: WatchlistInstrument,
    },
    RemoveFromList {
        list: EntryId,
        instrument: InstrumentId,
    },
    RemoveEntry {
        list: EntryId,
        entry: EntryId,
    },
    MoveToSection {
        list: EntryId,
        entry: EntryId,
        section: Option<EntryId>,
    },
    Move {
        list: EntryId,
        mv: watchlist_layout::Move,
    },
    MoveToSectionByDrop {
        list: EntryId,
        mv: watchlist_layout::SectionMove,
    },
    AddSection {
        list: EntryId,
        title: String,
    },
    RenameSection {
        list: EntryId,
        section: EntryId,
        title: String,
    },
    DeleteSection {
        list: EntryId,
        section: EntryId,
    },
    SetSectionCollapsed {
        list: EntryId,
        section: EntryId,
        collapsed: bool,
    },
    // Display.
    SetSort {
        list: EntryId,
        sort: WatchlistSort,
    },
    SetColumn {
        list: EntryId,
        column: WatchlistColumn,
        visible: bool,
    },
    SetShowsDescription {
        list: EntryId,
        shows: bool,
    },
    SetFlag {
        instrument: InstrumentId,
        flag: Option<WatchlistFlag>,
    },
    // Exchange.
    Import {
        list: EntryId,
        text: String,
    },
    BeginImport,
    /// "Export to File…": the host owns the save dialog.
    Export {
        list: EntryId,
    },
    CopyText(String),
    DismissError,
}

pub struct WatchlistSidebarInput<'a> {
    pub lists: &'a [Watchlist],
    pub flags: &'a Flags,
    pub quote: &'a dyn Fn(&InstrumentId) -> Option<WatchlistQuote>,
    /// The market the tab's focused chart shows; the matching row is highlighted.
    pub focused_market: Option<&'a InstrumentId>,
    /// The saved lists couldn't be read, so changes are off.
    pub load_failed: bool,
    pub error: Option<&'a str>,
    /// What the last import did, shown in the import sheet.
    pub import_report: Option<&'a ImportReport>,
}

#[derive(Clone, Debug)]
enum PromptKind {
    NewList,
    RenameList(EntryId),
    CopyList(EntryId),
    NewSection(EntryId),
    RenameSection { list: EntryId, section: EntryId },
    NewListWith(Box<WatchlistInstrument>),
}

impl PromptKind {
    fn title(&self) -> &'static str {
        match self {
            Self::NewList | Self::NewListWith(_) => "New Watchlist",
            Self::RenameList(_) => "Rename Watchlist",
            Self::CopyList(_) => "Make a Copy",
            Self::NewSection(_) => "New Section",
            Self::RenameSection { .. } => "Rename Section",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NewList => "Name the new watchlist.".to_owned(),
            Self::NewListWith(item) => {
                format!("Name the new watchlist. {} will be added to it.", item.name)
            }
            Self::RenameList(_) => "Enter a new name.".to_owned(),
            Self::CopyList(_) => "Name the copy.".to_owned(),
            Self::NewSection(_) => "Sections group symbols under a heading.".to_owned(),
            Self::RenameSection { .. } => "Enter a new name.".to_owned(),
        }
    }

    fn confirm(&self) -> &'static str {
        match self {
            Self::NewList | Self::NewListWith(_) => "Create",
            Self::RenameList(_) | Self::RenameSection { .. } => "Rename",
            Self::CopyList(_) => "Copy",
            Self::NewSection(_) => "Create",
        }
    }
}

struct Drag {
    /// Index into the rows as shown.
    index: usize,
}

#[derive(Default)]
pub struct WatchlistSidebarState {
    selected_list: Option<EntryId>,
    pub filter: Filter,
    pub filter_visible: bool,
    focus_filter: bool,
    pub selected_entry: Option<EntryId>,
    prompt: Option<PromptKind>,
    prompt_text: String,
    confirm_delete: bool,
    import_text: Option<String>,
    import_picker: Option<Receiver<Result<Option<String>, String>>>,
    import_error: Option<String>,
    shows_detail: bool,
    drag: Option<Drag>,
    synced_focus: Option<String>,
    synced_list: Option<EntryId>,
}

impl WatchlistSidebarState {
    pub fn minimum_width(&mut self, lists: &[Watchlist]) -> f32 {
        let columns = self
            .resolved(lists)
            .map(|list| list.display.columns.as_slice());
        style::minimum_panel_width(
            columns.unwrap_or(&[WatchlistColumn::Last, WatchlistColumn::ChangePercent]),
        )
    }

    #[cfg(test)]
    pub fn selected_list_id(&self) -> Option<EntryId> {
        self.selected_list
    }

    pub fn select_list(&mut self, id: EntryId) {
        self.selected_list = Some(id);
    }

    /// Opens the "name your watchlist" prompt seeded with a market, for a membership
    /// menu's "New Watchlist…" elsewhere in the app.
    pub fn prompt_new_list_with(&mut self, item: WatchlistInstrument) {
        self.prompt_text.clear();
        self.prompt = Some(PromptKind::NewListWith(Box::new(item)));
    }

    fn open_prompt(&mut self, kind: PromptKind, text: String) {
        self.prompt_text = text;
        self.prompt = Some(kind);
    }

    fn clear_filter(&mut self) {
        self.filter = Filter::default();
    }

    /// The list this window shows: its pick if it still exists, else Favorites, else the first.
    fn resolved<'a>(&mut self, lists: &'a [Watchlist]) -> Option<&'a Watchlist> {
        let found = self
            .selected_list
            .and_then(|id| lists.iter().find(|list| list.id == id))
            .or_else(|| lists.iter().find(|list| list.is_favorites))
            .or_else(|| lists.first());
        self.selected_list = found.map(|list| list.id);
        found
    }

    /// The highlighted row follows the focused chart's market.
    fn sync_selection(&mut self, list: &Watchlist, focused: Option<&InstrumentId>) {
        let key = focused.map(InstrumentId::key);
        if self.synced_focus == key && self.synced_list == Some(list.id) {
            return;
        }
        self.synced_focus = key.clone();
        self.synced_list = Some(list.id);
        self.selected_entry = key.and_then(|key| {
            list.instruments()
                .find(|item| item.instrument.key() == key)
                .map(|item| item.id)
        });
    }
}

// MARK: Entry point

pub fn show(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
) -> Vec<WatchlistAction> {
    let mut actions = Vec::new();
    if let Some(receiver) = &state.import_picker {
        match receiver.try_recv() {
            Ok(Ok(Some(text))) => {
                state.import_text = Some(text);
                state.import_picker = None;
            }
            Ok(Ok(None)) => {
                state.import_picker = None;
            }
            Ok(Err(error)) => {
                state.import_error = Some(error);
                state.import_picker = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                state.import_picker = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    let list = state.resolved(input.lists);

    header(ui, state, input, list, &mut actions);
    if state.filter_visible {
        filter_field(ui, state);
    }
    let separator = ui.painter().clone();
    let rect = ui.available_rect_before_wrap();
    separator.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, theme::BORDER),
    );
    ui.add_space(1.0);

    if input.load_failed {
        centered_message(
            ui,
            Icon::Triangle,
            "Watchlists Unavailable",
            &WatchlistError::PersistenceUnavailable.to_string(),
        );
    } else if let Some(list) = list {
        state.sync_selection(list, input.focused_market);
        if state.shows_detail {
            let detail_height = state
                .selected_entry
                .and_then(|id| list.instruments().find(|item| item.id == id))
                .map(|item| {
                    let quote = (input.quote)(&item.instrument);
                    let extra_rows = quote.as_ref().map_or(0, |quote| {
                        usize::from(quote.change_basis.is_some())
                            + usize::from(quote.note.is_some())
                            + usize::from(quote.timestamp_ms.is_some())
                    });
                    174.0 + 17.0 * extra_rows as f32
                })
                .unwrap_or(0.0);
            if detail_height > 0.0 {
                egui::Panel::bottom(Id::new(("watchlist-detail", list.id)))
                    .exact_size(detail_height)
                    .resizable(false)
                    .frame(
                        egui::Frame::new()
                            .fill(theme::CHROME)
                            .inner_margin(egui::Margin::ZERO),
                    )
                    .show(ui, |ui| {
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| detail_panel(ui, state, input, list));
                    });
            }
        }
        content(ui, state, input, list, &mut actions);
    } else {
        centered_message(
            ui,
            Icon::List,
            "No Watchlists",
            "Create a watchlist from the menu above.",
        );
    }

    modals(ui.ctx(), state, input, list, &mut actions);
    actions
}

fn centered_message(ui: &mut egui::Ui, icon: Icon, title: &str, detail: &str) {
    ui.add_space(((ui.available_height() - 170.0) / 2.0).max(24.0));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        icons::paint(ui.painter(), rect, icon, theme::TEXT_TERTIARY);
        ui.add_space(12.0);
        ui.label(
            RichText::new(title)
                .strong()
                .size(16.0)
                .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(5.0);
        ui.add_sized(
            [ui.available_width() - 36.0, 0.0],
            egui::Label::new(
                RichText::new(detail)
                    .size(12.0)
                    .color(ui.visuals().weak_text_color()),
            )
            .wrap()
            .halign(Align::Center),
        );
    });
}

// MARK: Header

fn header(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    let controls_right = rect.right() - TRAILING_INSET;
    let options_rect = Rect::from_center_size(
        Pos2::new(controls_right - 12.0, rect.center().y),
        Vec2::splat(24.0),
    );
    let filter_rect = options_rect.translate(Vec2::new(-32.0, 0.0));
    let add_rect = filter_rect.translate(Vec2::new(-32.0, 0.0));
    let selector_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET, rect.center().y - 13.0),
        Pos2::new(
            (add_rect.left() - 8.0).max(rect.left() + LEADING_INSET + 64.0),
            rect.center().y + 13.0,
        ),
    );

    list_selector(ui, state, input, list, selector_rect);
    let enabled = !input.load_failed && list.is_some();
    let add = kit::hover_button(
        ui,
        add_rect,
        Id::new("watchlist-add"),
        Icon::Plus,
        "Add Symbol",
        true,
        enabled,
    );
    if add.clicked() {
        actions.push(WatchlistAction::AddSymbol {
            list: list.map(|list| list.id),
        });
    }
    let filtering = state.filter_visible || state.filter.is_active();
    let filter = ui.interact(
        filter_rect,
        Id::new("watchlist-filter-toggle"),
        Sense::click(),
    );
    let filter = crate::ui::accessibility::label(filter, "Filter this watchlist")
        .on_hover_text("Filter this watchlist");
    if filter.hovered() {
        ui.painter()
            .rect_filled(filter_rect, 6.0, Color32::from_white_alpha(17));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(filter_rect.center(), Vec2::splat(15.0)),
        Icon::Magnifier,
        if filtering {
            theme::SYSTEM_ACCENT
        } else {
            theme::TEXT_PRIMARY
        },
    );
    if filter.clicked() {
        state.filter_visible = !state.filter_visible;
        if state.filter_visible {
            state.focus_filter = true;
        } else {
            state.clear_filter();
        }
    }
    options_menu(ui, state, list, actions, options_rect, enabled);
}

fn list_selector(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    rect: Rect,
) {
    let title = list.map_or("Watchlists", |list| list.name.as_str());
    let response = ui.interact(
        rect,
        Id::new("watchlist-selector"),
        if input.load_failed {
            Sense::hover()
        } else {
            Sense::click()
        },
    );
    let response = crate::ui::accessibility::label(response, format!("Watchlist: {title}"));
    if response.hovered() && !input.load_failed {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_white_alpha(14));
    }
    let available = (rect.width() - 20.0).max(10.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        FontId::proportional(14.0),
        if input.load_failed {
            theme::TEXT_TERTIARY
        } else {
            theme::TEXT_PRIMARY
        },
    );
    job.wrap.max_width = available;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let title_galley = ui.painter().layout_job(job);
    ui.painter().galley(
        Pos2::new(rect.left(), rect.center().y - title_galley.size().y / 2.0),
        title_galley,
        Color32::WHITE,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.right() - 8.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronDown,
        theme::TEXT_SECONDARY,
    );
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            for candidate in input.lists {
                let selected = list.is_some_and(|list| list.id == candidate.id);
                if ui.radio(selected, &candidate.name).clicked() {
                    state.select_list(candidate.id);
                    state.selected_entry = None;
                    state.synced_list = None;
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Create New Watchlist…").clicked() {
                state.open_prompt(PromptKind::NewList, String::new());
                ui.close();
            }
            let Some(list) = list else {
                return;
            };
            if ui.button("Rename Watchlist…").clicked() {
                state.open_prompt(PromptKind::RenameList(list.id), list.name.clone());
                ui.close();
            }
            if ui.button("Make a Copy…").clicked() {
                state.open_prompt(PromptKind::CopyList(list.id), format!("{} copy", list.name));
                ui.close();
            }
            if ui
                .add_enabled(!list.is_favorites, egui::Button::new("Delete Watchlist…"))
                .on_disabled_hover_text("Favorites can be renamed but not deleted")
                .clicked()
            {
                state.confirm_delete = true;
                ui.close();
            }
        });
}

fn options_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
    rect: Rect,
    enabled: bool,
) {
    let response = kit::hover_button(
        ui,
        rect,
        Id::new("watchlist-options"),
        Icon::Ellipsis,
        "Watchlist options",
        true,
        enabled,
    );
    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            let Some(list) = list else {
                return;
            };
            if ui.button("Add Section…").clicked() {
                state.open_prompt(PromptKind::NewSection(list.id), String::new());
                ui.close();
            }

            ui.menu_button("Sort By", |ui| {
                for key in WatchlistSortKey::ALL {
                    if ui
                        .radio(list.display.sort.key == key, key.title())
                        .clicked()
                    {
                        let sort = if key == WatchlistSortKey::Manual {
                            WatchlistSort::MANUAL
                        } else {
                            WatchlistSort {
                                key,
                                ascending: key == WatchlistSortKey::Symbol,
                            }
                        };
                        actions.push(WatchlistAction::SetSort {
                            list: list.id,
                            sort,
                        });
                        ui.close();
                    }
                }
                ui.separator();
                let manual = list.display.sort.key == WatchlistSortKey::Manual;
                let mut descending = !list.display.sort.ascending;
                if ui
                    .add_enabled(!manual, egui::Checkbox::new(&mut descending, "Descending"))
                    .changed()
                {
                    actions.push(WatchlistAction::SetSort {
                        list: list.id,
                        sort: WatchlistSort {
                            key: list.display.sort.key,
                            ascending: !descending,
                        },
                    });
                }
            });

            ui.menu_button("Columns", |ui| {
                for column in WatchlistColumn::ALL {
                    let mut visible = list.display.columns.contains(&column);
                    if ui
                        .checkbox(&mut visible, column.accessibility_title())
                        .changed()
                    {
                        actions.push(WatchlistAction::SetColumn {
                            list: list.id,
                            column,
                            visible,
                        });
                    }
                }
                ui.separator();
                let mut shows = list.display.shows_description;
                if ui.checkbox(&mut shows, "Show Symbol Subtitle").changed() {
                    actions.push(WatchlistAction::SetShowsDescription {
                        list: list.id,
                        shows,
                    });
                }
            });

            ui.menu_button("Filter by Flag", |ui| {
                for flag in WatchlistFlag::ALL {
                    ui.horizontal(|ui| {
                        style::swatch(ui, flag);
                        if ui
                            .radio(state.filter.flag == Some(flag), flag.title())
                            .clicked()
                        {
                            state.filter.flag = Some(flag);
                            ui.close();
                        }
                    });
                }
                ui.separator();
                if ui
                    .add_enabled(state.filter.flag.is_some(), egui::Button::new("Show All"))
                    .clicked()
                {
                    state.filter.flag = None;
                    ui.close();
                }
            });

            let mut details = state.shows_detail;
            if ui.checkbox(&mut details, "Show Details").changed() {
                state.shows_detail = details;
            }

            ui.separator();
            if ui.button("Import Symbols…").clicked() {
                state.import_text = Some(String::new());
                state.import_error = None;
                actions.push(WatchlistAction::BeginImport);
                ui.close();
            }
            if ui.button("Export to File…").clicked() {
                actions.push(WatchlistAction::Export { list: list.id });
                ui.close();
            }
            if ui.button("Copy as Text").clicked() {
                actions.push(WatchlistAction::CopyText(
                    crate::model::watchlist_text::export(list),
                ));
                ui.close();
            }
        });
}

fn filter_field(ui: &mut egui::Ui, state: &mut WatchlistSidebarState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.left() + LEADING_INSET + 7.0, rect.top() + 11.0),
            Vec2::splat(14.0),
        ),
        Icon::Magnifier,
        theme::TEXT_SECONDARY,
    );
    let clear_width = if state.filter.is_active() { 24.0 } else { 0.0 };
    let flag_width = if state.filter.flag.is_some() {
        14.0
    } else {
        0.0
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + LEADING_INSET + 22.0, rect.top() + 1.0),
        Pos2::new(
            rect.right() - TRAILING_INSET - clear_width - flag_width,
            rect.top() + 25.0,
        ),
    );
    let response = ui.put(
        text_rect,
        egui::TextEdit::singleline(&mut state.filter.text)
            .hint_text("Filter symbols")
            .frame(egui::Frame::NONE),
    );
    if std::mem::take(&mut state.focus_filter) {
        response.request_focus();
    }
    if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.filter_visible = false;
        state.clear_filter();
    }
    if let Some(flag) = state.filter.flag {
        style::paint_flag(
            ui.painter(),
            rect.right() - TRAILING_INSET - clear_width - 8.0,
            rect.top() + 12.0,
            flag,
        );
    }
    if state.filter.is_active() {
        let clear_rect = Rect::from_center_size(
            Pos2::new(rect.right() - TRAILING_INSET - 10.0, rect.top() + 12.0),
            Vec2::splat(20.0),
        );
        if kit::hover_button(
            ui,
            clear_rect,
            Id::new("watchlist-filter-clear"),
            Icon::XmarkCircleFill,
            "Clear filter",
            true,
            true,
        )
        .clicked()
        {
            state.clear_filter();
            response.request_focus();
        }
    }
}

// MARK: Content

fn content(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    actions: &mut Vec<WatchlistAction>,
) {
    let rows = watchlist_layout::rows(
        list,
        &state.filter,
        input.flags,
        list.display.sort,
        input.quote,
    );
    if rows.is_empty() {
        empty_state(ui, state, list, actions);
        return;
    }

    if let Some(key) = watchlist_rows::column_header(ui, &list.display) {
        let sort = list.display.sort.after_clicking(key);
        actions.push(WatchlistAction::SetSort {
            list: list.id,
            sort,
        });
    }

    // Order on screen is the stored order only in Manual with no filter.
    let derived_order =
        list.display.sort.key != WatchlistSortKey::Manual || state.filter.is_active();
    let mut rects: Vec<Rect> = Vec::with_capacity(rows.len());
    let mut drop_requested = false;
    let mut drag_started = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, row) in rows.iter().enumerate() {
                let dragged = state.drag.as_ref().is_some_and(|drag| drag.index == index);
                let response = match row {
                    Row::Section { section, count } => {
                        let response =
                            watchlist_rows::section_row(ui, section, *count, index == 0, dragged);
                        if response.clicked() {
                            actions.push(WatchlistAction::SetSectionCollapsed {
                                list: list.id,
                                section: section.id,
                                collapsed: !section.is_collapsed,
                            });
                        }
                        response.context_menu(|ui| {
                            section_menu(
                                ui,
                                state,
                                list,
                                section.id,
                                &section.title,
                                section.is_collapsed,
                                actions,
                            );
                        });
                        response
                    }
                    Row::Instrument { item, .. } => {
                        let style = RowStyle {
                            display: &list.display,
                            flags: input.flags,
                            quote: input.quote,
                            selected: state.selected_entry == Some(item.id),
                            dragged,
                        };
                        let response = watchlist_rows::instrument_row(ui, item, &style);
                        let more_rect = Rect::from_center_size(
                            Pos2::new(
                                response.rect.right() - TRAILING_INSET - 11.0,
                                response.rect.center().y,
                            ),
                            Vec2::splat(22.0),
                        );
                        let more = ui.interact(
                            more_rect,
                            Id::new(("watchlist-row-more", item.id)),
                            Sense::click(),
                        );
                        let more = crate::ui::accessibility::label(
                            more,
                            format!("Actions for {}", item.name),
                        );
                        let popup = egui::Popup::menu(&more).frame(kit::menu_frame());
                        if response.hovered() || more.hovered() || popup.is_open() {
                            ui.painter()
                                .circle_filled(more_rect.center(), 11.0, theme::CHROME);
                            if more.hovered() {
                                ui.painter().circle_filled(
                                    more_rect.center(),
                                    10.0,
                                    Color32::from_white_alpha(25),
                                );
                            }
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(more_rect.center(), Vec2::splat(13.0)),
                                Icon::Ellipsis,
                                theme::TEXT_SECONDARY,
                            );
                        }
                        if response.clicked() && !more.clicked() {
                            state.selected_entry = Some(item.id);
                            actions.push(WatchlistAction::Open(item.clone()));
                        }
                        popup.show(|ui| instrument_menu(ui, state, input, list, item, actions));
                        response.context_menu(|ui| {
                            instrument_menu(ui, state, input, list, item, actions);
                        });
                        response
                    }
                    Row::EmptySection { .. } => watchlist_rows::empty_section_row(ui),
                };
                rects.push(response.rect);

                if row.entry_id().is_some() {
                    if response.drag_started() {
                        drag_started = Some(index);
                    }
                    if response.drag_stopped() {
                        drop_requested = true;
                    }
                }
            }
        });

    if let Some(index) = drag_started {
        state.drag = Some(Drag { index });
    }
    if let Some(drag) = &state.drag {
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let destination = pointer.map(|pointer| insertion_index(&rects, pointer.y));
        if let Some(destination) = destination {
            paint_insertion_line(ui, &rects, destination);
        }
        if drop_requested {
            if let Some(destination) = destination {
                let source = drag.index;
                let dragged_is_symbol = matches!(rows.get(source), Some(Row::Instrument { .. }));
                if derived_order && dragged_is_symbol {
                    if let Some(mv) =
                        watchlist_layout::resolve_section_move(&rows, &[source], destination)
                    {
                        actions.push(WatchlistAction::MoveToSectionByDrop { list: list.id, mv });
                    }
                } else if let Some(mv) =
                    watchlist_layout::resolve_move(&rows, &[source], destination)
                {
                    actions.push(WatchlistAction::Move { list: list.id, mv });
                }
            }
            state.drag = None;
        }
    }
    if !ui.ctx().egui_wants_keyboard_input()
        && let Some(item) = state
            .selected_entry
            .and_then(|id| list.instruments().find(|item| item.id == id))
    {
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            actions.push(WatchlistAction::Open(item.clone()));
        }
        if ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) {
            actions.push(WatchlistAction::RemoveEntry {
                list: list.id,
                entry: item.id,
            });
            state.selected_entry = None;
        }
    }
}

/// The row boundary the pointer is nearest to: before the first row whose middle is
/// below it, or the end.
fn insertion_index(rects: &[Rect], pointer_y: f32) -> usize {
    rects
        .iter()
        .position(|rect| rect.center().y > pointer_y)
        .unwrap_or(rects.len())
}

fn paint_insertion_line(ui: &egui::Ui, rects: &[Rect], destination: usize) {
    let Some(first) = rects.first() else {
        return;
    };
    let y = match rects.get(destination) {
        Some(rect) => rect.top(),
        None => rects.last().map_or(first.top(), |rect| rect.bottom()),
    };
    let painter = ui.painter();
    let (left, right) = (first.left() + LEADING_INSET, first.right() - TRAILING_INSET);
    painter.line_segment(
        [Pos2::new(left, y), Pos2::new(right, y)],
        Stroke::new(2.0, theme::ACCENT),
    );
    painter.circle_filled(Pos2::new(left, y), 3.0, theme::ACCENT);
}

fn empty_state(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    actions: &mut Vec<WatchlistAction>,
) {
    if state.filter.is_active() {
        centered_message(
            ui,
            Icon::Magnifier,
            "No Matches",
            &format!("Nothing in {} matches this filter.", list.name),
        );
        ui.vertical_centered(|ui| {
            if ui.button("Clear Filter").clicked() {
                state.clear_filter();
            }
        });
    } else {
        centered_message(
            ui,
            Icon::Star,
            "No Symbols",
            "Add a stock, crypto, or prediction-market item to follow it here.",
        );
        ui.vertical_centered(|ui| {
            if ui.button("Add Symbol").clicked() {
                actions.push(WatchlistAction::AddSymbol {
                    list: Some(list.id),
                });
            }
        });
    }
}

fn detail_panel(
    ui: &mut egui::Ui,
    state: &WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
) {
    let Some(item) = state
        .selected_entry
        .and_then(|id| list.instruments().find(|item| item.id == id))
    else {
        return;
    };
    let quote = (input.quote)(&item.instrument);
    let quote = quote.as_ref();
    let source = item.instrument.source;
    let weak = ui.visuals().weak_text_color();
    egui::Frame::new()
        .fill(theme::CHROME)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                source_logo::show(ui, source, 18.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(&item.name).strong().size(13.0));
                    ui.label(RichText::new(source.display_name()).small().color(weak));
                });
                if let Some(flag) = input.flags.get(&item.instrument.key()) {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(flag.title()).small().color(weak));
                        style::swatch(ui, *flag);
                    });
                }
            });
            ui.add_space(8.0);
            egui::Grid::new(("watchlist-detail-grid", list.id))
                .num_columns(2)
                .spacing([12.0, 3.0])
                .show(ui, |ui| {
                    let mut row = |label: &str, value: String| {
                        ui.label(RichText::new(label).small().color(weak));
                        ui.label(RichText::new(value).small());
                        ui.end_row();
                    };
                    row(
                        "Symbol",
                        format!("{}:{}", source.watchlist_slug(), item.instrument.symbol),
                    );
                    row("Last", watchlist_quote::last_text(quote, Some(source)));
                    row(
                        "Change",
                        format!(
                            "{} ({})",
                            watchlist_quote::change_text(quote, Some(source)),
                            watchlist_quote::percent_text(quote, Some(source))
                        ),
                    );
                    if let Some(basis) = quote.and_then(|quote| quote.change_basis) {
                        row("Measured", basis.label().to_owned());
                    }
                    row("Volume", watchlist_quote::volume_text(quote));
                    row(
                        "Status",
                        quote
                            .map_or(Freshness::Unavailable, |quote| quote.freshness)
                            .label()
                            .to_owned(),
                    );
                    if let Some(note) = quote.and_then(|quote| quote.note.as_ref()) {
                        row("Note", note.clone());
                    }
                    if let Some(timestamp) = quote.and_then(|quote| quote.timestamp_ms) {
                        row(
                            "Updated",
                            crate::model::paper_format::timestamp(
                                timestamp,
                                crate::service::timezone::local_utc_offset_secs(timestamp),
                            ),
                        );
                    }
                    let names = input
                        .lists
                        .iter()
                        .filter(|list| {
                            list.instruments().any(|candidate| {
                                candidate.instrument.is_same_market(&item.instrument)
                            })
                        })
                        .map(|list| list.name.as_str())
                        .collect::<Vec<_>>();
                    row(
                        "In lists",
                        if names.is_empty() {
                            "–".to_owned()
                        } else {
                            names.join(", ")
                        },
                    );
                });
        });
}

// MARK: Menus

fn section_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    list: &Watchlist,
    section: EntryId,
    title: &str,
    collapsed: bool,
    actions: &mut Vec<WatchlistAction>,
) {
    if ui
        .button(if collapsed {
            "Expand Section"
        } else {
            "Collapse Section"
        })
        .clicked()
    {
        actions.push(WatchlistAction::SetSectionCollapsed {
            list: list.id,
            section,
            collapsed: !collapsed,
        });
        ui.close();
    }
    if ui.button("Rename Section…").clicked() {
        state.open_prompt(
            PromptKind::RenameSection {
                list: list.id,
                section,
            },
            title.to_owned(),
        );
        ui.close();
    }
    if ui.button("Add Section…").clicked() {
        state.open_prompt(PromptKind::NewSection(list.id), String::new());
        ui.close();
    }
    ui.separator();
    if ui.button("Delete Section").clicked() {
        actions.push(WatchlistAction::DeleteSection {
            list: list.id,
            section,
        });
        ui.close();
    }
}

/// The actions for one market, used by its context menu.
fn instrument_menu(
    ui: &mut egui::Ui,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
    item: &WatchlistInstrument,
    actions: &mut Vec<WatchlistAction>,
) {
    if ui.button("Open in Focused Chart").clicked() {
        actions.push(WatchlistAction::Open(item.clone()));
        ui.close();
    }
    if ui.button("Add as New Chart").clicked() {
        actions.push(WatchlistAction::AddChart(item.clone()));
        ui.close();
    }
    if ui.button("Open in New Tab").clicked() {
        actions.push(WatchlistAction::OpenInNewTab(item.clone()));
        ui.close();
    }
    ui.separator();

    membership_menu(ui, input.lists, item, input.load_failed, actions, |item| {
        state.prompt_new_list_with(item);
    });

    if list.sections().next().is_some() {
        ui.menu_button("Move to Section", |ui| {
            if ui.button("No Section").clicked() {
                actions.push(WatchlistAction::MoveToSection {
                    list: list.id,
                    entry: item.id,
                    section: None,
                });
                ui.close();
            }
            for section in list.sections() {
                if ui.button(&section.title).clicked() {
                    actions.push(WatchlistAction::MoveToSection {
                        list: list.id,
                        entry: item.id,
                        section: Some(section.id),
                    });
                    ui.close();
                }
            }
        });
    }

    let current = input.flags.get(&item.instrument.key()).copied();
    ui.menu_button("Set Flag", |ui| {
        for flag in WatchlistFlag::ALL {
            ui.horizontal(|ui| {
                style::swatch(ui, flag);
                if ui.radio(current == Some(flag), flag.title()).clicked() {
                    actions.push(WatchlistAction::SetFlag {
                        instrument: item.instrument.clone(),
                        flag: (current != Some(flag)).then_some(flag),
                    });
                    ui.close();
                }
            });
        }
        ui.separator();
        if ui
            .add_enabled(current.is_some(), egui::Button::new("Clear Flag"))
            .clicked()
        {
            actions.push(WatchlistAction::SetFlag {
                instrument: item.instrument.clone(),
                flag: None,
            });
            ui.close();
        }
    });

    if !item.instrument.source.is_prediction_market()
        && item.instrument.source != crate::model::DataSource::CoinMarketCap
        && ui.button("Create Price Alert…").clicked()
    {
        actions.push(WatchlistAction::CreateAlert(item.clone()));
        ui.close();
    }

    ui.separator();
    if ui.button("Copy Symbol").clicked() {
        actions.push(WatchlistAction::CopyText(item.instrument.symbol.clone()));
        ui.close();
    }
    if ui.button("Copy Qualified Symbol").clicked() {
        actions.push(WatchlistAction::CopyText(format!(
            "{}:{}",
            item.instrument.source.watchlist_slug(),
            item.instrument.symbol
        )));
        ui.close();
    }
    ui.separator();
    if ui
        .button(RichText::new("Remove from Watchlist").color(theme::DOWN))
        .clicked()
    {
        actions.push(WatchlistAction::RemoveEntry {
            list: list.id,
            entry: item.id,
        });
        ui.close();
    }
}

/// "Add to Watchlist": every list with a check where the market already is, then New
/// Watchlist…. Shared by the sidebar, chart cards and search results so membership
/// reads the same everywhere. `on_new_list` is called for "New Watchlist…"; the caller
/// decides where the name prompt lives.
pub fn membership_menu(
    ui: &mut egui::Ui,
    lists: &[Watchlist],
    item: &WatchlistInstrument,
    disabled: bool,
    actions: &mut Vec<WatchlistAction>,
    on_new_list: impl FnOnce(WatchlistInstrument),
) {
    ui.add_enabled_ui(!disabled, |ui| {
        ui.menu_button("Add to Watchlist", |ui| {
            for list in lists {
                let member = list.contains(&item.instrument);
                let mut checked = member;
                if ui.checkbox(&mut checked, &list.name).changed() {
                    actions.push(if checked {
                        WatchlistAction::AddToList {
                            list: list.id,
                            item: item.clone(),
                        }
                    } else {
                        WatchlistAction::RemoveFromList {
                            list: list.id,
                            instrument: item.instrument.clone(),
                        }
                    });
                }
            }
            ui.separator();
            if ui.button("New Watchlist…").clicked() {
                on_new_list(item.clone());
                ui.close();
            }
        });
    });
}

// MARK: Modals

fn modals(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    prompt_window(ctx, state, actions);
    delete_window(ctx, state, list, actions);
    import_window(ctx, state, input, list, actions);
    if let Some(message) = input.error {
        sheet::show(ctx, Id::new("watchlist-error"), 340.0, |ui| {
            ui.add_space(20.0);
            sheet::inset(ui, |ui| {
                sheet::header(ui, Icon::Triangle, "Watchlist", message, theme::ORANGE);
                ui.add_space(20.0);
            });
            ui.separator();
            sheet::inset(ui, |ui| {
                ui.add_space(12.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        actions.push(WatchlistAction::DismissError);
                    }
                });
                ui.add_space(12.0);
            });
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            actions.push(WatchlistAction::DismissError);
        }
    }
}

fn prompt_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    actions: &mut Vec<WatchlistAction>,
) {
    let Some(kind) = state.prompt.clone() else {
        return;
    };
    let mut commit = false;
    let mut cancel = false;
    let message = kind.message();
    sheet::show(ctx, Id::new("watchlist-prompt"), 360.0, |ui| {
        ui.add_space(22.0);
        sheet::inset(ui, |ui| {
            sheet::header(ui, Icon::List, kind.title(), &message, theme::ACCENT);
            ui.add_space(18.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.prompt_text)
                    .hint_text("Name")
                    .desired_width(f32::INFINITY),
            );
            if !response.has_focus() && !response.lost_focus() {
                response.request_focus();
            }
            let enter =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if enter && !state.prompt_text.trim().is_empty() {
                commit = true;
            }
            ui.add_space(18.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let valid = !state.prompt_text.trim().is_empty();
                if ui
                    .add_enabled(valid, egui::Button::new(kind.confirm()))
                    .clicked()
                {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            ui.add_space(12.0);
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    if commit {
        let name = state.prompt_text.trim().to_owned();
        match kind {
            PromptKind::NewList => actions.push(WatchlistAction::CreateList { name, with: None }),
            PromptKind::NewListWith(item) => actions.push(WatchlistAction::CreateList {
                name,
                with: Some(*item),
            }),
            PromptKind::RenameList(id) => actions.push(WatchlistAction::RenameList { id, name }),
            PromptKind::CopyList(id) => actions.push(WatchlistAction::DuplicateList { id, name }),
            PromptKind::NewSection(list) => {
                actions.push(WatchlistAction::AddSection { list, title: name })
            }
            PromptKind::RenameSection { list, section } => {
                actions.push(WatchlistAction::RenameSection {
                    list,
                    section,
                    title: name,
                });
            }
        }
        state.prompt = None;
    } else if cancel {
        state.prompt = None;
    }
}

fn delete_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    if !state.confirm_delete {
        return;
    }
    let Some(list) = list else {
        state.confirm_delete = false;
        return;
    };
    let title = format!("Delete “{}”?", list.name);
    let result = kit::confirm(
        ctx,
        Id::new("watchlist-delete"),
        &Confirm {
            icon: Icon::Trash,
            title: &title,
            message: "Its symbols stay in your other watchlists. Charts, drawings and alerts are not affected.",
            destructive: "Delete",
        },
    );
    if result == Some(ConfirmOutcome::Confirmed) {
        actions.push(WatchlistAction::DeleteList(list.id));
    }
    if result.is_some() {
        state.confirm_delete = false;
    }
}

fn import_window(
    ctx: &egui::Context,
    state: &mut WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: Option<&Watchlist>,
    actions: &mut Vec<WatchlistAction>,
) {
    let (Some(text), Some(list)) = (state.import_text.as_mut(), list) else {
        return;
    };
    let mut close = false;
    let mut submit = false;
    sheet::show(ctx, Id::new("watchlist-import"), 460.0, |ui| {
        ui.set_height(380.0);
        ui.add_space(20.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::Download,
                &format!("Import into {}", list.name),
                "Paste symbols or open a text file.",
                theme::ACCENT,
            );
            ui.add_space(14.0);
            if let Some(report) = input.import_report {
                ui.label(RichText::new(report.summary()).strong().color(
                    if report.skipped.is_empty() {
                        theme::UP
                    } else {
                        theme::ORANGE
                    },
                ));
                if !report.skipped.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("SKIPPED")
                            .small()
                            .strong()
                            .color(ui.visuals().weak_text_color()),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(190.0)
                        .show(ui, |ui| {
                            for skipped in &report.skipped {
                                ui.label(RichText::new(&skipped.text).monospace().small());
                                ui.label(
                                    RichText::new(format!(
                                        "Line {} · {}",
                                        skipped.line, skipped.reason
                                    ))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                                );
                                ui.add_space(7.0);
                            }
                        });
                }
            } else {
                ui.label(RichText::new("One symbol per line as SOURCE:SYMBOL, for example BINANCE:BTCUSDT or COINBASE:ETH-USD. Lines starting with ### become sections. A single comma-separated line from TradingView is read too; markets DegenView has no provider for are skipped.").small().color(ui.visuals().weak_text_color()));
                ui.add_space(10.0);
                ui.add_sized(
                    [ui.available_width(), 155.0],
                    egui::TextEdit::multiline(text)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("BINANCE:BTCUSDT\nCOINBASE:ETH-USD"),
                );
                if let Some(error) = &state.import_error {
                    ui.label(RichText::new(error).small().color(theme::DOWN));
                }
            }
            ui.add_space(14.0);
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if input.import_report.is_none()
                    && ui
                        .add_enabled(
                            state.import_picker.is_none(),
                            egui::Button::new("Open File…"),
                        )
                        .clicked()
                {
                    let (sender, receiver) = mpsc::channel();
                    state.import_picker = Some(receiver);
                    std::thread::spawn(move || {
                        let result =
                            crate::service::file_dialog::choose_csv("Choose watchlist symbols")
                                .and_then(|path| {
                                    path.map(|path| crate::service::file_dialog::read_text(&path))
                                        .transpose()
                                });
                        let _ = sender.send(result);
                    });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if input.import_report.is_some() {
                        if ui.button("Done").clicked() {
                            close = true;
                        }
                    } else {
                        if ui
                            .add_enabled(!text.trim().is_empty(), egui::Button::new("Import"))
                            .clicked()
                        {
                            submit = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
            ui.add_space(12.0);
        });
    });
    if submit {
        actions.push(WatchlistAction::Import {
            list: list.id,
            text: text.clone(),
        });
    }
    if close || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.import_text = None;
        state.import_picker = None;
        state.import_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::watchlist::Watchlist;

    fn lists() -> Vec<Watchlist> {
        let mut favorites = Watchlist::new("Favorites");
        favorites.is_favorites = true;
        vec![Watchlist::new("Crypto"), favorites]
    }

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::new(
            InstrumentId::new(DataSource::Binance, symbol),
            symbol,
            symbol,
        )
    }

    #[test]
    fn the_window_shows_its_pick_else_favorites_else_the_first() {
        let lists = lists();
        let mut state = WatchlistSidebarState::default();
        assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
        state.select_list(lists[0].id);
        assert_eq!(state.resolved(&lists).unwrap().name, "Crypto");
        // The picked list was deleted elsewhere: fall back, and remember the fallback.
        state.select_list(999);
        assert_eq!(state.resolved(&lists).unwrap().name, "Favorites");
        assert_eq!(state.selected_list_id(), Some(lists[1].id));
        assert!(state.resolved(&[]).is_none());
        assert_eq!(state.selected_list_id(), None);
        let only_one = vec![Watchlist::new("Solo")];
        assert_eq!(state.resolved(&only_one).unwrap().name, "Solo");
    }

    #[test]
    fn highlight_follows_the_focused_chart_and_only_resyncs_on_change() {
        let mut list = Watchlist::new("L");
        let btc = item("BTCUSDT");
        let eth = item("ETHUSDT");
        let (btc_id, eth_id) = (btc.id, eth.id);
        list.add(btc, None).unwrap();
        list.add(eth, None).unwrap();

        let mut state = WatchlistSidebarState::default();
        let focused = InstrumentId::new(DataSource::Binance, "ethusdt");
        state.sync_selection(&list, Some(&focused));
        assert_eq!(state.selected_entry, Some(eth_id));

        // A click selects BTC; with the same focus the next frame must not undo it.
        state.selected_entry = Some(btc_id);
        state.sync_selection(&list, Some(&focused));
        assert_eq!(state.selected_entry, Some(btc_id));

        // Focus moves to a market not in the list: nothing is highlighted.
        let elsewhere = InstrumentId::new(DataSource::Coinbase, "BTC-USD");
        state.sync_selection(&list, Some(&elsewhere));
        assert_eq!(state.selected_entry, None);
        state.sync_selection(&list, None);
        assert_eq!(state.selected_entry, None);
    }

    #[test]
    fn insertion_follows_row_middles() {
        let rect =
            |top: f32| Rect::from_min_size(Pos2::new(0.0, top), egui::Vec2::new(100.0, 40.0));
        let rects = [rect(0.0), rect(40.0), rect(80.0)];
        assert_eq!(insertion_index(&rects, -10.0), 0);
        assert_eq!(insertion_index(&rects, 19.0), 0);
        assert_eq!(insertion_index(&rects, 21.0), 1);
        assert_eq!(insertion_index(&rects, 95.0), 2);
        assert_eq!(insertion_index(&rects, 105.0), 3);
        assert_eq!(insertion_index(&[], 5.0), 0);
    }

    #[test]
    fn prompts_describe_themselves() {
        assert_eq!(PromptKind::NewList.confirm(), "Create");
        assert_eq!(PromptKind::RenameList(1).confirm(), "Rename");
        assert_eq!(PromptKind::CopyList(1).title(), "Make a Copy");
        assert_eq!(
            PromptKind::RenameSection {
                list: 1,
                section: 2
            }
            .message(),
            "Enter a new name."
        );
    }

    #[test]
    fn new_list_prompt_can_carry_a_market() {
        let mut state = WatchlistSidebarState::default();
        state.prompt_new_list_with(item("SOLUSDT"));
        assert!(matches!(state.prompt, Some(PromptKind::NewListWith(_))));
        assert_eq!(
            state.prompt.as_ref().unwrap().message(),
            "Name the new watchlist. SOLUSDT will be added to it."
        );
        assert!(state.prompt_text.is_empty());
    }
}
