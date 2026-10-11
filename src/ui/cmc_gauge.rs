//! The CoinMarketCap "Latest" charts: the Fear & Greed semicircle gauge and the Altcoin Season
//! scale, drawn from the cached reading instead of a price series.

use crate::service::cmc::{self, LatestReading};
use crate::ui::kit::{self, BOLD, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2, pos2,
    text::LayoutJob, vec2,
};
use std::f32::consts::PI;

const RED: Color32 = kit::RED;
const ORANGE: Color32 = kit::ORANGE;
const YELLOW: Color32 = Color32::from_rgb(255, 214, 10);
const GREEN: Color32 = kit::GREEN;
const MINT: Color32 = Color32::from_rgb(99, 230, 226);
const GRAY: Color32 = Color32::from_rgb(142, 142, 147);
const BLUE: Color32 = theme::SYSTEM_ACCENT;

/// Paints the gauge for `symbol` into `rect`. Returns its accessibility description.
pub fn paint(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    symbol: &str,
    reading: &LatestReading,
) -> String {
    if symbol == cmc::ALTCOIN_LATEST {
        altcoin(ui, painter, rect, reading);
        format!(
            "CMC Altcoin Season Index, {} out of 100, {}.",
            reading.value as i64, reading.classification
        )
    } else {
        fear_greed(ui, painter, rect, reading);
        format!(
            "CMC Crypto Fear and Greed Index, {} out of 100, {}.",
            reading.value as i64, reading.classification
        )
    }
}

/// "56" in a large bold face with a small secondary " / 100" sharing its baseline.
fn score(
    ui: &Ui,
    value: f64,
    size: f32,
    suffix_size: f32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    let format = |size: f32, weight: f32, color: Color32| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        coords: egui::epaint::text::VariationCoords::new([("wght", weight)]),
        valign: egui::Align::BOTTOM,
        ..Default::default()
    };
    job.append(
        &(value as i64).to_string(),
        0.0,
        format(size, BOLD, kit::primary(1.0)),
    );
    job.append(
        " / 100",
        0.0,
        format(suffix_size, REGULAR, theme::text_secondary()),
    );
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    ui.painter().layout_job(job)
}

fn fear_greed(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let compact = rect.height() < 170.0;
    let inner = rect.shrink2(vec2(if compact { 12.0 } else { 16.0 }, 8.0));
    let value_width = if compact { 92.0 } else { 112.0 };
    let gap = if compact { 8.0 } else { 14.0 };

    let number = score(
        ui,
        reading.value,
        if compact { 35.0 } else { 44.0 },
        if compact { 11.0 } else { 13.0 },
        value_width + 40.0,
    );
    let label = kit::galley(
        ui,
        &reading.classification,
        if compact { 14.0 } else { 17.0 },
        SEMIBOLD,
        kit::primary(1.0),
        value_width + 40.0,
    );
    let block = number.size().y + 2.0 + label.size().y;
    let top = inner.center().y - block / 2.0;
    painter.galley(pos2(inner.min.x, top), number.clone(), Color32::PLACEHOLDER);
    painter.galley(
        pos2(inner.min.x, top + number.size().y + 2.0),
        label,
        Color32::PLACEHOLDER,
    );

    let gauge_width = ((inner.width() - value_width - gap) * 0.88).max(80.0);
    let gauge = Rect::from_center_size(
        pos2(
            inner.min.x + value_width + gap + (inner.width() - value_width - gap) / 2.0,
            inner.center().y,
        ),
        vec2(gauge_width, inner.height() * 0.9),
    );
    sentiment_gauge(painter, gauge, reading.value);
}

/// Five arcs from red (fear) to mint (greed), with a needle at `value`.
fn sentiment_gauge(painter: &Painter, rect: Rect, value: f64) {
    let center = pos2(rect.center().x, rect.min.y + rect.height() * 0.88);
    let radius = (rect.width() / 2.0 - 18.0)
        .min(rect.height() * 0.78)
        .max(10.0);
    let thickness = (radius * 0.13).max(8.0);
    for (index, color) in [RED, ORANGE, YELLOW, GREEN, MINT].into_iter().enumerate() {
        let start = PI + index as f32 * PI / 5.0;
        let end = start + PI / 5.0;
        let points: Vec<Pos2> = (0..=24)
            .map(|step| {
                let angle = start + (end - start) * step as f32 / 24.0;
                center + Vec2::angled(angle) * radius
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(thickness, color)));
    }
    let angle = PI * (1.0 + value.clamp(0.0, 100.0) as f32 / 100.0);
    let tip = center + Vec2::angled(angle) * radius * 0.72;
    painter.line_segment([center, tip], Stroke::new(3.0, kit::primary(1.0)));
    painter.circle_filled(tip, 1.5, kit::primary(1.0));
    painter.circle_filled(center, 5.0, kit::primary(1.0));
    for (text, x) in [("0", center.x - radius), ("100", center.x + radius)] {
        painter.text(
            pos2(x, center.y + 12.0),
            Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(10.0),
            theme::text_secondary(),
        );
    }
}

fn altcoin(ui: &Ui, painter: &Painter, rect: Rect, reading: &LatestReading) {
    let compact = rect.height() < 190.0;
    let very_compact = rect.height() < 145.0;
    let spacing = if very_compact {
        2.0
    } else if compact {
        4.0
    } else {
        7.0
    };
    let inner = rect.shrink2(vec2(16.0, if compact { 6.0 } else { 12.0 }));
    let width = inner.width();
    let mut y = inner.min.y;

    let number = score(
        ui,
        reading.value,
        if very_compact {
            25.0
        } else if compact {
            29.0
        } else {
            34.0
        },
        if very_compact { 11.0 } else { 13.0 },
        width,
    );
    painter.galley(
        pos2(inner.center().x - number.size().x / 2.0, y),
        number.clone(),
        Color32::PLACEHOLDER,
    );
    y += number.size().y + spacing;

    let label = kit::galley(
        ui,
        &reading.classification,
        if very_compact {
            12.0
        } else if compact {
            14.0
        } else {
            16.0
        },
        SEMIBOLD,
        kit::primary(1.0),
        width,
    );
    painter.galley(
        pos2(inner.center().x - label.size().x / 2.0, y),
        label.clone(),
        Color32::PLACEHOLDER,
    );
    y += label.size().y + spacing;

    let scale_height = if compact { 26.0 } else { 34.0 };
    season_scale(
        ui,
        painter,
        Rect::from_min_size(pos2(inner.min.x, y), vec2(width, scale_height)),
        reading.value,
        compact,
    );
    y += scale_height + spacing;

    let statistics = [
        reading
            .yearly_high
            .as_ref()
            .map(|(value, date)| ("Yearly High", (*value as i64).to_string(), short_date(date))),
        reading
            .yearly_low
            .as_ref()
            .map(|(value, date)| ("Yearly Low", (*value as i64).to_string(), short_date(date))),
        Some((
            "Altcoin Market Cap",
            reading.altcoin_market_cap.map_or_else(
                || "—".to_owned(),
                |cap| format!("${}", crate::model::watchlist_quote::compact_number(cap)),
            ),
            String::new(),
        )),
    ];
    let statistics: Vec<_> = statistics.into_iter().flatten().collect();
    let height = if compact { 44.0 } else { 52.0 };
    if y + height > inner.max.y + 6.0 {
        return;
    }
    let column = width / statistics.len() as f32;
    for (index, (title, value, detail)) in statistics.iter().enumerate() {
        let x = inner.min.x + column * index as f32;
        if index > 0 {
            painter.vline(
                x,
                y + 4.0..=y + height - 4.0,
                Stroke::new(1.0, theme::separator()),
            );
        }
        let center = x + column / 2.0;
        let max = column - 8.0;
        let title = kit::galley(
            ui,
            title,
            if compact { 10.0 } else { 11.0 },
            REGULAR,
            theme::text_secondary(),
            max,
        );
        let value = kit::digits(ui, value, 13.0, SEMIBOLD, kit::primary(1.0), max);
        let detail = (!compact && !detail.is_empty())
            .then(|| kit::galley(ui, detail, 10.0, REGULAR, theme::text_secondary(), max));
        let gap = if compact { 0.0 } else { 2.0 };
        let total = title.size().y
            + gap
            + value.size().y
            + detail.as_ref().map_or(0.0, |detail| gap + detail.size().y);
        let mut row = y + (height - total) / 2.0;
        for galley in [Some(title), Some(value), detail].into_iter().flatten() {
            let size = galley.size();
            painter.galley(
                pos2(center - size.x / 2.0, row),
                galley,
                Color32::PLACEHOLDER,
            );
            row += size.y + gap;
        }
    }
}

/// An orange → gray → blue capsule with a triangle marker above `value` and the season labels
/// underneath.
fn season_scale(ui: &Ui, painter: &Painter, rect: Rect, value: f64, compact: bool) {
    let bar_height = if compact { 7.0 } else { 9.0 };
    let marker = if compact {
        vec2(9.0, 7.0)
    } else {
        vec2(11.0, 8.0)
    };
    let bar = Rect::from_min_size(
        pos2(rect.min.x, rect.min.y + marker.y + 2.0),
        vec2(rect.width(), bar_height),
    );
    painter.add(gradient_capsule(bar, &[ORANGE, GRAY, BLUE]));

    let fraction = value.clamp(0.0, 100.0) as f32 / 100.0;
    let left = bar.min.x + fraction * (bar.width() - marker.x);
    let top = bar.min.y - marker.y - 1.0;
    painter.add(Shape::convex_polygon(
        vec![
            pos2(left, top),
            pos2(left + marker.x, top),
            pos2(left + marker.x / 2.0, top + marker.y),
        ],
        kit::primary(1.0),
        Stroke::NONE,
    ));

    let size = if compact { 8.0 } else { 10.0 };
    let label_y = bar.max.y + 3.0;
    let third = rect.width() / 3.0;
    for (text, align, x) in [
        ("0  Bitcoin Season", Align2::LEFT_TOP, rect.min.x),
        ("Neutral", Align2::CENTER_TOP, rect.center().x),
        ("Altcoin Season  100", Align2::RIGHT_TOP, rect.max.x),
    ] {
        let galley = kit::galley(ui, text, size, REGULAR, theme::text_secondary(), third);
        let anchored = align.anchor_size(pos2(x, label_y), galley.size());
        painter.galley(anchored.min, galley, Color32::PLACEHOLDER);
    }
}

/// A capsule filled with a horizontal gradient through `stops`, as a mesh of one-point columns
/// whose height follows the rounded ends.
fn gradient_capsule(rect: Rect, stops: &[Color32]) -> Shape {
    let mut mesh = Mesh::default();
    let radius = rect.height() / 2.0;
    let columns = rect.width().ceil().max(2.0) as usize;
    for column in 0..=columns {
        let t = column as f32 / columns as f32;
        let x = rect.min.x + rect.width() * t;
        let from_edge = (x - rect.min.x).min(rect.max.x - x);
        let half = if from_edge >= radius {
            radius
        } else {
            let offset = radius - from_edge;
            (radius * radius - offset * offset).max(0.0).sqrt()
        };
        let color = sample(stops, t);
        let index = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x, rect.center().y - half), color);
        mesh.colored_vertex(pos2(x, rect.center().y + half), color);
        if column > 0 {
            mesh.add_triangle(index - 2, index - 1, index);
            mesh.add_triangle(index - 1, index, index + 1);
        }
    }
    Shape::mesh(mesh)
}

fn sample(stops: &[Color32], t: f32) -> Color32 {
    let scaled = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(stops.len() - 2);
    let local = scaled - index as f32;
    let (from, to) = (stops[index], stops[index + 1]);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * local).round() as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

/// "2026-09-27" → "Sep 27".
fn short_date(text: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = text.get(..10).unwrap_or(text).split('-');
    let (Some(_), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return text.to_owned();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month @ 1..=12), Ok(day)) => format!("{} {day}", MONTHS[month - 1]),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_dates_read_like_swift() {
        assert_eq!(short_date("2026-09-27"), "Sep 27");
        assert_eq!(short_date("2025-12-19T00:00:00.000Z"), "Dec 19");
        assert_eq!(short_date("soon"), "soon");
    }

    #[test]
    fn the_gradient_passes_through_each_stop() {
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 0.0), ORANGE);
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 0.5), GRAY);
        assert_eq!(sample(&[ORANGE, GRAY, BLUE], 1.0), BLUE);
    }
}
