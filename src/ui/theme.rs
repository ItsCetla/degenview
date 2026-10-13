use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The app theme, as Settings ▸ Appearance stores it under [`THEME_KEY`] (Swift's `appTheme`).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum AppTheme {
    #[default]
    System,
    Light,
    Dark,
}

impl AppTheme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn title(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        }
    }
}

pub const THEME_KEY: &str = "appTheme";

/// The neutral colours that change with the theme. Accents and chart semantics (up, down,
/// replay…) stay the same in both.
#[derive(Clone, Copy, Debug)]
struct Palette {
    background: Color32,
    chrome: Color32,
    surface: Color32,
    surface_selected: Color32,
    border: Color32,
    grid: Color32,
    text_primary: Color32,
    text_secondary: Color32,
    text_tertiary: Color32,
    separator: Color32,
    editor_background: Color32,
    /// `labelColor` itself, for translucent fills: white on dark, black on light.
    ink: Color32,
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(28, 36, 40),
    chrome: Color32::from_rgb(37, 45, 49),
    surface: Color32::from_rgb(31, 40, 44),
    surface_selected: Color32::from_rgb(45, 55, 60),
    border: Color32::from_rgb(57, 68, 73),
    grid: Color32::from_rgb(48, 59, 64),
    text_primary: Color32::from_rgb(235, 235, 240),
    text_secondary: Color32::from_rgb(152, 152, 157),
    text_tertiary: Color32::from_rgb(98, 98, 102),
    separator: Color32::from_rgb(58, 69, 74),
    editor_background: Color32::from_rgb(24, 31, 34),
    ink: Color32::WHITE,
};

/// macOS's light system colours: `windowBackgroundColor`, white content, `labelColor` at the
/// secondary/tertiary opacities over white.
const LIGHT: Palette = Palette {
    background: Color32::from_rgb(246, 246, 247),
    chrome: Color32::from_rgb(236, 236, 238),
    surface: Color32::from_rgb(255, 255, 255),
    surface_selected: Color32::from_rgb(226, 228, 232),
    border: Color32::from_rgb(208, 209, 213),
    grid: Color32::from_rgb(236, 237, 240),
    text_primary: Color32::from_rgb(29, 29, 31),
    text_secondary: Color32::from_rgb(110, 110, 115),
    text_tertiary: Color32::from_rgb(170, 170, 175),
    separator: Color32::from_rgb(218, 219, 222),
    editor_background: Color32::from_rgb(255, 255, 255),
    ink: Color32::BLACK,
};

/// Whether the light palette is active. Set from the context's resolved theme every frame, so
/// System follows the Mac as it switches.
static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The theme picked in Settings, as an index into [`AppTheme::ALL`].
static CHOICE: AtomicU8 = AtomicU8::new(0);

/// The theme picked in Settings ▸ Appearance.
pub fn choice() -> AppTheme {
    AppTheme::ALL
        .get(CHOICE.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or_default()
}

fn palette() -> &'static Palette {
    if is_light() { &LIGHT } else { &DARK }
}

pub fn is_light() -> bool {
    LIGHT_ACTIVE.load(Ordering::Relaxed)
}

/// Picks `dark` or `light` for the active theme.
pub fn pick(dark: Color32, light: Color32) -> Color32 {
    if is_light() { light } else { dark }
}

pub fn background() -> Color32 {
    palette().background
}
pub fn chrome() -> Color32 {
    palette().chrome
}
pub fn surface() -> Color32 {
    palette().surface
}
pub fn surface_selected() -> Color32 {
    palette().surface_selected
}
pub fn border() -> Color32 {
    palette().border
}
pub fn grid() -> Color32 {
    palette().grid
}
/// `labelColor`.
pub fn text_primary() -> Color32 {
    palette().text_primary
}
/// `Color.secondary` / `secondaryLabelColor`.
pub fn text_secondary() -> Color32 {
    palette().text_secondary
}
/// `tertiaryLabelColor`.
pub fn text_tertiary() -> Color32 {
    palette().text_tertiary
}
/// `NSColor.separatorColor`.
pub fn separator() -> Color32 {
    palette().separator
}
/// `NSColor.textBackgroundColor`: the editor surface.
pub fn editor_background() -> Color32 {
    palette().editor_background
}
/// The raised plate under a selected segment: a lifted grey on dark, white on light.
pub fn selected_plate() -> Color32 {
    pick(Color32::from_rgb(88, 98, 104), Color32::WHITE)
}
/// Text and glyphs on [`selected_plate`].
pub fn on_selected_plate() -> Color32 {
    pick(Color32::WHITE, palette().text_primary)
}
/// A bordered push button or stepper's bezel (`controlColor`).
pub fn control_fill() -> Color32 {
    pick(Color32::from_rgb(63, 72, 77), Color32::WHITE)
}
/// `NSColor.windowBackgroundColor`: the gutter.
pub fn gutter_background() -> Color32 {
    palette().background
}

/// The editor's unsaved-change marks: a bar for added lines, a wedge for deleted ones, a blue bar
/// for modified ones. Muted so they sit beside the code rather than over it, and each pair keeps
/// its contrast on the gutter in both appearances.
pub fn editor_diff_added() -> Color32 {
    pick(
        Color32::from_rgb(63, 185, 96),
        Color32::from_rgb(30, 140, 62),
    )
}
pub fn editor_diff_deleted() -> Color32 {
    pick(
        Color32::from_rgb(236, 90, 82),
        Color32::from_rgb(207, 34, 46),
    )
}
pub fn editor_diff_modified() -> Color32 {
    pick(
        Color32::from_rgb(76, 156, 255),
        Color32::from_rgb(0, 102, 214),
    )
}
/// Row tints in the diff preview: green for current lines, red for saved ones (the modified
/// colour marks the gutter only; a modified hunk's two sides are still saved/current).
pub fn editor_diff_added_background() -> Color32 {
    with_alpha(editor_diff_added(), pick_alpha(0.16, 0.14))
}
pub fn editor_diff_deleted_background() -> Color32 {
    with_alpha(editor_diff_deleted(), pick_alpha(0.16, 0.12))
}
fn pick_alpha(dark: f32, light: f32) -> f32 {
    if is_light() { light } else { dark }
}

pub const UP: Color32 = Color32::from_rgb(23, 201, 114);
pub const DOWN: Color32 = Color32::from_rgb(245, 72, 76);
pub const ACCENT: Color32 = Color32::from_rgb(191, 82, 203);
pub const RSI: Color32 = Color32::from_rgb(160, 130, 255);
pub const EMA: Color32 = Color32::from_rgb(255, 165, 0);
pub const BOLLINGER: Color32 = Color32::from_rgb(0, 200, 255);
/// Marks a time-travelled chart: replay bar, plot border and badge.
pub const REPLAY: Color32 = Color32::from_rgb(255, 149, 0);

pub fn install(ctx: &egui::Context, theme: AppTheme) {
    install_fonts(ctx);
    for (egui_theme, palette) in [(egui::Theme::Dark, &DARK), (egui::Theme::Light, &LIGHT)] {
        ctx.set_visuals_of(egui_theme, visuals(egui_theme, palette));
        ctx.style_mut_of(egui_theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(9.0, 5.0);
            style.spacing.interact_size.y = 26.0;
            style.visuals.widgets.inactive.corner_radius = 6.0.into();
            style.visuals.widgets.hovered.corner_radius = 6.0.into();
            style.visuals.widgets.active.corner_radius = 6.0.into();
        });
    }
    apply(ctx, theme);
}

/// Switches to `theme`; System follows the Mac's appearance.
pub fn apply(ctx: &egui::Context, theme: AppTheme) {
    let index = AppTheme::ALL.iter().position(|each| *each == theme);
    CHOICE.store(index.unwrap_or_default() as u8, Ordering::Relaxed);
    ctx.set_theme(theme.preference());
    sync(ctx);
}

/// Points the palette at the context's resolved theme. Called at the top of every frame.
pub fn sync(ctx: &egui::Context) {
    LIGHT_ACTIVE.store(ctx.theme() == egui::Theme::Light, Ordering::Relaxed);
}

fn visuals(theme: egui::Theme, palette: &Palette) -> egui::Visuals {
    let light = theme == egui::Theme::Light;
    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let shade = |dark: (u8, u8, u8), light_rgb: (u8, u8, u8)| {
        let (r, g, b) = if light { light_rgb } else { dark };
        Color32::from_rgb(r, g, b)
    };
    visuals.override_text_color = Some(palette.text_primary);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.chrome;
    visuals.window_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.extreme_bg_color = palette.editor_background;
    visuals.faint_bg_color = palette.surface;
    visuals.selection.bg_fill = if light {
        ACCENT.gamma_multiply(0.35)
    } else {
        ACCENT
    };
    visuals.selection.stroke.color = if light { ACCENT } else { Color32::WHITE };
    visuals.hyperlink_color = SYSTEM_ACCENT;
    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke.color = palette.text_primary;
    visuals.widgets.inactive.bg_fill = shade((51, 61, 66), (228, 229, 232));
    visuals.widgets.inactive.weak_bg_fill = shade((51, 61, 66), (228, 229, 232));
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, palette.border);
    visuals.widgets.inactive.fg_stroke.color = palette.text_primary;
    visuals.widgets.hovered.bg_fill = shade((65, 76, 81), (218, 219, 223));
    visuals.widgets.hovered.weak_bg_fill = shade((65, 76, 81), (218, 219, 223));
    visuals.widgets.hovered.fg_stroke.color = palette.text_primary;
    visuals.widgets.active.bg_fill = shade((77, 88, 94), (206, 208, 212));
    visuals.widgets.active.weak_bg_fill = shade((77, 88, 94), (206, 208, 212));
    visuals.widgets.active.fg_stroke.color = palette.text_primary;
    visuals.window_corner_radius = 8.0.into();
    visuals
}

// MARK: Script Manager tokens (Swift dark appearance: macOS system colours, quaternary fills)

pub const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const WIN: Color32 = Color32::from_rgb(0x26, 0xA6, 0x9A);
pub const LOSS: Color32 = Color32::from_rgb(0xEF, 0x53, 0x50);

/// The label colour at `alpha` (0–255): white on dark, black on light. The `.quaternary`
/// family of fills and hover washes.
pub fn ink(alpha: u8) -> Color32 {
    let ink = palette().ink;
    Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), alpha)
}

/// `Color.quaternary` ≈ 0.18, `.quaternary.opacity(0.45)` ≈ 0.08 of the label colour (light
/// windows need a touch less to read the same).
pub fn quaternary(opacity: f32) -> Color32 {
    let full = if is_light() { 30.0 } else { 46.0 };
    ink((full * opacity.clamp(0.0, 1.0)).round() as u8)
}

pub fn with_alpha(color: Color32, opacity: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (255.0 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

/// Syntax colours: `PineSyntaxTheme.standard`, the system colours of the active appearance.
pub mod syntax {
    use super::{Color32, pick};

    macro_rules! system {
        ($($(#[$doc:meta])* $name:ident: $dark:expr, $light:expr;)*) => {$(
            $(#[$doc])*
            pub fn $name() -> Color32 {
                let (dark, light): ((u8, u8, u8), (u8, u8, u8)) = ($dark, $light);
                pick(
                    Color32::from_rgb(dark.0, dark.1, dark.2),
                    Color32::from_rgb(light.0, light.1, light.2),
                )
            }
        )*};
    }

    system! {
        /// `systemPurple`.
        keyword: (0xBF, 0x5A, 0xF2), (0xAF, 0x52, 0xDE);
        /// `systemIndigo`.
        r#type: (0x5E, 0x5C, 0xE6), (0x58, 0x56, 0xD6);
        /// `systemTeal`.
        namespace: (0x40, 0xC8, 0xE0), (0x30, 0xB0, 0xC7);
        /// `systemBlue`.
        function: (0x0A, 0x84, 0xFF), (0x00, 0x7A, 0xFF);
        /// `systemGreen`.
        variable: (0x30, 0xD1, 0x58), (0x28, 0xA7, 0x45);
        /// `systemMint`.
        constant: (0x63, 0xE6, 0xE2), (0x00, 0xA8, 0xA0);
        /// `systemOrange`.
        number: (0xFF, 0x9F, 0x0A), (0xE8, 0x80, 0x00);
        /// `systemRed`.
        string: (0xFF, 0x45, 0x3A), (0xFF, 0x3B, 0x30);
        /// `secondaryLabelColor`.
        comment: (152, 152, 157), (110, 110, 115);
        /// `systemBrown`.
        annotation: (0xAC, 0x8E, 0x68), (0xA2, 0x84, 0x5E);
        /// `systemGray`.
        operator: (0x8E, 0x8E, 0x93), (0x8E, 0x8E, 0x93);
        /// `systemPink`.
        color_literal: (0xFF, 0x37, 0x5F), (0xFF, 0x2D, 0x55);
    }
}

/// macOS's default system accent (`Color.accentColor`), which the Swift Script Manager uses for
/// selection, tabs and links. The rest of the Rust app keeps its own `ACCENT`.
pub const SYSTEM_ACCENT: Color32 = Color32::from_rgb(10, 132, 255);

/// Puts the macOS system fonts (SF Pro for text, SF Mono for code) ahead of egui's bundled ones,
/// like the Swift app, which uses them everywhere. The bundled fonts stay as glyph fallbacks, and
/// a machine without the files simply keeps the defaults. STIX Two Math and Apple Symbols go last
/// in both families so characters the others lack — "𝓑𝓾𝔂" and other math alphanumerics, arrows,
/// geometric shapes — draw instead of showing tofu squares.
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
    for (name, path) in [
        (
            "stix-math",
            "/System/Library/Fonts/Supplemental/STIXTwoMath.otf",
        ),
        ("apple-symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_alphanumerics_have_a_fallback_glyph_in_both_families() {
        if !std::path::Path::new("/System/Library/Fonts/Supplemental/STIXTwoMath.otf").exists() {
            return;
        }
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let font = egui::FontId::new(14.0, family);
            let width = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '𝓑'));
            let missing = ctx.fonts_mut(|fonts| fonts.glyph_width(&font, '\u{10FFFF}'));
            assert!(width > 0.0 && width != missing);
        }
    }

    #[test]
    fn the_theme_round_trips_under_the_swift_key_and_names() {
        assert_eq!(THEME_KEY, "appTheme");
        for (theme, raw) in AppTheme::ALL
            .into_iter()
            .zip(["\"System\"", "\"Light\"", "\"Dark\""])
        {
            assert_eq!(serde_json::to_string(&theme).unwrap(), raw);
            assert_eq!(serde_json::from_str::<AppTheme>(raw).unwrap(), theme);
        }
        assert_eq!(AppTheme::default(), AppTheme::System);
    }

    #[test]
    fn the_palette_follows_the_resolved_theme() {
        let ctx = egui::Context::default();
        install(&ctx, AppTheme::Light);
        assert!(is_light());
        assert_eq!(choice(), AppTheme::Light);
        assert_eq!(surface(), LIGHT.surface);
        assert_eq!(ink(255), Color32::BLACK);
        apply(&ctx, AppTheme::Dark);
        assert!(!is_light());
        assert_eq!(choice(), AppTheme::Dark);
        assert_eq!(text_primary(), DARK.text_primary);
        assert_eq!(ink(255), Color32::WHITE);
    }
}
