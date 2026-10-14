//! Small building blocks shared by the Swift-styled windows: the macOS type scale with real SF
//! weights, the system palette, segmented section bars, status pills, ticker icons, menus,
//! confirmation alerts and empty states.

use crate::model::DataSource;
use crate::ui::icons::{self, Icon};
use crate::ui::{source_logo, theme};
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Galley, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, text::LayoutJob, vec2,
};
use std::sync::Arc;

// MARK: Type

/// SF Pro weights on its `wght` axis.
pub const REGULAR: f32 = 400.0;
pub const MEDIUM: f32 = 510.0;
pub const SEMIBOLD: f32 = 590.0;
pub const BOLD: f32 = 700.0;

/// The macOS text styles, in points.
pub mod size {
    pub const TITLE2: f32 = 17.0;
    pub const HEADLINE: f32 = 13.0;
    pub const BODY: f32 = 13.0;
    pub const SUBHEADLINE: f32 = 11.0;
    pub const CAPTION: f32 = 10.0;
}

pub fn text(value: impl Into<String>, size: f32, weight: f32, color: Color32) -> RichText {
    RichText::new(value)
        .size(size)
        .variation("wght", weight)
        .color(color)
}

/// A one-line galley at `weight`, cut with an ellipsis past `max_width`.
pub fn galley(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_font(
        ui,
        value,
        FontId::proportional(size),
        weight,
        color,
        max_width,
    )
}

/// A price or time. SF's tabular figures are not reachable from egui, and the monospaced face
/// reads heavier than Swift's `.monospacedDigit()`; these columns are right-aligned, so the
/// proportional face is what matches.
pub fn digits(
    ui: &Ui,
    value: &str,
    size: f32,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    galley_font(
        ui,
        value,
        FontId::proportional(size),
        weight,
        color,
        max_width,
    )
}

fn galley_font(
    ui: &Ui,
    value: &str,
    font_id: FontId,
    weight: f32,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(
        value,
        0.0,
        egui::TextFormat {
            font_id,
            color,
            coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

// MARK: Palette

/// `Color.accentColor` with the system default (blue) accent.
pub const ACCENT: Color32 = theme::SYSTEM_ACCENT;
pub const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
pub const ORANGE: Color32 = Color32::from_rgb(255, 159, 10);
pub const RED: Color32 = Color32::from_rgb(255, 69, 58);
/// `windowBackgroundColor`, behind cards and around sheets.
pub fn window() -> Color32 {
    theme::chrome()
}
/// `controlBackgroundColor`: the raised plate of a selected segment.
pub fn control() -> Color32 {
    theme::editor_background()
}
/// `.separator` at the half opacity cards use.
pub fn hairline() -> Color32 {
    theme::ink(20)
}

/// `Color.primary.opacity(x)`.
pub fn primary(opacity: f32) -> Color32 {
    theme::ink((255.0 * opacity.clamp(0.0, 1.0)).round() as u8)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Good,
    Warning,
    Bad,
    Neutral,
}

impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Self::Good => GREEN,
            Self::Warning => ORANGE,
            Self::Bad => RED,
            Self::Neutral => theme::text_secondary(),
        }
    }
}

// MARK: Pills

/// The size `paint_status_badge` draws `label` at.
pub fn status_badge_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    vec2(
        9.0 + 7.0 + 6.0 + galley.size().x + 9.0,
        galley.size().y + 8.0,
    )
}

/// A tinted capsule with a dot and a semibold caption, its right edge at `right`.
pub fn paint_status_badge(ui: &Ui, right: f32, center_y: f32, label: &str, tone: Tone) -> Rect {
    let color = tone.color();
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = status_badge_size(ui, label);
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, theme::with_alpha(color, 0.12));
    painter.circle_filled(pos2(rect.left() + 12.5, center_y), 3.5, color);
    painter.galley(
        pos2(rect.left() + 22.0, center_y - galley.size().y / 2.0),
        galley,
        color,
    );
    rect
}

/// `paint_status_badge`, laid out inline where the cursor is.
pub fn status_badge(ui: &mut Ui, label: &str, tone: Tone) -> Rect {
    let (rect, _) = ui.allocate_exact_size(status_badge_size(ui, label), Sense::hover());
    paint_status_badge(ui, rect.right(), rect.center().y, label, tone)
}

/// `paint_chip`'s size for `label`.
pub fn chip_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        MEDIUM,
        Color32::WHITE,
        f32::INFINITY,
    );
    vec2(
        8.0 + 10.0 + 5.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    )
}

/// A tinted capsule with a small bold glyph and a medium caption, right edge at `right`.
pub fn paint_chip(
    ui: &Ui,
    right: f32,
    center_y: f32,
    icon: Icon,
    label: &str,
    tint: Color32,
) -> Rect {
    let galley = galley(ui, label, size::CAPTION + 0.5, MEDIUM, tint, f32::INFINITY);
    let size = vec2(
        8.0 + 10.0 + 5.0 + galley.size().x + 8.0,
        galley.size().y + 8.0,
    );
    let rect = Rect::from_min_size(pos2(right - size.x, center_y - size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, size.y / 2.0, theme::with_alpha(tint, 0.14));
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 13.0, center_y), Vec2::splat(9.5)),
        icon,
        tint,
    );
    painter.galley(
        pos2(rect.left() + 23.0, center_y - galley.size().y / 2.0),
        galley,
        tint,
    );
    rect
}

/// A neutral count capsule ("12"), its left edge at `left`.
pub fn paint_count(
    ui: &Ui,
    left: f32,
    center_y: f32,
    count: usize,
    color: Color32,
    fill: Color32,
) -> Rect {
    let galley = digits(
        ui,
        &count.to_string(),
        size::CAPTION + 0.5,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 12.0, galley.size().y + 2.0);
    let rect = Rect::from_min_size(pos2(left, center_y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    ui.painter().galley(
        pos2(rect.left() + 6.0, center_y - galley.size().y / 2.0),
        galley,
        color,
    );
    rect
}

// MARK: Icons

/// A coin, company or market logo in a circle; a hue-seeded monogram until (or unless) the
/// shared icon cache resolves one. Crypto shares one lookup per base asset.
pub fn ticker_icon(ui: &Ui, rect: Rect, symbol: &str, source: DataSource) {
    if source == DataSource::CoinMarketCap {
        // Its charts are indices, not assets: the CoinMarketCap mark stands in for a coin logo.
        source_logo::paint(ui, source, rect);
        return;
    }
    let base: String = symbol
        .split(['/', '-', ' '])
        .next()
        .unwrap_or(symbol)
        .to_uppercase();
    let lookup = match source {
        DataSource::Binance | DataSource::Coinbase | DataSource::CoinGecko => {
            Some((DataSource::Binance, format!("{base}USDT")))
        }
        DataSource::Alpaca => Some((DataSource::Alpaca, base.clone())),
        _ => None,
    };
    if let Some((source, market)) = &lookup {
        if let Some(url) = crate::service::icon::get_url(*source, market) {
            egui::Image::new(url)
                .fit_to_exact_size(rect.size())
                .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
                .paint_at(ui, rect);
            return;
        }
        crate::service::icon::request(*source, market);
    }
    monogram(ui, rect, &base);
}

/// `ChartIconView`: the artwork for a chart's own market, resolved per (source, symbol) as the
/// chart does, clipped to a circle, with a monogram while it loads or when none exists and, when
/// `shows_source`, the data source's logo on the bottom-right corner. CoinMarketCap's charts are
/// indices, not assets: its mark stands in for the artwork and needs no second badge.
pub fn chart_icon(ui: &Ui, rect: Rect, symbol: &str, source: DataSource, shows_source: bool) {
    chart_icon_with_label(ui, rect, symbol, symbol, source, shows_source);
}

/// [`chart_icon`] with a separate human-readable fallback label. This keeps provider ids such as
/// DEX pair addresses as the lookup key while the unresolved monogram still uses the ticker.
pub fn chart_icon_with_label(
    ui: &Ui,
    rect: Rect,
    symbol: &str,
    label: &str,
    source: DataSource,
    shows_source: bool,
) {
    if source == DataSource::CoinMarketCap {
        source_logo::paint(ui, source, rect);
        return;
    }
    match crate::service::icon::get_url(source, symbol) {
        Some(url) => {
            egui::Image::new(url)
                .fit_to_exact_size(rect.size())
                .corner_radius(CornerRadius::same((rect.width() / 2.0) as u8))
                .paint_at(ui, rect);
        }
        None => {
            let display = source.display_symbol(label);
            let base = display.split(['/', '-', ' ']).next().unwrap_or(&display);
            crate::service::icon::request_with_base(source, symbol, base);
            monogram(ui, rect, base);
        }
    }
    if shows_source {
        source_badge(ui, rect, source);
    }
}

/// `TickerIconView`'s fallback: two letters on a hue derived from the symbol.
pub fn monogram(ui: &Ui, rect: Rect, symbol: &str) {
    let seed = symbol
        .to_uppercase()
        .chars()
        .fold(0u32, |sum, c| sum.wrapping_add((c as u32).wrapping_mul(31)));
    let fill: Color32 = egui::ecolor::Hsva::new((seed % 360) as f32 / 360.0, 0.55, 0.7, 1.0).into();
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, fill);
    let initials: String = symbol
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let label = galley(
        ui,
        &initials,
        rect.width() * 0.4,
        BOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    ui.painter()
        .galley(rect.center() - label.size() / 2.0, label, Color32::WHITE);
}

/// The source's logo badged half outside an icon's bottom-right corner, ringed in the window
/// colour so it reads against any artwork.
pub fn source_badge(ui: &Ui, icon: Rect, source: DataSource) {
    let side = (icon.width() * 0.55).round();
    let center = icon.right_bottom() - Vec2::splat(side * 0.5) + Vec2::splat(side * 0.3);
    let rect = Rect::from_center_size(center, Vec2::splat(side));
    ui.painter()
        .rect_filled(rect.expand(1.0), side * 0.3, window());
    source_logo::paint(ui, source, rect);
}

// MARK: Controls

pub struct TabItem<T> {
    pub value: T,
    pub title: &'static str,
    pub icon: Icon,
    pub count: usize,
}

/// `IconTabBar` (compact, filling the width): a pill track of equal segments, each an icon, a
/// title and an optional count, the selected one raised on a darker plate.
pub fn tab_bar<T: Copy + PartialEq>(ui: &mut Ui, id: Id, items: &[TabItem<T>], selected: &mut T) {
    tab_bar_impl(ui, id, items, selected, true);
}

/// The compact `IconTabBar` that hugs its titles instead of filling the row, as the Paper
/// Trading panel's section tabs do in Swift.
#[allow(dead_code)] // for the paper panel's tabs
pub fn tab_bar_compact<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
) {
    tab_bar_impl(ui, id, items, selected, false);
}

/// The regular `IconTabBar` (14 pt titles, 16 pt of padding) that hugs its titles, as the chart
/// settings sheet draws it; the sheet centres it.
pub fn tab_bar_regular<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
) {
    tab_bar_with(ui, id, items, selected, false, Density::Regular);
}

/// The two `IconTabBar` sizes: Swift's `isCompact` flag.
#[derive(Clone, Copy, PartialEq)]
enum Density {
    Compact,
    Regular,
}

impl Density {
    fn font(self) -> f32 {
        match self {
            Self::Compact => 12.5,
            Self::Regular => 14.0,
        }
    }

    fn glyph(self) -> f32 {
        match self {
            Self::Compact => 12.0,
            Self::Regular => 14.0,
        }
    }

    fn pad(self) -> f32 {
        match self {
            Self::Compact => 11.0,
            Self::Regular => 16.0,
        }
    }

    fn gap(self) -> f32 {
        match self {
            Self::Compact => 5.0,
            Self::Regular => 7.0,
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Compact => 28.0,
            Self::Regular => 34.0,
        }
    }
}

/// Widths for a bar that fills `available`: equal segments, like Swift's `fillsWidth`, unless
/// one would be too narrow for its title, in which case every segment keeps its natural width
/// and shares what is left equally, so no title is ever cut.
fn fill_widths(natural: &[f32], available: f32) -> Vec<f32> {
    let count = natural.len().max(1) as f32;
    let equal = available / count;
    if natural.iter().all(|width| *width <= equal) {
        return vec![equal; natural.len()];
    }
    let spare = (available - natural.iter().sum::<f32>()).max(0.0) / count;
    natural.iter().map(|width| width + spare).collect()
}

/// How wide a hugging segment is: padding, glyph, title and count capsule.
fn segment_width<T: Copy + PartialEq>(
    ui: &Ui,
    item: &TabItem<T>,
    active: bool,
    density: Density,
) -> f32 {
    let weight = if active { SEMIBOLD } else { MEDIUM };
    let title = galley(
        ui,
        item.title,
        density.font(),
        weight,
        Color32::WHITE,
        f32::INFINITY,
    );
    let count = (item.count > 0).then(|| {
        let text = item.count.to_string();
        digits(
            ui,
            &text,
            size::CAPTION + 0.5,
            SEMIBOLD,
            Color32::WHITE,
            f32::INFINITY,
        )
    });
    let count_width = count.map_or(0.0, |count| 5.0 + count.size().x + 12.0);
    density.pad() + density.glyph() + density.gap() + title.size().x + count_width + density.pad()
}

fn tab_bar_impl<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
    fills: bool,
) {
    tab_bar_with(ui, id, items, selected, fills, Density::Compact);
}

fn tab_bar_with<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    items: &[TabItem<T>],
    selected: &mut T,
    fills: bool,
    density: Density,
) {
    let height = density.height();
    let widths: Vec<f32> = items
        .iter()
        .map(|item| segment_width(ui, item, item.value == *selected, density))
        .collect();
    let track_width = if fills {
        ui.available_width()
    } else {
        widths.iter().sum::<f32>() + 2.0 * (items.len() - 1) as f32 + 6.0
    };
    let (track, _) = ui.allocate_exact_size(vec2(track_width, height + 6.0), Sense::hover());
    ui.painter()
        .rect_filled(track, 11.0, theme::quaternary(0.6));
    ui.painter().rect_stroke(
        track,
        11.0,
        Stroke::new(1.0, hairline()),
        StrokeKind::Inside,
    );
    let segment_widths = if fills {
        fill_widths(
            &widths,
            track.width() - 6.0 - 2.0 * (items.len() - 1) as f32,
        )
    } else {
        widths.clone()
    };
    let mut left = track.left() + 3.0;
    for (index, item) in items.iter().enumerate() {
        let width = segment_widths[index];
        let segment = Rect::from_min_size(pos2(left, track.top() + 3.0), vec2(width, height));
        left += width + 2.0;
        let response = ui
            .interact(segment, id.with(index), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, item.title);
        let active = item.value == *selected;
        let painter = ui.painter();
        if active {
            painter.add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                }
                .as_shape(segment, 8.0),
            );
            painter.rect_filled(segment, 8.0, control());
            painter.rect_stroke(
                segment,
                8.0,
                Stroke::new(1.0, hairline()),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, 8.0, primary(0.04));
        }
        let title_color = if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let title = galley(
            ui,
            item.title,
            density.font(),
            if active { SEMIBOLD } else { MEDIUM },
            title_color,
            // The segment's room less its padding, glyph and gap; the count capsule is measured
            // into the segment, so it never squeezes the title of a hugging bar.
            (width - 2.0 * density.pad() - density.glyph() - density.gap()).max(24.0),
        );
        let count = (item.count > 0).then(|| {
            digits(
                ui,
                &item.count.to_string(),
                size::CAPTION + 0.5,
                SEMIBOLD,
                if active {
                    ACCENT
                } else {
                    theme::text_secondary()
                },
                f32::INFINITY,
            )
        });
        let count_width = count
            .as_ref()
            .map_or(0.0, |count| 5.0 + count.size().x + 12.0);
        let glyph = density.glyph();
        let content = glyph + density.gap() + title.size().x + count_width;
        let mut x = segment.center().x - content / 2.0;
        let y = segment.center().y;
        icons::paint(
            painter,
            Rect::from_min_size(pos2(x, y - glyph / 2.0), Vec2::splat(glyph)),
            item.icon,
            if active {
                ACCENT
            } else {
                theme::text_secondary()
            },
        );
        x += glyph + density.gap();
        let title_width = title.size().x;
        painter.galley(pos2(x, y - title.size().y / 2.0), title, title_color);
        x += title_width + 5.0;
        if let Some(count) = count {
            let capsule = Rect::from_min_size(
                pos2(x, y - (count.size().y + 2.0) / 2.0),
                vec2(count.size().x + 12.0, count.size().y + 2.0),
            );
            painter.rect_filled(
                capsule,
                capsule.height() / 2.0,
                theme::with_alpha(
                    if active {
                        ACCENT
                    } else {
                        theme::text_secondary()
                    },
                    0.15,
                ),
            );
            painter.galley(
                pos2(capsule.left() + 6.0, y - count.size().y / 2.0),
                count,
                Color32::WHITE,
            );
        }
        if response.clicked() {
            *selected = item.value;
        }
    }
}

/// `PortfolioSearchField`: a compact rounded field with a magnifier and a clear button.
pub fn search_field(ui: &mut Ui, id: Id, query: &mut String, prompt: &str, width: f32) {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, 7.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(ACCENT, 0.7)
            } else {
                primary(0.1)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 6.0;
    icons::inline(&mut inner, Icon::Magnifier, 12.0, theme::text_secondary());
    let clear = if query.is_empty() { 0.0 } else { 22.0 };
    let response = inner.add(
        egui::TextEdit::singleline(query)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(size::BODY))
            .text_color(theme::text_primary())
            .hint_text(text(prompt, size::BODY, REGULAR, theme::text_tertiary()))
            .desired_width((inner.available_width() - clear).max(40.0)),
    );
    if !query.is_empty()
        && icons::button(
            &mut inner,
            Icon::XmarkCircleFill,
            18.0,
            theme::text_secondary(),
            "Clear search",
        )
        .clicked()
    {
        query.clear();
        response.request_focus();
    }
}

/// An icon button that is drawn only while `shown` (the row is hovered), like the hover actions
/// on a card; returns whether it was clicked.
pub fn hover_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    icon: Icon,
    tooltip: &str,
    shown: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let response = crate::ui::accessibility::label(response, tooltip);
    if shown || response.hovered() {
        let painter = ui.painter();
        if response.hovered() && enabled {
            painter.rect_filled(
                rect,
                6.0,
                primary(if response.is_pointer_button_down_on() {
                    0.12
                } else {
                    0.07
                }),
            );
        }
        icons::paint(
            painter,
            Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
            icon,
            if enabled {
                theme::text_secondary()
            } else {
                theme::text_tertiary()
            },
        );
    }
    if shown {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

/// A row of a pop-up menu: an icon, a label, the system highlight on hover. Returns whether it
/// was clicked.
pub fn menu_item(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, destructive: bool) -> bool {
    menu_item_with_shortcut(ui, icon, label, None, enabled, destructive)
}

/// A menu row with a keyboard shortcut hint (like `⌘S`) right-aligned.
pub fn menu_item_with_shortcut(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    destructive: bool,
) -> bool {
    let width = ui.available_width();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let highlighted = enabled && response.hovered();
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, 5.0, ACCENT);
    }
    let color = match (enabled, highlighted, destructive) {
        (false, _, _) => theme::text_tertiary(),
        (true, true, _) => Color32::WHITE,
        (true, false, true) => RED,
        (true, false, false) => theme::text_primary(),
    };
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(12.0)),
        icon,
        color,
    );
    let label = galley(ui, label, size::BODY, REGULAR, color, width - 40.0);
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if let Some(shortcut) = shortcut {
        let hint_color = if highlighted {
            Color32::WHITE.gamma_multiply(0.8)
        } else {
            theme::text_tertiary()
        };
        let hint = galley(ui, shortcut, size::BODY, REGULAR, hint_color, width);
        painter.galley(
            pos2(
                rect.right() - 10.0 - hint.size().x,
                rect.center().y - hint.size().y / 2.0,
            ),
            hint,
            hint_color,
        );
    }
    enabled && response.clicked()
}

/// A menu row whose leading glyph is a check shown only on the chosen item.
pub fn menu_item_marked(ui: &mut Ui, icon: Icon, label: &str, marked: bool) -> bool {
    menu_item_marked_response(ui, icon, label, marked).0
}

/// `menu_item_marked`, also handing back the row's response.
fn menu_item_marked_response(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    marked: bool,
) -> (bool, egui::Response) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let highlighted = response.hovered();
    if highlighted {
        ui.painter().rect_filled(rect, 5.0, ACCENT);
    }
    let color = if highlighted {
        Color32::WHITE
    } else {
        theme::text_primary()
    };
    if marked {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            icon,
            color,
        );
    }
    let label = galley(ui, label, size::BODY, REGULAR, color, width - 40.0);
    ui.painter().galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    (response.clicked(), response)
}

/// A thin divider between groups of menu items.
pub fn menu_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        rect.shrink2(vec2(8.0, 0.0)).x_range(),
        rect.center().y,
        Stroke::new(1.0, hairline()),
    );
}

/// What menus and popovers are filled with.
pub fn menu_fill() -> Color32 {
    theme::pick(
        Color32::from_rgb(42, 50, 55),
        Color32::from_rgb(250, 250, 251),
    )
}

/// The frame menus pop up in.
pub fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(menu_fill())
        .corner_radius(9.0)
        .stroke(Stroke::new(1.0, primary(0.1)))
        .inner_margin(egui::Margin::same(5))
        .shadow(egui::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
}

/// A bordered push button with an optional leading icon, at the regular control size.
pub fn push_button(ui: &mut Ui, icon: Option<Icon>, label: &str, enabled: bool) -> egui::Response {
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let label_galley = galley(ui, label, size::BODY, REGULAR, color, f32::INFINITY);
    let icon_width = if icon.is_some() { 17.0 } else { 0.0 };
    let size = vec2(label_galley.size().x + icon_width + 22.0, 24.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = crate::ui::accessibility::label(response, label);
    let fill = primary(
        match (
            enabled,
            response.is_pointer_button_down_on(),
            response.hovered(),
        ) {
            (false, _, _) => 0.05,
            (true, true, _) => 0.2,
            (true, false, true) => 0.14,
            _ => 0.1,
        },
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, fill);
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, primary(0.06)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 11.0;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_min_size(pos2(x, rect.center().y - 6.0), Vec2::splat(12.0)),
            icon,
            color,
        );
        x += icon_width;
    }
    painter.galley(
        pos2(x, rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        color,
    );
    response
}

/// How a sheet-footer button looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FooterButton {
    /// Cancel and the like: a quiet bordered button.
    Secondary,
    /// The sheet's main action, filled with the given tint.
    Prominent(Color32),
}

/// A sheet-footer button at the large control size (28 pt). `min_width` keeps a primary action
/// from shrinking to its label.
pub fn footer_button(
    ui: &mut Ui,
    label: &str,
    kind: FooterButton,
    enabled: bool,
    min_width: f32,
) -> egui::Response {
    let text_color = match (kind, enabled) {
        (FooterButton::Prominent(_), true) => Color32::WHITE,
        (FooterButton::Prominent(_), false) => Color32::WHITE.gamma_multiply(0.7),
        (FooterButton::Secondary, true) => theme::text_primary(),
        (FooterButton::Secondary, false) => theme::text_tertiary(),
    };
    let weight = match kind {
        FooterButton::Prominent(_) => SEMIBOLD,
        FooterButton::Secondary => REGULAR,
    };
    let text = galley(ui, label, size::BODY, weight, text_color, 400.0);
    let width = (text.size().x + 28.0).max(min_width);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = enabled && response.is_pointer_button_down_on();
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    match kind {
        FooterButton::Prominent(tint) => {
            let fill = if !enabled {
                tint.gamma_multiply(0.4)
            } else if pressed {
                tint.gamma_multiply(0.8)
            } else if hovered {
                tint.gamma_multiply(0.92)
            } else {
                tint
            };
            painter.rect_filled(rect, 7.0, fill);
        }
        FooterButton::Secondary => {
            let fill = primary(match (enabled, pressed, hovered) {
                (false, _, _) => 0.05,
                (true, true, _) => 0.2,
                (true, false, true) => 0.14,
                _ => 0.1,
            });
            painter.rect_filled(rect, 7.0, fill);
            painter.rect_stroke(
                rect,
                7.0,
                Stroke::new(1.0, primary(0.1)),
                StrokeKind::Inside,
            );
        }
    }
    painter.galley(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        Color32::PLACEHOLDER,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

// MARK: Pages

/// `ContentUnavailableView`: a large glyph, a title and a description, centred in `rect`.
pub fn empty_state(ui: &Ui, rect: Rect, icon: Icon, title: &str, detail: &str) {
    let painter = ui.painter();
    let width = (rect.width() - 60.0).max(80.0);
    let title = galley(ui, title, size::TITLE2, BOLD, theme::text_primary(), width);
    let mut job = LayoutJob::default();
    job.append(
        detail,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(size::BODY),
            color: theme::text_secondary(),
            ..Default::default()
        },
    );
    job.wrap.max_width = width.min(380.0);
    job.halign = egui::Align::Center;
    let detail = painter.layout_job(job);
    let total = 38.0 + 14.0 + title.size().y + 6.0 + detail.size().y;
    let mut y = rect.center().y - total / 2.0;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.center().x, y + 19.0), Vec2::splat(38.0)),
        icon,
        theme::text_secondary(),
    );
    y += 52.0;
    painter.galley(
        pos2(rect.center().x - title.size().x / 2.0, y),
        title.clone(),
        theme::text_primary(),
    );
    y += title.size().y + 6.0;
    painter.galley(pos2(rect.center().x, y), detail, theme::text_secondary());
}

/// `AlertFooterBar`: a hairline, then a note on the left and the page's action on the right.
/// Returns whether the action was clicked.
pub fn footer_bar(
    ui: &mut Ui,
    icon: Icon,
    note: &str,
    action_icon: Icon,
    action: &str,
    enabled: bool,
    tooltip: &str,
) -> bool {
    footer_bar_with(ui, icon, note, Some(action_icon), action, enabled, tooltip)
}

/// `footer_bar` whose action may go without a glyph, like the Paper Trading panel's Close All.
pub fn footer_bar_with(
    ui: &mut Ui,
    icon: Icon,
    note: &str,
    action_icon: Option<Icon>,
    action: &str,
    enabled: bool,
    tooltip: &str,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 45.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, window());
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        Stroke::new(1.0, theme::separator()),
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(20.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let clicked = push_button(&mut row, action_icon, action, enabled)
        .on_hover_text(tooltip)
        .clicked();
    let note_rect = Rect::from_min_max(
        pos2(rect.left() + 20.0, rect.top()),
        pos2(row.min_rect().left() - 12.0, rect.bottom()),
    );
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(note_rect.left(), rect.center().y - 5.5),
            Vec2::splat(11.0),
        ),
        icon,
        theme::text_secondary(),
    );
    let note = galley(
        ui,
        note,
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        note_rect.width() - 18.0,
    );
    ui.painter().galley(
        pos2(
            note_rect.left() + 17.0,
            rect.center().y - note.size().y / 2.0,
        ),
        note,
        theme::text_secondary(),
    );
    clicked
}

/// A tinted callout: a glyph, a semibold title, a detail line and an optional trailing button.
/// Returns whether the button was clicked.
pub fn notice_card(
    ui: &mut Ui,
    icon: Icon,
    tint: Color32,
    title: &str,
    detail: Option<&str>,
    action: Option<&str>,
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.08))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::with_alpha(tint, 0.25)))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                icons::inline(ui, icon, 14.0, tint);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(text(
                        title,
                        size::SUBHEADLINE + 1.0,
                        SEMIBOLD,
                        theme::text_primary(),
                    ));
                    if let Some(detail) = detail {
                        ui.add(
                            egui::Label::new(text(
                                detail,
                                size::CAPTION + 0.5,
                                REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    }
                });
                if let Some(action) = action {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked = push_button(ui, None, action, true).clicked();
                    });
                }
            });
        });
    clicked
}

/// What a confirmation alert's buttons say.
pub struct Confirm<'a> {
    pub icon: Icon,
    pub title: &'a str,
    pub message: &'a str,
    pub destructive: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmOutcome {
    Confirmed,
    Cancelled,
}

/// A macOS-style confirmation alert: a badge, a bold title, the message, then the destructive
/// action over Cancel. Escape or a click outside cancels.
pub fn confirm(ctx: &egui::Context, id: Id, confirm: &Confirm) -> Option<ConfirmOutcome> {
    alert(
        ctx,
        id,
        &Alert {
            icon: confirm.icon,
            tint: RED,
            title: confirm.title,
            message: confirm.message,
            accept: confirm.destructive,
            dismiss: "Cancel",
        },
    )
}

/// What an `alert`'s badge and buttons say.
pub struct Alert<'a> {
    pub icon: Icon,
    /// The badge and the accept button.
    pub tint: Color32,
    pub title: &'a str,
    pub message: &'a str,
    pub accept: &'a str,
    pub dismiss: &'a str,
}

/// The same alert as `confirm` for a question that is not a warning (a restart to finish an
/// update): `tint` colours the badge and the accept button, and the dismiss button says
/// what it does.
pub fn alert(ctx: &egui::Context, id: Id, spec: &Alert) -> Option<ConfirmOutcome> {
    let mut outcome = None;
    let modal = egui::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(90))
        .frame(
            egui::Frame::new()
                .fill(theme::pick(
                    Color32::from_rgb(44, 52, 57),
                    Color32::from_rgb(246, 246, 247),
                ))
                .corner_radius(12.0)
                .stroke(Stroke::new(1.0, primary(0.1)))
                .inner_margin(egui::Margin::same(18))
                .shadow(egui::Shadow {
                    offset: [0, 10],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha(130),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(232.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.vertical_centered(|ui| {
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
                ui.painter()
                    .rect_filled(badge, 12.0, theme::with_alpha(spec.tint, 0.14));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(badge.center(), Vec2::splat(24.0)),
                    spec.icon,
                    spec.tint,
                );
                ui.add_space(12.0);
                ui.label(text(
                    spec.title,
                    size::HEADLINE,
                    BOLD,
                    theme::text_primary(),
                ));
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(text(
                        spec.message,
                        size::SUBHEADLINE,
                        REGULAR,
                        theme::text_primary(),
                    ))
                    .wrap()
                    .halign(egui::Align::Center),
                );
                ui.add_space(16.0);
                if alert_button(ui, spec.accept, Some(spec.tint)).clicked() {
                    outcome = Some(ConfirmOutcome::Confirmed);
                }
                ui.add_space(8.0);
                if alert_button(ui, spec.dismiss, None).clicked() {
                    outcome = Some(ConfirmOutcome::Cancelled);
                }
            });
        });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(ConfirmOutcome::Cancelled);
    }
    outcome
}

fn alert_button(ui: &mut Ui, label: &str, fill: Option<Color32>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on();
    let painter = ui.painter();
    let background = match fill {
        Some(color) if pressed => color.gamma_multiply(0.8),
        Some(color) => color,
        None => primary(if pressed {
            0.2
        } else if response.hovered() {
            0.15
        } else {
            0.1
        }),
    };
    painter.rect_filled(rect, 7.0, background);
    let galley = galley(
        ui,
        label,
        size::BODY,
        if fill.is_some() { MEDIUM } else { REGULAR },
        if fill.is_some() {
            Color32::WHITE
        } else {
            theme::text_primary()
        },
        rect.width(),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    response
}

// MARK: Form controls

/// A macOS switch at the small control size; returns whether it was flipped.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label);
    if response.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let painter = ui.painter();
    let track = egui::lerp(
        egui::Rgba::from(primary(0.18))..=egui::Rgba::from(ACCENT),
        t,
    );
    painter.rect_filled(rect, 9.0, Color32::from(track));
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    painter.circle_filled(
        pos2(x, rect.center().y + 0.5),
        7.5,
        Color32::from_black_alpha(50),
    );
    painter.circle_filled(pos2(x, rect.center().y), 7.5, Color32::WHITE);
    response
}

/// A small segmented control that hugs its labels (`Picker` with `.segmented` style).
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
) {
    segmented_with(ui, id, options, selected, None);
}

/// `segmented` stretched to `width`, as a `.segmented` picker fills a column.
pub fn segmented_filling<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
    width: f32,
) {
    segmented_with(ui, id, options, selected, Some(width));
}

fn segmented_with<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: Id,
    options: &[(T, &str)],
    selected: &mut T,
    fill: Option<f32>,
) {
    let labels: Vec<Arc<Galley>> = options
        .iter()
        .map(|(value, label)| {
            galley(
                ui,
                label,
                size::SUBHEADLINE + 0.5,
                if *value == *selected { MEDIUM } else { REGULAR },
                theme::text_primary(),
                f32::INFINITY,
            )
        })
        .collect();
    let widths: Vec<f32> = match fill {
        Some(total) => vec![(total - 4.0) / options.len().max(1) as f32; options.len()],
        None => labels.iter().map(|label| label.size().x + 20.0).collect(),
    };
    let total = widths.iter().sum::<f32>() + 4.0;
    let (track, _) = ui.allocate_exact_size(vec2(total, 22.0), Sense::hover());
    ui.painter().rect_filled(track, 6.0, primary(0.1));
    let mut x = track.left() + 2.0;
    for (index, ((value, label), galley)) in options.iter().zip(labels).enumerate() {
        let segment = Rect::from_min_size(pos2(x, track.top() + 2.0), vec2(widths[index], 18.0));
        let response = ui
            .interact(segment, id.with(index), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, *label);
        if *value == *selected {
            ui.painter().add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 2,
                    spread: 0,
                    color: Color32::from_black_alpha(50),
                }
                .as_shape(segment, 5.0),
            );
            ui.painter()
                .rect_filled(segment, 5.0, theme::pick(primary(0.24), Color32::WHITE));
        } else if index > 0 && options[index - 1].0 != *selected {
            ui.painter().vline(
                segment.left() - 0.5,
                (segment.top() + 4.0)..=(segment.bottom() - 4.0),
                Stroke::new(1.0, primary(0.12)),
            );
        }
        ui.painter().galley(
            segment.center() - galley.size() / 2.0,
            galley,
            theme::text_primary(),
        );
        if response.clicked() {
            *selected = *value;
        }
        x += widths[index];
    }
}

/// Widest a pull-down menu grows before its rows start to ellipsize.
const MENU_MAX_WIDTH: f32 = 360.0;
/// Tallest a pull-down menu gets before it scrolls.
const MENU_MAX_HEIGHT: f32 = 320.0;
/// A menu row's leading check gutter and trailing padding, around its label.
const MENU_ROW_INSET: f32 = 32.0 + 12.0;
/// The menu frame's margin on both sides.
const MENU_FRAME_MARGIN: f32 = 10.0;

/// The width a pull-down menu needs for `labels`: its widest row, the trigger's width at least,
/// at most `MENU_MAX_WIDTH`. The trigger can be capped narrower than its choices (a long title
/// would crowd its card), so the menu is sized by its content, not the trigger.
fn menu_width_for(ui: &Ui, labels: &[String], trigger_width: f32) -> f32 {
    let widest = labels
        .iter()
        .map(|label| {
            galley(
                ui,
                label,
                size::BODY,
                REGULAR,
                Color32::PLACEHOLDER,
                f32::INFINITY,
            )
            .size()
            .x
        })
        .fold(0.0, f32::max);
    let wanted = (widest + MENU_ROW_INSET + MENU_FRAME_MARGIN).max(trigger_width.max(72.0));
    // A trigger already wider than the cap keeps its own width.
    wanted.min(MENU_MAX_WIDTH.max(trigger_width))
}

/// `Picker` in the `.menu` style: a bordered pull-down showing the current choice with up/down
/// chevrons. Returns the index chosen from `labels` this frame.
///
/// `width` is the trigger's width; a choice too long for it ends in an ellipsis there (with the
/// whole text on hover) while the menu opens wide enough for every row, scrolling when tall.
pub fn menu_picker(
    ui: &mut Ui,
    width: f32,
    labels: &[String],
    selected: usize,
    enabled: bool,
) -> Option<usize> {
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let current = labels.get(selected).map_or("", String::as_str);
    let text = galley(ui, current, size::BODY, REGULAR, color, width - 30.0);
    let truncated = galley(ui, current, size::BODY, REGULAR, color, f32::INFINITY)
        .size()
        .x
        > width - 30.0 + 0.5;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let response = crate::ui::accessibility::label(response, current);
    let response = if truncated && !current.is_empty() {
        response.on_hover_text(current)
    } else {
        response
    };
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let fill = if !enabled {
        primary(0.05)
    } else if open || response.is_pointer_button_down_on() {
        primary(0.2)
    } else if response.hovered() {
        primary(0.14)
    } else {
        primary(0.1)
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, primary(0.06)),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 12.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronUpDown,
        if enabled {
            theme::text_secondary()
        } else {
            theme::text_tertiary()
        },
    );
    // The chosen row is scrolled into view once per opening, not every frame (that would fight
    // the user's own scrolling).
    let scrolled = popup_id.with("scrolled-to-choice");
    if !open {
        ui.data_mut(|data| data.remove::<bool>(scrolled));
    }
    let menu_width = menu_width_for(ui, labels, width);
    let mut chosen = None;
    egui::Popup::menu(&response)
        .frame(menu_frame())
        .width(menu_width)
        .show(|ui| {
            ui.set_min_width(menu_width - MENU_FRAME_MARGIN);
            egui::ScrollArea::vertical()
                .max_height(MENU_MAX_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, label) in labels.iter().enumerate() {
                        let (clicked, row) =
                            menu_item_marked_response(ui, Icon::Check, label, index == selected);
                        if index == selected
                            && !ui.data(|data| data.get_temp::<bool>(scrolled).unwrap_or(false))
                        {
                            row.scroll_to_me(Some(egui::Align::Center));
                            ui.data_mut(|data| data.insert_temp(scrolled, true));
                        }
                        if clicked {
                            chosen = Some(index);
                            ui.close();
                        }
                    }
                });
        });
    chosen
}

/// A large sheet button: the prominent one is accent-filled, the other a quiet fill.
pub fn sheet_button(ui: &mut Ui, label: &str, prominent: bool, enabled: bool) -> egui::Response {
    let text = galley(
        ui,
        label,
        size::BODY,
        if prominent { MEDIUM } else { REGULAR },
        Color32::PLACEHOLDER,
        f32::INFINITY,
    );
    let width = (text.size().x + 32.0).max(76.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() && enabled;
    let (fill, color) = match (prominent, enabled) {
        (true, true) => (
            if pressed {
                ACCENT.gamma_multiply(0.8)
            } else if hovered {
                ACCENT.gamma_multiply(1.1)
            } else {
                ACCENT
            },
            Color32::WHITE,
        ),
        (true, false) => (theme::with_alpha(ACCENT, 0.3), primary(0.45)),
        (false, _) => (
            primary(if pressed {
                0.18
            } else if hovered {
                0.14
            } else {
                0.1
            }),
            theme::text_primary(),
        ),
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter()
        .galley(rect.center() - text.size() / 2.0, text, color);
    response
}

/// The neutral partner of `tinted_button`: a quiet 32 pt button, for a sheet's Cancel.
pub fn large_button(ui: &mut Ui, label: &str) -> egui::Response {
    let text = galley(
        ui,
        label,
        size::BODY + 1.0,
        REGULAR,
        theme::text_primary(),
        f32::INFINITY,
    );
    let (rect, response) =
        ui.allocate_exact_size(vec2((text.size().x + 36.0).max(80.0), 32.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label);
    let fill = primary(if response.is_pointer_button_down_on() {
        0.18
    } else if response.hovered() {
        0.14
    } else {
        0.1
    });
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        theme::text_primary(),
    );
    response
}

/// A large (32 pt) button filled with `tint`: the order ticket's Buy / Sell. While `busy` a
/// spinner replaces the label's left margin and the button ignores clicks.
pub fn tinted_button(
    ui: &mut Ui,
    label: &str,
    tint: Color32,
    enabled: bool,
    busy: bool,
    min_width: f32,
) -> egui::Response {
    let color = if enabled || busy {
        Color32::WHITE
    } else {
        primary(0.55)
    };
    let text = galley(ui, label, size::BODY + 1.0, SEMIBOLD, color, f32::INFINITY);
    let spinner = if busy { 22.0 } else { 0.0 };
    let width = (text.size().x + 36.0 + spinner).max(min_width);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on() && enabled;
    let hovered = response.hovered() && enabled;
    let fill = if !enabled && !busy {
        theme::with_alpha(tint, 0.3)
    } else if pressed {
        tint.gamma_multiply(0.8)
    } else if hovered {
        tint.gamma_multiply(1.12)
    } else {
        tint
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    let content = text.size().x + spinner;
    let mut x = rect.center().x - content / 2.0;
    if busy {
        egui::Spinner::new()
            .size(14.0)
            .color(Color32::WHITE)
            .paint_at(
                ui,
                Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
            );
        x += spinner;
    }
    ui.painter()
        .galley(pos2(x, rect.center().y - text.size().y / 2.0), text, color);
    response
}

/// A large bordered destructive button with a leading glyph (`Button(role: .destructive)`):
/// red title and glyph on a quiet fill.
pub fn destructive_sheet_button(ui: &mut Ui, icon: Icon, label: &str) -> egui::Response {
    let text = galley(ui, label, size::BODY, REGULAR, RED, f32::INFINITY);
    let (rect, response) =
        ui.allocate_exact_size(vec2(text.size().x + 17.0 + 32.0, 28.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed {
        theme::with_alpha(RED, 0.2)
    } else if response.hovered() {
        theme::with_alpha(RED, 0.14)
    } else {
        theme::with_alpha(RED, 0.09)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::with_alpha(RED, 0.22)),
        StrokeKind::Inside,
    );
    let left = rect.center().x - (text.size().x + 17.0) / 2.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(left + 6.0, rect.center().y), Vec2::splat(12.0)),
        icon,
        RED,
    );
    ui.painter().galley(
        pos2(left + 17.0, rect.center().y - text.size().y / 2.0),
        text,
        RED,
    );
    response
}

/// A quick-value pill; the one already chosen is tinted with the accent.
pub fn value_chip(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let color = if selected {
        ACCENT
    } else {
        theme::text_secondary()
    };
    let text = galley(
        ui,
        label,
        size::CAPTION + 0.5,
        SEMIBOLD,
        color,
        f32::INFINITY,
    );
    let size = vec2(text.size().x + 20.0, text.size().y + 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if selected {
        theme::with_alpha(ACCENT, 0.14)
    } else {
        primary(if response.hovered() { 0.1 } else { 0.06 })
    };
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    ui.painter()
        .galley(rect.center() - text.size() / 2.0, text, color);
    response
}

/// The rounded well a form input sits in; `focused` rings it in the accent colour.
pub fn field_frame(focused: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .corner_radius(10.0)
        .stroke(Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(ACCENT, 0.7)
            } else {
                primary(0.12)
            },
        ))
        .inner_margin(egui::Margin::symmetric(12, 9))
}

/// A plain single-line text field at `size` and `weight`, drawn without a frame of its own.
pub fn plain_field(
    ui: &mut Ui,
    id: Id,
    text_value: &mut String,
    hint: &str,
    font_size: f32,
    weight: f32,
    width: f32,
) -> egui::Response {
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = LayoutJob::default();
        job.append(
            buffer.as_str(),
            0.0,
            egui::TextFormat {
                font_id: FontId::proportional(font_size),
                color: theme::text_primary(),
                coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
                ..Default::default()
            },
        );
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    ui.add(
        egui::TextEdit::singleline(text_value)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(FontId::proportional(font_size))
            .hint_text(text(hint, font_size, REGULAR, theme::text_tertiary()))
            .desired_width(width)
            .layouter(&mut layouter),
    )
}
/// Which chevron of a [`stepper`] was clicked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepDirection {
    Up,
    Down,
}

/// The macOS stepper: a 16×22 capsule of up/down chevrons. Returns the value after a click,
/// clamped to `range` and moved by `step`.
pub fn stepper(
    ui: &mut Ui,
    id: Id,
    value: i64,
    range: std::ops::RangeInclusive<i64>,
    step: i64,
) -> Option<i64> {
    let delta = match stepper_arrows(ui, id)? {
        StepDirection::Up => step,
        StepDirection::Down => -step,
    };
    Some(
        value
            .saturating_add(delta)
            .clamp(*range.start(), *range.end()),
    )
    .filter(|next| *next != value)
}

/// The stepper's capsule alone: reports which chevron was clicked and leaves the arithmetic to
/// the caller, for values that are not whole numbers.
pub fn stepper_arrows(ui: &mut Ui, id: Id) -> Option<StepDirection> {
    use StepDirection as Direction;
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, theme::control_fill());
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );
    let mut clicked = None;
    for (index, (icon, direction)) in [
        (Icon::ChevronUp, Direction::Up),
        (Icon::ChevronDown, Direction::Down),
    ]
    .into_iter()
    .enumerate()
    {
        let half = Rect::from_min_size(
            pos2(rect.min.x, rect.min.y + index as f32 * 11.0),
            vec2(16.0, 11.0),
        );
        let response = ui.interact(half, id.with(index), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(half.shrink(1.0), 4.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(half.center(), Vec2::splat(7.0)),
            icon,
            theme::text_primary(),
        );
        if response.clicked() {
            clicked = Some(direction);
        }
    }
    clicked
}

/// `ColorPicker` as the macOS well: a rounded swatch (over a checkerboard when it is translucent)
/// that opens [`color_picker`](crate::ui::color_picker). Returns the new colour when it changed.
pub fn color_well(
    ui: &mut Ui,
    size: Vec2,
    name: &str,
    color: Color32,
    alpha: egui::color_picker::Alpha,
) -> Option<Color32> {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = crate::ui::accessibility::label(response, name)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::ui::color_picker::paint_swatch(ui.painter(), rect, color, 6.0);
    if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, theme::ink(60)),
            StrokeKind::Inside,
        );
    }
    let mut picked = None;
    egui::Popup::from_toggle_button_response(&response)
        .frame(menu_frame().inner_margin(egui::Margin::same(12)))
        .show(|ui| {
            let mut value = color;
            if crate::ui::color_picker::show(ui, &mut value, alpha) {
                picked = Some(value);
            }
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filling_bar_is_equal_unless_a_title_would_be_cut() {
        assert_eq!(
            fill_widths(&[60.0, 70.0, 80.0], 300.0),
            [100.0, 100.0, 100.0]
        );
        // 300 / 3 = 100, but the middle title needs 120: natural widths plus an even share.
        let widths = fill_widths(&[60.0, 120.0, 80.0], 300.0);
        for (found, wanted) in widths.iter().zip([73.333, 133.333, 93.333]) {
            assert!((found - wanted).abs() < 0.01);
        }
        assert!((widths.iter().sum::<f32>() - 300.0).abs() < 1e-3);
        // Too little room for anyone: natural widths, the overflow is the caller's.
        assert_eq!(fill_widths(&[60.0, 120.0], 100.0), [60.0, 120.0]);
    }

    /// Opens a `menu_picker` with a 95-point trigger over `labels` and returns the popup's
    /// on-screen rect.
    fn open_menu(labels: &[String], selected: usize) -> Option<Rect> {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0));
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                menu_picker(ui, 95.0, labels, selected, true);
            });
            output.textures_delta.clear();
        };
        let at = pos2(40.0, 12.0);
        frame(vec![egui::Event::PointerMoved(at)]);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![button(true)]);
        frame(vec![button(false)]);
        for _ in 0..3 {
            frame(Vec::new());
        }
        ctx.memory(|memory| {
            memory
                .layer_ids()
                .filter(|layer| layer.order == egui::Order::Foreground)
                .find_map(|layer| memory.area_rect(layer.id))
        })
    }

    #[test]
    fn a_menu_picker_opens_wide_enough_for_its_longest_choice() {
        let long = "A choice with a rather long title that overflows".to_owned();
        let labels = vec!["Short".to_owned(), long.clone()];
        let menu = open_menu(&labels, 0).expect("the menu opens");
        let ctx = egui::Context::default();
        let widest = {
            let mut width = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                width = galley(
                    ui,
                    &long,
                    size::BODY,
                    REGULAR,
                    Color32::WHITE,
                    f32::INFINITY,
                )
                .size()
                .x;
            });
            output.textures_delta.clear();
            width
        };
        assert!(
            menu.width() >= widest + MENU_ROW_INSET,
            "menu {} wide for a {} wide row",
            menu.width(),
            widest
        );
        // A choice list that fits keeps to the trigger.
        let narrow = open_menu(&["A".to_owned(), "B".to_owned()], 0).expect("the menu opens");
        assert!(narrow.width() <= 100.0, "{}", narrow.width());
    }

    #[test]
    fn a_menu_never_outgrows_its_cap_and_scrolls_when_tall() {
        let labels: Vec<String> = (0..60)
            .map(|i| format!("{i} {}", "x".repeat(120)))
            .collect();
        let menu = open_menu(&labels, 30).expect("the menu opens");
        // The frame's one-point border sits outside the width the menu is given.
        assert!(menu.width() <= MENU_MAX_WIDTH + 2.5, "{}", menu.width());
        assert!(menu.height() <= MENU_MAX_HEIGHT + 20.0, "{}", menu.height());
    }
}
