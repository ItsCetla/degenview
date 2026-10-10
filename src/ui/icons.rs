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
    SidebarRight,
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
    Circle,
    BitcoinCircle,
    BuildingColumns,
    Percent,
    Gauge,
    GaugeHigh,
    Sparkles,
    ClockArrow,
    Flame,
    Diamond,
    ArrowDownRightCircle,
    CheckmarkSeal,
    PersonCircle,
    SportsCourt,
    CloudSun,
    Briefcase,
    PieChart,
    Layers,
    Exchange,
    Banknote,
    ArrowUpRight,
    ArrowDownRight,
    ArrowUp,
    ArrowDown,
    Pencil,
    PauseFill,
    PlayFill,
    Ellipsis,
    Repeat,
    OneCircle,
    Trash,
    BellSlash,
    PauseCircle,
    CheckCircle,
    ClockBadge,
    LineDiagonal,
    FibPoints,
    Paintbrush,
    Ruler,
    ArrowLeftRight,
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

    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        let steps = (((to - from).abs() / 12.0).ceil() as usize).max(4);
        (0..=steps)
            .map(|k| {
                let a = (from + (to - from) * k as f32 / steps as f32).to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    let sparkle = |cx: f32, cy: f32, r: f32| {
        let points = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = if i % 2 == 0 { r } else { r * 0.3 };
                p(cx + d * a.cos(), cy + d * a.sin())
            })
            .collect();
        fill_poly(points);
    };

    match icon {
        Icon::Briefcase => {
            painter.rect_stroke(
                rect_of(p(1.5, 5.0), p(14.5, 13.5)),
                round(1.8),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(5.3, 5.0), p(5.3, 2.7), p(10.7, 2.7), p(10.7, 5.0)]);
            line(&[p(1.8, 8.3), p(6.7, 9.4), p(9.3, 9.4), p(14.2, 8.3)]);
        }
        Icon::PieChart => {
            line(&arc(7.6, 8.3, 5.6, -64.0, 296.0));
            line(&[p(8.0, 8.0), p(8.0, 2.6)]);
            line(&[p(8.0, 8.0), p(13.3, 8.0)]);
        }
        Icon::Layers => {
            for (left, top, right, bottom) in [(3.0, 2.0, 12.5, 10.0), (1.5, 5.0, 11.0, 13.0)] {
                painter.rect_stroke(
                    rect_of(p(left, top), p(right, bottom)),
                    round(1.5),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        Icon::Exchange => {
            line(&[p(2.0, 5.0), p(13.3, 5.0), p(10.6, 2.4)]);
            line(&[p(13.3, 5.0), p(10.6, 7.6)]);
            line(&[p(14.0, 11.0), p(2.7, 11.0), p(5.4, 8.4)]);
            line(&[p(2.7, 11.0), p(5.4, 13.6)]);
        }
        Icon::Banknote => {
            painter.rect_stroke(
                rect_of(p(1.2, 3.5), p(14.8, 12.5)),
                round(1.5),
                stroke,
                StrokeKind::Middle,
            );
            circle(p(8.0, 8.0), 2.5, false);
            line(&[p(3.0, 6.0), p(4.0, 6.0)]);
            line(&[p(12.0, 10.0), p(13.0, 10.0)]);
        }
        Icon::SidebarRight => {
            painter.rect_stroke(
                rect_of(p(1.5, 2.5), p(14.5, 13.5)),
                round(2.5),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(10.0, 2.8), p(10.0, 13.2)]);
        }
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
        Icon::Circle => circle(p(8.0, 8.0), 6.6, false),
        Icon::BitcoinCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            let thin = Stroke::new((1.15 * scale).max(1.0), color);
            let mut b = vec![p(6.3, 8.0), p(6.3, 4.7), p(8.9, 4.7)];
            b.extend(arc(8.9, 6.35, 1.65, -90.0, 90.0));
            b.push(p(6.3, 8.0));
            b.push(p(9.3, 8.0));
            b.extend(arc(9.3, 9.65, 1.65, -90.0, 90.0));
            b.extend([p(6.3, 11.3), p(6.3, 8.0)]);
            painter.add(Shape::Path(PathShape::line(b, thin)));
            for x in [7.2, 8.7] {
                painter.line_segment([p(x, 3.6), p(x, 4.7)], thin);
                painter.line_segment([p(x, 11.3), p(x, 12.4)], thin);
            }
        }
        Icon::BuildingColumns => {
            closed(&[p(1.8, 5.6), p(8.0, 2.0), p(14.2, 5.6)]);
            for x in [4.0, 8.0, 12.0] {
                line(&[p(x, 7.2), p(x, 11.6)]);
            }
            line(&[p(1.8, 13.6), p(14.2, 13.6)]);
        }
        Icon::Percent => {
            line(&[p(12.8, 3.0), p(3.2, 13.0)]);
            circle(p(4.6, 4.6), 2.0, false);
            circle(p(11.4, 11.4), 2.0, false);
        }
        Icon::Gauge | Icon::GaugeHigh => {
            line(&arc(8.0, 9.0, 6.4, 135.0, 405.0));
            let angle: f32 = if icon == Icon::Gauge { 270.0 } else { 315.0 };
            let a = angle.to_radians();
            line(&[p(8.0, 9.0), p(8.0 + 4.3 * a.cos(), 9.0 + 4.3 * a.sin())]);
            circle(p(8.0, 9.0), 1.3, true);
        }
        Icon::Sparkles => {
            sparkle(6.5, 9.5, 5.6);
            sparkle(12.6, 3.6, 2.8);
            sparkle(12.8, 12.6, 1.9);
        }
        Icon::ClockArrow => {
            line(&arc(8.5, 8.0, 5.9, 200.0, 515.0));
            line(&[p(1.3, 4.6), p(3.0, 6.0), p(4.8, 4.5)]);
            line(&[p(8.5, 4.8), p(8.5, 8.0), p(10.7, 9.4)]);
        }
        Icon::Flame => closed(&[
            p(8.4, 1.6),
            p(10.9, 4.6),
            p(12.6, 8.2),
            p(12.3, 11.4),
            p(10.4, 13.8),
            p(8.0, 14.5),
            p(5.6, 13.8),
            p(3.8, 11.6),
            p(3.6, 8.6),
            p(5.0, 6.0),
            p(6.3, 8.2),
            p(7.0, 4.6),
        ]),
        Icon::Diamond => closed(&[p(8.0, 1.5), p(14.5, 8.0), p(8.0, 14.5), p(1.5, 8.0)]),
        Icon::ArrowDownRightCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            line(&[p(5.3, 5.3), p(10.6, 10.6)]);
            line(&[p(10.7, 6.4), p(10.7, 10.7), p(6.4, 10.7)]);
        }
        Icon::CheckmarkSeal => {
            let seal = (0..24)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 12.0;
                    let r = if i % 2 == 0 { 7.1 } else { 6.1 };
                    p(8.0 + r * a.cos(), 8.0 + r * a.sin())
                })
                .collect::<Vec<_>>();
            closed(&seal);
            line(&[p(5.2, 8.2), p(7.2, 10.2), p(10.9, 5.9)]);
        }
        Icon::PersonCircle => {
            circle(p(8.0, 8.0), 6.9, false);
            circle(p(8.0, 6.4), 2.3, false);
            line(&arc(8.0, 14.6, 4.6, 222.0, 318.0));
        }
        Icon::SportsCourt => {
            painter.rect_stroke(
                rect_of(p(1.5, 3.5), p(14.5, 12.5)),
                round(1.2),
                stroke,
                StrokeKind::Middle,
            );
            line(&[p(8.0, 3.5), p(8.0, 12.5)]);
            circle(p(8.0, 8.0), 2.0, false);
        }
        Icon::CloudSun => {
            circle(p(5.6, 5.4), 2.2, false);
            for (dx, dy) in [(0.0, -1.0), (-1.0, 0.0), (-0.7, -0.7), (0.7, -0.7)] {
                line(&[
                    p(5.6 + dx * 3.3, 5.4 + dy * 3.3),
                    p(5.6 + dx * 4.4, 5.4 + dy * 4.4),
                ]);
            }
            let mut cloud = arc(12.3, 11.4, 2.2, 90.0, -70.0);
            cloud.extend(arc(9.4, 9.9, 3.0, -25.0, -175.0));
            cloud.extend(arc(5.4, 11.7, 1.9, -120.0, -270.0));
            closed(&cloud);
        }
        Icon::ArrowUpRight => {
            line(&[p(4.0, 12.0), p(12.0, 4.0)]);
            line(&[p(6.0, 4.0), p(12.0, 4.0), p(12.0, 10.0)]);
        }
        Icon::ArrowDownRight => {
            line(&[p(4.0, 4.0), p(12.0, 12.0)]);
            line(&[p(12.0, 6.0), p(12.0, 12.0), p(6.0, 12.0)]);
        }
        Icon::ArrowUp => {
            line(&[p(8.0, 13.5), p(8.0, 2.8)]);
            line(&[p(3.8, 7.0), p(8.0, 2.8), p(12.2, 7.0)]);
        }
        Icon::ArrowDown => {
            line(&[p(8.0, 2.5), p(8.0, 13.2)]);
            line(&[p(3.8, 9.0), p(8.0, 13.2), p(12.2, 9.0)]);
        }
        Icon::Pencil => {
            closed(&[
                p(10.6, 2.4),
                p(13.6, 5.4),
                p(5.6, 13.4),
                p(2.2, 13.8),
                p(2.6, 10.4),
            ]);
            line(&[p(9.0, 4.0), p(12.0, 7.0)]);
        }
        Icon::PauseFill => {
            painter.rect_filled(rect_of(p(3.8, 2.5), p(6.6, 13.5)), round(0.9), color);
            painter.rect_filled(rect_of(p(9.4, 2.5), p(12.2, 13.5)), round(0.9), color);
        }
        Icon::PlayFill => fill_poly(vec![p(4.2, 2.2), p(13.6, 8.0), p(4.2, 13.8)]),
        Icon::Ellipsis => {
            for x in [3.0, 8.0, 13.0] {
                circle(p(x, 8.0), 1.35, true);
            }
        }
        Icon::Repeat => {
            line(&[p(2.5, 9.0), p(2.5, 5.0), p(13.0, 5.0)]);
            line(&[p(10.8, 2.8), p(13.0, 5.0), p(10.8, 7.2)]);
            line(&[p(13.5, 7.0), p(13.5, 11.0), p(3.0, 11.0)]);
            line(&[p(5.2, 8.8), p(3.0, 11.0), p(5.2, 13.2)]);
        }
        Icon::OneCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(6.4, 5.6), p(8.4, 4.4), p(8.4, 11.6)]);
        }
        Icon::Trash => {
            line(&[p(2.0, 4.0), p(14.0, 4.0)]);
            line(&[p(6.0, 4.0), p(6.0, 2.2), p(10.0, 2.2), p(10.0, 4.0)]);
            line(&[p(3.6, 4.0), p(4.5, 14.0), p(11.5, 14.0), p(12.4, 4.0)]);
            line(&[p(6.6, 6.8), p(6.8, 11.4)]);
            line(&[p(9.4, 6.8), p(9.2, 11.4)]);
        }
        Icon::BellSlash => {
            closed(&[
                p(3.5, 11.5),
                p(4.5, 10.0),
                p(4.5, 7.0),
                p(5.5, 4.5),
                p(8.0, 3.5),
                p(10.5, 4.5),
                p(11.5, 7.0),
                p(11.5, 10.0),
                p(12.5, 11.5),
            ]);
            line(&[p(6.5, 13.3), p(9.5, 13.3)]);
            line(&[p(2.0, 2.0), p(14.0, 14.0)]);
        }
        Icon::PauseCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(6.4, 5.4), p(6.4, 10.6)]);
            line(&[p(9.6, 5.4), p(9.6, 10.6)]);
        }
        Icon::CheckCircle => {
            circle(p(8.0, 8.0), 6.6, false);
            line(&[p(5.0, 8.2), p(7.2, 10.4), p(11.2, 5.8)]);
        }
        Icon::ClockBadge => {
            line(&arc(7.0, 8.5, 5.6, 300.0, 600.0));
            line(&[p(7.0, 5.6), p(7.0, 8.5), p(9.0, 9.6)]);
            line(&[p(13.6, 2.0), p(13.6, 6.0)]);
            circle(p(13.6, 8.4), 0.9, true);
        }
        Icon::LineDiagonal => line(&[p(2.5, 13.5), p(13.5, 2.5)]),
        Icon::FibPoints => {
            // Three points joined by a dotted triangle, like `point.3.connected.trianglepath.dotted`.
            let dots = |a: Pos2, b: Pos2| {
                for k in 1..5 {
                    let t = k as f32 / 5.0;
                    painter.circle_filled(a + (b - a) * t, 0.55 * scale, color);
                }
            };
            let (top, left, right) = (p(8.0, 2.6), p(2.6, 13.0), p(13.4, 13.0));
            dots(top, left);
            dots(left, right);
            dots(top, right);
            for point in [top, left, right] {
                painter.circle_stroke(point, 1.7 * scale, stroke);
            }
        }
        Icon::Paintbrush => {
            closed(&[p(13.8, 1.8), p(14.4, 2.4), p(8.4, 9.6), p(6.6, 8.0)]);
            fill_poly(vec![
                p(6.0, 8.6),
                p(7.8, 10.2),
                p(7.0, 12.6),
                p(4.2, 14.2),
                p(1.6, 14.4),
                p(2.6, 12.6),
                p(3.4, 10.0),
            ]);
        }
        Icon::Ruler => {
            closed(&[p(1.4, 10.6), p(10.6, 1.4), p(14.6, 5.4), p(5.4, 14.6)]);
            for (k, long) in [
                (0.0, true),
                (1.0, false),
                (2.0, true),
                (3.0, false),
                (4.0, true),
            ] {
                let base = p(4.3 + k * 1.75, 7.7 - k * 1.75);
                let reach = if long { 2.4 } else { 1.4 };
                line(&[base, base + vec2(reach, reach) * scale]);
            }
        }
        Icon::ArrowLeftRight => {
            line(&[p(2.0, 5.2), p(13.5, 5.2)]);
            line(&[p(10.6, 2.4), p(13.5, 5.2), p(10.6, 8.0)]);
            line(&[p(14.0, 10.8), p(2.5, 10.8)]);
            line(&[p(5.4, 8.0), p(2.5, 10.8), p(5.4, 13.6)]);
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
            SidebarRight,
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
            Circle,
            BitcoinCircle,
            BuildingColumns,
            Percent,
            Gauge,
            GaugeHigh,
            Sparkles,
            ClockArrow,
            Flame,
            Diamond,
            ArrowDownRightCircle,
            CheckmarkSeal,
            PersonCircle,
            SportsCourt,
            CloudSun,
            ArrowUpRight,
            ArrowDownRight,
            ArrowUp,
            ArrowDown,
            Pencil,
            PauseFill,
            PlayFill,
            Ellipsis,
            Repeat,
            OneCircle,
            Trash,
            BellSlash,
            PauseCircle,
            CheckCircle,
            ClockBadge,
            LineDiagonal,
            FibPoints,
            Paintbrush,
            Ruler,
            ArrowLeftRight,
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
