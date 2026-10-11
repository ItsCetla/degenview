//! The strip above the charts while a replay is open: status, transport, speed and
//! resolution, the timeline, the cursor's clock and the way back to live. Pure view:
//! it reads the engine and returns `ReplayBarAction`s for the workspace to apply.

use crate::format::civil_from_days;
use crate::model::{ReplayEngine, ReplayInterval, ReplaySpeed, ReplayStatus};
use crate::service::timezone::local_utc_offset_secs;
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, Response, RichText, Sense, Shape,
    Stroke, Vec2,
};

/// Below this width the timeline drops to a second row.
const WIDE_ROW_MIN_WIDTH: f32 = 930.0;
/// Below this width the start options collapse into one menu.
const START_OPTIONS_MIN_WIDTH: f32 = 420.0;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub struct ReplayBarInput<'a> {
    pub engine: &'a ReplayEngine,
    /// History is loading: transport and timeline are disabled.
    pub is_preparing: bool,
    /// A dismissible fallback notice.
    pub notice: Option<&'a str>,
    /// What Auto resolved to for the primary chart, when it has granular data.
    pub resolved_interval: Option<ReplayInterval>,
    pub available_intervals: &'a [ReplayInterval],
}

/// Reserve the space Swift's one-row or wrapped transport occupies, including the tint border.
pub fn preferred_height(status: ReplayStatus, width: f32, has_notice: bool) -> f32 {
    let rows = if matches!(status, ReplayStatus::Ready | ReplayStatus::SelectingStart)
        || width >= WIDE_ROW_MIN_WIDTH
    {
        50.0
    } else {
        82.0
    };
    rows + if has_notice { 38.0 } else { 0.0 }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplayBarAction {
    SelectOnChart,
    ChooseDate,
    RandomBar,
    FirstBar,
    CancelSelection,
    ReturnToLive,
    ChangeStart,
    Restart,
    StepBack,
    StepForward,
    TogglePlayback,
    /// Timeline fraction, 0…=1.
    Seek(f64),
    SetSpeed(ReplaySpeed),
    SetInterval(ReplayInterval),
    DismissNotice,
}

// MARK: Pure helpers

pub fn tint(status: ReplayStatus, preparing: bool) -> Color32 {
    if preparing {
        return Color32::from_gray(150);
    }
    match status {
        ReplayStatus::Playing => theme::UP,
        ReplayStatus::Completed => Color32::from_gray(150),
        _ => theme::REPLAY,
    }
}

pub fn title(status: ReplayStatus, preparing: bool) -> &'static str {
    if preparing {
        return "Loading history";
    }
    match status {
        ReplayStatus::Inactive => "Off",
        ReplayStatus::Ready => "Choose a start",
        ReplayStatus::SelectingStart => "Pick a start",
        ReplayStatus::Paused => "Paused",
        ReplayStatus::Playing => "Playing",
        ReplayStatus::Completed => "Ended",
    }
}

fn last_index(engine: &ReplayEngine) -> usize {
    engine.timeline.len().saturating_sub(1)
}

/// Cursor position along the timeline, 0…=1 (0 for a single bar).
pub fn progress(engine: &ReplayEngine) -> f64 {
    let Some(session) = &engine.session else {
        return 0.0;
    };
    match last_index(engine) {
        0 => 0.0,
        last => (session.bar_index.min(last) as f64) / last as f64,
    }
}

/// Where the replay started along the timeline, 0…=1.
pub fn start_fraction(engine: &ReplayEngine) -> f64 {
    let Some(session) = &engine.session else {
        return 0.0;
    };
    let last = last_index(engine);
    if last == 0 {
        return 0.0;
    }
    let index = engine
        .timeline
        .partition_point(|time| *time <= session.start_ms)
        .saturating_sub(1);
    index as f64 / last as f64
}

/// 1-based step number and total, for "Step 3 of 40".
pub fn step_of(engine: &ReplayEngine) -> (usize, usize) {
    let number = engine
        .session
        .as_ref()
        .map_or(0, |session| session.bar_index + 1);
    (number, engine.timeline.len())
}

pub fn can_step_back(engine: &ReplayEngine) -> bool {
    engine
        .session
        .as_ref()
        .is_some_and(|session| session.bar_index > 0)
}

pub fn can_advance(engine: &ReplayEngine) -> bool {
    engine
        .session
        .as_ref()
        .is_some_and(|session| session.bar_index + 1 < engine.timeline.len())
}

fn can_toggle(engine: &ReplayEngine, preparing: bool) -> bool {
    !preparing
        && matches!(
            engine.status,
            ReplayStatus::Paused | ReplayStatus::Playing | ReplayStatus::Completed
        )
}

fn split_utc(epoch_ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = epoch_ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// "Oct 9, 2026"
pub fn date_text(epoch_ms: i64) -> String {
    let (year, month, day, ..) = split_utc(epoch_ms);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// "14:03:07"
pub fn time_text(epoch_ms: i64) -> String {
    let (_, _, _, hour, minute, second) = split_utc(epoch_ms);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// "Oct 9, 14:03"
pub fn compact_text(epoch_ms: i64) -> String {
    let local_ms = epoch_ms + local_utc_offset_secs(epoch_ms) * 1_000;
    let (_, month, day, hour, minute, _) = split_utc(local_ms);
    format!(
        "{} {day}, {hour:02}:{minute:02}",
        MONTHS[(month - 1) as usize]
    )
}

fn interval_text(engine: &ReplayEngine, resolved: Option<ReplayInterval>) -> String {
    let chosen = engine
        .session
        .as_ref()
        .map_or(ReplayInterval::Automatic, |session| session.interval);
    match (chosen, resolved) {
        (ReplayInterval::Automatic, Some(resolved)) => format!("Auto · {}", resolved.label()),
        _ => chosen.label().to_owned(),
    }
}

fn mix(base: Color32, over: Color32, amount: f32) -> Color32 {
    let blend = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount) as u8;
    Color32::from_rgb(
        blend(base.r(), over.r()),
        blend(base.g(), over.g()),
        blend(base.b(), over.b()),
    )
}

// MARK: Bar

pub fn show(ui: &mut egui::Ui, input: &ReplayBarInput) -> Vec<ReplayBarAction> {
    let mut actions = Vec::new();
    let engine = input.engine;

    let frame = egui::Frame::new()
        .fill(mix(theme::CHROME, theme::REPLAY, 0.06))
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match engine.status {
                ReplayStatus::Ready => ready_row(ui, input, &mut actions),
                ReplayStatus::SelectingStart => selecting_row(ui, input, &mut actions),
                _ if ui.available_width() >= WIDE_ROW_MIN_WIDTH => {
                    ui.horizontal(|ui| wide_row(ui, input, &mut actions));
                }
                _ => narrow_rows(ui, input, &mut actions),
            }
            if let Some(message) = input.notice {
                notice_chip(ui, message, &mut actions);
            }
        });
    let rect = frame.response.rect;
    if input.is_preparing {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0)),
            0.0,
            theme::REPLAY,
        );
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::REPLAY.gamma_multiply(0.35)),
    );

    shortcuts(ui, input, &mut actions);
    actions
}

fn shortcuts(ui: &egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    if !can_toggle(input.engine, input.is_preparing) || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    ui.input_mut(|state| {
        if state.consume_key(Modifiers::SHIFT, Key::ArrowLeft) && can_step_back(input.engine) {
            actions.push(ReplayBarAction::StepBack);
        }
        if state.consume_key(Modifiers::SHIFT, Key::ArrowRight) && can_advance(input.engine) {
            actions.push(ReplayBarAction::StepForward);
        }
        if state.consume_key(Modifiers::SHIFT, Key::ArrowDown) {
            toggle_actions(input.engine, actions);
        }
    });
}

/// At the end of the data play becomes "replay again".
fn toggle_actions(engine: &ReplayEngine, actions: &mut Vec<ReplayBarAction>) {
    if engine.status == ReplayStatus::Completed {
        actions.push(ReplayBarAction::Restart);
    }
    actions.push(ReplayBarAction::TogglePlayback);
}

fn wide_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    status_chip(ui, input.engine.status, input.is_preparing);
    transport(ui, input, actions);
    pickers(ui, input, actions);
    let timeline = (ui.available_width() - 220.0).max(140.0);
    scrubber(ui, input, timeline, actions);
    clock(ui, input.engine, false);
    row_actions(ui, actions);
}

fn narrow_rows(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    ui.horizontal(|ui| {
        status_chip(ui, input.engine.status, input.is_preparing);
        transport(ui, input, actions);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clock(ui, input.engine, true);
        });
    });
    ui.horizontal(|ui| {
        pickers(ui, input, actions);
        let timeline = (ui.available_width() - 145.0).max(60.0);
        scrubber(ui, input, timeline, actions);
        row_actions(ui, actions);
    });
}

fn ready_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    ui.horizontal(|ui| {
        status_chip(ui, ReplayStatus::Ready, input.is_preparing);
        ui.add_enabled_ui(!input.is_preparing, |ui| {
            start_options(ui, actions);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close_actions(ui, actions);
        });
    });
}

fn selecting_row(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    ui.horizontal(|ui| {
        status_chip(ui, ReplayStatus::SelectingStart, input.is_preparing);
        let (icon, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        paint_glyph(ui.painter(), icon.center(), Glyph::Scope, theme::REPLAY);
        ui.label(
            RichText::new("Click a candle to start the replay")
                .size(12.0)
                .strong(),
        );
        ui.label(RichText::new("Esc to cancel").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Cancel")
                .on_hover_text("Cancel (Esc)")
                .clicked()
            {
                actions.push(ReplayBarAction::CancelSelection);
            }
        });
    });
}

// MARK: Pieces

fn status_chip(ui: &mut egui::Ui, status: ReplayStatus, preparing: bool) {
    let color = tint(status, preparing);
    let label = title(status, preparing);
    let pulse = status == ReplayStatus::Playing && !preparing;
    egui::Frame::new()
        .fill(theme::REPLAY.gamma_multiply(0.14))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                icons::paint(ui.painter(), icon, Icon::ClockArrow, theme::REPLAY);
                ui.label(
                    RichText::new("REPLAY")
                        .color(theme::REPLAY)
                        .size(10.5)
                        .strong(),
                );
                let (divider, _) = ui.allocate_exact_size(Vec2::new(1.0, 10.0), Sense::hover());
                ui.painter()
                    .rect_filled(divider, 0.0, theme::REPLAY.gamma_multiply(0.35));
                if preparing {
                    ui.add(egui::Spinner::new().size(8.0));
                } else {
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    let alpha = if pulse {
                        let wave = (ui.input(|input| input.time) * 2.5).sin() as f32;
                        0.65 + 0.35 * wave
                    } else {
                        1.0
                    };
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, color.gamma_multiply(alpha));
                    if pulse {
                        ui.ctx().request_repaint();
                    }
                }
                ui.label(RichText::new(label).color(theme::REPLAY).size(11.0));
            });
        })
        .response
        .on_hover_text(format!("Historical replay, {label}"));
}

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Restart,
    StepBack,
    StepForward,
    Scope,
    Close,
}

fn glyph_button(ui: &mut egui::Ui, glyph: Glyph, enabled: bool, tip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(24.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = accessibility::label(response, tip).on_hover_text(tip);
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, 5.0, Color32::from_white_alpha(18));
    }
    let base = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    };
    paint_glyph(ui.painter(), rect.center(), glyph, base);
    response
}

fn triangle(center: Pos2, half: f32, pointing_right: bool) -> Vec<Pos2> {
    let sign = if pointing_right { 1.0 } else { -1.0 };
    vec![
        Pos2::new(center.x + sign * half, center.y),
        Pos2::new(center.x - sign * half * 0.8, center.y - half),
        Pos2::new(center.x - sign * half * 0.8, center.y + half),
    ]
}

fn paint_glyph(painter: &egui::Painter, center: Pos2, glyph: Glyph, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    match glyph {
        Glyph::Restart => {
            painter.line_segment(
                [
                    center + Vec2::new(-5.5, -5.0),
                    center + Vec2::new(-5.5, 5.0),
                ],
                stroke,
            );
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(1.5, 0.0), 5.0, false),
                color,
                Stroke::NONE,
            ));
        }
        Glyph::StepBack => {
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(-1.0, 0.0), 5.0, false),
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [center + Vec2::new(5.0, -5.0), center + Vec2::new(5.0, 5.0)],
                stroke,
            );
        }
        Glyph::StepForward => {
            painter.line_segment(
                [
                    center + Vec2::new(-5.0, -5.0),
                    center + Vec2::new(-5.0, 5.0),
                ],
                stroke,
            );
            painter.add(Shape::convex_polygon(
                triangle(center + Vec2::new(1.0, 0.0), 5.0, true),
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Scope => {
            painter.circle_stroke(center, 5.5, stroke);
            painter.circle_filled(center, 1.6, color);
            for (from, to) in [
                (Vec2::new(0.0, -8.0), Vec2::new(0.0, -4.0)),
                (Vec2::new(0.0, 4.0), Vec2::new(0.0, 8.0)),
                (Vec2::new(-8.0, 0.0), Vec2::new(-4.0, 0.0)),
                (Vec2::new(4.0, 0.0), Vec2::new(8.0, 0.0)),
            ] {
                painter.line_segment([center + from, center + to], Stroke::new(1.2, color));
            }
        }
        Glyph::Close => {
            for (a, b) in [
                (Vec2::new(-4.0, -4.0), Vec2::new(4.0, 4.0)),
                (Vec2::new(-4.0, 4.0), Vec2::new(4.0, -4.0)),
            ] {
                painter.line_segment([center + a, center + b], stroke);
            }
        }
    }
}

fn transport(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let busy = input.is_preparing;
    ui.spacing_mut().item_spacing.x = 2.0;
    if glyph_button(
        ui,
        Glyph::Restart,
        !busy && engine.session.is_some(),
        "Back to start",
    )
    .clicked()
    {
        actions.push(ReplayBarAction::Restart);
    }
    if glyph_button(
        ui,
        Glyph::StepBack,
        !busy && can_step_back(engine),
        "Step Back (Shift–Left Arrow)",
    )
    .clicked()
    {
        actions.push(ReplayBarAction::StepBack);
    }
    play_button(ui, input, actions);
    if glyph_button(
        ui,
        Glyph::StepForward,
        !busy && can_advance(engine),
        "Step Forward (Shift–Right Arrow)",
    )
    .clicked()
    {
        actions.push(ReplayBarAction::StepForward);
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

fn play_button(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let enabled = can_toggle(engine, input.is_preparing);
    let ended = engine.status == ReplayStatus::Completed;
    let playing = engine.status == ReplayStatus::Playing;
    let verb = if ended {
        "Replay Again"
    } else if playing {
        "Pause"
    } else {
        "Play"
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(28.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let fill = theme::REPLAY.gamma_multiply(if !enabled {
        0.4
    } else if response.hovered() {
        1.0
    } else {
        0.92
    });
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 14.0, fill);
    let white = Color32::WHITE;
    if ended {
        // Counter-clockwise arrow: an open arc with a head.
        let radius = 5.5;
        let arc: Vec<Pos2> = (0..=20)
            .map(|step| {
                let angle = -0.4 + (step as f32 / 20.0) * 4.9;
                rect.center() + Vec2::new(angle.cos(), -angle.sin()) * radius
            })
            .collect();
        let head = *arc.last().unwrap_or(&rect.center());
        painter.add(Shape::line(arc, Stroke::new(1.8, white)));
        painter.add(Shape::convex_polygon(
            vec![
                head + Vec2::new(-3.5, -1.0),
                head + Vec2::new(3.0, -2.5),
                head + Vec2::new(0.5, 3.5),
            ],
            white,
            Stroke::NONE,
        ));
    } else if playing {
        for offset in [-3.0, 3.0] {
            painter.rect_filled(
                Rect::from_center_size(
                    rect.center() + Vec2::new(offset, 0.0),
                    Vec2::new(3.0, 10.0),
                ),
                1.0,
                white,
            );
        }
    } else {
        painter.add(Shape::convex_polygon(
            triangle(rect.center() + Vec2::new(1.0, 0.0), 5.5, true),
            white,
            Stroke::NONE,
        ));
    }
    if response
        .on_hover_text(format!("{verb} (Shift–Down Arrow)"))
        .clicked()
        && enabled
    {
        toggle_actions(engine, actions);
    }
}

fn pickers(ui: &mut egui::Ui, input: &ReplayBarInput, actions: &mut Vec<ReplayBarAction>) {
    let engine = input.engine;
    let speed = engine.session.as_ref().map(|session| session.speed);
    picker_menu(
        ui,
        (
            "Speed",
            speed.unwrap_or(ReplaySpeed::Normal).label(),
            "Playback speed",
        ),
        &ReplaySpeed::ALL,
        speed,
        ReplaySpeed::label,
        ReplayBarAction::SetSpeed,
        actions,
    );
    let chosen = engine.session.as_ref().map(|session| session.interval);
    ui.add_enabled_ui(!input.is_preparing, |ui| {
        picker_menu(
            ui,
            ("Bars", &interval_text(engine, input.resolved_interval), "Replay resolution. Auto picks the finest history that fits; 1 bar steps whole chart bars."),
            input.available_intervals,
            chosen,
            ReplayInterval::label,
            ReplayBarAction::SetInterval,
            actions,
        );
    });
}

fn picker_menu<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    (caption, value, tip): (&str, &str, &str),
    options: &[T],
    selected: Option<T>,
    label: impl Fn(T) -> &'static str,
    action: impl Fn(T) -> ReplayBarAction,
    actions: &mut Vec<ReplayBarAction>,
) {
    let response = ui
        .menu_button(
            RichText::new(format!("        {value}"))
                .size(11.5)
                .strong(),
            |ui| {
                for &option in options {
                    let text = if selected == Some(option) {
                        format!("✓  {}", label(option))
                    } else {
                        format!("    {}", label(option))
                    };
                    if ui.button(text).clicked() {
                        actions.push(action(option));
                        ui.close();
                    }
                }
            },
        )
        .response
        .on_hover_text(tip);
    ui.painter().text(
        Pos2::new(response.rect.left() + 8.0, response.rect.center().y),
        Align2::LEFT_CENTER,
        caption,
        FontId::proportional(10.0),
        theme::TEXT_SECONDARY,
    );
}

fn scrubber(
    ui: &mut egui::Ui,
    input: &ReplayBarInput,
    width: f32,
    actions: &mut Vec<ReplayBarAction>,
) {
    let engine = input.engine;
    let enabled = !input.is_preparing && engine.session.is_some();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, 18.0),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let active = enabled && (response.hovered() || response.dragged());
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
    let fraction = progress(engine) as f32;
    let thumb_x = rect.left() + rect.width() * fraction;
    let alpha = if enabled { 1.0 } else { 0.4 };
    let painter = ui.painter();
    painter.rect_filled(
        track,
        2.0,
        Color32::from_white_alpha(30).gamma_multiply(alpha),
    );
    painter.rect_filled(
        Rect::from_min_max(track.min, Pos2::new(thumb_x, track.max.y)),
        2.0,
        theme::REPLAY.gamma_multiply(alpha),
    );
    if engine.session.is_some() {
        let tick_x = rect.left() + rect.width() * start_fraction(engine) as f32;
        painter.line_segment(
            [
                Pos2::new(tick_x, rect.center().y - 5.0),
                Pos2::new(tick_x, rect.center().y + 5.0),
            ],
            Stroke::new(2.0, Color32::from_white_alpha(115).gamma_multiply(alpha)),
        );
        painter.circle(
            Pos2::new(thumb_x, rect.center().y),
            if active { 6.5 } else { 5.5 },
            Color32::WHITE.gamma_multiply(alpha),
            Stroke::new(2.0, theme::REPLAY.gamma_multiply(alpha)),
        );
    }
    if enabled
        && (response.dragged() || response.clicked())
        && let Some(position) = response.interact_pointer_pos()
    {
        let target = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        actions.push(ReplayBarAction::Seek(f64::from(target)));
    }
    let (number, count) = step_of(engine);
    response.on_hover_text(format!("Step {number} of {count}"));
}

fn clock(ui: &mut egui::Ui, engine: &ReplayEngine, compact: bool) {
    let Some(ms) = engine.current_ms() else {
        ui.label(RichText::new("—").weak());
        return;
    };
    if compact {
        ui.label(RichText::new(compact_text(ms)).monospace().strong());
    } else {
        let local_ms = ms + local_utc_offset_secs(ms) * 1_000;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.label(RichText::new(date_text(local_ms)).small().weak());
                ui.label(
                    RichText::new(time_text(local_ms))
                        .monospace()
                        .strong()
                        .size(13.0),
                );
            });
        });
    }
}

fn row_actions(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    ui.spacing_mut().item_spacing.x = 2.0;
    if glyph_button(ui, Glyph::Scope, true, "Choose a new starting point").clicked() {
        actions.push(ReplayBarAction::ChangeStart);
    }
    // Laid out right-to-left elsewhere; here the pieces follow left to right.
    live_button(ui, actions);
    if glyph_button(ui, Glyph::Close, true, "Close Replay").clicked() {
        actions.push(ReplayBarAction::ReturnToLive);
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

/// Right-to-left variant for the ready row, where the group hugs the right edge.
fn close_actions(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    if glyph_button(ui, Glyph::Close, true, "Close Replay").clicked() {
        actions.push(ReplayBarAction::ReturnToLive);
    }
    live_button(ui, actions);
}

fn live_button(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(68.0, 26.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 999.0, theme::REPLAY.gamma_multiply(0.10));
    }
    painter.rect_stroke(
        rect,
        999.0,
        Stroke::new(1.0, theme::REPLAY.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    let centre = Pos2::new(rect.left() + 16.0, rect.center().y);
    painter.circle_filled(centre, 2.0, theme::REPLAY);
    for side in [-1.0, 1.0] {
        painter.line_segment(
            [
                centre + Vec2::new(side * 5.0, -3.5),
                centre + Vec2::new(side * 5.0, 3.5),
            ],
            Stroke::new(1.0, theme::REPLAY),
        );
    }
    painter.text(
        Pos2::new(rect.left() + 29.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Live",
        FontId::proportional(11.5),
        theme::REPLAY,
    );
    if accessibility::label(response, "Return to live market")
        .on_hover_text("Leave replay and return to the live market")
        .clicked()
    {
        actions.push(ReplayBarAction::ReturnToLive);
    }
}

fn start_options(ui: &mut egui::Ui, actions: &mut Vec<ReplayBarAction>) {
    if ui.available_width() < START_OPTIONS_MIN_WIDTH {
        let response = ui
            .menu_button("Start     ", |ui| {
                for (label, action) in [
                    ("Select Bar on Chart", ReplayBarAction::SelectOnChart),
                    ("Date & Time…", ReplayBarAction::ChooseDate),
                    ("Random Bar", ReplayBarAction::RandomBar),
                    ("First Bar", ReplayBarAction::FirstBar),
                ] {
                    if ui.button(label).clicked() {
                        actions.push(action);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Choose where the replay starts");
        ui.painter()
            .rect_filled(response.rect, 999.0, theme::REPLAY);
        let icon = response.rect.left_center() + Vec2::new(13.0, 0.0);
        ui.painter().add(Shape::convex_polygon(
            triangle(icon, 3.5, true),
            Color32::WHITE,
            Stroke::NONE,
        ));
        ui.painter().text(
            Pos2::new(response.rect.left() + 25.0, response.rect.center().y),
            Align2::LEFT_CENTER,
            "Start",
            FontId::proportional(11.5),
            Color32::WHITE,
        );
        let chevron = Pos2::new(response.rect.right() - 11.0, response.rect.center().y + 1.0);
        ui.painter().line_segment(
            [chevron + Vec2::new(-3.0, -2.0), chevron],
            Stroke::new(1.2, Color32::WHITE),
        );
        ui.painter().line_segment(
            [chevron, chevron + Vec2::new(3.0, -2.0)],
            Stroke::new(1.2, Color32::WHITE),
        );
        return;
    }
    let labels = ui.available_width() >= 650.0;
    let primary_width = if labels { 149.0 } else { 80.0 };
    let (primary, response) =
        ui.allocate_exact_size(Vec2::new(primary_width, 28.0), Sense::click());
    ui.painter().rect_filled(
        primary,
        999.0,
        theme::REPLAY.gamma_multiply(if response.hovered() { 1.0 } else { 0.92 }),
    );
    paint_glyph(
        ui.painter(),
        Pos2::new(primary.left() + 15.0, primary.center().y),
        Glyph::Scope,
        Color32::WHITE,
    );
    ui.painter().text(
        Pos2::new(primary.left() + 28.0, primary.center().y),
        Align2::LEFT_CENTER,
        if labels {
            "Select Bar on Chart"
        } else {
            "Select"
        },
        FontId::proportional(11.5),
        Color32::WHITE,
    );
    if accessibility::label(response, "Select bar on chart")
        .on_hover_text("Click a candle on a chart to start the replay there")
        .clicked()
    {
        actions.push(ReplayBarAction::SelectOnChart);
    }
    if labels {
        ui.label(RichText::new("or").size(10.5).color(theme::TEXT_TERTIARY));
    }
    egui::Frame::new()
        .fill(Color32::from_white_alpha(14))
        .corner_radius(7.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                start_segment(
                    ui,
                    (
                        "Date & Time",
                        Icon::Calendar,
                        "Pick a date and time to start from",
                    ),
                    labels,
                    111.0,
                    ReplayBarAction::ChooseDate,
                    actions,
                );
                start_divider(ui);
                start_segment(
                    ui,
                    (
                        "Random",
                        Icon::Circle,
                        "Start from a random bar, leaving room to play forward",
                    ),
                    labels,
                    83.0,
                    ReplayBarAction::RandomBar,
                    actions,
                );
                start_divider(ui);
                start_segment(
                    ui,
                    (
                        "First Bar",
                        Icon::ClockArrow,
                        "Start from the first bar the chart has loaded",
                    ),
                    labels,
                    91.0,
                    ReplayBarAction::FirstBar,
                    actions,
                );
            });
        });
}

fn start_segment(
    ui: &mut egui::Ui,
    (title, icon, tip): (&str, Icon, &str),
    labels: bool,
    full_width: f32,
    action: ReplayBarAction,
    actions: &mut Vec<ReplayBarAction>,
) {
    let width = if labels { full_width } else { 33.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_white_alpha(20));
    }
    let icon_left = if labels {
        rect.left() + 10.0
    } else {
        rect.center().x - 6.0
    };
    let icon_rect = Rect::from_min_size(
        Pos2::new(icon_left, rect.center().y - 6.0),
        Vec2::splat(12.0),
    );
    if title == "Random" {
        ui.painter().rect_stroke(
            icon_rect.shrink(0.8),
            2.0,
            Stroke::new(1.2, theme::REPLAY),
            egui::StrokeKind::Middle,
        );
        for (x, y) in [(3.5, 3.5), (8.5, 3.5), (6.0, 6.0), (3.5, 8.5), (8.5, 8.5)] {
            ui.painter()
                .circle_filled(icon_rect.min + Vec2::new(x, y), 0.8, theme::REPLAY);
        }
    } else if title == "First Bar" {
        paint_glyph(
            ui.painter(),
            icon_rect.center(),
            Glyph::Restart,
            theme::REPLAY,
        );
    } else {
        icons::paint(ui.painter(), icon_rect, icon, theme::REPLAY);
    }
    if labels {
        ui.painter().text(
            Pos2::new(rect.left() + 28.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(11.5),
            theme::TEXT_PRIMARY,
        );
    }
    if accessibility::label(response, title)
        .on_hover_text(tip)
        .clicked()
    {
        actions.push(action);
    }
}

fn start_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 14.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, Color32::from_white_alpha(26));
}

fn notice_chip(ui: &mut egui::Ui, message: &str, actions: &mut Vec<ReplayBarAction>) {
    egui::Frame::new()
        .fill(theme::REPLAY.gamma_multiply(0.10))
        .corner_radius(7.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").color(theme::REPLAY));
                ui.label(RichText::new(message).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if glyph_button(ui, Glyph::Close, true, "Dismiss").clicked() {
                        actions.push(ReplayBarAction::DismissNotice);
                    }
                });
            });
        });
}

/// Small text badge used on a replaying chart's plot.
pub fn badge(painter: &egui::Painter, top_right: Pos2) {
    let text = "Replay";
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let rect = Rect::from_min_size(Pos2::new(top_right.x - size.x, top_right.y), size);
    painter.rect_filled(rect, 999.0, theme::REPLAY);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> ReplayEngine {
        let mut engine = ReplayEngine::default();
        engine.start(1_100, "BTCUSDT", (0..5).map(|i| 1_000 + i * 100).collect());
        engine
    }

    #[test]
    fn progress_and_start_tick_follow_the_timeline() {
        let mut engine = engine();
        assert_eq!(start_fraction(&engine), 0.25);
        assert_eq!(progress(&engine), 0.25);
        engine.seek(1.0);
        assert_eq!(progress(&engine), 1.0);
        assert_eq!(start_fraction(&engine), 0.25);
    }

    #[test]
    fn single_bar_and_no_session_do_not_divide_by_zero() {
        assert_eq!(progress(&ReplayEngine::default()), 0.0);
        let mut one = ReplayEngine::default();
        one.start(5, "X", vec![5]);
        assert_eq!((progress(&one), start_fraction(&one)), (0.0, 0.0));
    }

    #[test]
    fn step_gates() {
        let mut engine = engine();
        assert!(can_step_back(&engine) && can_advance(&engine));
        assert_eq!(step_of(&engine), (2, 5));
        engine.seek(0.0);
        assert!(!can_step_back(&engine));
        engine.seek(1.0);
        assert!(!can_advance(&engine));
        assert!(!can_step_back(&ReplayEngine::default()));
    }

    #[test]
    fn titles_and_tints() {
        assert_eq!(title(ReplayStatus::Playing, false), "Playing");
        assert_eq!(title(ReplayStatus::Playing, true), "Loading history");
        assert_eq!(tint(ReplayStatus::Playing, false), theme::UP);
        assert_eq!(tint(ReplayStatus::Paused, false), theme::REPLAY);
        assert_eq!(tint(ReplayStatus::Playing, true), Color32::from_gray(150));
    }

    #[test]
    fn utc_date_and_time_text() {
        // 2023-11-14 22:13:20 UTC
        let ms = 1_700_000_000_000;
        assert_eq!(date_text(ms), "Nov 14, 2023");
        assert_eq!(time_text(ms), "22:13:20");
        assert_eq!(compact_text(ms), "Nov 14, 22:13");
        assert_eq!(date_text(0), "Jan 1, 1970");
    }

    #[test]
    fn interval_text_shows_the_resolved_auto() {
        let engine = engine();
        assert_eq!(interval_text(&engine, None), "Auto");
        assert_eq!(
            interval_text(&engine, Some(ReplayInterval::FiveMinutes)),
            "Auto · 5m"
        );
    }

    #[test]
    fn completed_toggle_restarts_first() {
        let mut engine = engine();
        engine.seek(1.0);
        let mut actions = Vec::new();
        toggle_actions(&engine, &mut actions);
        assert_eq!(
            actions,
            vec![ReplayBarAction::Restart, ReplayBarAction::TogglePlayback]
        );
    }

    #[test]
    fn wrapped_bar_reserves_a_second_row() {
        assert_eq!(preferred_height(ReplayStatus::Ready, 500.0, false), 50.0);
        assert_eq!(preferred_height(ReplayStatus::Paused, 500.0, false), 82.0);
        assert_eq!(preferred_height(ReplayStatus::Paused, 1000.0, true), 88.0);
    }
}
