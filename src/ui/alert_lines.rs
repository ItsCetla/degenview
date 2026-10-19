//! Price alerts on the chart: a dashed line at each alert's level, ending in a small tag against
//! the price axis. Right-clicking a tag opens the alert's menu (edit, pause, delete); the line
//! itself only marks the level.
//!
//! Lines sit on the chart's own price scale, like the paper-trading markers, so they stay on
//! their level through zooms and axis drags. Pure view: it reports the click, the workspace acts.

use super::chart_card::PriceScale;
use crate::model::PriceFormat;
use crate::model::alert::AlertId;
use crate::model::alert_line::AlertLine;
use crate::ui::{accessibility, theme};
use eframe::egui::{self, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Vec2};

const HEIGHT: f32 = 20.0;
const MARGIN: f32 = 8.0;

/// A tag that was right-clicked, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMenu {
    pub id: AlertId,
    pub at: Pos2,
}

/// The tag's words: what the alert waits for, at the chart's price format.
fn caption(line: &AlertLine, format: PriceFormat) -> String {
    let price = format.format(line.price);
    let direction = if line.above { "↑" } else { "↓" };
    if line.paused {
        format!("Alert {direction} {price} · paused")
    } else {
        format!("Alert {direction} {price}")
    }
}

/// A line's tag, as drawn.
struct Tag {
    id: AlertId,
    rect: Rect,
    caption: String,
}

/// Paints one alert's dashed line and its tag, and says where the tag is. Nothing when the level
/// is off the plot.
fn paint_line(
    painter: &egui::Painter,
    plot: Rect,
    scale: &PriceScale,
    line: &AlertLine,
    format: PriceFormat,
) -> Option<Tag> {
    let y = scale.y(line.price);
    if !y.is_finite() || !plot.y_range().contains(y) {
        return None;
    }
    let tint = if line.paused {
        theme::text_tertiary()
    } else {
        theme::YELLOW
    };
    let caption = caption(line, format);
    let galley = painter.layout_no_wrap(caption.clone(), FontId::monospace(10.5), Color32::WHITE);
    let width = galley.size().x + 22.0;
    let tag = Rect::from_min_size(
        Pos2::new(plot.right() - MARGIN - width, y - HEIGHT / 2.0),
        Vec2::new(width, HEIGHT),
    );
    let stroke = Stroke::new(1.0, tint.gamma_multiply(0.7));
    for dash in egui::Shape::dashed_line(
        &[Pos2::new(plot.left(), y), Pos2::new(tag.left(), y)],
        stroke,
        2.0,
        4.0,
    ) {
        painter.add(dash);
    }
    painter.rect_filled(tag, 6.0, theme::chrome().gamma_multiply(0.94));
    painter.rect_stroke(
        tag,
        6.0,
        Stroke::new(1.0, tint.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(tag.left() + 8.0, tag.center().y),
        Align2::LEFT_CENTER,
        "●",
        FontId::proportional(7.0),
        tint,
    );
    painter.galley(
        Pos2::new(tag.left() + 16.0, tag.center().y - galley.size().y / 2.0),
        galley,
        if line.paused {
            theme::text_tertiary()
        } else {
            Color32::WHITE
        },
    );
    Some(Tag {
        id: line.id,
        rect: tag,
        caption,
    })
}

/// Draws the alert lines over `plot` and nothing else: no tag answers a pointer. What a
/// snapshot shows.
pub(super) fn paint(
    painter: &egui::Painter,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
) {
    for line in lines {
        paint_line(painter, plot, scale, line, format);
    }
}

/// Draws the alert lines over `plot` and returns the tag right-clicked this frame, if any.
pub(super) fn show(
    ui: &mut egui::Ui,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
) -> Option<LineMenu> {
    let painter = ui.painter().with_clip_rect(plot);
    let mut menu = None;
    for line in lines {
        let Some(tag) = paint_line(&painter, plot, scale, line, format) else {
            continue;
        };
        let response = ui.interact(tag.rect, Id::new(("alert-line", tag.id)), Sense::click());
        let response = accessibility::label(response, tag.caption)
            .on_hover_text("Right-click for alert options");
        if response.secondary_clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            menu = Some(LineMenu { id: tag.id, at });
            // The card reads it next frame.
            ui.ctx().request_repaint();
        }
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_says_which_way_and_whether_it_is_paused() {
        let mut line = AlertLine {
            id: 1,
            price: 118_150.99,
            paused: false,
            above: true,
        };
        assert_eq!(caption(&line, PriceFormat::AUTO), "Alert ↑ 118,151");
        line.above = false;
        line.paused = true;
        assert_eq!(
            caption(&line, PriceFormat::AUTO),
            "Alert ↓ 118,151 · paused"
        );
    }
}
