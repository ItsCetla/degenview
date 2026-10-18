//! The Object Tree: the chart's symbol, indicators, scripts and drawings as a list you can
//! select, show or hide, lock, reorder and delete from.
//!
//! A docked panel over `model::object_tree`'s rows. It owns nothing: selecting a drawing here is
//! selecting it on the chart (the workspace's one selection), and every change is returned as a
//! [`TreeAction`] for the workspace to apply through the same paths the chart's own controls use.

use crate::model::object_tree::{IndicatorKey, Key, Row, Section};
use crate::ui::icons::{self, Icon};
use crate::ui::{accessibility, kit, theme};
use eframe::egui::{self, Align2, Rect, Sense, Ui, pos2, vec2};

const ROW_HEIGHT: f32 = 26.0;
const BUTTON: f32 = 22.0;

#[derive(Default)]
pub struct TreeState {
    pub filter: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TreeAction {
    Close,
    /// A row was clicked.
    Select(Key),
    /// A row was double-clicked or "Settings…" was chosen.
    Settings(Key),
    ToggleShown(Key),
    ToggleLocked(u64),
    Delete(Key),
    /// Move a drawing in the stack: positive toward the front.
    Reorder(u64, i32),
}

pub fn show(
    ui: &mut Ui,
    state: &mut TreeState,
    title: &str,
    rows: &[Row],
    selected: &[u64],
) -> Vec<TreeAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(kit::text(
            "Object tree",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if icons::button(
                ui,
                Icon::Xmark,
                24.0,
                theme::text_secondary(),
                "Close object tree",
            )
            .clicked()
            {
                actions.push(TreeAction::Close);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(kit::text(
            title,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let width = ui.available_width() - 10.0;
        let field = egui::TextEdit::singleline(&mut state.filter)
            .hint_text("Filter objects")
            .desired_width(width);
        accessibility::label(ui.add(field), "Filter objects");
    });
    ui.add_space(6.0);

    if rows.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(kit::text(
                "Nothing matches.",
                kit::size::BODY,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
        return actions;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for row in &rows[range] {
                row_ui(ui, row, selected, &mut actions);
            }
        });
    actions
}

/// The buttons a row can have, rightmost first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Btn {
    Delete,
    Lock,
    Show,
}

/// A row's buttons, in slot order: everything but the symbol can be deleted, only drawings lock,
/// and whatever can be hidden has an eye. A locked drawing keeps its (greyed) delete so locking
/// moves nothing.
fn row_buttons(row: &Row) -> Vec<Btn> {
    let mut buttons = Vec::new();
    if !matches!(row.key, Key::Symbol) {
        buttons.push(Btn::Delete);
    }
    if matches!(row.key, Key::Drawing(_)) && row.locked.is_some() {
        buttons.push(Btn::Lock);
    }
    if row.shown.is_some() {
        buttons.push(Btn::Show);
    }
    buttons
}

/// Whether a button shows without the pointer on the row: a lock that is on, an object hidden.
fn shows_at_rest(row: &Row, button: Btn) -> bool {
    match button {
        Btn::Delete => false,
        Btn::Lock => row.locked == Some(true),
        Btn::Show => row.shown == Some(false),
    }
}

/// Where a row's button `index` sits: 0 is the rightmost.
fn button_slot(row: Rect, index: usize) -> Rect {
    let center_x = row.right() - 8.0 - BUTTON / 2.0 - index as f32 * (BUTTON + 2.0);
    Rect::from_center_size(pos2(center_x, row.center().y), vec2(BUTTON, BUTTON))
}

fn row_ui(ui: &mut Ui, row: &Row, selected: &[u64], actions: &mut Vec<TreeAction>) {
    let width = ui.available_width();
    if let Key::Heading(section) = row.key {
        let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
        ui.painter().text(
            pos2(rect.left() + 12.0, rect.center().y + 3.0),
            Align2::LEFT_CENTER,
            heading_text(section),
            egui::FontId::proportional(kit::size::CAPTION),
            theme::text_tertiary(),
        );
        return;
    }
    let is_selected = matches!(row.key, Key::Drawing(id) if selected.contains(&id));
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    // Over the row by position, not by the row's own response: a button on top of the row takes
    // the pointer from it, and a row that stopped counting as hovered would drop its buttons the
    // moment the pointer reached one.
    let over = ui.rect_contains_pointer(rect);
    let dimmed = row.shown == Some(false);
    if is_selected {
        ui.painter().rect_filled(
            rect.shrink2(vec2(4.0, 1.0)),
            5.0,
            kit::ACCENT.gamma_multiply(0.28),
        );
    } else if over {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(4.0, 1.0)), 5.0, theme::ink(12));
    }
    let color = if dimmed {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };

    // The row's buttons sit in slots numbered over the buttons it can have, so a row without a
    // lock has no gap where one would be, and nothing shifts under the pointer as buttons
    // appear: a lock or a hide stays visible, the rest show on hover.
    let buttons = row_buttons(row);
    let mut reserved = 0;
    for (index, kind) in buttons.iter().enumerate() {
        let at_rest = shows_at_rest(row, *kind);
        if !(over || at_rest) {
            continue;
        }
        reserved = index + 1;
        let slot = button_slot(rect, index);
        let (icon, tip, tint, enabled) = match *kind {
            Btn::Delete if row.locked == Some(true) => (
                Icon::Trash,
                "Unlock to delete",
                theme::text_tertiary(),
                false,
            ),
            Btn::Delete => (Icon::Trash, "Delete", theme::text_secondary(), true),
            Btn::Lock if row.locked == Some(true) => (Icon::LockFill, "Unlock", kit::ACCENT, true),
            Btn::Lock => (Icon::LockOpen, "Lock", theme::text_secondary(), true),
            Btn::Show if row.shown == Some(false) => {
                (Icon::EyeSlash, "Show", theme::text_secondary(), true)
            }
            Btn::Show => (Icon::Eye, "Hide", theme::text_secondary(), true),
        };
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let button = ui.interact(slot, response.id.with(tip), sense);
        let button = accessibility::label(button, tip).on_hover_text(tip);
        let hot = enabled && button.hovered();
        if hot {
            ui.painter().rect_filled(slot, 5.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), vec2(13.0, 13.0)),
            icon,
            if hot { theme::text_primary() } else { tint },
        );
        if enabled && button.clicked() {
            actions.push(match (*kind, row.key) {
                (Btn::Lock, Key::Drawing(id)) => TreeAction::ToggleLocked(id),
                (Btn::Delete, key) => TreeAction::Delete(key),
                (_, key) => TreeAction::ToggleShown(key),
            });
        }
    }
    // Room the label leaves for the buttons that are showing.
    let right = rect.right() - 8.0 - reserved as f32 * (BUTTON + 2.0);
    #[cfg(test)]
    probe::record(row.key, rect);

    let label_width = (right - rect.left() - 24.0).max(40.0);
    let galley = kit::galley(
        ui,
        &row.label,
        kit::size::BODY,
        kit::REGULAR,
        color,
        label_width,
    );
    let text_left = rect.left() + 14.0;
    ui.painter().galley(
        pos2(text_left, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        color,
    );
    if !row.detail.is_empty() {
        let detail_left = text_left + galley.size().x + 8.0;
        let room = right - detail_left - 6.0;
        if room > 30.0 {
            let detail = kit::galley(
                ui,
                &row.detail,
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
                room,
            );
            ui.painter().galley(
                pos2(detail_left, rect.center().y - detail.size().y / 2.0),
                detail,
                theme::text_tertiary(),
            );
        }
    }
    let response = accessibility::label(response, accessible_name(row));

    if response.double_clicked() {
        actions.push(TreeAction::Settings(row.key));
    } else if response.clicked() {
        actions.push(TreeAction::Select(row.key));
    }
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| {
            ui.set_width(190.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if !matches!(row.key, Key::Symbol)
                && kit::menu_item(
                    ui,
                    Icon::Sliders,
                    "Settings…",
                    row.locked != Some(true),
                    false,
                )
            {
                actions.push(TreeAction::Settings(row.key));
                ui.close();
            }
            if let Key::Drawing(id) = row.key {
                kit::menu_separator(ui);
                for (label, step) in [
                    ("Bring forward", 1),
                    ("Send backward", -1),
                    ("Bring to front", i32::MAX / 2),
                    ("Send to back", i32::MIN / 2),
                ] {
                    if kit::menu_item(ui, Icon::Layers, label, true, false) {
                        actions.push(TreeAction::Reorder(id, step));
                        ui.close();
                    }
                }
            }
            if !matches!(row.key, Key::Symbol) && row.locked != Some(true) {
                kit::menu_separator(ui);
                if kit::menu_item(ui, Icon::Trash, "Delete", true, true) {
                    actions.push(TreeAction::Delete(row.key));
                    ui.close();
                }
            }
        });
}

fn heading_text(section: Section) -> &'static str {
    match section {
        Section::Symbol => "SYMBOL",
        Section::Indicators => "INDICATORS",
        Section::Drawings => "DRAWINGS",
    }
}

fn accessible_name(row: &Row) -> String {
    let kind = match row.key {
        Key::Symbol => "Symbol",
        Key::Indicator(IndicatorKey::BuiltIn(_)) => "Indicator",
        Key::Indicator(IndicatorKey::Script(_)) => "Script",
        Key::Drawing(_) => "Drawing",
        Key::Heading(_) => "",
    };
    let mut name = format!("{kind} {}", row.label);
    if row.shown == Some(false) {
        name.push_str(", hidden");
    }
    if row.locked == Some(true) {
        name.push_str(", locked");
    }
    name
}

/// Where the rows were drawn last, for the tests that click their buttons.
#[cfg(test)]
mod probe {
    use super::Key;
    use eframe::egui::Rect;
    use std::cell::RefCell;

    thread_local! {
        static ROWS: RefCell<Vec<(Key, Rect)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn record(key: Key, rect: Rect) {
        ROWS.with(|rows| {
            let mut rows = rows.borrow_mut();
            rows.retain(|(existing, _)| *existing != key);
            rows.push((key, rect));
        });
    }

    pub fn rect_of(key: Key) -> Option<Rect> {
        ROWS.with(|rows| {
            rows.borrow()
                .iter()
                .find(|(existing, _)| *existing == key)
                .map(|(_, rect)| *rect)
        })
    }

    pub fn clear() {
        ROWS.with(|rows| rows.borrow_mut().clear());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::object_tree::IndicatorKey;
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    fn rows() -> Vec<Row> {
        vec![
            Row {
                key: Key::Heading(Section::Drawings),
                label: "Drawings".to_owned(),
                detail: String::new(),
                shown: None,
                locked: None,
            },
            Row {
                key: Key::Drawing(7),
                label: "Trend line 1".to_owned(),
                detail: String::new(),
                shown: Some(true),
                locked: Some(false),
            },
            Row {
                key: Key::Drawing(8),
                label: "Rectangle 1".to_owned(),
                detail: String::new(),
                shown: Some(true),
                locked: Some(true),
            },
            Row {
                key: Key::Indicator(IndicatorKey::Script(3)),
                label: "My script".to_owned(),
                detail: "Script".to_owned(),
                shown: Some(true),
                locked: None,
            },
        ]
    }

    /// The tree on a throwaway context, driven frame by frame.
    struct Rig {
        ctx: egui::Context,
        state: TreeState,
        rows: Vec<Row>,
        actions: Vec<TreeAction>,
    }

    impl Rig {
        fn new() -> Self {
            probe::clear();
            let rig = Self {
                ctx: egui::Context::default(),
                state: TreeState::default(),
                rows: rows(),
                actions: Vec::new(),
            };
            rig.settled()
        }

        fn settled(mut self) -> Self {
            for _ in 0..3 {
                self.frame(Vec::new());
            }
            self.actions.clear();
            self
        }

        fn frame(&mut self, events: Vec<Event>) {
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(320.0, 600.0))),
                events,
                ..RawInput::default()
            };
            let rows = self.rows.clone();
            let state = &mut self.state;
            let mut found = Vec::new();
            let mut output = self.ctx.run_ui(input, |ui| {
                found = show(ui, state, "BTCUSDT", &rows, &[]);
            });
            output.textures_delta.clear();
            self.actions.extend(found);
        }

        fn row(&self, key: Key) -> Rect {
            probe::rect_of(key).expect("row drawn")
        }

        fn move_to(&mut self, point: Pos2) {
            self.frame(vec![Event::PointerMoved(point)]);
            // The buttons are hit-tested from the frame before: one more at rest.
            self.frame(Vec::new());
        }

        fn click(&mut self, point: Pos2, button: PointerButton) {
            self.move_to(point);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton {
                    pos: point,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                }]);
            }
        }
    }

    #[test]
    fn the_buttons_stay_while_the_pointer_is_on_them_and_can_be_clicked() {
        let mut rig = Rig::new();
        let row = rig.row(Key::Drawing(7));
        // Hover the row, then slide onto the eye: the row's own hover ends there, the buttons
        // must not.
        rig.move_to(row.center());
        rig.move_to(button_slot(row, 2).center());
        rig.click(button_slot(row, 2).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
    }

    #[test]
    fn delete_lock_and_show_each_do_their_own_thing() {
        let mut rig = Rig::new();
        let row = rig.row(Key::Drawing(7));
        rig.click(button_slot(row, 0).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::Delete(Key::Drawing(7))]);
        rig.actions.clear();
        rig.click(button_slot(row, 1).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleLocked(7)]);
        rig.actions.clear();
        rig.click(button_slot(row, 2).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
    }

    #[test]
    fn a_script_row_can_be_hidden_and_removed() {
        let mut rig = Rig::new();
        let key = Key::Indicator(IndicatorKey::Script(3));
        let row = rig.row(key);
        // No lock on a script: the eye sits right next to the delete, with no empty slot.
        rig.click(button_slot(row, 1).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleShown(key)]);
        rig.actions.clear();
        rig.click(button_slot(row, 0).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::Delete(key)]);
    }

    #[test]
    fn a_locked_drawing_cannot_be_deleted_from_the_tree() {
        let mut rig = Rig::new();
        let row = rig.row(Key::Drawing(8));
        rig.move_to(row.center());
        // Its delete slot is empty: nothing is there to click.
        rig.click(button_slot(row, 0).center(), PointerButton::Primary);
        assert!(
            rig.actions
                .iter()
                .all(|action| !matches!(action, TreeAction::Delete(_)))
        );
        // Unlocking it is possible.
        rig.actions.clear();
        rig.click(button_slot(row, 1).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleLocked(8)]);
    }

    #[test]
    fn clicking_the_row_selects_it_and_a_button_does_not() {
        let mut rig = Rig::new();
        let row = rig.row(Key::Drawing(7));
        rig.click(row.left_center() + vec2(30.0, 0.0), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::Select(Key::Drawing(7))]);
        rig.actions.clear();
        rig.click(button_slot(row, 2).center(), PointerButton::Primary);
        assert!(
            rig.actions
                .iter()
                .all(|action| !matches!(action, TreeAction::Select(_))),
            "{:?}",
            rig.actions
        );
    }

    #[test]
    fn a_hidden_row_keeps_its_show_button_without_hover() {
        let mut rig = Rig::new();
        rig.rows[1].shown = Some(false);
        let mut rig = rig.settled();
        let row = rig.row(Key::Drawing(7));
        // The pointer is nowhere near; the eye is there to click anyway.
        rig.frame(vec![Event::PointerMoved(button_slot(row, 2).center())]);
        rig.frame(Vec::new());
        rig.click(button_slot(row, 2).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
    }

    #[test]
    fn a_row_without_a_lock_has_no_gap_where_the_lock_would_be() {
        let rows = rows();
        let drawing = row_buttons(&rows[1]);
        assert_eq!(drawing, [Btn::Delete, Btn::Lock, Btn::Show]);
        let script = row_buttons(&rows[3]);
        assert_eq!(script, [Btn::Delete, Btn::Show]);
        // The two slots a script uses are neighbours.
        let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, ROW_HEIGHT));
        let gap = button_slot(row, 0).left() - button_slot(row, 1).right();
        assert!(gap <= 2.0 + f32::EPSILON, "{gap}");
    }

    #[test]
    fn the_symbol_and_headings_have_no_buttons() {
        let symbol = Row {
            key: Key::Symbol,
            label: "BTCUSDT".to_owned(),
            detail: String::new(),
            shown: None,
            locked: None,
        };
        assert!(row_buttons(&symbol).is_empty());
    }

    #[test]
    fn a_hidden_script_shows_its_eye_next_to_the_edge_without_hover() {
        let mut rig = Rig::new();
        rig.rows[3].shown = Some(false);
        let mut rig = rig.settled();
        let key = Key::Indicator(IndicatorKey::Script(3));
        let row = rig.row(key);
        // At rest the eye is the only button: it sits in the second slot, delete's is empty.
        rig.frame(vec![Event::PointerMoved(button_slot(row, 1).center())]);
        rig.frame(Vec::new());
        rig.click(button_slot(row, 1).center(), PointerButton::Primary);
        assert_eq!(rig.actions, [TreeAction::ToggleShown(key)]);
    }

    #[test]
    fn button_slots_are_fixed_and_do_not_overlap() {
        let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, ROW_HEIGHT));
        let slots: Vec<Rect> = (0..3).map(|index| button_slot(row, index)).collect();
        assert!(slots[0].left() > slots[1].right());
        assert!(slots[1].left() > slots[2].right());
        assert!(slots.iter().all(|slot| row.contains_rect(*slot)));
    }
}
