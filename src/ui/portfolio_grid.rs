//! Painter-drawn table primitives for the portfolio tabs: proportional column widths, the
//! rounded container, a sortable header cell and aligned cell text. Rows are painted at
//! measured positions, so columns line up with their headers whatever the content.

use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};

/// Side margin of a table inside its tab.
pub const INSET: f32 = 16.0;
pub const HEADER_HEIGHT: f32 = 30.0;
/// Rows and their highlights sit this far in from the container's edge, so a highlight's
/// rounded corners never poke past the container's own.
pub const ROW_INSET: f32 = 6.0;
/// Space between a cell's edge and its text.
pub const CELL_PAD: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Col {
    pub title: &'static str,
    /// The narrowest the column gets.
    pub min: f32,
    /// Its share of the width: columns grow in proportion to this.
    pub ideal: f32,
    pub right: bool,
    /// Extra space kept clear at the cell's trailing edge (for a row's action button).
    pub trailing: f32,
}

impl Col {
    pub const fn new(title: &'static str, min: f32, ideal: f32, right: bool) -> Self {
        Self {
            title,
            min,
            ideal,
            right,
            trailing: 0.0,
        }
    }

    pub const fn trailing(mut self, trailing: f32) -> Self {
        self.trailing = trailing;
        self
    }
}

/// Column widths for `available` points: each column takes its share of `ideal`, none goes
/// under its `min`, and the others give up the difference. Narrower than every `min` and the
/// columns keep their minimums (the overflow is clipped).
pub fn widths(columns: &[Col], available: f32) -> Vec<f32> {
    let mut fixed = vec![false; columns.len()];
    let mut result = vec![0.0; columns.len()];
    loop {
        let pinned: f32 = columns
            .iter()
            .zip(&fixed)
            .filter(|(_, fixed)| **fixed)
            .map(|(column, _)| column.min)
            .sum();
        let share: f32 = columns
            .iter()
            .zip(&fixed)
            .filter(|(_, fixed)| !**fixed)
            .map(|(column, _)| column.ideal)
            .sum();
        if share <= 0.0 {
            break;
        }
        let scale = ((available - pinned) / share).max(0.0);
        let mut changed = false;
        for (index, column) in columns.iter().enumerate() {
            if fixed[index] {
                result[index] = column.min;
                continue;
            }
            let wanted = column.ideal * scale;
            if wanted < column.min {
                fixed[index] = true;
                result[index] = column.min;
                changed = true;
            } else {
                result[index] = wanted;
            }
        }
        if !changed {
            break;
        }
    }
    result
}

/// The columns' rects side by side from `left`, `height` tall starting at `top`.
pub fn column_rects(
    columns: &[Col],
    left: f32,
    top: f32,
    height: f32,
    available: f32,
) -> Vec<Rect> {
    let mut x = left;
    widths(columns, available)
        .into_iter()
        .map(|width| {
            let rect = Rect::from_min_size(pos2(x, top), vec2(width, height));
            x += width;
            rect
        })
        .collect()
}

/// The rounded, hairline-bordered area a table lives in.
pub fn paint_frame(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 8.0, theme::surface());
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );
}

/// A small filled triangle for a sort direction.
pub fn paint_sort_arrow(ui: &Ui, center: Pos2, ascending: bool, color: Color32) {
    let (half_width, half_height) = (3.5, 2.5);
    let points = if ascending {
        vec![
            pos2(center.x - half_width, center.y + half_height),
            pos2(center.x + half_width, center.y + half_height),
            pos2(center.x, center.y - half_height),
        ]
    } else {
        vec![
            pos2(center.x - half_width, center.y - half_height),
            pos2(center.x + half_width, center.y - half_height),
            pos2(center.x, center.y + half_height),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A header cell in `rect`: the title (primary and arrowed while sorted by it), a wash on
/// hover. Returns whether it was clicked.
pub fn header_cell(ui: &mut Ui, id: Id, rect: Rect, column: &Col, sorted: Option<bool>) -> bool {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(2.0, 3.0)), 5.0, theme::ink(10));
    }
    let color = if sorted.is_some() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let text_rect = text_span(rect, column);
    let galley = kit::galley(
        ui,
        column.title,
        kit::size::SUBHEADLINE,
        SEMIBOLD,
        color,
        text_rect.width(),
    );
    let x = if column.right {
        text_rect.right() - galley.size().x
    } else {
        text_rect.left()
    };
    let y = rect.center().y - galley.size().y / 2.0;
    let galley_width = galley.size().x;
    ui.painter()
        .galley(pos2(x, y), galley, Color32::PLACEHOLDER);
    if let Some(ascending) = sorted {
        // On the side facing away from the cell's edge, so titles keep lining up with figures.
        let arrow_x = if column.right {
            x - 9.0
        } else {
            x + galley_width + 9.0
        };
        paint_sort_arrow(ui, pos2(arrow_x, rect.center().y), ascending, color);
    }
    response.clicked()
}

/// Where a cell's text may go: the cell less its padding and trailing reserve.
pub fn text_span(cell: Rect, column: &Col) -> Rect {
    Rect::from_min_max(
        pos2(cell.left() + CELL_PAD, cell.top()),
        pos2(
            (cell.right() - CELL_PAD - column.trailing).max(cell.left() + CELL_PAD + 1.0),
            cell.bottom(),
        ),
    )
}

/// One line of text in `cell`, aligned as its column is and cut with an ellipsis if it does
/// not fit. Returns the rect it covers.
pub fn cell_text(
    ui: &Ui,
    cell: Rect,
    column: &Col,
    text: &str,
    size: f32,
    weight: f32,
    color: Color32,
) -> Rect {
    let span = text_span(cell, column);
    let galley = kit::galley(ui, text, size, weight, color, span.width());
    let anchor = if column.right {
        Align2::RIGHT_CENTER
    } else {
        Align2::LEFT_CENTER
    };
    let at = if column.right {
        pos2(span.right(), cell.center().y)
    } else {
        pos2(span.left(), cell.center().y)
    };
    let rect = anchor.anchor_size(at, galley.size());
    ui.painter().galley(rect.min, galley, Color32::PLACEHOLDER);
    rect
}

/// A figure in a table cell: regular weight, 13 pt.
pub fn figure(ui: &Ui, cell: Rect, column: &Col, text: &str, color: Color32) {
    cell_text(ui, cell, column, text, kit::size::BODY, REGULAR, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [Col; 3] = [
        Col::new("A", 100.0, 200.0, false),
        Col::new("B", 50.0, 100.0, true),
        Col::new("C", 50.0, 100.0, true),
    ];

    #[test]
    fn widths_follow_the_ideal_shares_and_fill_the_space() {
        let widths = widths(&COLUMNS, 800.0);
        assert_eq!(widths, [400.0, 200.0, 200.0]);
        assert!((widths.iter().sum::<f32>() - 800.0).abs() < 1e-3);
    }

    #[test]
    fn a_column_never_goes_under_its_minimum() {
        // 240 shared 2:1:1 gives A 120, B 60, C 60: all fit.
        for (found, wanted) in widths(&COLUMNS, 240.0).into_iter().zip([120.0, 60.0, 60.0]) {
            assert!((found - wanted).abs() < 1e-3);
        }
        // 200 would give A 100, B 50, C 50: exactly the minimums.
        for (found, wanted) in widths(&COLUMNS, 200.0).into_iter().zip([100.0, 50.0, 50.0]) {
            assert!((found - wanted).abs() < 1e-3);
        }
        // Squeezed past the minimums they hold, and the overflow is clipped by the caller.
        assert_eq!(widths(&COLUMNS, 120.0), [100.0, 50.0, 50.0]);
    }

    #[test]
    fn a_pinned_column_hands_its_excess_to_the_others() {
        let columns = [
            Col::new("Wide", 10.0, 100.0, false),
            Col::new("Narrow", 80.0, 10.0, true),
        ];
        let widths = widths(&columns, 200.0);
        assert_eq!(widths[1], 80.0, "pinned at its minimum");
        assert!(
            (widths[0] - 120.0).abs() < 1e-3,
            "the rest goes to the other"
        );
    }

    #[test]
    fn column_rects_sit_side_by_side() {
        let rects = column_rects(&COLUMNS, 10.0, 20.0, 30.0, 400.0);
        assert_eq!(rects[0].left(), 10.0);
        assert_eq!(rects[1].left(), rects[0].right());
        assert_eq!(rects[2].left(), rects[1].right());
        assert_eq!((rects[0].top(), rects[0].height()), (20.0, 30.0));
    }

    #[test]
    fn trailing_space_is_kept_clear_of_text() {
        let column = Col::new("P&L", 100.0, 100.0, true).trailing(30.0);
        let cell = Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0, 40.0));
        let span = text_span(cell, &column);
        assert_eq!(
            (span.left(), span.right()),
            (CELL_PAD, 120.0 - CELL_PAD - 30.0)
        );
    }
}
