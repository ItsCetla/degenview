//! The vertical tool strip down the left edge of a chart window (Swift `ToolSidebar`): the
//! drawing tools at the top; Paper Trading, Replay and the alerts bell at the bottom.

use crate::model::Tool;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::time::Duration;

pub const WIDTH: f32 = 52.0;
const BUTTON: f32 = 40.0;
const ICON: f32 = 18.0;
const SPACING: f32 = 6.0;
/// How long the pointer rests on a tile before its label appears beside the strip.
const TOOLTIP_DELAY: Duration = Duration::from_millis(250);

pub struct ToolSidebarInput {
    pub active_tool: Tool,
    pub paper_open: bool,
    pub replay_open: bool,
    /// Replay needs a market chart to step through.
    pub has_market: bool,
    /// Alerts fired since the Alerts window was last looked at.
    pub unseen_alerts: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSidebarAction {
    /// Arm `tool`, or disarm it when it already is (back to the crosshair).
    Select(Tool),
    TogglePaper,
    ToggleReplay,
    OpenAlerts,
}

/// How an active tile is drawn: a lit accent tile for an armed tool, or a tinted glyph on a
/// soft wash for a toggle (a panel open, replay running).
#[derive(Clone, Copy, Eq, PartialEq)]
enum Active {
    Off,
    Filled,
    Tinted,
}

struct Tile<'a> {
    icon: Icon,
    label: &'a str,
    tooltip: &'a str,
    active: Active,
    tint: Color32,
    enabled: bool,
    badge: usize,
}

const TOOLS: [(Tool, Icon, &str); 5] = [
    (Tool::Cursor, Icon::Plus, "Crosshair"),
    (Tool::TrendLine, Icon::LineDiagonal, "Trend line"),
    (Tool::Fib, Icon::FibPoints, "Fib Retracement"),
    (Tool::Brush, Icon::Paintbrush, "Brush"),
    (Tool::Ruler, Icon::Ruler, "Ruler"),
];

/// The tooltip a tile shows, separate from its accessibility label where Swift's differs.
pub fn replay_tooltip(replay_open: bool, has_market: bool) -> &'static str {
    if replay_open {
        "Close Replay"
    } else if has_market {
        "Historical Replay"
    } else {
        "Historical Replay — add a market chart first"
    }
}

pub fn show(ui: &mut Ui, input: &ToolSidebarInput) -> Vec<ToolSidebarAction> {
    let mut actions = Vec::new();
    let strip = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(strip, 0.0, theme::CHROME);
    painter.vline(
        strip.right() - 0.5,
        strip.y_range(),
        Stroke::new(1.0, theme::SEPARATOR),
    );

    let x = strip.center().x - BUTTON / 2.0;
    let mut y = strip.top() + 10.0;
    for (tool, icon, label) in TOOLS {
        let rect = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
        let tile = Tile {
            icon,
            label,
            tooltip: label,
            // The crosshair is the resting state, so like Swift's default nothing is lit
            // until a drawing tool is armed; clicking it disarms whichever one is.
            active: if input.active_tool == tool && tool != Tool::Cursor {
                Active::Filled
            } else {
                Active::Off
            },
            tint: kit::ACCENT,
            enabled: true,
            badge: 0,
        };
        if tile_button(ui, rect, &tile, strip.right()).clicked() {
            actions.push(ToolSidebarAction::Select(tool));
        }
        y += BUTTON + SPACING;
    }

    // The bottom group, laid out upwards from the strip's foot.
    let mut y = strip.bottom() - 10.0 - BUTTON;
    let bell = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    let bell_label = if input.unseen_alerts > 0 {
        format!("View Price Alerts, {} new", input.unseen_alerts)
    } else {
        "View Price Alerts".to_owned()
    };
    if tile_button(
        ui,
        bell,
        &Tile {
            icon: Icon::Bell,
            label: &bell_label,
            tooltip: "View Price Alerts",
            active: Active::Off,
            tint: kit::ACCENT,
            enabled: true,
            badge: input.unseen_alerts,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::OpenAlerts);
    }

    // A short inset rule between the groups.
    y -= SPACING + 1.0 + 2.0;
    ui.painter().rect_filled(
        Rect::from_center_size(pos2(strip.center().x, y + 0.5), vec2(22.0, 1.0)),
        0.5,
        theme::SEPARATOR,
    );
    y -= 2.0 + SPACING + BUTTON;

    let replay = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    if tile_button(
        ui,
        replay,
        &Tile {
            icon: Icon::ClockArrow,
            label: if input.replay_open {
                "Close historical bar replay"
            } else {
                "Historical bar replay"
            },
            tooltip: replay_tooltip(input.replay_open, input.has_market),
            active: if input.replay_open {
                Active::Tinted
            } else {
                Active::Off
            },
            tint: theme::REPLAY,
            enabled: input.has_market || input.replay_open,
            badge: 0,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::ToggleReplay);
    }
    y -= SPACING + BUTTON;

    let paper = Rect::from_min_size(pos2(x, y), Vec2::splat(BUTTON));
    if tile_button(
        ui,
        paper,
        &Tile {
            icon: Icon::ArrowLeftRight,
            label: if input.paper_open {
                "Hide Paper Trading panel"
            } else {
                "Show Paper Trading panel"
            },
            tooltip: if input.paper_open {
                "Hide Paper Trading"
            } else {
                "Paper Trading"
            },
            active: if input.paper_open {
                Active::Tinted
            } else {
                Active::Off
            },
            tint: kit::ACCENT,
            enabled: true,
            badge: 0,
        },
        strip.right(),
    )
    .clicked()
    {
        actions.push(ToolSidebarAction::TogglePaper);
    }

    actions
}

/// The tile every strip control shares: glyph, hover wash, press feedback, active styles, the
/// red count badge and the label that slides out beside the strip.
fn tile_button(ui: &mut Ui, rect: Rect, tile: &Tile, strip_right: f32) -> egui::Response {
    let id = Id::new(("tool-sidebar", tile.tooltip));
    let sense = if tile.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    let response = crate::ui::accessibility::label(response, tile.label);
    let hovered = response.hovered() && tile.enabled;
    let pressed = response.is_pointer_button_down_on() && tile.enabled;

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.expand(8.0)));
    if !tile.enabled {
        child.multiply_opacity(0.35);
    }
    let painter = child.painter();
    // Pressed tiles shrink a touch, like the Swift `scaleEffect(0.94)`.
    let shape = if pressed {
        Rect::from_center_size(rect.center(), rect.size() * 0.94)
    } else {
        rect
    };
    let lit = tile.active == Active::Filled;
    let (fill, glyph) = match tile.active {
        Active::Filled => (
            if pressed {
                tile.tint.gamma_multiply(0.85)
            } else {
                tile.tint
            },
            Color32::WHITE,
        ),
        Active::Tinted => (
            theme::with_alpha(
                tile.tint,
                if pressed {
                    0.26
                } else if hovered {
                    0.22
                } else {
                    0.16
                },
            ),
            tile.tint,
        ),
        Active::Off => (
            if pressed {
                kit::primary(0.14)
            } else if hovered {
                kit::primary(0.08)
            } else {
                Color32::TRANSPARENT
            },
            if hovered || pressed {
                theme::TEXT_PRIMARY
            } else {
                kit::primary(0.75)
            },
        ),
    };
    if lit {
        painter.add(
            egui::Shadow {
                offset: [0, 2],
                blur: 10,
                spread: 0,
                color: theme::with_alpha(tile.tint, 0.35),
            }
            .as_shape(shape, 10.0),
        );
    }
    painter.rect_filled(shape, 10.0, fill);
    if lit {
        // A faint rim, brightest along the top edge, like the Swift gradient stroke.
        painter.rect_stroke(
            shape,
            10.0,
            Stroke::new(1.0, Color32::from_white_alpha(18)),
            StrokeKind::Inside,
        );
        painter.hline(
            (shape.left() + 7.0)..=(shape.right() - 7.0),
            shape.top() + 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(60)),
        );
    }
    icons::paint(
        painter,
        Rect::from_center_size(
            shape.center(),
            Vec2::splat(ICON * if pressed { 0.94 } else { 1.0 }),
        ),
        tile.icon,
        glyph,
    );
    if tile.badge > 0 {
        badge(
            &child,
            rect.right_top() + vec2(5.0 - 8.0, -5.0 + 8.0),
            tile.badge,
        );
    }

    tooltip(ui, id, &response, rect, strip_right, tile.tooltip);
    response
}

/// The red count bubble, centred on `at` (the tile's top-right corner, nudged outwards).
fn badge(ui: &Ui, at: Pos2, count: usize) {
    let text = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let galley = kit::digits(
        ui,
        &text,
        10.0,
        kit::SEMIBOLD,
        Color32::WHITE,
        f32::INFINITY,
    );
    let size = vec2((galley.size().x + 8.0).max(16.0), 16.0);
    let bubble = Rect::from_center_size(at, size);
    let painter = ui.painter();
    painter.rect_filled(bubble.expand(1.5), 9.5, theme::CHROME);
    painter.rect_filled(bubble, 8.0, kit::RED);
    painter.galley(
        bubble.center() - galley.size() / 2.0,
        galley,
        Color32::WHITE,
    );
}

/// The label that appears beside the strip once the pointer has rested on a tile.
fn tooltip(ui: &Ui, id: Id, response: &egui::Response, tile: Rect, strip_right: f32, label: &str) {
    let since = id.with("hover-since");
    let now = ui.input(|input| input.time);
    if !response.hovered() {
        ui.ctx().data_mut(|data| data.remove::<f64>(since));
        return;
    }
    let started = ui
        .ctx()
        .data_mut(|data| *data.get_temp_mut_or_insert_with(since, || now));
    let waited = now - started;
    if waited < TOOLTIP_DELAY.as_secs_f64() {
        ui.ctx().request_repaint_after(Duration::from_secs_f64(
            TOOLTIP_DELAY.as_secs_f64() - waited,
        ));
        return;
    }
    let galley = kit::galley(
        ui,
        label,
        kit::size::BODY - 1.0,
        kit::REGULAR,
        theme::TEXT_PRIMARY,
        360.0,
    );
    let size = galley.size() + vec2(24.0, 14.0);
    let rect = Rect::from_min_size(
        pos2(strip_right + 8.0, tile.center().y - size.y / 2.0),
        size,
    );
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("tooltip")));
    painter.add(
        egui::Shadow {
            offset: [0, 3],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(36),
        }
        .as_shape(rect, 8.0),
    );
    painter.rect_filled(rect, 8.0, Color32::from_rgb(48, 57, 62));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(0.5, kit::primary(0.12)),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(12.0, 7.0), galley, theme::TEXT_PRIMARY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_tooltips_say_what_a_click_does() {
        assert_eq!(replay_tooltip(true, true), "Close Replay");
        assert_eq!(replay_tooltip(false, true), "Historical Replay");
        assert_eq!(
            replay_tooltip(false, false),
            "Historical Replay — add a market chart first"
        );
    }

    #[test]
    fn the_strip_draws_headlessly_in_every_state() {
        let ctx = egui::Context::default();
        for (tool, paper, replay, market, unseen) in [
            (Tool::Cursor, false, false, true, 0),
            (Tool::Fib, true, true, true, 3),
            (Tool::Ruler, false, false, false, 120),
        ] {
            let state = ToolSidebarInput {
                active_tool: tool,
                paper_open: paper,
                replay_open: replay,
                has_market: market,
                unseen_alerts: unseen,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &state).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }
}
