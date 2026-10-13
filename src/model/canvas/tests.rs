#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::model::ChartAppearance;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

// MARK: Defaults and persistence

#[test]
fn the_default_canvas_follows_the_theme_everywhere() {
    let canvas = ChartCanvas::default();
    assert_eq!(canvas.background.mode, BackgroundMode::Solid);
    assert_eq!(canvas.background.solid, None);
    assert_eq!(canvas.vertical_grid.color, None);
    assert_eq!(canvas.horizontal_grid.color, None);
    assert_eq!(canvas.scales.text_color, None);
    assert_eq!(canvas.scales.line_color, None);
    assert_eq!(canvas.watermark.mode, WatermarkMode::Hidden);
    assert_eq!(canvas.crosshair.dash, LineDash::Dashed);
    assert_eq!(canvas.buttons.navigation, ButtonVisibility::OnHover);
    assert_eq!(
        canvas.margins,
        ChartMargins {
            top: 10,
            bottom: 8,
            right: 10
        }
    );
}

#[test]
fn the_canvas_round_trips_through_json() {
    let mut canvas = ChartCanvas::default();
    canvas.background.mode = BackgroundMode::Gradient;
    canvas.background.top = Some([1, 2, 3, 255]);
    canvas.background.bottom = Some([4, 5, 6, 128]);
    canvas.vertical_grid.visible = false;
    canvas.horizontal_grid.dash = LineDash::Dotted;
    canvas.crosshair.width = 3;
    canvas.watermark.mode = WatermarkMode::Replay;
    canvas.scales.text_size = ScaleTextSize::Pt14;
    canvas.buttons.pane = ButtonVisibility::Never;
    canvas.margins.right = 42;
    let encoded = serde_json::to_string(&canvas).expect("serialize");
    let decoded: ChartCanvas = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded, canvas);
}

#[test]
fn an_old_appearance_without_a_canvas_keeps_its_palette_and_gets_the_default_canvas() {
    let mut appearance = ChartAppearance::default();
    appearance.bullish = [1, 2, 3];
    appearance.axis_decimals = Some(4);
    let mut row = serde_json::to_value(appearance).expect("serialize");
    row.as_object_mut().expect("object").remove("canvas");
    let restored: ChartAppearance = serde_json::from_value(row).expect("decode");
    assert_eq!(restored.bullish, [1, 2, 3]);
    assert_eq!(restored.axis_decimals, Some(4));
    assert_eq!(restored.canvas, ChartCanvas::default());
}

#[test]
fn a_canvas_with_only_some_fields_fills_the_rest_with_defaults() {
    let decoded: ChartCanvas =
        serde_json::from_str(r#"{"margins":{"top":20},"crosshair":{"width":2}}"#).expect("decode");
    assert_eq!(decoded.margins.top, 20);
    assert_eq!(decoded.margins.bottom, 8);
    assert_eq!(decoded.crosshair.width, 2);
    assert_eq!(decoded.crosshair.dash, LineDash::Solid);
    assert!(decoded.crosshair.visible);
    assert_eq!(decoded.vertical_grid, LineAppearance::default());
}

// MARK: Enums

#[test]
fn background_modes_and_line_styles_have_labels() {
    assert_eq!(
        BackgroundMode::ALL.map(BackgroundMode::label),
        ["Solid", "Gradient"]
    );
    assert_eq!(
        LineDash::ALL.map(LineDash::label),
        ["Solid", "Dashed", "Dotted"]
    );
}

#[test]
fn a_watermark_shows_by_mode_and_replay_state() {
    assert!(!WatermarkMode::Hidden.shows(true));
    assert!(!WatermarkMode::Hidden.shows(false));
    assert!(WatermarkMode::Symbol.shows(false));
    assert!(WatermarkMode::Symbol.shows(true));
    // Replay mode with no replay running falls back to nothing, never to the symbol.
    assert!(!WatermarkMode::Replay.shows(false));
    assert!(WatermarkMode::Replay.shows(true));
}

#[test]
fn button_visibility_decides_by_hover() {
    assert!(ButtonVisibility::Always.wanted(false));
    assert!(ButtonVisibility::OnHover.wanted(true));
    assert!(!ButtonVisibility::OnHover.wanted(false));
    assert!(!ButtonVisibility::Never.wanted(true));
}

#[test]
fn scale_text_sizes_ascend_and_the_time_axis_is_one_point_smaller() {
    let points = ScaleTextSize::ALL.map(ScaleTextSize::points);
    assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ScaleTextSize::default().points(), 10.0);
    assert_eq!(ScaleTextSize::Pt14.time_points(), 13.0);
    assert_eq!(ScaleTextSize::Pt12.label(), "12 pt");
}

// MARK: Margins

#[test]
fn margins_are_forced_into_their_bounds() {
    let wild = ChartMargins {
        top: 200,
        bottom: 99,
        right: 60_000,
    }
    .sanitized();
    assert_eq!(wild.top + wild.bottom, ChartMargins::MAX_COMBINED);
    assert!(wild.top <= ChartMargins::MAX_VERTICAL);
    assert!(wild.bottom <= ChartMargins::MAX_VERTICAL);
    assert_eq!(wild.right, ChartMargins::MAX_RIGHT);
    let fine = ChartMargins {
        top: 30,
        bottom: 20,
        right: 5,
    };
    assert_eq!(fine.sanitized(), fine);
}

#[test]
fn a_canvas_with_bad_values_is_healed_as_a_whole() {
    let mut canvas = ChartCanvas::default();
    canvas.crosshair.width = 99;
    canvas.vertical_grid.width = 0;
    canvas.margins.top = 255;
    let healed = canvas.sanitized();
    assert_eq!(healed.crosshair.width, 4);
    assert_eq!(healed.vertical_grid.width, 1);
    assert_eq!(healed.margins.top, ChartMargins::MAX_VERTICAL);
}

#[test]
fn margins_place_the_data_by_their_share_of_the_height() {
    let margins = ChartMargins {
        top: 10,
        bottom: 20,
        right: 0,
    };
    let (minimum, maximum) = price_range(100.0, 200.0, margins, false);
    let total = maximum - minimum;
    // The data spans 70% of the plot, 20% sits below it and 10% above.
    assert!(close((100.0 - minimum) / total, 0.20));
    assert!(close((maximum - 200.0) / total, 0.10));
    assert!(close(100.0 / total, 100.0 / total));
}

#[test]
fn zero_margins_fit_the_data_edge_to_edge() {
    let margins = ChartMargins {
        top: 0,
        bottom: 0,
        right: 0,
    };
    assert_eq!(price_range(5.0, 9.0, margins, false), (5.0, 9.0));
}

#[test]
fn logarithmic_margins_are_the_same_share_of_the_height() {
    let margins = ChartMargins {
        top: 10,
        bottom: 10,
        right: 0,
    };
    let (minimum, maximum) = price_range(10.0, 1000.0, margins, true);
    let (low, high) = (minimum.ln(), maximum.ln());
    assert!(close((10f64.ln() - low) / (high - low), 0.10));
    assert!(close((high - 1000f64.ln()) / (high - low), 0.10));
    assert!(minimum > 0.0);
}

#[test]
fn a_flat_series_still_gets_a_scale_around_it() {
    for logarithmic in [false, true] {
        let (minimum, maximum) = price_range(42.0, 42.0, ChartMargins::default(), logarithmic);
        assert!(minimum < 42.0 && 42.0 < maximum, "{logarithmic}");
    }
    let (minimum, maximum) = price_range(0.0, 0.0, ChartMargins::default(), false);
    assert!(minimum < 0.0 && 0.0 < maximum);
}

#[test]
fn the_price_range_stays_finite_for_extreme_and_bad_input() {
    let cases = [
        (1e-12, 3e-12),
        (1e15, 2e15),
        (-500.0, -100.0),
        (f64::NAN, 1.0),
        (f64::INFINITY, 1.0),
        (9.0, 3.0),
        (0.0, 1e300),
    ];
    for (low, high) in cases {
        for logarithmic in [false, true] {
            let (minimum, maximum) = price_range(low, high, ChartMargins::default(), logarithmic);
            assert!(
                minimum.is_finite() && maximum.is_finite() && minimum < maximum,
                "{low} {high} {logarithmic}: {minimum} {maximum}"
            );
        }
    }
}

// MARK: Ticks

#[test]
fn linear_ticks_are_round_multiples_inside_the_range() {
    let ticks = price_ticks(98.0, 207.0, false, 6);
    assert!(!ticks.is_empty() && ticks.len() <= 6, "{ticks:?}");
    assert!(ticks.iter().all(|tick| (98.0..=207.0).contains(tick)));
    let step = ticks[1] - ticks[0];
    assert!(
        close(step, 20.0) || close(step, 25.0) || close(step, 50.0),
        "{step}"
    );
    assert!(ticks.windows(2).all(|pair| close(pair[1] - pair[0], step)));
    assert!(
        ticks
            .iter()
            .all(|tick| close(tick / step, (tick / step).round()))
    );
}

#[test]
fn ticks_cope_with_tiny_and_huge_prices() {
    let tiny = price_ticks(0.000_001_2, 0.000_001_9, false, 5);
    assert!(!tiny.is_empty());
    assert!(
        tiny.iter()
            .all(|tick| *tick >= 0.000_001_2 && *tick <= 0.000_001_9)
    );
    let huge = price_ticks(1.0e9, 3.5e9, false, 5);
    assert!(!huge.is_empty());
    assert!(huge.iter().all(|tick| tick.is_finite()));
}

#[test]
fn ticks_cross_zero_for_negative_values() {
    let ticks = price_ticks(-30.0, 40.0, false, 7);
    assert!(ticks.contains(&0.0), "{ticks:?}");
    assert!(ticks.iter().any(|tick| *tick < 0.0));
}

#[test]
fn logarithmic_ticks_are_decades_and_thin_to_fit() {
    let roomy = price_ticks(1.0, 1000.0, true, 12);
    assert!(roomy.contains(&1.0) && roomy.contains(&10.0) && roomy.contains(&100.0));
    assert!(roomy.contains(&1000.0));
    assert!(roomy.len() > 4, "{roomy:?}");
    let tight = price_ticks(1.0, 1000.0, true, 4);
    assert_eq!(tight, vec![1.0, 10.0, 100.0, 1000.0]);
    let tighter = price_ticks(1e-3, 1e9, true, 4);
    assert!(!tighter.is_empty() && tighter.len() <= 4, "{tighter:?}");
}

#[test]
fn a_narrow_logarithmic_range_falls_back_to_round_prices() {
    let ticks = price_ticks(100.0, 140.0, true, 6);
    assert!(ticks.len() >= 2);
    assert!(ticks.iter().all(|tick| (100.0..=140.0).contains(tick)));
}

#[test]
fn ticks_are_empty_for_a_degenerate_scale() {
    assert!(price_ticks(5.0, 5.0, false, 6).is_empty());
    assert!(price_ticks(f64::NAN, 5.0, false, 6).is_empty());
    assert!(price_ticks(1.0, 5.0, false, 0).is_empty());
    assert!(price_ticks(9.0, 1.0, false, 4).is_empty());
}

#[test]
fn the_tick_budget_follows_the_height() {
    assert_eq!(tick_budget(560.0, 56.0), 10);
    assert_eq!(tick_budget(10.0, 56.0), 1);
    assert_eq!(tick_budget(0.0, 56.0), 0);
    assert_eq!(tick_budget(f32::NAN, 56.0), 0);
}

#[test]
fn time_ticks_stay_on_their_bars_while_the_chart_pans() {
    // 20-point slots: a label every fifth bar of the series.
    let at = |first| {
        time_ticks(first, 30, 20.0)
            .into_iter()
            .map(|index| first + index)
            .collect::<Vec<_>>()
    };
    let before = at(100);
    let panned = at(103);
    for bar in &before {
        assert!(bar % 5 == 0);
        if (103..130).contains(bar) {
            assert!(panned.contains(bar), "{bar} moved");
        }
    }
    assert_eq!(time_tick_step(20.0), 5);
    assert_eq!(time_tick_step(500.0), 1);
    assert_eq!(time_tick_step(f32::NAN), 100);
}

// MARK: Window

#[test]
fn the_window_without_a_margin_is_the_last_visible_bars() {
    let window = visible_window(500, 120, 0, 0);
    assert_eq!((window.start, window.end), (380, 500));
    assert_eq!((window.blank_left, window.slots), (0, 120));
    assert_eq!(window.count(), 120);
}

#[test]
fn a_right_margin_reserves_empty_slots_past_the_newest_bar() {
    let window = visible_window(500, 120, 0, 10);
    assert_eq!((window.start, window.end), (380, 500));
    assert_eq!(window.slots, 130, "the margin is part of the plot");
    // The newest bar sits ten slots in from the right edge.
    let newest_slot = window.blank_left + window.count() - 1;
    assert_eq!(window.slots - 1 - newest_slot, 10);
}

#[test]
fn scrolling_back_fills_the_margin_with_later_bars_at_the_same_spacing() {
    let at_rest = visible_window(500, 120, 0, 10);
    let scrolled = visible_window(500, 120, 4, 10);
    assert_eq!(scrolled.slots, at_rest.slots, "bar spacing does not change");
    assert_eq!(scrolled.end, 500, "still inside the margin");
    assert_eq!(scrolled.start, 376);
    let far = visible_window(500, 120, 30, 10);
    assert_eq!((far.start, far.end), (350, 480));
    assert_eq!(far.slots, 130);
    assert_eq!(far.blank_left, 0);
    assert_eq!(far.count(), 130, "the margin's slots show real bars now");
}

#[test]
fn a_short_series_sits_at_the_right_of_the_window() {
    let window = visible_window(30, 120, 0, 0);
    assert_eq!((window.start, window.end), (0, 30));
    assert_eq!(window.blank_left, 90);
    let with_margin = visible_window(30, 120, 0, 10);
    assert_eq!(
        with_margin.blank_left, 90,
        "the margin is at the right, not the left"
    );
    assert_eq!(with_margin.slots, 130);
}

#[test]
fn an_empty_or_overscrolled_series_has_an_empty_window() {
    assert_eq!(visible_window(0, 120, 0, 10).count(), 0);
    assert_eq!(visible_window(10, 120, 500, 10).count(), 0);
    let window = visible_window(10, 0, 0, 0);
    assert_eq!(window.slots, 1);
}

// MARK: Panes

const AXIS: f32 = 24.0;

fn spec(id: u64, collapsed: bool) -> PaneSpec {
    PaneSpec { id, collapsed }
}

#[test]
fn without_panes_the_main_chart_has_everything() {
    let split = split_panes(600.0, &[], None, AXIS);
    assert_eq!(split.main, 600.0);
    assert!(split.panes.is_empty() && !split.main_collapsed);
}

#[test]
fn expanded_panes_share_thirty_percent_and_fill_the_area_exactly() {
    let split = split_panes(600.0, &[spec(1, false), spec(2, false)], None, AXIS);
    assert_eq!(split.panes, vec![90.0, 90.0]);
    assert_eq!(split.main, 420.0);
    let odd = split_panes(
        601.0,
        &[spec(1, false), spec(2, false), spec(3, false)],
        None,
        AXIS,
    );
    assert_eq!(odd.main + odd.panes.iter().sum::<f32>(), 601.0);
}

#[test]
fn a_collapsed_pane_is_its_strip_and_returns_nothing_to_the_others() {
    let split = split_panes(600.0, &[spec(1, true), spec(2, false)], None, AXIS);
    assert_eq!(split.panes[0], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[1], 180.0);
    assert_eq!(split.main, 600.0 - 180.0 - COLLAPSED_PANE_HEIGHT);
    let all = split_panes(600.0, &[spec(1, true), spec(2, true)], None, AXIS);
    assert_eq!(all.panes, vec![COLLAPSED_PANE_HEIGHT; 2]);
    assert_eq!(all.main, 600.0 - 2.0 * COLLAPSED_PANE_HEIGHT);
}

#[test]
fn a_maximized_pane_takes_everything_but_the_strips_and_the_time_axis() {
    let panes = [spec(1, false), spec(2, false), spec(3, true)];
    let split = split_panes(600.0, &panes, Some(2), AXIS);
    assert!(split.main_collapsed);
    assert_eq!(split.main, AXIS);
    assert_eq!(split.panes[0], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[2], COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.panes[1], 600.0 - AXIS - 2.0 * COLLAPSED_PANE_HEIGHT);
    assert_eq!(split.main + split.panes.iter().sum::<f32>(), 600.0);
}

#[test]
fn maximizing_a_pane_that_is_gone_or_too_tall_for_the_chart_changes_nothing() {
    let panes = [spec(1, false)];
    let gone = split_panes(600.0, &panes, Some(9), AXIS);
    assert!(!gone.main_collapsed);
    let tiny = split_panes(70.0, &panes, Some(1), AXIS);
    assert!(!tiny.main_collapsed, "no room to maximize into");
}

#[test]
fn a_short_chart_keeps_its_main_area_before_its_panes() {
    let split = split_panes(150.0, &[spec(1, false), spec(2, false)], None, AXIS);
    assert!(split.main >= 60.0, "{split:?}");
    assert!(split.panes.iter().all(|height| *height >= 0.0));
    assert!((split.main + split.panes.iter().sum::<f32>() - 150.0).abs() < 0.001);
    let zero = split_panes(0.0, &[spec(1, false)], None, AXIS);
    assert!(zero.main >= 0.0 && zero.panes.iter().all(|height| *height >= 0.0));
    let nan = split_panes(f32::NAN, &[spec(1, false)], None, AXIS);
    assert!(nan.main.is_finite());
}
