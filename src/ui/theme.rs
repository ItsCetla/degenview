use eframe::egui::{self, Color32};

pub const BACKGROUND: Color32 = Color32::from_rgb(28, 36, 40);
pub const CHROME: Color32 = Color32::from_rgb(37, 45, 49);
pub const SURFACE: Color32 = Color32::from_rgb(31, 40, 44);
pub const SURFACE_SELECTED: Color32 = Color32::from_rgb(45, 55, 60);
pub const BORDER: Color32 = Color32::from_rgb(57, 68, 73);
pub const GRID: Color32 = Color32::from_rgb(48, 59, 64);
pub const UP: Color32 = Color32::from_rgb(23, 201, 114);
pub const DOWN: Color32 = Color32::from_rgb(245, 72, 76);
pub const ACCENT: Color32 = Color32::from_rgb(191, 82, 203);
pub const BITCOIN: Color32 = Color32::from_rgb(247, 147, 26);
pub const RSI: Color32 = Color32::from_rgb(160, 130, 255);
pub const EMA: Color32 = Color32::from_rgb(255, 165, 0);
pub const BOLLINGER: Color32 = Color32::from_rgb(0, 200, 255);

pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CHROME;
    visuals.extreme_bg_color = Color32::from_rgb(24, 31, 34);
    visuals.faint_bg_color = SURFACE;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke.color = Color32::WHITE;
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(51, 61, 66);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(65, 76, 81);
    visuals.widgets.active.bg_fill = Color32::from_rgb(77, 88, 94);
    visuals.window_corner_radius = 8.0.into();
    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.visuals.widgets.inactive.corner_radius = 6.0.into();
        style.visuals.widgets.hovered.corner_radius = 6.0.into();
        style.visuals.widgets.active.corner_radius = 6.0.into();
    });
}
