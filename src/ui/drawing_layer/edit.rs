//! Editing a placed drawing: which part the pointer grabbed, what dragging it does, and the
//! structural edits (inserting and removing a vertex, duplicating). All pure functions of a
//! drawing and a projection, so every tool is tested without a window.

use super::Projection;
use crate::model::{Anchor, Drawing, Geometry};
use crate::ui::drawing_render::{self, RulerCorner};
use eframe::egui::{Pos2, Vec2};

/// How far a duplicate is offset from the original, in px (right and down).
pub(super) const DUPLICATE_OFFSET: Vec2 = Vec2::new(16.0, 16.0);

/// One of the eight handles of a box-shaped drawing (a rectangle or an ellipse).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoxHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl BoxHandle {
    pub(super) const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Right,
        Self::BottomRight,
        Self::Bottom,
        Self::BottomLeft,
        Self::Left,
    ];

    /// `Some(true)` for a handle on the left edge, `Some(false)` on the right, `None` when it
    /// does not move the horizontal extent.
    fn horizontal(self) -> Option<bool> {
        match self {
            Self::TopLeft | Self::BottomLeft | Self::Left => Some(true),
            Self::TopRight | Self::BottomRight | Self::Right => Some(false),
            Self::Top | Self::Bottom => None,
        }
    }

    /// `Some(true)` for a handle on the top edge, `Some(false)` on the bottom.
    fn vertical(self) -> Option<bool> {
        match self {
            Self::TopLeft | Self::TopRight | Self::Top => Some(true),
            Self::BottomLeft | Self::BottomRight | Self::Bottom => Some(false),
            Self::Left | Self::Right => None,
        }
    }

    /// Where the handle sits on the box.
    pub(super) fn position(self, rect: eframe::egui::Rect) -> Pos2 {
        let x = match self.horizontal() {
            Some(true) => rect.left(),
            Some(false) => rect.right(),
            None => rect.center().x,
        };
        let y = match self.vertical() {
            Some(true) => rect.top(),
            Some(false) => rect.bottom(),
            None => rect.center().y,
        };
        Pos2::new(x, y)
    }

    pub(super) fn is_corner(self) -> bool {
        self.horizontal().is_some() && self.vertical().is_some()
    }
}

/// The part of a drawing the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Part {
    /// The shape itself: dragging moves all of it.
    Body,
    /// The anchor at this index of `Drawing::anchors`.
    Anchor(usize),
    Box(BoxHandle),
    /// A ruler's corner.
    Corner(RulerCorner),
}

/// A box's two corner anchors with `handle` dragged to `to`, the opposite edges left in place.
/// Each axis is written to whichever stored anchor owns that edge, so the stored order survives
/// and dragging past the opposite edge flips the box without a special case.
pub(super) fn resize_box(a: Anchor, b: Anchor, handle: BoxHandle, to: Anchor) -> (Anchor, Anchor) {
    let (mut a, mut b) = (a, b);
    if let Some(left) = handle.horizontal() {
        if left == (a.time_ms <= b.time_ms) {
            a.time_ms = to.time_ms;
        } else {
            b.time_ms = to.time_ms;
        }
    }
    if let Some(top) = handle.vertical() {
        if top == (a.price >= b.price) {
            a.price = to.price;
        } else {
            b.price = to.price;
        }
    }
    (a, b)
}

/// `original` after dragging `part` from `press` to `now`. `target` is where an anchor handle
/// lands (the pointer, or the magnet's pick); a body drag moves every anchor by the pointer's
/// pixel delta, which keeps it right on a log scale too.
pub(super) fn edited(
    original: &Drawing,
    part: Part,
    press: Pos2,
    now: Pos2,
    target: Anchor,
    proj: &Projection,
) -> Drawing {
    let delta = now - press;
    let mut result = original.clone();
    match part {
        Part::Body => {
            for anchor in result.anchors_mut() {
                *anchor = proj.anchor(proj.point(*anchor) + delta);
            }
        }
        Part::Anchor(index) => {
            if let Some(anchor) = result.anchors_mut().into_iter().nth(index) {
                *anchor = target;
            }
        }
        Part::Box(handle) => match &mut result {
            Drawing::Shape {
                geometry: Geometry::Rectangle { a, b } | Geometry::Ellipse { a, b },
                ..
            } => (*a, *b) = resize_box(*a, *b, handle, target),
            _ => {}
        },
        Part::Corner(corner) => {
            if let Drawing::Ruler { a, b, .. } = &mut result {
                (*a, *b) = drawing_render::resize_ruler(*a, *b, corner, target);
            }
        }
    }
    result
}

/// The vertices of an editable polyline-like shape.
fn vertices_mut(drawing: &mut Drawing) -> Option<(&mut Vec<Anchor>, usize)> {
    match drawing {
        Drawing::Shape {
            geometry: Geometry::Path { points },
            ..
        } => Some((points, 2)),
        Drawing::Shape {
            geometry: Geometry::Polyline { points, closed },
            ..
        } => {
            let minimum = if *closed { 3 } else { 2 };
            Some((points, minimum))
        }
        _ => None,
    }
}

/// `drawing` with a vertex added after vertex `segment` (the segment's start), at `at`.
/// `None` when the drawing has no vertices to add to, or is at its size limit.
pub(super) fn with_inserted_vertex(
    drawing: &Drawing,
    segment: usize,
    at: Anchor,
) -> Option<Drawing> {
    let mut result = drawing.clone();
    let (points, _) = vertices_mut(&mut result)?;
    if points.len() >= crate::model::MAX_POINTS || segment >= points.len() {
        return None;
    }
    points.insert(segment + 1, at);
    Some(result)
}

/// `drawing` without vertex `index`; `None` when that would leave too few for the shape.
pub(super) fn without_vertex(drawing: &Drawing, index: usize) -> Option<Drawing> {
    let mut result = drawing.clone();
    let (points, minimum) = vertices_mut(&mut result)?;
    if points.len() <= minimum || index >= points.len() {
        return None;
    }
    points.remove(index);
    Some(result)
}

/// A copy of `drawing` under `id`, moved by [`DUPLICATE_OFFSET`] on screen so it does not sit
/// exactly on the original, and unlocked.
pub(super) fn duplicate(drawing: &Drawing, id: u64, proj: &Projection) -> Drawing {
    let mut copy = drawing.clone().with_id(id);
    for anchor in copy.anchors_mut() {
        *anchor = proj.anchor(proj.point(*anchor) + DUPLICATE_OFFSET);
    }
    copy.set_locked(false);
    copy
}
