//! Per-instance style overrides: a typed view over a chart script's `styleOverrides` dictionary,
//! ported from `View/PineStyleOverrides.swift` + `View/PineStyleRow.swift`. Pure — no AppKit or UI.
//!
//! Keys are `"<kind>.<siteId>.<attribute>"`. `visible` is stored only as `"false"`, `color` as an
//! 8-digit RRGGBBAA hex string and `width` as an integer. A missing key means "as the script has
//! it", and a key that matches no output is ignored.

use super::outputs::{ColorOutput, DISPLAY_PANE, MarkerKind, PlotOutput, PlotStyle};
use super::program::VisualOutput;
use std::collections::HashMap;

/// The kinds of script output the Style section lists.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleKind {
    Plot,
    Hline,
    Fill,
    Marker,
    Candle,
    Bgcolor,
    Barcolor,
}

impl StyleKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Plot => "plot",
            Self::Hline => "hline",
            Self::Fill => "fill",
            Self::Marker => "marker",
            Self::Candle => "candle",
            Self::Bgcolor => "bgcolor",
            Self::Barcolor => "barcolor",
        }
    }
}

/// What a user can change about one output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleAttribute {
    Visible,
    Color,
    Width,
}

impl StyleAttribute {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Color => "color",
            Self::Width => "width",
        }
    }
}

/// A typed view over a chart script's style-override dictionary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyleOverrides {
    raw: HashMap<String, String>,
}

impl StyleOverrides {
    pub fn new(raw: HashMap<String, String>) -> Self {
        Self { raw }
    }

    pub fn raw(&self) -> &HashMap<String, String> {
        &self.raw
    }

    pub fn key(kind: StyleKind, id: usize, attribute: StyleAttribute) -> String {
        format!("{}.{}.{}", kind.raw_value(), id, attribute.raw_value())
    }

    pub fn is_hidden(&self, kind: StyleKind, id: usize) -> bool {
        self.raw
            .get(&Self::key(kind, id, StyleAttribute::Visible))
            .is_some_and(|value| value == "false")
    }

    pub fn color(&self, kind: StyleKind, id: usize) -> Option<u32> {
        self.raw
            .get(&Self::key(kind, id, StyleAttribute::Color))
            .and_then(|value| u32::from_str_radix(value, 16).ok())
    }

    pub fn width(&self, kind: StyleKind, id: usize) -> Option<usize> {
        self.raw
            .get(&Self::key(kind, id, StyleAttribute::Width))
            .and_then(|value| value.parse::<usize>().ok())
            .map(|width| width.clamp(1, 20))
    }

    /// `raw` with one attribute set, or removed when `value` is `None`.
    pub fn setting(
        mut raw: HashMap<String, String>,
        kind: StyleKind,
        id: usize,
        attribute: StyleAttribute,
        value: Option<String>,
    ) -> HashMap<String, String> {
        let key = Self::key(kind, id, attribute);
        match value {
            Some(value) => {
                raw.insert(key, value);
            }
            None => {
                raw.remove(&key);
            }
        }
        raw
    }

    /// Whether the user changed anything about `row`.
    pub fn has_changes(&self, row: &StyleRow) -> bool {
        let prefix = row.id() + ".";
        self.raw.keys().any(|key| key.starts_with(&prefix))
    }

    /// `raw` without any choice about `row`.
    pub fn cleared(raw: HashMap<String, String>, row: &StyleRow) -> HashMap<String, String> {
        let prefix = row.id() + ".";
        raw.into_iter()
            .filter(|(key, _)| !key.starts_with(&prefix))
            .collect()
    }

    /// An RRGGBBAA color as an 8-digit hex string.
    pub fn hex(rgba: u32) -> String {
        format!("{rgba:08X}")
    }

    /// The overrides that still belong to one of `rows`; the rest are left over from an edited
    /// script.
    pub fn pruned(raw: HashMap<String, String>, rows: &[StyleRow]) -> HashMap<String, String> {
        let prefixes: Vec<String> = rows.iter().map(|row| row.id() + ".").collect();
        raw.into_iter()
            .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
            .collect()
    }

    /// `output` with the user's style choices applied. A hidden plot, marker or candle stays in the
    /// output with its pane display cleared, because a `fill()` may still reference a hidden plot;
    /// the other kinds are dropped.
    pub fn apply(&self, output: &VisualOutput) -> VisualOutput {
        if self.raw.is_empty() {
            return output.clone();
        }
        let mut result = output.clone();

        for plot in &mut result.plots {
            if self.is_hidden(StyleKind::Plot, plot.id) {
                plot.display &= !DISPLAY_PANE;
            }
            if let Some(override_color) = self.color(StyleKind::Plot, plot.id) {
                plot.color = recolored(plot.color, override_color);
                plot.colors = plot
                    .colors
                    .iter()
                    .map(|color| recolored_optional(*color, override_color))
                    .collect();
            }
            if let Some(width) = self.width(StyleKind::Plot, plot.id) {
                plot.line_width = width;
            }
        }

        result
            .hlines
            .retain(|line| !self.is_hidden(StyleKind::Hline, line.id));
        for line in &mut result.hlines {
            if let Some(color) = self.color(StyleKind::Hline, line.id) {
                line.color = color;
            }
        }

        result
            .fills
            .retain(|fill| !self.is_hidden(StyleKind::Fill, fill.id));
        for fill in &mut result.fills {
            if let Some(override_color) = self.color(StyleKind::Fill, fill.id) {
                fill.colors = fill
                    .colors
                    .iter()
                    .map(|color| recolored_optional(*color, override_color))
                    .collect();
            }
        }

        for marker in &mut result.markers {
            if self.is_hidden(StyleKind::Marker, marker.id) {
                marker.display &= !DISPLAY_PANE;
            }
            if let Some(override_color) = self.color(StyleKind::Marker, marker.id) {
                marker.color = recolored(marker.color, override_color);
                marker.colors = marker
                    .colors
                    .iter()
                    .map(|color| recolored_optional(*color, override_color))
                    .collect();
            }
        }

        for candle in &mut result.candles {
            if self.is_hidden(StyleKind::Candle, candle.id) {
                candle.display &= !DISPLAY_PANE;
            }
        }

        result.backgrounds = styled(result.backgrounds, StyleKind::Bgcolor, self);
        result.bar_colors = styled(result.bar_colors, StyleKind::Barcolor, self);
        result
    }
}

/// One line of the Style section: a script output the user can hide, recolor or thicken.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleRow {
    pub kind: StyleKind,
    pub output_id: usize,
    pub title: String,
    /// The script's color when it is one fixed color; `None` when the script varies it per bar.
    pub default_color: Option<u32>,
    /// The script's line width for the plot styles that draw one.
    pub default_width: Option<usize>,
}

impl StyleRow {
    /// The key prefix shared by this row's overrides.
    pub fn id(&self) -> String {
        format!("{}.{}", self.kind.raw_value(), self.output_id)
    }
}

/// Every output of `output` a user can style, in the order the script made them. Outputs the script
/// itself keeps off the pane (`display.none`) are left out.
pub fn style_rows(output: &VisualOutput) -> Vec<StyleRow> {
    let mut rows = Vec::new();
    let mut counts: HashMap<StyleKind, usize> = HashMap::new();

    for plot in &output.plots {
        if plot.display & DISPLAY_PANE != 0 {
            let width = width_styles(plot.style).then_some(plot.line_width);
            push_row(
                &mut rows,
                &mut counts,
                StyleKind::Plot,
                plot.id,
                plot.title.as_deref(),
                "Plot",
                fixed_color(&plot.colors),
                width,
            );
        }
    }
    for line in &output.hlines {
        push_row(
            &mut rows,
            &mut counts,
            StyleKind::Hline,
            line.id,
            line.title.as_deref(),
            "Level",
            Some(line.color),
            None,
        );
    }
    for fill in &output.fills {
        let color = if fill.gradients.iter().any(Option::is_some) {
            None
        } else {
            fixed_color(&fill.colors)
        };
        push_row(
            &mut rows,
            &mut counts,
            StyleKind::Fill,
            fill.id,
            fill.title.as_deref(),
            "Fill",
            color,
            None,
        );
    }
    for marker in &output.markers {
        if marker.display & DISPLAY_PANE != 0 {
            let fallback = if marker.kind == MarkerKind::Shape {
                "Shape"
            } else {
                "Character"
            };
            push_row(
                &mut rows,
                &mut counts,
                StyleKind::Marker,
                marker.id,
                marker.title.as_deref(),
                fallback,
                fixed_color(&marker.colors),
                None,
            );
        }
    }
    for candle in &output.candles {
        if candle.display & DISPLAY_PANE != 0 {
            push_row(
                &mut rows,
                &mut counts,
                StyleKind::Candle,
                candle.id,
                candle.title.as_deref(),
                "Candles",
                None,
                None,
            );
        }
    }
    for background in &output.backgrounds {
        push_row(
            &mut rows,
            &mut counts,
            StyleKind::Bgcolor,
            background.id,
            background.title.as_deref(),
            "Background",
            fixed_color(&background.colors),
            None,
        );
    }
    for bar_color in &output.bar_colors {
        push_row(
            &mut rows,
            &mut counts,
            StyleKind::Barcolor,
            bar_color.id,
            bar_color.title.as_deref(),
            "Bar Color",
            fixed_color(&bar_color.colors),
            None,
        );
    }

    rows.sort_by_key(|row| (row.output_id, row.kind.raw_value()));
    rows
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    rows: &mut Vec<StyleRow>,
    counts: &mut HashMap<StyleKind, usize>,
    kind: StyleKind,
    id: usize,
    title: Option<&str>,
    fallback: &str,
    color: Option<u32>,
    width: Option<usize>,
) {
    let count = counts.entry(kind).or_insert(0);
    *count += 1;
    let title = match title.filter(|title| !title.is_empty()) {
        Some(title) => title.to_owned(),
        None => format!("{fallback} {}", *count),
    };
    rows.push(StyleRow {
        kind,
        output_id: id,
        title,
        default_color: color,
        default_width: width,
    });
}

/// The plot styles that draw a single line and so offer a width control.
fn width_styles(style: PlotStyle) -> bool {
    matches!(
        style,
        PlotStyle::Line | PlotStyle::Stepline | PlotStyle::Circles | PlotStyle::Cross
    )
}

/// The one color every visible bar uses, or `None` when the script varies it (or never shows one).
fn fixed_color(colors: &[Option<u32>]) -> Option<u32> {
    let mut visible: Vec<u32> = colors
        .iter()
        .flatten()
        .copied()
        .filter(|color| color & 0xFF != 0)
        .collect();
    visible.sort_unstable();
    visible.dedup();
    (visible.len() == 1).then(|| visible[0])
}

/// A color the user picked replaces a visible one; `na` (nil) and fully transparent stay as they are.
fn recolored_optional(color: Option<u32>, to: u32) -> Option<u32> {
    match color {
        Some(color) if color & 0xFF != 0 => Some(to),
        other => other,
    }
}

fn recolored(color: u32, to: u32) -> u32 {
    if color & 0xFF != 0 { to } else { color }
}

fn styled(outputs: Vec<ColorOutput>, kind: StyleKind, style: &StyleOverrides) -> Vec<ColorOutput> {
    outputs
        .into_iter()
        .filter(|output| !style.is_hidden(kind, output.id))
        .map(|mut output| {
            if let Some(override_color) = style.color(kind, output.id) {
                output.colors = output
                    .colors
                    .iter()
                    .map(|color| recolored_optional(*color, override_color))
                    .collect();
            }
            output
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::outputs::{DISPLAY_ALL, PlotStyle};
    use std::collections::HashMap;

    fn plot(id: usize, color: u32) -> PlotOutput {
        PlotOutput {
            id,
            title: Some(format!("Plot {id}")),
            values: vec![Some(1.0)],
            color,
            line_width: 2,
            style: PlotStyle::Line,
            colors: vec![Some(color)],
            display: DISPLAY_ALL,
            hist_base: 0.0,
            offset: 0,
        }
    }

    #[test]
    fn apply_hides_recolors_and_thickens() {
        let mut output = VisualOutput::default();
        output.plots.push(plot(0, 0xFF0000FF));
        output.plots.push(plot(1, 0x00FF00FF));

        let raw = HashMap::from([
            (
                StyleOverrides::key(StyleKind::Plot, 0, StyleAttribute::Visible),
                "false".to_owned(),
            ),
            (
                StyleOverrides::key(StyleKind::Plot, 1, StyleAttribute::Color),
                "0000FFFF".to_owned(),
            ),
            (
                StyleOverrides::key(StyleKind::Plot, 1, StyleAttribute::Width),
                "6".to_owned(),
            ),
        ]);
        let applied = StyleOverrides::new(raw).apply(&output);

        assert_eq!(applied.plots[0].display & DISPLAY_PANE, 0);
        assert_eq!(applied.plots[1].color, 0x0000FFFF);
        assert_eq!(applied.plots[1].line_width, 6);
    }

    #[test]
    fn width_is_clamped() {
        let raw = HashMap::from([(
            StyleOverrides::key(StyleKind::Plot, 0, StyleAttribute::Width),
            "99".to_owned(),
        )]);
        let style = StyleOverrides::new(raw);
        assert_eq!(style.width(StyleKind::Plot, 0), Some(20));
    }

    #[test]
    fn style_rows_skip_hidden_plots() {
        let mut output = VisualOutput::default();
        output.plots.push(plot(0, 0xFF0000FF));
        output.plots[0].display = 0; // display.none
        output.plots.push(plot(1, 0x00FF00FF));

        let rows = style_rows(&output);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].output_id, 1);
        assert_eq!(rows[0].default_color, Some(0x00FF00FF));
    }
}
