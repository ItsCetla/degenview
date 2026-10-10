//! Painter-drawn glyphs standing in for the SF Symbols the Swift app uses. Every icon is drawn
//! on a 16×16 grid, scaled to the target rect and tinted with one colour, so they stay crisp at
//! any DPI and need no font or image loader.

use eframe::egui::{
    Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, epaint::PathShape, vec2,
};

/// The glyph set the Script Manager uses; a few (eye, braces, bell) wait for its phase-2 panels.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    SidebarLeft,
    ChartLine,
    ChartLineFlat,
    ChartUptrend,
    RectLeft,
    RectTop,
    RectBottom,
    Star,
    StarFill,
    Plus,
    Magnifier,
    ChevronDown,
    ChevronUp,
    ChevronRight,
    ChevronUpDown,
    Eye,
    EyeSlash,
    Curlybraces,
    Check,
    CheckCircleFill,
    XmarkOctagonFill,
    Triangle,
    TriangleFill,
    XmarkCircleFill,
    Xmark,
    Sliders,
    CircleDashed,
    Key,
    Books,
    ArrowUpRightSquare,
    CopyDoc,
    Bell,
    BellBadge,
    BellFill,
    Clock,
    Download,
    Switch,
    Calendar,
    Hash,
    TextFormat,
    List,
    Palette,
    BarChart,
    Waveform,
    Cycle,
    Bands,
}

/// Stroke width on the 16 grid.
const W: f32 = 1.35;

/// Draws `icon` centred in `rect`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let scale = side / 16.0;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let p = |x: f32, y: f32| origin + vec2(x, y) * scale;
    let stroke = Stroke::new((W * scale).max(1.0), color);
    let line = |points: &[Pos2]| {
        painter.add(Shape::Path(PathShape::line(points.to_vec(), stroke)));
    };
    let closed = |points: &[Pos2]| {
        painter.add(Shape::Path(PathShape::closed_line(points.to_vec(), stroke)));
    };
    let fill_poly = |points: Vec<Pos2>| {
        // Fan from the centroid so concave star-shaped polygons fill without artifacts.
        let centre = points.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / points.len() as f32;
        let centre = centre.to_pos2();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(centre, color);
        for point in &points {
            mesh.colored_vertex(*point, color);
        }
        let n = points.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        painter.add(Shape::mesh(mesh));
    };
    let round = |r: f32| CornerRadius::same((r * scale).round().clamp(0.0, 255.0) as u8);
    let rect_of = |a: Pos2, b: Pos2| Rect::from_two_pos(a, b);
    let circle = |c: Pos2, r: f32, fill: bool| {
        if fill {
            painter.circle_filled(c, r * scale, color);
        } else {
            painter.circle_stroke(c, r * scale, stroke);
        }
    };
    let star_points = |outer: f32, inner: f32| -> Vec<Pos2> {
        (0..10)
            .map(|i| {
                let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                let r = if i % 2 == 0 { outer } else { inner };
                p(8.0 + r * angle.cos(), 8.6 + r * angle.sin())
            })
            .collect()
    };

    match icon {
        Icon::SidebarLeft => {
            painter.rect_stroke(
                rect_of(p(1.5, 2.5), p(14.5, 13.5)),
                round(2.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(6.0, 2.8), p(6.0, 13.2)]);
        }
        Icon::ChartLine | Icon::ChartLineFlat => {
            line(&[p(2.0, 2.0), p(2.0, 14.0), p(14.0, 14.0)]);
            if icon == Icon::ChartLine {
                line(&[p(4.5, 10.5), p(7.0, 7.0), p(9.5, 9.0), p(13.0, 4.0)]);
            } else {
                line(&[p(4.5, 9.0), p(13.0, 9.0)]);
            }
        }
        Icon::ChartUptrend => {
            line(&[p(2.0, 2.0), p(2.0, 14.0), p(14.0, 14.0)]);
            line(&[p(4.5, 11.0), p(8.0, 7.5), p(10.0, 9.5), p(13.5, 5.0)]);
            line(&[p(11.0, 4.8), p(13.5, 5.0), p(13.3, 7.5)]);
        }
        Icon::RectLeft | Icon::RectTop | Icon::RectBottom => {
            let outer = rect_of(p(1.5, 2.5), p(14.5, 13.5));
            let half = match icon {
                Icon::RectLeft => rect_of(p(1.5, 2.5), p(8.0, 13.5)),
                Icon::RectTop => rect_of(p(1.5, 2.5), p(14.5, 8.0)),
                _ => rect_of(p(1.5, 8.0), p(14.5, 13.5)),
            };
            painter.rect_filled(
                half.shrink(0.6 * scale),
                round(1.8),
                color.gamma_multiply(0.55),
            );
            painter.rect_stroke(outer, round(2.5), stroke, StrokeKind::Middle);
        }
        Icon::Star => closed(&star_points(6.6, 3.0)),
        Icon::StarFill => fill_poly(star_points(6.9, 3.2)),
        Icon::Plus => {
            line(&[p(8.0, 3.0), p(8.0, 13.0)]);
            line(&[p(3.0, 8.0), p(13.0, 8.0)]);
        }
        Icon::Magnifier => {
            circle(p(7.0, 7.0), 4.4, false);
            line(&[p(10.3, 10.3), p(13.6, 13.6)]);
        }
        Icon::ChevronDown => line(&[p(3.5, 6.0), p(8.0, 10.5), p(12.5, 6.0)]),
        Icon::ChevronUp => line(&[p(3.5, 10.0), p(8.0, 5.5), p(12.5, 10.0)]),
        Icon::ChevronRight => line(&[p(6.0, 3.5), p(10.5, 8.0), p(6.0, 12.5)]),
        Icon::ChevronUpDown => {
            line(&[p(4.5, 6.2), p(8.0, 2.8), p(11.5, 6.2)]);
            line(&[p(4.5, 9.8), p(8.0, 13.2), p(11.5, 9.8)]);
        }
        Icon::Eye | Icon::EyeSlash => {
            line(&[
                p(1.5, 8.0),
                p(4.0, 5.0),
                p(8.0, 3.8),
                p(12.0, 5.0),
                p(14.5, 8.0),
                p(12.0, 11.0),
                p(8.0, 12.2),
                p(4.0, 11.0),
                p(1.5, 8.0),
            ]);
            circle(p(8.0, 8.0), 2.0, false);
            if icon == Icon::EyeSlash {
                line(&[p(3.0, 13.5), p(13.0, 2.5)]);
            }
        }
        Icon::Curlybraces => {
            line(&[
                p(6.5, 2.5),
                p(5.2, 3.2),
                p(5.2, 6.6),
                p(3.5, 8.0),
                p(5.2, 9.4),
                p(5.2, 12.8),
                p(6.5, 13.5),
            ]);
            line(&[
                p(9.5, 2.5),
                p(10.8, 3.2),
                p(10.8, 6.6),
                p(12.5, 8.0),
                p(10.8, 9.4),
                p(10.8, 12.8),
                p(9.5, 13.5),
            ]);
        }
        Icon::Check => line(&[p(3.0, 8.5), p(6.5, 12.0), p(13.0, 4.5)]),
        Icon::CheckCircleFill => {
            circle(p(8.0, 8.0), 7.0, true);
            let knock = Stroke::new((1.6 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.add(Shape::Path(PathShape::line(
                vec![p(4.8, 8.2), p(7.1, 10.5), p(11.3, 5.8)],
                knock,
            )));
        }
        Icon::XmarkOctagonFill => {
            let pts: Vec<Pos2> = (0..8)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_8 + i as f32 * std::f32::consts::FRAC_PI_4;
                    p(8.0 + 7.4 * a.cos(), 8.0 + 7.4 * a.sin())
                })
                .collect();
            fill_poly(pts);
            let knock = Stroke::new((1.6 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(5.5, 5.5), p(10.5, 10.5)], knock);
            painter.line_segment([p(10.5, 5.5), p(5.5, 10.5)], knock);
        }
        Icon::Triangle => closed(&[p(8.0, 2.0), p(14.5, 13.5), p(1.5, 13.5)]),
        Icon::TriangleFill => {
            fill_poly(vec![p(8.0, 1.6), p(14.8, 13.8), p(1.2, 13.8)]);
            let knock = Stroke::new((1.5 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(8.0, 6.0), p(8.0, 9.6)], knock);
            painter.circle_filled(p(8.0, 11.6), 0.85 * scale, Color32::from_black_alpha(200));
        }
        Icon::XmarkCircleFill => {
            circle(p(8.0, 8.0), 7.0, true);
            let knock = Stroke::new((1.5 * scale).max(1.0), Color32::from_black_alpha(200));
            painter.line_segment([p(5.6, 5.6), p(10.4, 10.4)], knock);
            painter.line_segment([p(10.4, 5.6), p(5.6, 10.4)], knock);
        }
        Icon::Xmark => {
            line(&[p(3.5, 3.5), p(12.5, 12.5)]);
            line(&[p(12.5, 3.5), p(3.5, 12.5)]);
        }
        Icon::Sliders => {
            for (y, x) in [(3.5, 10.0), (8.0, 5.5), (12.5, 9.5)] {
                line(&[p(2.0, y), p(14.0, y)]);
                painter.circle_filled(p(x, y), 1.9 * scale, color);
            }
        }
        Icon::CircleDashed => {
            for i in 0..8 {
                let a0 = i as f32 * std::f32::consts::FRAC_PI_4;
                let a1 = a0 + std::f32::consts::FRAC_PI_4 * 0.55;
                let pts: Vec<Pos2> = (0..=4)
                    .map(|k| {
                        let a = a0 + (a1 - a0) * k as f32 / 4.0;
                        p(8.0 + 6.0 * a.cos(), 8.0 + 6.0 * a.sin())
                    })
                    .collect();
                line(&pts);
            }
        }
        Icon::Key => {
            circle(p(5.0, 10.5), 3.0, false);
            line(&[p(7.2, 8.3), p(13.5, 2.0)]);
            line(&[p(11.5, 4.0), p(13.5, 6.0)]);
        }
        Icon::Books => {
            for (x0, x1) in [(2.0, 5.0), (6.0, 9.0)] {
                painter.rect_stroke(
                    rect_of(p(x0, 2.5), p(x1, 13.5)),
                    round(0.8),
                    stroke,
                    StrokeKind::Middle,
                );
            }
            line(&[
                p(10.3, 3.5),
                p(13.2, 2.8),
                p(14.6, 12.3),
                p(11.5, 13.0),
                p(10.3, 3.5),
            ]);
        }
        Icon::ArrowUpRightSquare => {
            line(&[
                p(7.0, 2.5),
                p(3.0, 2.5),
                p(3.0, 13.0),
                p(13.5, 13.0),
                p(13.5, 9.0),
            ]);
            line(&[p(7.5, 8.5), p(14.0, 2.0)]);
            line(&[p(9.5, 2.0), p(14.0, 2.0), p(14.0, 6.5)]);
        }
        Icon::CopyDoc => {
            painter.rect_stroke(
                rect_of(p(5.5, 5.5), p(13.5, 14.0)),
                round(1.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[
                p(3.0, 10.5),
                p(2.5, 10.5),
                p(2.5, 2.5),
                p(10.0, 2.5),
                p(10.0, 3.5),
            ]);
        }
        Icon::Bell | Icon::BellBadge | Icon::BellFill => {
            let outline = [
                p(3.5, 11.5),
                p(4.5, 10.0),
                p(4.5, 7.0),
                p(5.5, 4.5),
                p(8.0, 3.5),
                p(10.5, 4.5),
                p(11.5, 7.0),
                p(11.5, 10.0),
                p(12.5, 11.5),
            ];
            if icon == Icon::BellFill {
                fill_poly(outline.to_vec());
            } else {
                closed(&outline);
            }
            line(&[p(6.5, 13.3), p(9.5, 13.3)]);
            if icon == Icon::BellBadge {
                painter.circle_filled(p(12.5, 4.0), 2.8 * scale, super::theme::SURFACE);
                painter.circle_filled(p(12.5, 4.0), 1.7 * scale, color);
            }
        }
        Icon::Clock => {
            circle(p(8.0, 8.0), 6.2, false);
            line(&[p(8.0, 4.5), p(8.0, 8.0), p(10.5, 9.5)]);
        }
        Icon::Download => {
            line(&[p(8.0, 2.5), p(8.0, 10.0)]);
            line(&[p(4.8, 7.0), p(8.0, 10.2), p(11.2, 7.0)]);
            line(&[p(2.5, 11.0), p(2.5, 13.5), p(13.5, 13.5), p(13.5, 11.0)]);
        }
        Icon::Switch => {
            painter.rect_stroke(
                rect_of(p(1.5, 4.5), p(14.5, 11.5)),
                round(3.5),
                stroke,
                StrokeKind::Middle,
            );
            painter.circle_filled(p(11.0, 8.0), 2.0 * scale, color);
        }
        Icon::Calendar => {
            painter.rect_stroke(
                rect_of(p(2.0, 3.5), p(14.0, 14.0)),
                round(2.0),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(2.0, 6.8), p(14.0, 6.8)]);
            line(&[p(5.2, 2.0), p(5.2, 4.8)]);
            line(&[p(10.8, 2.0), p(10.8, 4.8)]);
        }
        Icon::Hash => {
            line(&[p(6.0, 2.5), p(4.5, 13.5)]);
            line(&[p(11.5, 2.5), p(10.0, 13.5)]);
            line(&[p(2.5, 5.8), p(13.5, 5.8)]);
            line(&[p(2.0, 10.2), p(13.0, 10.2)]);
        }
        Icon::TextFormat => {
            line(&[p(2.5, 13.0), p(6.5, 3.0), p(10.5, 13.0)]);
            line(&[p(4.0, 9.5), p(9.0, 9.5)]);
            line(&[p(11.0, 13.0), p(14.0, 13.0)]);
        }
        Icon::List => {
            for y in [4.0, 8.0, 12.0] {
                painter.circle_filled(p(3.0, y), 0.9 * scale, color);
                line(&[p(6.0, y), p(14.0, y)]);
            }
        }
        Icon::BarChart => {
            for (x, top) in [(3.0, 8.0), (7.0, 3.5), (11.0, 6.0)] {
                painter.rect_filled(rect_of(p(x, top), p(x + 2.4, 13.5)), round(0.8), color);
            }
        }
        Icon::Waveform => line(&[
            p(1.0, 8.0),
            p(4.0, 8.0),
            p(6.0, 3.0),
            p(8.0, 13.0),
            p(10.0, 6.0),
            p(11.5, 8.0),
            p(15.0, 8.0),
        ]),
        Icon::Cycle => {
            let arc = |from: f32, to: f32| -> Vec<Pos2> {
                (0..=10)
                    .map(|k| {
                        let a = (from + (to - from) * k as f32 / 10.0).to_radians();
                        p(8.0 + 5.6 * a.cos(), 8.0 + 5.6 * a.sin())
                    })
                    .collect()
            };
            line(&arc(200.0, 340.0));
            line(&arc(20.0, 160.0));
            line(&[p(12.6, 2.6), p(13.8, 5.8), p(10.4, 6.0)]);
            line(&[p(3.4, 13.4), p(2.2, 10.2), p(5.6, 10.0)]);
        }
        Icon::Bands => {
            line(&[
                p(1.5, 5.0),
                p(5.0, 3.0),
                p(8.0, 5.0),
                p(11.0, 3.0),
                p(14.5, 5.0),
            ]);
            line(&[
                p(1.5, 11.0),
                p(5.0, 13.0),
                p(8.0, 11.0),
                p(11.0, 13.0),
                p(14.5, 11.0),
            ]);
            line(&[
                p(1.5, 8.0),
                p(5.0, 8.0),
                p(8.0, 8.0),
                p(11.0, 8.0),
                p(14.5, 8.0),
            ]);
        }
        Icon::Palette => {
            circle(p(8.0, 8.0), 6.4, false);
            for (x, y) in [(5.2, 6.2), (8.0, 4.8), (10.8, 6.4)] {
                painter.circle_filled(p(x, y), 0.9 * scale, color);
            }
        }
    }
}

/// An icon-only button: a square hit target with a hover/press fill, like a toolbar item.
pub fn button(ui: &mut Ui, icon: Icon, size: f32, color: Color32, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let response = super::accessibility::label(response, tooltip).on_hover_text(tooltip);
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            super::theme::white_alpha(26)
        } else if response.hovered() {
            super::theme::white_alpha(14)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 5.0, fill);
        paint(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(size * 0.62)),
            icon,
            color,
        );
    }
    response
}

/// An inline glyph, `size` square, allocated in the layout.
pub fn inline(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui.painter(), rect, icon, color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, pos2};

    #[test]
    fn every_icon_paints_shapes() {
        use Icon::*;
        let all = [
            SidebarLeft,
            ChartLine,
            ChartLineFlat,
            ChartUptrend,
            RectLeft,
            RectTop,
            RectBottom,
            Star,
            StarFill,
            Plus,
            Magnifier,
            ChevronDown,
            ChevronUp,
            ChevronRight,
            ChevronUpDown,
            Eye,
            EyeSlash,
            Curlybraces,
            Check,
            CheckCircleFill,
            XmarkOctagonFill,
            Triangle,
            TriangleFill,
            XmarkCircleFill,
            Xmark,
            Sliders,
            CircleDashed,
            Key,
            Books,
            ArrowUpRightSquare,
            CopyDoc,
            Bell,
            BellFill,
            Clock,
            Download,
            Switch,
            Calendar,
            Hash,
            TextFormat,
            List,
            Palette,
            BarChart,
            Waveform,
            Cycle,
            Bands,
        ];
        for icon in all {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint(
                    ui.painter(),
                    Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(16.0)),
                    icon,
                    Color32::WHITE,
                );
            });
            output.textures_delta.clear();
            assert!(!output.shapes.is_empty(), "{icon:?} drew nothing");
        }
    }
}
