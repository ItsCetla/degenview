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
/// Marks a time-travelled chart: replay bar, plot border and badge.
pub const REPLAY: Color32 = Color32::from_rgb(255, 149, 0);

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
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

// MARK: Script Manager tokens (Swift dark appearance: macOS system colours, quaternary fills)

/// `Color.secondary` / `secondaryLabelColor` on a dark window.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(152, 152, 157);
/// `tertiaryLabelColor`.
pub const TEXT_TERTIARY: Color32 = Color32::from_rgb(98, 98, 102);
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(235, 235, 240);
/// `NSColor.separatorColor`.
pub const SEPARATOR: Color32 = Color32::from_rgb(58, 69, 74);
/// `NSColor.textBackgroundColor`: the editor surface.
pub const EDITOR_BACKGROUND: Color32 = Color32::from_rgb(24, 31, 34);
/// `NSColor.windowBackgroundColor`: the gutter.
pub const GUTTER_BACKGROUND: Color32 = BACKGROUND;
pub const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const WIN: Color32 = Color32::from_rgb(0x26, 0xA6, 0x9A);
pub const LOSS: Color32 = Color32::from_rgb(0xEF, 0x53, 0x50);

/// White at `alpha` (0–255): the `.quaternary` family of fills on a dark window.
pub fn white_alpha(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

/// `Color.quaternary` ≈ 0.18, `.quaternary.opacity(0.45)` ≈ 0.08 of white on dark.
pub fn quaternary(opacity: f32) -> Color32 {
    white_alpha((46.0 * opacity.clamp(0.0, 1.0)).round() as u8)
}

pub fn with_alpha(color: Color32, opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (255.0 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

/// Syntax colours: `PineSyntaxTheme.standard`, dark-appearance system colours.
pub mod syntax {
    use super::Color32;
    pub const KEYWORD: Color32 = Color32::from_rgb(0xBF, 0x5A, 0xF2);
    pub const TYPE: Color32 = Color32::from_rgb(0x5E, 0x5C, 0xE6);
    pub const NAMESPACE: Color32 = Color32::from_rgb(0x40, 0xC8, 0xE0);
    pub const FUNCTION: Color32 = Color32::from_rgb(0x0A, 0x84, 0xFF);
    pub const VARIABLE: Color32 = Color32::from_rgb(0x30, 0xD1, 0x58);
    pub const CONSTANT: Color32 = Color32::from_rgb(0x63, 0xE6, 0xE2);
    pub const NUMBER: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x0A);
    pub const STRING: Color32 = Color32::from_rgb(0xFF, 0x45, 0x3A);
    pub const COMMENT: Color32 = Color32::from_rgb(152, 152, 157);
    pub const ANNOTATION: Color32 = Color32::from_rgb(0xAC, 0x8E, 0x68);
    pub const OPERATOR: Color32 = Color32::from_rgb(0x8E, 0x8E, 0x93);
    pub const COLOR_LITERAL: Color32 = Color32::from_rgb(0xFF, 0x37, 0x5F);
}

/// macOS's default system accent (`Color.accentColor`), which the Swift Script Manager uses for
/// selection, tabs and links. The rest of the Rust app keeps its own `ACCENT`.
pub const SYSTEM_ACCENT: Color32 = Color32::from_rgb(10, 132, 255);

/// Puts the macOS system fonts (SF Pro for text, SF Mono for code) ahead of egui's bundled ones,
/// like the Swift app, which uses them everywhere. The bundled fonts stay as glyph fallbacks, and
/// a machine without the files simply keeps the defaults.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, path, family) in [
        (
            "sf-pro",
            "/System/Library/Fonts/SFNS.ttf",
            egui::FontFamily::Proportional,
        ),
        (
            "sf-mono",
            "/System/Library/Fonts/SFNSMono.ttf",
            egui::FontFamily::Monospace,
        ),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.to_owned());
        }
    }
    ctx.set_fonts(fonts);
}
