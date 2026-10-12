//! A date-and-time field with a calendar pop-over, in the macOS compact `DatePicker` style:
//! a field showing the moment, and a month grid with a time stepper under it.
//!
//! The moment is held as the editor's `YYYY-MM-DD HH:MM` text (local time), so a field nobody
//! touched keeps whatever exact instant it opened with. The calendar arithmetic is pure and
//! tested; only `show` paints.

use crate::format::{civil_from_days, days_from_civil};
use crate::model::portfolio_editor::{date_text, parse_date_text};
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio_transactions::date_time_text;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, PopupCloseBehavior, Pos2, Rect, Sense, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

const CELL: Vec2 = Vec2::new(34.0, 28.0);
const POPOVER_WIDTH: f32 = 7.0 * CELL.x;

// MARK: Calendar arithmetic

/// A calendar day.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Day {
    pub year: i64,
    pub month: i64,
    pub day: i64,
}

/// Local calendar fields of an instant, `utc_offset_secs` east of UTC.
pub fn fields(epoch_ms: i64, utc_offset_secs: i64) -> (Day, i64, i64) {
    let local = epoch_ms.div_euclid(1_000) + utc_offset_secs;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let rest = local.rem_euclid(86_400);
    (Day { year, month, day }, rest / 3_600, rest % 3_600 / 60)
}

/// The instant of a local day and clock time (seconds zero).
pub fn instant(day: Day, hour: i64, minute: i64, utc_offset_secs: i64) -> i64 {
    (days_from_civil(day.year, day.month, day.day) * 86_400 + hour * 3_600 + minute * 60
        - utc_offset_secs)
        * 1_000
}

/// The month's six weeks, Monday first: each day with whether it belongs to the month.
/// Days of the neighbouring months fill the first and last rows.
pub fn month_grid(year: i64, month: i64) -> [[(Day, bool); 7]; 6] {
    let first = days_from_civil(year, month, 1);
    // 1970-01-01 was a Thursday: Monday-first column 3.
    let leading = (first + 3).rem_euclid(7);
    let mut grid = [[(
        Day {
            year,
            month,
            day: 1,
        },
        false,
    ); 7]; 6];
    for (slot, cell) in grid.iter_mut().flatten().enumerate() {
        let (y, m, d) = civil_from_days(first - leading + slot as i64);
        *cell = (
            Day {
                year: y,
                month: m,
                day: d,
            },
            y == year && m == month,
        );
    }
    grid
}

/// The month `delta` months from `(year, month)`.
pub fn shift_month(year: i64, month: i64, delta: i64) -> (i64, i64) {
    let index = year * 12 + month - 1 + delta;
    (index.div_euclid(12), index.rem_euclid(12) + 1)
}

/// `value` moved by `delta` within `0..modulus`, wrapping.
pub fn step(value: i64, delta: i64, modulus: i64) -> i64 {
    (value + delta).rem_euclid(modulus)
}

// MARK: Field

/// The field and its pop-over. `text` is `YYYY-MM-DD HH:MM` in local time; returns whether the
/// picker changed it. `error` outlines the field in red.
pub fn show(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    utc_offset_secs: i64,
    width: f32,
    error: bool,
) -> bool {
    let parsed = parse_date_text(text, utc_offset_secs);
    let popup_id = id.with("popup");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);

    let (rect, response) = ui.allocate_exact_size(vec2(width, 41.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let (stroke_width, stroke_color) = if error {
        (1.5, theme::DOWN)
    } else if open {
        (1.5, theme::SYSTEM_ACCENT)
    } else {
        (1.0, theme::separator().gamma_multiply(0.6))
    };
    let fill = if response.hovered() {
        theme::quaternary(0.75)
    } else {
        theme::quaternary(0.5)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(stroke_width, stroke_color),
        StrokeKind::Inside,
    );
    let label = match parsed {
        Some(ms) => date_time_text(ms, utc_offset_secs),
        None => text.clone(),
    };
    let galley = kit::galley(
        ui,
        &label,
        kit::size::BODY,
        REGULAR,
        theme::text_primary(),
        width - 44.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        Color32::PLACEHOLDER,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 18.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        Icon::Calendar,
        theme::text_secondary(),
    );
    let response = accessibility::label(response, format!("Date and time, {label}"));

    let now = parsed.unwrap_or_else(crate::model::watchlist::now_ms);
    let month_key = popup_id.with("month");
    if !open {
        ui.data_mut(|data| data.remove::<(i64, i64)>(month_key));
    }
    let mut changed = false;
    egui::Popup::from_response(&response)
        .id(popup_id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .frame(kit::menu_frame().inner_margin(egui::Margin::same(12)))
        .gap(4.0)
        .show(|ui| {
            let selected = fields(now, utc_offset_secs).0;
            let shown = ui
                .data_mut(|data| data.get_temp::<(i64, i64)>(month_key))
                .unwrap_or((selected.year, selected.month));
            if let Some(moment) = popover(ui, month_key, shown, now, utc_offset_secs) {
                *text = date_text(moment, utc_offset_secs);
                changed = true;
            }
        });
    changed
}

/// What the pop-over shows for `shown` (year, month); returns the new moment when one is
/// picked or the time is stepped.
fn popover(
    ui: &mut Ui,
    month_key: Id,
    shown: (i64, i64),
    now: i64,
    utc_offset_secs: i64,
) -> Option<i64> {
    ui.set_width(POPOVER_WIDTH);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let (selected, hour, minute) = fields(now, utc_offset_secs);
    let (today, ..) = fields(crate::model::watchlist::now_ms(), utc_offset_secs);
    let mut picked = None;
    let (mut year, mut month) = shown;

    // Month header: the name, and chevrons to either side of it.
    let (header, _) = ui.allocate_exact_size(vec2(POPOVER_WIDTH, 26.0), Sense::hover());
    let title = kit::galley(
        ui,
        &format!("{} {year}", MONTHS[(month - 1) as usize]),
        kit::size::BODY,
        SEMIBOLD,
        theme::text_primary(),
        POPOVER_WIDTH - 80.0,
    );
    ui.painter().galley(
        pos2(
            header.left() + 6.0,
            header.center().y - title.size().y / 2.0,
        ),
        title,
        Color32::PLACEHOLDER,
    );
    for (index, delta) in [-1_i64, 1].into_iter().enumerate() {
        let button = Rect::from_center_size(
            pos2(
                header.right() - 52.0 + index as f32 * 28.0 + 14.0,
                header.center().y,
            ),
            Vec2::splat(24.0),
        );
        let response = ui
            .interact(button, month_key.with(("shift", delta)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter().rect_filled(button, 6.0, theme::ink(18));
        }
        paint_chevron(ui, button.center(), delta < 0, theme::text_secondary());
        if response.clicked() {
            (year, month) = shift_month(year, month, delta);
        }
    }
    ui.add_space(6.0);

    // Weekday initials.
    let (weekdays, _) = ui.allocate_exact_size(vec2(POPOVER_WIDTH, 20.0), Sense::hover());
    for (index, initial) in WEEKDAYS.iter().enumerate() {
        let galley = kit::galley(
            ui,
            initial,
            kit::size::CAPTION,
            MEDIUM,
            theme::text_tertiary(),
            CELL.x,
        );
        ui.painter().galley(
            pos2(
                weekdays.left() + (index as f32 + 0.5) * CELL.x - galley.size().x / 2.0,
                weekdays.center().y - galley.size().y / 2.0,
            ),
            galley,
            Color32::PLACEHOLDER,
        );
    }

    // The days.
    for week in month_grid(year, month) {
        let (row, _) = ui.allocate_exact_size(vec2(POPOVER_WIDTH, CELL.y + 2.0), Sense::hover());
        for (index, (day, in_month)) in week.into_iter().enumerate() {
            let cell = Rect::from_center_size(
                pos2(row.left() + (index as f32 + 0.5) * CELL.x, row.center().y),
                vec2(CELL.x - 2.0, CELL.y),
            );
            let response = ui
                .interact(
                    cell,
                    month_key.with((day.year, day.month, day.day)),
                    Sense::click(),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            let is_selected = day == selected;
            let is_today = day == today;
            let color = if is_selected {
                theme::SYSTEM_ACCENT
            } else {
                Color32::TRANSPARENT
            };
            if is_selected {
                ui.painter().rect_filled(cell, 7.0, color);
            } else if response.hovered() {
                ui.painter().rect_filled(cell, 7.0, theme::ink(18));
            }
            if is_today && !is_selected {
                ui.painter().rect_stroke(
                    cell,
                    7.0,
                    Stroke::new(1.0, theme::SYSTEM_ACCENT.gamma_multiply(0.8)),
                    StrokeKind::Inside,
                );
            }
            let text_color = if is_selected {
                Color32::WHITE
            } else if in_month {
                theme::text_primary()
            } else {
                theme::text_tertiary()
            };
            let galley = kit::galley(
                ui,
                &day.day.to_string(),
                kit::size::BODY - 1.0,
                if is_selected || is_today {
                    SEMIBOLD
                } else {
                    REGULAR
                },
                text_color,
                CELL.x,
            );
            ui.painter().galley(
                cell.center() - galley.size() / 2.0,
                galley,
                Color32::PLACEHOLDER,
            );
            if response.clicked() {
                picked = Some(instant(day, hour, minute, utc_offset_secs));
                year = day.year;
                month = day.month;
            }
        }
    }
    ui.add_space(8.0);
    let (rule, _) = ui.allocate_exact_size(vec2(POPOVER_WIDTH, 1.0), Sense::hover());
    ui.painter().hline(
        rule.x_range(),
        rule.center().y,
        Stroke::new(1.0, theme::separator()),
    );
    ui.add_space(10.0);

    // Time: hour and minute steppers, and "Now".
    let (time_row, _) = ui.allocate_exact_size(vec2(POPOVER_WIDTH, 30.0), Sense::hover());
    let label = kit::galley(
        ui,
        "Time",
        kit::size::BODY,
        REGULAR,
        theme::text_secondary(),
        60.0,
    );
    ui.painter().galley(
        pos2(
            time_row.left() + 6.0,
            time_row.center().y - label.size().y / 2.0,
        ),
        label,
        Color32::PLACEHOLDER,
    );
    let hour_rect = Rect::from_center_size(
        pos2(time_row.left() + 100.0, time_row.center().y),
        vec2(62.0, 30.0),
    );
    let minute_rect = Rect::from_center_size(
        pos2(time_row.left() + 100.0 + 62.0 + 14.0, time_row.center().y),
        vec2(62.0, 30.0),
    );
    let colon = kit::galley(
        ui,
        ":",
        kit::size::BODY,
        SEMIBOLD,
        theme::text_secondary(),
        20.0,
    );
    ui.painter().galley(
        pos2(
            (hour_rect.right() + minute_rect.left()) / 2.0 - colon.size().x / 2.0,
            time_row.center().y - colon.size().y / 2.0,
        ),
        colon,
        Color32::PLACEHOLDER,
    );
    let new_hour = stepper(ui, month_key.with("hour"), hour_rect, hour, 24);
    let new_minute = stepper(ui, month_key.with("minute"), minute_rect, minute, 60);
    if new_hour != hour || new_minute != minute {
        picked = Some(instant(selected, new_hour, new_minute, utc_offset_secs));
    }
    let now_button = Rect::from_min_max(
        pos2(time_row.right() - 44.0, time_row.top()),
        pos2(time_row.right(), time_row.bottom()),
    );
    let now_response = ui
        .interact(now_button, month_key.with("now"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let now_label = kit::galley(
        ui,
        "Now",
        kit::size::BODY,
        MEDIUM,
        theme::SYSTEM_ACCENT,
        40.0,
    );
    ui.painter().galley(
        Align2::RIGHT_CENTER
            .anchor_size(
                pos2(now_button.right() - 2.0, now_button.center().y),
                now_label.size(),
            )
            .min,
        now_label,
        Color32::PLACEHOLDER,
    );
    if now_response.clicked() {
        let moment = crate::model::watchlist::now_ms();
        let (day, hour, minute) = fields(moment, utc_offset_secs);
        picked = Some(instant(day, hour, minute, utc_offset_secs));
        (year, month) = (day.year, day.month);
    }

    ui.data_mut(|data| data.insert_temp(month_key, (year, month)));
    picked
}

/// A two-digit number with up and down arrows at its right edge, like a macOS stepper. The
/// scroll wheel and the arrow keys (while hovered) step it too. Returns the new value.
fn stepper(ui: &mut Ui, id: Id, rect: Rect, value: i64, modulus: i64) -> i64 {
    ui.painter().rect_filled(rect, 7.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    let number = kit::galley(
        ui,
        &format!("{value:02}"),
        kit::size::BODY,
        REGULAR,
        theme::text_primary(),
        40.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 11.0, rect.center().y - number.size().y / 2.0),
        number,
        Color32::PLACEHOLDER,
    );
    let mut value = value;
    let arrows = Rect::from_min_max(pos2(rect.right() - 22.0, rect.top()), rect.max);
    for (direction, half) in [(1_i64, 0.0_f32), (-1, 1.0)] {
        let zone = Rect::from_min_size(
            pos2(arrows.left(), arrows.top() + half * rect.height() / 2.0),
            vec2(arrows.width(), rect.height() / 2.0),
        );
        let response = ui
            .interact(zone, id.with(direction), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter()
                .rect_filled(zone.shrink(2.0), 4.0, theme::ink(18));
        }
        paint_triangle(ui, zone.center(), direction > 0, theme::text_secondary());
        if response.clicked() {
            value = step(value, direction, modulus);
        }
    }
    let hovered = ui
        .interact(rect, id.with("field"), Sense::hover())
        .hovered();
    if hovered {
        let (scroll, up, down) = ui.input_mut(|input| {
            (
                input.smooth_scroll_delta.y,
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            )
        });
        if up {
            value = step(value, 1, modulus);
        }
        if down {
            value = step(value, -1, modulus);
        }
        if scroll.abs() >= 4.0 {
            value = step(value, if scroll > 0.0 { 1 } else { -1 }, modulus);
            ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
        }
    }
    value
}

fn paint_triangle(ui: &Ui, center: Pos2, up: bool, color: Color32) {
    let (half_width, half_height) = (3.0, 2.0);
    let points = if up {
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
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A `‹` or `›` drawn as two strokes.
fn paint_chevron(ui: &Ui, center: Pos2, left: bool, color: Color32) {
    let direction = if left { -1.0 } else { 1.0 };
    let tip = pos2(center.x + 2.0 * direction, center.y);
    let stroke = Stroke::new(1.6, color);
    ui.painter().line_segment(
        [pos2(center.x - 2.0 * direction, center.y - 4.5), tip],
        stroke,
    );
    ui.painter().line_segment(
        [tip, pos2(center.x - 2.0 * direction, center.y + 4.5)],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-02 13:26 at UTC+2.
    fn moment() -> i64 {
        instant(
            Day {
                year: 2026,
                month: 10,
                day: 2,
            },
            13,
            26,
            2 * 3_600,
        )
    }

    #[test]
    fn instants_round_trip_through_local_fields() {
        let offset = 2 * 3_600;
        let (day, hour, minute) = fields(moment(), offset);
        assert_eq!(
            (day.year, day.month, day.day, hour, minute),
            (2026, 10, 2, 13, 26)
        );
        // The same instant is an hour earlier in UTC.
        assert_eq!(fields(moment(), 0).1, 11);
        // A zone west of UTC can still be on the previous calendar day.
        let (day, hour, _) = fields(instant(day, 0, 5, -5 * 3_600), -5 * 3_600);
        assert_eq!((day.day, hour), (2, 0));
    }

    #[test]
    fn the_editor_text_and_the_calendar_agree() {
        let offset = -3 * 3_600;
        let text = date_text(moment(), offset);
        assert_eq!(text, "2026-10-02 08:26");
        assert_eq!(parse_date_text(&text, offset), Some(moment()));
    }

    #[test]
    fn the_grid_starts_on_monday_and_spills_into_neighbours() {
        // October 2026 begins on a Thursday: three September days lead.
        let grid = month_grid(2026, 10);
        let first = grid[0];
        assert_eq!(
            (first[0].0.month, first[0].0.day, first[0].1),
            (9, 28, false)
        );
        assert_eq!(
            (first[3].0.month, first[3].0.day, first[3].1),
            (10, 1, true)
        );
        let in_month = grid.iter().flatten().filter(|(_, inside)| *inside).count();
        assert_eq!(in_month, 31);
        // Consecutive days all the way down.
        let days: Vec<i64> = grid
            .iter()
            .flatten()
            .map(|(day, _)| days_from_civil(day.year, day.month, day.day))
            .collect();
        assert!(days.windows(2).all(|pair| pair[1] == pair[0] + 1));
    }

    #[test]
    fn a_month_starting_on_monday_has_no_leading_days() {
        // June 2026 begins on a Monday.
        let grid = month_grid(2026, 6);
        assert_eq!(
            (grid[0][0].0.month, grid[0][0].0.day, grid[0][0].1),
            (6, 1, true)
        );
    }

    #[test]
    fn months_and_clock_fields_wrap() {
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2026, 6, -18), (2024, 12));
        assert_eq!(step(23, 1, 24), 0);
        assert_eq!(step(0, -1, 24), 23);
        assert_eq!(step(59, 1, 60), 0);
        assert_eq!(step(5, -1, 60), 4);
    }
}
