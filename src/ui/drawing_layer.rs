//! The drawing layer of a chart card: projects anchors onto the plot, hit-tests,
//! runs the draw/edit gestures and paints. It never mutates the drawing model; it
//! reports changes as `DrawingCommand`s in a `DrawingFrame` that the workspace
//! applies through its undo stack.

use super::chart_card::PriceScale;
use super::drawing_render::{
    self, FIB_COLORS, FIB_RATIOS, FibLevelLine, HANDLE_RADIUS, HIT_TOLERANCE, LineStyle,
    RulerCorner, RulerPart, RulerReadout, TimeIndex,
};
use crate::model::{Anchor, Drawing, DrawingCommand, Tool};
use eframe::egui::{self, Color32, CursorIcon, Painter, Pos2, Rect, Response};

const BRUSH_COLOR: Color32 = Color32::from_rgb(255, 159, 10);
const BRUSH_WIDTH: f32 = 2.0;
/// Brush samples closer than this (px) to the previous one are dropped.
const BRUSH_MIN_STEP: f32 = 1.5;
/// A drag shorter than this (px) starts a click-move-click draft instead.
const MIN_DRAG: f32 = 4.0;

/// Per-frame exchange between the workspace and one chart card's drawing layer.
/// The workspace fills the inputs and applies the outputs after the card draws.
pub struct DrawingFrame<'a> {
    // Inputs.
    pub drawings: &'a [Drawing],
    pub tool: Tool,
    /// The selected drawing, only when it belongs to this chart.
    pub selected: Option<u64>,
    /// First unused id; the layer increments it for every drawing it creates.
    pub next_id: u64,
    // Outputs.
    pub commands: Vec<DrawingCommand>,
    /// `Some(x)` means set the selection to `x` (`Some(None)` clears it).
    pub select: Option<Option<u64>>,
    /// An Escape press was spent cancelling a draft or edit on this card.
    pub escape_consumed: bool,
}

impl<'a> DrawingFrame<'a> {
    pub fn new(drawings: &'a [Drawing], tool: Tool, selected: Option<u64>, next_id: u64) -> Self {
        Self {
            drawings,
            tool,
            selected,
            next_id,
            commands: Vec::new(),
            select: None,
            escape_consumed: false,
        }
    }
}

/// Anchor <-> screen mapping for the current frame.
pub(super) struct Projection<'a> {
    pub plot: Rect,
    pub scale: &'a PriceScale,
    pub times: TimeIndex<'a>,
    /// Global index of the first shown candle.
    pub start: usize,
    pub slot_width: f32,
    pub blank_slots: usize,
}

impl Projection<'_> {
    fn x(&self, time_ms: i64) -> f32 {
        let index = self.times.index_of(time_ms) - self.start as f64;
        self.plot.left()
            + self.slot_width * (index as f32 + 0.5)
            + self.slot_width * self.blank_slots as f32
    }

    fn time(&self, x: f32) -> i64 {
        let slots = (x - self.plot.left()) / self.slot_width - 0.5 - self.blank_slots as f32;
        self.times.time_at(f64::from(slots) + self.start as f64)
    }

    fn point(&self, anchor: Anchor) -> Pos2 {
        Pos2::new(self.x(anchor.time_ms), self.scale.y(anchor.price))
    }

    fn anchor(&self, point: Pos2) -> Anchor {
        Anchor {
            time_ms: self.time(point.x),
            price: self.scale.price(point.y),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Body,
    End(End),
    Corner(RulerCorner),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
    id: u64,
    part: Part,
    locked: bool,
}

struct Draft {
    tool: Tool,
    a: Anchor,
    b: Anchor,
    points: Vec<Anchor>,
    /// True while the button is held; false once it waits for the second click.
    dragging: bool,
    pending_click: bool,
}

struct Edit {
    original: Drawing,
    current: Drawing,
    part: Part,
    press: Pos2,
}

enum Gesture {
    Draft(Draft),
    Edit(Edit),
}

#[derive(Default)]
pub(super) struct DrawingLayer {
    gesture: Option<Gesture>,
    hover: Option<Hit>,
    /// Escape cancelled a gesture mid-drag; keep the plot from panning until release.
    cancelled_drag: bool,
}

fn is_locked(drawing: &Drawing) -> bool {
    match drawing {
        Drawing::TrendLine { locked, .. }
        | Drawing::Fib { locked, .. }
        | Drawing::Brush { locked, .. } => *locked,
        Drawing::Ruler { .. } => false,
    }
}

fn fib_levels(a: Anchor, b: Anchor, proj: &Projection) -> Vec<FibLevelLine> {
    FIB_RATIOS
        .iter()
        .zip(FIB_COLORS)
        .filter_map(|(ratio, color)| {
            drawing_render::fib_price(a.price, b.price, *ratio, false, proj.scale.logarithmic).map(
                |price| FibLevelLine {
                    ratio: *ratio,
                    y: proj.scale.y(price),
                    price,
                    color,
                },
            )
        })
        .collect()
}

fn ruler_rect(a: Anchor, b: Anchor, proj: &Projection) -> Rect {
    Rect::from_two_pos(proj.point(a), proj.point(b))
}

fn end_hit(pointer: Pos2, a: Pos2, b: Pos2) -> Option<Part> {
    let reach = HANDLE_RADIUS + 3.0;
    if a.distance(pointer) <= reach {
        Some(Part::End(End::A))
    } else if b.distance(pointer) <= reach {
        Some(Part::End(End::B))
    } else {
        None
    }
}

fn hit_drawing(drawing: &Drawing, pointer: Pos2, proj: &Projection) -> Option<Part> {
    match drawing {
        Drawing::TrendLine { a, b, .. } => {
            let (pa, pb) = (proj.point(*a), proj.point(*b));
            end_hit(pointer, pa, pb).or_else(|| {
                (drawing_render::distance_to_segment(pointer, pa, pb) <= HIT_TOLERANCE)
                    .then_some(Part::Body)
            })
        }
        Drawing::Fib { a, b, .. } => {
            let (pa, pb) = (proj.point(*a), proj.point(*b));
            end_hit(pointer, pa, pb).or_else(|| {
                drawing_render::hit_fibonacci(pa, pb, &fib_levels(*a, *b, proj), pointer)
                    .then_some(Part::Body)
            })
        }
        Drawing::Brush { points, width, .. } => {
            let screen: Vec<Pos2> = points.iter().map(|point| proj.point(*point)).collect();
            drawing_render::hit_polyline(&screen, *width, pointer).then_some(Part::Body)
        }
        Drawing::Ruler { a, b, .. } => drawing_render::hit_ruler(ruler_rect(*a, *b, proj), pointer)
            .map(|part| match part {
                RulerPart::Corner(corner) => Part::Corner(corner),
                RulerPart::Edge => Part::Body,
            }),
    }
}

fn hit_test(drawings: &[Drawing], pointer: Pos2, proj: &Projection) -> Option<Hit> {
    // Topmost (latest) drawing first.
    drawings.iter().rev().find_map(|drawing| {
        hit_drawing(drawing, pointer, proj).map(|part| Hit {
            id: drawing.id(),
            part,
            locked: is_locked(drawing),
        })
    })
}

/// `original` after dragging `part` from `press` to `now`.
fn edited(original: &Drawing, part: Part, press: Pos2, now: Pos2, proj: &Projection) -> Drawing {
    let delta = now - press;
    let shift = |anchor: Anchor| proj.anchor(proj.point(anchor) + delta);
    let mut result = original.clone();
    match (&mut result, part) {
        (
            Drawing::TrendLine { a, b, .. }
            | Drawing::Fib { a, b, .. }
            | Drawing::Ruler { a, b, .. },
            Part::Body,
        ) => {
            *a = shift(*a);
            *b = shift(*b);
        }
        (Drawing::TrendLine { a, .. } | Drawing::Fib { a, .. }, Part::End(End::A)) => {
            *a = proj.anchor(now);
        }
        (Drawing::TrendLine { b, .. } | Drawing::Fib { b, .. }, Part::End(End::B)) => {
            *b = proj.anchor(now);
        }
        (Drawing::Brush { points, .. }, Part::Body) => {
            for point in points.iter_mut() {
                *point = shift(*point);
            }
        }
        (Drawing::Ruler { a, b, .. }, Part::Corner(corner)) => {
            (*a, *b) = drawing_render::resize_ruler(*a, *b, corner, proj.anchor(now));
        }
        _ => {}
    }
    result
}

fn commit_draft(draft: &Draft, frame: &mut DrawingFrame) {
    let id = frame.next_id;
    let (a, b) = (draft.a, draft.b);
    let drawing = match draft.tool {
        Tool::TrendLine => Drawing::TrendLine {
            id,
            a,
            b,
            locked: false,
        },
        Tool::Fib => Drawing::Fib {
            id,
            a,
            b,
            locked: false,
        },
        Tool::Ruler => Drawing::Ruler { id, a, b },
        Tool::Brush => Drawing::Brush {
            id,
            points: draft.points.clone(),
            width: BRUSH_WIDTH,
            locked: false,
        },
        Tool::Cursor => return,
    };
    frame.next_id += 1;
    frame.commands.push(DrawingCommand::Add(drawing));
    frame.select = Some(Some(id));
}

impl DrawingLayer {
    /// Whether a gesture owns the plot drag, so the card must not pan.
    pub(super) fn claims_drag(&self) -> bool {
        self.gesture.is_some() || self.cancelled_drag
    }

    /// Runs one frame of drawing input. Returns whether the plot drag is claimed.
    pub(super) fn update(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        proj: &Projection,
        frame: &mut DrawingFrame,
    ) -> bool {
        let plot = proj.plot;
        let pointer = ui.input(|input| input.pointer.latest_pos());
        let press = ui.input(|input| input.pointer.press_origin());
        // Switching tool abandons a draft made with the old one.
        if matches!(&self.gesture, Some(Gesture::Draft(draft)) if draft.tool != frame.tool) {
            self.gesture = None;
        }
        self.handle_keys(ui, response, frame);

        self.hover = None;
        if self.gesture.is_none()
            && response.hovered()
            && let Some(point) = pointer.filter(|point| plot.contains(*point))
        {
            self.hover = hit_test(frame.drawings, point, proj);
        }

        if response.drag_started()
            && self.gesture.is_none()
            && let Some(point) = press.filter(|point| plot.contains(*point))
        {
            self.begin_drag(point, frame, proj);
        }
        if response.dragged()
            && let Some(point) = pointer
        {
            self.continue_drag(point, proj);
        }
        if response.drag_stopped() {
            self.cancelled_drag = false;
            self.finish_drag(frame, proj);
        }
        if response.clicked()
            && let Some(point) = response
                .interact_pointer_pos()
                .or(pointer)
                .filter(|point| plot.contains(*point))
        {
            self.click(point, frame, proj);
        }

        // A draft waiting for its second click follows the pointer.
        if let Some(Gesture::Draft(draft)) = &mut self.gesture
            && draft.pending_click
            && let Some(point) = pointer.filter(|point| plot.contains(*point))
        {
            draft.b = proj.anchor(point);
        }

        self.set_cursor(ui, pointer, frame, plot);
        self.claims_drag()
    }

    fn handle_keys(&mut self, ui: &egui::Ui, response: &Response, frame: &mut DrawingFrame) {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) && self.gesture.take().is_some() {
            frame.escape_consumed = true;
            self.cancelled_drag = response.dragged();
        }
        if let Some(id) = frame.selected
            && self.gesture.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
            })
        {
            frame.commands.push(DrawingCommand::Remove(id));
            frame.select = Some(None);
        }
    }

    fn begin_drag(&mut self, press: Pos2, frame: &mut DrawingFrame, proj: &Projection) {
        match frame.tool {
            Tool::Cursor => {
                let Some(hit) = hit_test(frame.drawings, press, proj) else {
                    return;
                };
                frame.select = Some(Some(hit.id));
                if hit.locked {
                    return;
                }
                if let Some(original) = frame.drawings.iter().find(|d| d.id() == hit.id) {
                    self.gesture = Some(Gesture::Edit(Edit {
                        original: original.clone(),
                        current: original.clone(),
                        part: hit.part,
                        press,
                    }));
                }
            }
            tool => {
                let anchor = proj.anchor(press);
                self.gesture = Some(Gesture::Draft(Draft {
                    tool,
                    a: anchor,
                    b: anchor,
                    points: vec![anchor],
                    dragging: true,
                    pending_click: false,
                }));
            }
        }
    }

    fn continue_drag(&mut self, pointer: Pos2, proj: &Projection) {
        match &mut self.gesture {
            Some(Gesture::Edit(edit)) => {
                edit.current = edited(&edit.original, edit.part, edit.press, pointer, proj);
            }
            Some(Gesture::Draft(draft)) if draft.dragging => {
                let anchor = proj.anchor(pointer);
                draft.b = anchor;
                if draft.tool == Tool::Brush {
                    let far_enough = draft
                        .points
                        .last()
                        .is_none_or(|last| proj.point(*last).distance(pointer) >= BRUSH_MIN_STEP);
                    if far_enough {
                        draft.points.push(anchor);
                    }
                }
            }
            _ => {}
        }
    }

    fn finish_drag(&mut self, frame: &mut DrawingFrame, proj: &Projection) {
        match self.gesture.take() {
            Some(Gesture::Edit(edit)) => {
                if edit.current != edit.original {
                    frame.commands.push(DrawingCommand::Replace {
                        id: edit.original.id(),
                        drawing: edit.current,
                    });
                }
            }
            Some(Gesture::Draft(mut draft)) if draft.dragging => {
                draft.dragging = false;
                let dragged_far = proj.point(draft.a).distance(proj.point(draft.b)) >= MIN_DRAG;
                if draft.tool == Tool::Brush || dragged_far {
                    commit_draft(&draft, frame);
                } else {
                    // Barely moved: treat it as the first click of click-move-click.
                    draft.pending_click = true;
                    self.gesture = Some(Gesture::Draft(draft));
                }
            }
            other => self.gesture = other,
        }
    }

    fn click(&mut self, point: Pos2, frame: &mut DrawingFrame, proj: &Projection) {
        // Second click of click-move-click.
        if let Some(Gesture::Draft(draft)) = &mut self.gesture
            && draft.pending_click
        {
            draft.b = proj.anchor(point);
            if let Some(Gesture::Draft(draft)) = self.gesture.take() {
                commit_draft(&draft, frame);
            }
            return;
        }
        match frame.tool {
            Tool::Cursor => {
                frame.select = Some(hit_test(frame.drawings, point, proj).map(|hit| hit.id));
            }
            Tool::Brush => {
                let anchor = proj.anchor(point);
                commit_draft(
                    &Draft {
                        tool: Tool::Brush,
                        a: anchor,
                        b: anchor,
                        points: vec![anchor],
                        dragging: false,
                        pending_click: false,
                    },
                    frame,
                );
            }
            tool => {
                let anchor = proj.anchor(point);
                self.gesture = Some(Gesture::Draft(Draft {
                    tool,
                    a: anchor,
                    b: anchor,
                    points: vec![anchor],
                    dragging: false,
                    pending_click: true,
                }));
            }
        }
    }

    fn set_cursor(&self, ui: &egui::Ui, pointer: Option<Pos2>, frame: &DrawingFrame, plot: Rect) {
        let over_plot = pointer.is_some_and(|point| plot.contains(point));
        let icon = match &self.gesture {
            Some(Gesture::Edit(_)) => Some(CursorIcon::Grabbing),
            Some(Gesture::Draft(_)) => Some(CursorIcon::Crosshair),
            None if !over_plot => None,
            None if frame.tool != Tool::Cursor => Some(CursorIcon::Crosshair),
            None => self.hover.map(|hit| match hit.part {
                Part::Body => CursorIcon::Move,
                Part::End(_) => CursorIcon::Grab,
                Part::Corner(RulerCorner::TopLeft | RulerCorner::BottomRight) => {
                    CursorIcon::ResizeNwSe
                }
                Part::Corner(_) => CursorIcon::ResizeNeSw,
            }),
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    /// Paints every drawing (the one being edited at its live position) and any draft.
    pub(super) fn paint(&self, painter: &Painter, proj: &Projection, frame: &DrawingFrame) {
        let clipped = painter.with_clip_rect(proj.plot);
        let editing = match &self.gesture {
            Some(Gesture::Edit(edit)) => Some(edit),
            _ => None,
        };
        let hovered = self.hover.map(|hit| hit.id);
        for drawing in frame.drawings {
            let drawing = match editing {
                Some(edit) if edit.original.id() == drawing.id() => &edit.current,
                _ => drawing,
            };
            let id = drawing.id();
            let emphasized = frame.selected == Some(id) || hovered == Some(id);
            self.paint_drawing(&clipped, proj, drawing, emphasized, false);
        }
        if let Some(Gesture::Draft(draft)) = &self.gesture {
            let preview = match draft.tool {
                Tool::TrendLine => Some(Drawing::TrendLine {
                    id: 0,
                    a: draft.a,
                    b: draft.b,
                    locked: false,
                }),
                Tool::Fib => Some(Drawing::Fib {
                    id: 0,
                    a: draft.a,
                    b: draft.b,
                    locked: false,
                }),
                Tool::Ruler => Some(Drawing::Ruler {
                    id: 0,
                    a: draft.a,
                    b: draft.b,
                }),
                Tool::Brush => Some(Drawing::Brush {
                    id: 0,
                    points: draft.points.clone(),
                    width: BRUSH_WIDTH,
                    locked: false,
                }),
                Tool::Cursor => None,
            };
            if let Some(preview) = preview {
                self.paint_drawing(&clipped, proj, &preview, true, true);
            }
        }
    }

    fn paint_drawing(
        &self,
        painter: &Painter,
        proj: &Projection,
        drawing: &Drawing,
        emphasized: bool,
        draft: bool,
    ) {
        match drawing {
            Drawing::TrendLine { a, b, .. } => {
                drawing_render::draw_trend_line(
                    painter,
                    proj.point(*a),
                    proj.point(*b),
                    &LineStyle::default(),
                    emphasized,
                );
            }
            Drawing::Fib { a, b, .. } => {
                drawing_render::draw_fibonacci(
                    painter,
                    proj.plot,
                    proj.point(*a),
                    proj.point(*b),
                    &fib_levels(*a, *b, proj),
                    emphasized,
                );
            }
            Drawing::Brush { points, width, .. } => {
                let screen: Vec<Pos2> = points.iter().map(|point| proj.point(*point)).collect();
                drawing_render::draw_brush(
                    painter,
                    &screen,
                    BRUSH_COLOR,
                    *width,
                    emphasized && !draft,
                );
            }
            Drawing::Ruler { a, b, .. } => {
                let readout = RulerReadout::new(
                    a.price,
                    b.price,
                    proj.times.bars_between(a.time_ms, b.time_ms),
                    (b.time_ms - a.time_ms) / 1000,
                );
                drawing_render::draw_ruler(
                    painter,
                    proj.plot,
                    ruler_rect(*a, *b, proj),
                    &readout,
                    emphasized && !draft,
                    draft,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;

    fn candles() -> Vec<Kline> {
        (0..10)
            .map(|index| Kline {
                open_time: index * 60_000,
                close_time: index * 60_000 + 59_999,
                open: 100.0,
                high: 110.0,
                low: 90.0,
                close: 105.0,
                volume: 1.0,
            })
            .collect()
    }

    fn scale(logarithmic: bool) -> PriceScale {
        PriceScale {
            minimum: 80.0,
            maximum: 120.0,
            top: 0.0,
            height: 400.0,
            logarithmic,
        }
    }

    fn projection<'a>(data: &'a [Kline], scale: &'a PriceScale) -> Projection<'a> {
        Projection {
            plot: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1000.0, 400.0)),
            scale,
            times: TimeIndex::new(data),
            start: 0,
            slot_width: 100.0,
            blank_slots: 0,
        }
    }

    #[test]
    fn anchors_round_trip_through_the_screen() {
        let data = candles();
        for logarithmic in [false, true] {
            let scale = scale(logarithmic);
            let proj = projection(&data, &scale);
            let anchor = Anchor {
                time_ms: 150_000,
                price: 97.5,
            };
            let back = proj.anchor(proj.point(anchor));
            assert!((back.time_ms - anchor.time_ms).abs() <= 1, "{back:?}");
            // Screen coordinates are f32, so the price comes back to pixel precision.
            assert!((back.price - anchor.price).abs() < 1e-3, "{back:?}");
        }
    }

    #[test]
    fn candle_centres_land_mid_slot_and_blank_slots_shift_right() {
        let data = candles();
        let scale = scale(false);
        let mut proj = projection(&data, &scale);
        assert_eq!(proj.x(0), 50.0);
        assert_eq!(proj.x(60_000), 150.0);
        proj.blank_slots = 2;
        assert_eq!(proj.x(0), 250.0);
        proj.blank_slots = 0;
        proj.start = 3;
        assert_eq!(proj.x(3 * 60_000), 50.0);
    }

    #[test]
    fn dragging_the_body_moves_both_anchors_and_keeps_the_id() {
        let data = candles();
        let scale = scale(false);
        let proj = projection(&data, &scale);
        let original = Drawing::TrendLine {
            id: 7,
            a: Anchor {
                time_ms: 60_000,
                price: 90.0,
            },
            b: Anchor {
                time_ms: 180_000,
                price: 110.0,
            },
            locked: false,
        };
        let moved = edited(
            &original,
            Part::Body,
            Pos2::new(300.0, 200.0),
            Pos2::new(400.0, 200.0),
            &proj,
        );
        let Drawing::TrendLine { id, a, b, .. } = moved else {
            panic!("variant changed");
        };
        assert_eq!(id, 7);
        assert_eq!(a.time_ms, 120_000);
        assert_eq!(b.time_ms, 240_000);
        assert!((a.price - 90.0).abs() < 1e-3 && (b.price - 110.0).abs() < 1e-3);
    }

    #[test]
    fn dragging_an_end_moves_only_that_end() {
        let data = candles();
        let scale = scale(false);
        let proj = projection(&data, &scale);
        let (a, b) = (
            Anchor {
                time_ms: 60_000,
                price: 90.0,
            },
            Anchor {
                time_ms: 180_000,
                price: 110.0,
            },
        );
        let original = Drawing::Fib {
            id: 1,
            a,
            b,
            locked: false,
        };
        let moved = edited(
            &original,
            Part::End(End::B),
            Pos2::new(0.0, 0.0),
            Pos2::new(550.0, 200.0),
            &proj,
        );
        let Drawing::Fib { a: a2, b: b2, .. } = moved else {
            panic!("variant changed");
        };
        assert_eq!(a2, a);
        assert_eq!(b2.time_ms, 300_000);
        assert!((b2.price - 100.0).abs() < 1e-3);
    }

    #[test]
    fn locked_drawings_report_locked_hits() {
        let data = candles();
        let scale = scale(false);
        let proj = projection(&data, &scale);
        let line = Drawing::TrendLine {
            id: 3,
            a: Anchor {
                time_ms: 0,
                price: 100.0,
            },
            b: Anchor {
                time_ms: 540_000,
                price: 100.0,
            },
            locked: true,
        };
        let y = scale.y(100.0);
        let hit = hit_test(&[line], Pos2::new(500.0, y + 1.0), &proj).expect("hit");
        assert_eq!((hit.id, hit.part, hit.locked), (3, Part::Body, true));
        assert!(hit_test(&[], Pos2::new(500.0, y), &proj).is_none());
    }

    #[test]
    fn commit_assigns_sequential_ids_and_selects_the_new_drawing() {
        let anchor = Anchor {
            time_ms: 0,
            price: 1.0,
        };
        let mut frame = DrawingFrame::new(&[], Tool::Cursor, None, 5);
        for tool in [Tool::TrendLine, Tool::Brush] {
            commit_draft(
                &Draft {
                    tool,
                    a: anchor,
                    b: anchor,
                    points: vec![anchor],
                    dragging: false,
                    pending_click: false,
                },
                &mut frame,
            );
        }
        assert_eq!(frame.next_id, 7);
        assert_eq!(frame.commands.len(), 2);
        assert_eq!(frame.select, Some(Some(6)));
        // The cursor draws nothing.
        let before = frame.commands.len();
        commit_draft(
            &Draft {
                tool: Tool::Cursor,
                a: anchor,
                b: anchor,
                points: vec![],
                dragging: false,
                pending_click: false,
            },
            &mut frame,
        );
        assert_eq!((frame.commands.len(), frame.next_id), (before, 7));
    }
}
