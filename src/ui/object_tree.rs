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
    let hovered = response.hovered();
    let dimmed = row.shown == Some(false);
    if is_selected {
        ui.painter().rect_filled(
            rect.shrink2(vec2(4.0, 1.0)),
            5.0,
            kit::ACCENT.gamma_multiply(0.28),
        );
    } else if hovered {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(4.0, 1.0)), 5.0, theme::ink(12));
    }
    let color = if dimmed {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };

    // The buttons, right to left; they take their clicks before the row does.
    let mut right = rect.right() - 8.0;
    let mut button = |ui: &mut Ui, icon: Icon, tip: &str, tint: egui::Color32, id: &str| {
        let slot = Rect::from_center_size(
            pos2(right - BUTTON / 2.0, rect.center().y),
            vec2(BUTTON, BUTTON),
        );
        right -= BUTTON + 2.0;
        let response = ui.interact(slot, response.id.with(id), Sense::click());
        let response = accessibility::label(response, tip).on_hover_text(tip);
        let tint = if response.hovered() {
            theme::text_primary()
        } else {
            tint
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(slot.center(), vec2(13.0, 13.0)),
            icon,
            tint,
        );
        response.clicked()
    };
    let deletable = row.locked != Some(true) && !matches!(row.key, Key::Symbol);
    if deletable && hovered && button(ui, Icon::Trash, "Delete", theme::text_secondary(), "delete")
    {
        actions.push(TreeAction::Delete(row.key));
    }
    if let Some(locked) = row.locked
        && (locked || hovered)
        && let Key::Drawing(id) = row.key
        && button(
            ui,
            if locked {
                Icon::LockFill
            } else {
                Icon::LockOpen
            },
            if locked { "Unlock" } else { "Lock" },
            if locked {
                kit::ACCENT
            } else {
                theme::text_secondary()
            },
            "lock",
        )
    {
        actions.push(TreeAction::ToggleLocked(id));
    }
    if let Some(shown) = row.shown
        && (!shown || hovered)
        && button(
            ui,
            if shown { Icon::Eye } else { Icon::EyeSlash },
            if shown { "Hide" } else { "Show" },
            theme::text_secondary(),
            "shown",
        )
    {
        actions.push(TreeAction::ToggleShown(row.key));
    }

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
