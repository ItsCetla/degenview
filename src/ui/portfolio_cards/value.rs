//! The Portfolio Value card: today's total as one big number, what it did in the last day, and
//! a month of history faint behind it when the card is tall enough to carry it.

use super::{area_under, chip, fitted, loading_pill, reveal, tone_color};
use crate::model::portfolio_card::CardData;
use crate::ui::kit;
use crate::ui::theme;
use eframe::egui::{Color32, Id, Pos2, Rect, Stroke, Ui, Vec2};

/// Below this height the sparkline would crowd the number.
const SPARKLINE_MIN_HEIGHT: f32 = 170.0;

/// The part of the card the history is drawn in: the bottom third.
pub(super) fn sparkline_band(area: Rect) -> Option<Rect> {
    (area.height() >= SPARKLINE_MIN_HEIGHT).then(|| {
        Rect::from_min_max(
            Pos2::new(area.left() + 8.0, area.bottom() - area.height() * 0.34),
            Pos2::new(area.right() - 8.0, area.bottom() - 10.0),
        )
    })
}

pub(super) fn paint(ui: &Ui, area: Rect, id: Id, data: &CardData, _pointer: Option<Pos2>) {
    if data.loading {
        loading_pill(ui, area, "Loading market data…");
        return;
    }
    let band = sparkline_band(area);
    if let Some(band) = band {
        paint_sparkline(ui, band, id, data);
    }
    let anchor_y = band.map_or(area.center().y, |band| {
        area.top() + (band.top() - area.top()) / 2.0 + 4.0
    });
    let width = (area.width() - 32.0).max(40.0);
    let galley = fitted(
        ui,
        &data.formatter.money(data.total_value),
        46.0,
        18.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let top = anchor_y - galley.size().y / 2.0 - 14.0;
    ui.painter().galley(
        Pos2::new(area.center().x - galley.size().x / 2.0, top),
        galley.clone(),
        Color32::PLACEHOLDER,
    );

    // 24h change, centred under the number.
    let mut y = top + galley.size().y + 12.0;
    if let Some(change) = data.day_change {
        let color = tone_color(data.formatter.tone(Some(change.amount)));
        let arrow = if change.amount >= 0.0 { "↗" } else { "↘" };
        let text = match change.percentage {
            Some(fraction) => format!(
                "{arrow} {} · {}  24h",
                data.formatter.signed_money(change.amount),
                data.formatter.signed_percent(fraction, 2)
            ),
            None => format!(
                "{arrow} {}  24h",
                data.formatter.signed_money(change.amount)
            ),
        };
        let probe = kit::galley(ui, &text, 11.0, kit::SEMIBOLD, color, f32::INFINITY);
        let chip_width = (probe.size().x + 14.0).min(width);
        chip(
            ui,
            Pos2::new(area.center().x - chip_width / 2.0, y + 10.0),
            &text,
            color,
            width,
        );
        y += 28.0;
    }
    let caption = if data.unpriced > 0 {
        format!(
            "{} {} could not be priced",
            data.unpriced,
            if data.unpriced == 1 {
                "asset"
            } else {
                "assets"
            }
        )
    } else {
        format!(
            "{} · {} {}",
            data.scope_name,
            data.holdings.len(),
            if data.holdings.len() == 1 {
                "asset"
            } else {
                "assets"
            }
        )
    };
    let color = if data.unpriced > 0 {
        kit::ORANGE
    } else {
        theme::text_secondary()
    };
    let galley = kit::galley(ui, &caption, 11.0, kit::REGULAR, color, width);
    ui.painter().galley(
        Pos2::new(area.center().x - galley.size().x / 2.0, y),
        galley,
        Color32::PLACEHOLDER,
    );
}

fn paint_sparkline(ui: &Ui, band: Rect, id: Id, data: &CardData) {
    let values: Vec<f64> = data.history.iter().map(|point| point.value).collect();
    if values.len() < 2 {
        return;
    }
    let (minimum, maximum) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
            (low.min(*value), high.max(*value))
        });
    let spread = (maximum - minimum).max(f64::EPSILON);
    let line: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Pos2::new(
                band.left() + band.width() * index as f32 / (values.len() - 1) as f32,
                band.bottom() - band.height() * ((value - minimum) / spread) as f32,
            )
        })
        .collect();
    let rising = values.last() >= values.first();
    let color = if rising { theme::UP } else { theme::DOWN };
    let progress = reveal(ui.ctx(), id.with("spark"), 0.7);
    let clip = Rect::from_min_max(
        band.min - Vec2::new(0.0, 4.0),
        Pos2::new(
            band.left() + band.width() * progress + 2.0,
            band.bottom() + 4.0,
        ),
    );
    let painter = ui.painter().with_clip_rect(clip);
    area_under(&painter, &line, band.bottom(), color, 0.18);
    painter.add(eframe::egui::Shape::line(
        line,
        Stroke::new(1.5, color.gamma_multiply(0.85)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_card_has_no_sparkline() {
        let short = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 120.0));
        assert!(sparkline_band(short).is_none());
        let tall = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 300.0));
        let band = sparkline_band(tall).unwrap();
        assert!(band.top() > tall.center().y);
        assert!(band.bottom() < tall.bottom());
    }
}
