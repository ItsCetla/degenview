//! Modal "Start Replay" sheet: choose the date and time the chart rewinds to, with
//! presets and a live "snaps to bar" preview. Pure view — it renders an egui window
//! over `ctx` and returns a `ReplayDateSheetResult` only when the user commits or cancels.

use std::ops::RangeInclusive;

use eframe::egui::{self, Align2, Color32, RichText, Sense, Stroke, Vec2};

use crate::format::{civil_from_days, days_from_civil};
use crate::ui::replay_bar::{date_text, time_text};
use crate::ui::theme;

/// Owned by the workspace; `show` mutates it. `open` gates the modal, `draft_ms` is the
/// UTC instant (epoch milliseconds) the fields currently hold.
#[derive(Default)]
pub struct ReplayDateSheetState {
    open: bool,
    draft_ms: i64,
}

impl ReplayDateSheetState {
    /// Whether the sheet is on screen. The workspace uses this to keep its global Esc /
    /// tool handling out of the way while the modal is up.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the sheet, seeding the draft to the midpoint of the loaded chart (or now
    /// when the chart has no data yet).
    pub fn open(&mut self, timeline: &[i64]) {
        self.open = true;
        self.draft_ms = match range_of(timeline) {
            Some((lo, hi)) => lo + (hi - lo) / 2,
            None => now_ms(),
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplayDateSheetResult {
    /// Commit: the requested start instant, UTC, in epoch milliseconds.
    Start(i64),
    Cancel,
}

/// Render the sheet when `state.is_open()`. `timeline` is the loaded chart's bar open
/// times, sorted ascending; it bounds the picker and drives the snap preview.
pub fn show(
    ctx: &egui::Context,
    state: &mut ReplayDateSheetState,
    timeline: &[i64],
) -> Option<ReplayDateSheetResult> {
    if !state.open {
        return None;
    }
    let range = range_of(timeline);
    let mut result = None;
    let mut open = true;
    egui::Window::new("Start Replay")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_size([400.0, 250.0])
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            header(ui);
            presets(ui, &mut state.draft_ms, range);
            picker(ui, &mut state.draft_ms, range);
            snap_preview(ui, state.draft_ms, timeline);
            buttons(ui, &mut result, state.draft_ms, range.is_some());
        });
    state.open = open && result.is_none();
    if result.is_none() && !open {
        result = Some(ReplayDateSheetResult::Cancel);
    }
    result
}

// MARK: Pieces

fn header(ui: &mut egui::Ui) {
    ui.label(RichText::new("Start Replay").strong().size(16.0));
    ui.label(
        RichText::new("Pick the moment the chart rewinds to.")
            .weak()
            .small(),
    );
}

fn presets(ui: &mut egui::Ui, draft_ms: &mut i64, range: Option<(i64, i64)>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_enabled_ui(range.is_some(), |ui| {
            for (label, fraction) in [
                ("Start of data", Some(0.0)),
                ("25%", Some(0.25)),
                ("50%", Some(0.5)),
                ("75%", Some(0.75)),
                ("Random", None),
            ] {
                let button = egui::Button::new(RichText::new(label).size(11.5))
                    .fill(Color32::from_white_alpha(14))
                    .corner_radius(7.0)
                    .stroke(Stroke::NONE);
                if ui.add(button).clicked()
                    && let Some((lo, hi)) = range
                {
                    let span = (hi - lo) as f64;
                    // "Random" leaves the last fifth free, like the bar's Random Bar.
                    let offset = match fraction {
                        Some(f) => span * f,
                        None => span * 0.8 * random_unit(),
                    };
                    *draft_ms = lo + offset as i64;
                }
            }
        });
    });
}

fn picker(ui: &mut egui::Ui, draft_ms: &mut i64, range: Option<(i64, i64)>) {
    ui.add_space(6.0);
    ui.label(RichText::new("Replay starts at").small().weak());
    let Some((lo, hi)) = range else {
        ui.label(RichText::new("Load a chart with data first.").weak());
        return;
    };
    let (mut year, mut month, mut day, mut hour, mut minute, _) = split_utc(*draft_ms);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Date").small().weak());
        changed |= number_field(ui, &mut year, 1970..=2200, "Year");
        changed |= number_field(ui, &mut month, 1..=12, "Month");
        changed |= number_field(ui, &mut day, 1..=days_in_month(year, month), "Day");
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Time").small().weak());
        changed |= number_field(ui, &mut hour, 0..=23, "Hour");
        changed |= number_field(ui, &mut minute, 0..=59, "Minute");
    });
    if changed {
        *draft_ms = from_utc(year, month, day, hour, minute).clamp(lo, hi);
    }
}

fn snap_preview(ui: &mut egui::Ui, draft_ms: i64, timeline: &[i64]) {
    let Some(bar) = snap_to(timeline, draft_ms) else {
        return;
    };
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        scope_icon(ui);
        ui.label(
            RichText::new(format!(
                "Snaps to the bar at {} {}",
                date_text(bar),
                time_text(bar)
            ))
            .small()
            .weak(),
        );
    });
}

fn buttons(
    ui: &mut egui::Ui,
    result: &mut Option<ReplayDateSheetResult>,
    draft_ms: i64,
    has_range: bool,
) {
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add_enabled(has_range, egui::Button::new("Start Replay"))
            .clicked()
        {
            *result = Some(ReplayDateSheetResult::Start(draft_ms));
        }
        if ui.button("Cancel").clicked() {
            *result = Some(ReplayDateSheetResult::Cancel);
        }
    });
}

// MARK: Helpers

/// A DragValue that edits `value` in place, clamped to `range`, reporting whether it changed.
fn number_field(ui: &mut egui::Ui, value: &mut i64, range: RangeInclusive<i64>, tip: &str) -> bool {
    ui.add(egui::DragValue::new(value).range(range))
        .on_hover_text(tip)
        .changed()
}

/// A small scope glyph, matching the bar's "Choose a new starting point" affordance.
fn scope_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    let center = rect.center();
    let color = theme::REPLAY;
    let stroke = Stroke::new(1.2, color);
    let painter = ui.painter();
    painter.circle_stroke(center, 5.0, stroke);
    painter.circle_filled(center, 1.4, color);
    for (from, to) in [
        (Vec2::new(0.0, -7.5), Vec2::new(0.0, -3.5)),
        (Vec2::new(0.0, 3.5), Vec2::new(0.0, 7.5)),
        (Vec2::new(-7.5, 0.0), Vec2::new(-3.5, 0.0)),
        (Vec2::new(7.5, 0.0), Vec2::new(3.5, 0.0)),
    ] {
        painter.line_segment([center + from, center + to], Stroke::new(1.2, color));
    }
}

/// Split a UTC epoch-millisecond instant into calendar fields.
fn split_utc(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// Build a UTC epoch-millisecond instant from calendar fields (seconds forced to zero).
fn from_utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    (days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60) * 1_000
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    days_from_civil(next_year, next_month, 1) - days_from_civil(year, month, 1)
}

/// The `[min, max]` span the chart covers, if it has any bars.
fn range_of(timeline: &[i64]) -> Option<(i64, i64)> {
    Some((*timeline.iter().min()?, *timeline.iter().max()?))
}

/// The bar a given instant falls on: the latest bar whose open time is at or before it.
fn snap_to(timeline: &[i64], ms: i64) -> Option<i64> {
    let index = timeline
        .partition_point(|time| *time <= ms)
        .checked_sub(1)?;
    timeline.get(index).copied()
}

fn random_unit() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (RandomState::new().build_hasher().finish() as f64) / (u64::MAX as f64)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_fields_round_trip() {
        let ms = 1_700_000_000_000; // 2023-11-14 22:13:20 UTC
        assert_eq!(split_utc(ms), (2023, 11, 14, 22, 13, 20));
        // Seconds are dropped on the way back in.
        assert_eq!(from_utc(2023, 11, 14, 22, 13), 1_699_999_980_000);
    }

    #[test]
    fn month_lengths_follow_the_leap_year() {
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2024, 12), 31);
        assert_eq!(days_in_month(2024, 11), 30);
    }

    #[test]
    fn snap_lands_on_the_bar_at_or_before() {
        let timeline = vec![1_000, 1_100, 1_200, 1_300, 1_400];
        assert_eq!(snap_to(&timeline, 1_150), Some(1_100));
        assert_eq!(snap_to(&timeline, 1_200), Some(1_200));
        assert_eq!(snap_to(&timeline, 999), None);
        assert_eq!(snap_to(&timeline, 1_500), Some(1_400));
    }

    #[test]
    fn range_and_open_seed() {
        assert_eq!(range_of(&[]), None);
        assert_eq!(range_of(&[3_000, 1_000, 2_000]), Some((1_000, 3_000)));
        let mut state = ReplayDateSheetState::default();
        assert!(!state.is_open());
        state.open(&[1_000, 2_000, 3_000]);
        assert!(state.is_open());
        assert_eq!(state.draft_ms, 2_000);
    }
}
