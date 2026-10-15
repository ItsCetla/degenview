//! The "Link charts" popover: which parts of horizontal navigation the charts of a workspace
//! share. The toolbar owns the button; this draws what opens from it.

use crate::model::ChartSyncSettings;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

/// Width of the popover's content.
pub const WIDTH: f32 = 288.0;
const SWITCH: egui::Vec2 = vec2(32.0, 18.0);
const PADDING: f32 = 10.0;

/// What the toolbar button says, for its tooltip and its accessibility name.
pub fn summary(settings: ChartSyncSettings) -> &'static str {
    match (settings.horizontal_position, settings.horizontal_zoom) {
        (false, false) => "Link charts",
        (true, false) => "Link charts: position",
        (false, true) => "Link charts: zoom",
        (true, true) => "Link charts: position and zoom",
    }
}

/// The tooltip of the button while the layout holds a single chart.
pub const SINGLE_CHART_HINT: &str = "Available when the layout contains multiple charts.";

/// Draws the popover's content. Returns the settings when the user changed one.
///
/// `excluded` names the charts that cannot take part (price-based chart types); each is listed
/// with the reason, so a chart that does not follow is never a mystery.
pub fn show(
    ui: &mut Ui,
    current: ChartSyncSettings,
    excluded: &[String],
) -> Option<ChartSyncSettings> {
    let mut next = current;
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(PADDING);
        ui.label(kit::text(
            "Link charts",
            size::HEADLINE,
            SEMIBOLD,
            theme::text_primary(),
        ));
    });
    ui.add_space(4.0);
    option(
        ui,
        "link-position",
        "Sync horizontal position",
        "Panning one chart moves the others to the same point in time.",
        &mut next.horizontal_position,
    );
    option(
        ui,
        "link-zoom",
        "Sync zoom level",
        "Zooming one chart adjusts the others.",
        &mut next.horizontal_zoom,
    );
    kit::menu_separator(ui);
    note(ui, "Vertical scales remain independent.");
    if !excluded.is_empty() {
        note(
            ui,
            &format!(
                "Not linked: {}. Price-based chart types have no shared time.",
                excluded.join(", ")
            ),
        );
    }
    ui.add_space(4.0);
    (next != current).then_some(next)
}

/// A caption under the options.
fn note(ui: &mut Ui, text: &str) {
    let label = kit::galley(
        ui,
        text,
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        WIDTH - PADDING * 2.0,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, label.size().y + 2.0), Sense::hover());
    ui.painter().galley(
        pos2(rect.left() + PADDING, rect.top()),
        label,
        theme::text_secondary(),
    );
}

/// One switch with its title and what it does. The whole row is the control, so it can be
/// clicked anywhere and reached with the keyboard.
fn option(ui: &mut Ui, id: &'static str, title: &str, detail: &str, on: &mut bool) {
    let text_left = PADDING + SWITCH.x + 10.0;
    let text_width = WIDTH - text_left - PADDING;
    let title_galley = kit::galley(
        ui,
        title,
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        text_width,
    );
    let detail_galley = kit::galley(
        ui,
        detail,
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        text_width,
    );
    let height = 8.0 + title_galley.size().y + 2.0 + detail_galley.size().y + 8.0;
    let (rect, response) = ui.allocate_exact_size(vec2(WIDTH, height), Sense::click());
    let (name, state) = (title.to_owned(), *on);
    response
        .widget_info(move || WidgetInfo::selected(WidgetType::Checkbox, true, state, name.clone()));
    if response.clicked() {
        *on = !*on;
    }
    let row = rect.shrink2(vec2(4.0, 0.0));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(row, 6.0, kit::primary(0.06));
    }
    if response.has_focus() {
        painter.rect_stroke(row, 6.0, Stroke::new(2.0, kit::ACCENT), StrokeKind::Inside);
    }
    let progress = ui.ctx().animate_bool(Id::new(id), *on);
    let top = rect.top() + 8.0;
    let switch = Rect::from_min_size(
        pos2(
            rect.left() + PADDING,
            top + (title_galley.size().y - SWITCH.y) / 2.0,
        ),
        SWITCH,
    );
    paint_switch(ui, switch, progress);
    let title_height = title_galley.size().y;
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left() + text_left, top),
        title_galley,
        theme::text_primary(),
    );
    painter.galley(
        pos2(rect.left() + text_left, top + title_height + 2.0),
        detail_galley,
        theme::text_secondary(),
    );
}

/// The switch of [`kit::switch`], painted for a row that handles the click itself.
fn paint_switch(ui: &Ui, rect: Rect, progress: f32) {
    let track = egui::lerp(
        egui::Rgba::from(kit::primary(0.18))..=egui::Rgba::from(kit::ACCENT),
        progress,
    );
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, Color32::from(track));
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), progress);
    painter.circle_filled(
        pos2(x, rect.center().y + 0.5),
        7.5,
        Color32::from_black_alpha(50),
    );
    painter.circle_filled(pos2(x, rect.center().y), 7.5, Color32::WHITE);
}
