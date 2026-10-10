//! The chart window's toolbar, laid out like the Swift one: the timeframe as a segmented
//! control on the left; the layout menu, Portfolio and Script Manager, the watchlist toggle and
//! Add Chart as separate bubbles on the right. Replay and Alerts live on the tool strip.

use crate::model::Timeframe;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub const HEIGHT: f32 = 48.0;
const BUBBLE: f32 = 30.0;
const GAP: f32 = 10.0;
/// Long layout names are cut so the toolbar keeps its shape.
const NAME_MAX_WIDTH: f32 = 180.0;

pub struct ToolbarInput<'a> {
    pub timeframe: Timeframe,
    /// The focused chart's log scale, when a chart is focused.
    pub log_scale: Option<bool>,
    pub layout_name: &'a str,
    pub unsaved: bool,
    /// `(id, name)` of every saved layout, and which one is open.
    pub layouts: &'a [(u64, String)],
    pub active_layout: Option<u64>,
    pub watchlist_visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolbarAction {
    SetTimeframe(Timeframe),
    ToggleLogScale,
    SaveLayout,
    CopyLayout,
    RenameLayout,
    OpenLayout(u64),
    DeleteLayout(u64),
    ToggleWatchlist,
    OpenPortfolio,
    OpenScriptManager,
    AddChart,
}

pub fn show(ui: &mut Ui, input: &ToolbarInput) -> Vec<ToolbarAction> {
    let mut actions = Vec::new();
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0.0, theme::CHROME);
    ui.painter().hline(
        bar.x_range(),
        bar.bottom() - 0.5,
        Stroke::new(1.0, theme::SEPARATOR),
    );
    let y = bar.center().y - BUBBLE / 2.0;

    // Left: the timeframe, then the focused chart's log scale.
    let mut x = bar.left() + 12.0;
    if let Some(timeframe) = timeframes(ui, pos2(x, y), input.timeframe, &mut x) {
        actions.push(ToolbarAction::SetTimeframe(timeframe));
    }
    if let Some(on) = input.log_scale {
        x += GAP;
        let rect = Rect::from_min_size(pos2(x, y), vec2(46.0, BUBBLE));
        let response = bubble_button(ui, rect, Id::new("toolbar-log"), on).on_hover_text(if on {
            "Logarithmic price scale (click for linear)"
        } else {
            "Linear price scale (click for logarithmic)"
        });
        let response = crate::ui::accessibility::label(response, "Logarithmic scale");
        let label = kit::galley(
            ui,
            "Log",
            12.5,
            if on { SEMIBOLD } else { MEDIUM },
            if on { kit::ACCENT } else { theme::TEXT_PRIMARY },
            40.0,
        );
        ui.painter().galley(
            rect.center() - label.size() / 2.0,
            label,
            theme::TEXT_PRIMARY,
        );
        if response.clicked() {
            actions.push(ToolbarAction::ToggleLogScale);
        }
    }

    // Right, from the edge inwards: Add Chart, the watchlist, the app buttons, the layout.
    let mut right = bar.right() - 12.0;
    let add_label = kit::galley(
        ui,
        "Add Chart",
        size::BODY,
        MEDIUM,
        theme::TEXT_PRIMARY,
        120.0,
    );
    let add = Rect::from_min_max(
        pos2(right - (12.0 + 12.0 + 6.0 + add_label.size().x + 12.0), y),
        pos2(right, y + BUBBLE),
    );
    if crate::ui::accessibility::label(
        bubble_button(ui, add, Id::new("toolbar-add"), false),
        "Add Chart",
    )
    .on_hover_text("Add Chart")
    .clicked()
    {
        actions.push(ToolbarAction::AddChart);
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(add.left() + 12.0, add.center().y - 6.0),
            Vec2::splat(12.0),
        ),
        Icon::Plus,
        theme::TEXT_PRIMARY,
    );
    ui.painter().galley(
        pos2(add.left() + 30.0, add.center().y - add_label.size().y / 2.0),
        add_label,
        theme::TEXT_PRIMARY,
    );
    right = add.left() - GAP;

    let sidebar = Rect::from_min_max(pos2(right - 38.0, y), pos2(right, y + BUBBLE));
    let tip = if input.watchlist_visible {
        "Hide Watchlists"
    } else {
        "Show Watchlists"
    };
    if icon_bubble(
        ui,
        sidebar,
        Id::new("toolbar-watchlist"),
        Icon::SidebarRight,
        tip,
    )
    .clicked()
    {
        actions.push(ToolbarAction::ToggleWatchlist);
    }
    right = sidebar.left() - GAP;

    // Portfolio and Script Manager share one bubble, as the Swift title-bar group does.
    let group = Rect::from_min_max(pos2(right - 76.0, y), pos2(right, y + BUBBLE));
    paint_bubble(ui, group, false, false);
    let halves = [
        (
            Icon::Briefcase,
            "Portfolio Tracker",
            ToolbarAction::OpenPortfolio,
        ),
        (
            Icon::Curlybraces,
            "Script Manager",
            ToolbarAction::OpenScriptManager,
        ),
    ];
    for (index, (icon, tip, action)) in halves.into_iter().enumerate() {
        let half = Rect::from_min_size(
            pos2(group.left() + 2.0 + index as f32 * 36.0, group.top() + 2.0),
            vec2(36.0, BUBBLE - 4.0),
        );
        let response = ui
            .interact(half, Id::new(("toolbar-app", index)), Sense::click())
            .on_hover_text(tip);
        let response = crate::ui::accessibility::label(response, tip);
        if response.hovered() {
            ui.painter().rect_filled(
                half,
                (BUBBLE - 4.0) / 2.0,
                kit::primary(if response.is_pointer_button_down_on() {
                    0.14
                } else {
                    0.08
                }),
            );
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(half.center(), Vec2::splat(15.0)),
            icon,
            theme::TEXT_PRIMARY,
        );
        if response.clicked() {
            actions.push(action);
        }
    }
    right = group.left() - GAP;

    layout_menu(ui, right, y, input, &mut actions);
    actions
}

/// The bubble every toolbar control sits in: a faint capsule that lifts on hover.
fn paint_bubble(ui: &Ui, rect: Rect, hovered: bool, pressed: bool) {
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        rect.height() / 2.0,
        kit::primary(if pressed {
            0.16
        } else if hovered {
            0.12
        } else {
            0.07
        }),
    );
    painter.rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(1.0, kit::primary(0.06)),
        StrokeKind::Inside,
    );
}

fn bubble_button(ui: &mut Ui, rect: Rect, id: Id, active: bool) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    paint_bubble(
        ui,
        rect,
        response.hovered(),
        response.is_pointer_button_down_on(),
    );
    if active {
        ui.painter().rect_filled(
            rect,
            rect.height() / 2.0,
            theme::with_alpha(kit::ACCENT, 0.16),
        );
    }
    response
}

fn icon_bubble(ui: &mut Ui, rect: Rect, id: Id, icon: Icon, tip: &str) -> egui::Response {
    let response = bubble_button(ui, rect, id, false).on_hover_text(tip);
    let response = crate::ui::accessibility::label(response, tip);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(15.0)),
        icon,
        theme::TEXT_PRIMARY,
    );
    response
}

/// The timeframe picker: one capsule, the chosen segment raised. Advances `x` past it.
fn timeframes(ui: &mut Ui, at: egui::Pos2, current: Timeframe, x: &mut f32) -> Option<Timeframe> {
    let labels: Vec<_> = Timeframe::ALL
        .iter()
        .map(|timeframe| {
            kit::galley(
                ui,
                timeframe.label(),
                12.5,
                if *timeframe == current {
                    SEMIBOLD
                } else {
                    MEDIUM
                },
                theme::TEXT_PRIMARY,
                60.0,
            )
        })
        .collect();
    let widths: Vec<f32> = labels
        .iter()
        .map(|label| (label.size().x + 22.0).max(40.0))
        .collect();
    let track = Rect::from_min_size(at, vec2(widths.iter().sum::<f32>() + 4.0, BUBBLE));
    paint_bubble(ui, track, false, false);
    let mut picked = None;
    let mut left = track.left() + 2.0;
    for ((timeframe, label), width) in Timeframe::ALL.into_iter().zip(labels).zip(widths) {
        let segment = Rect::from_min_size(pos2(left, track.top() + 2.0), vec2(width, BUBBLE - 4.0));
        let response = ui
            .interact(
                segment,
                Id::new(("toolbar-timeframe", timeframe.label())),
                Sense::click(),
            )
            .on_hover_text(timeframe.description());
        let response = crate::ui::accessibility::label(response, timeframe.description());
        let selected = timeframe == current;
        let painter = ui.painter();
        if selected {
            painter.add(
                egui::Shadow {
                    offset: [0, 1],
                    blur: 3,
                    spread: 0,
                    color: Color32::from_black_alpha(45),
                }
                .as_shape(segment, segment.height() / 2.0),
            );
            painter.rect_filled(segment, segment.height() / 2.0, kit::primary(0.2));
        } else if response.hovered() {
            painter.rect_filled(segment, segment.height() / 2.0, kit::primary(0.06));
        }
        let color = if selected {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_SECONDARY
        };
        painter.galley(segment.center() - label.size() / 2.0, label, color);
        if response.clicked() && !selected {
            picked = Some(timeframe);
        }
        left += width;
    }
    *x = track.right();
    picked
}

/// The layout bubble, its right edge at `right`: the layout's name, "(unsaved)" while it has
/// changes, and a menu of what can be done with it.
fn layout_menu(
    ui: &mut Ui,
    right: f32,
    y: f32,
    input: &ToolbarInput,
    actions: &mut Vec<ToolbarAction>,
) {
    let name = kit::galley(
        ui,
        input.layout_name,
        size::BODY,
        MEDIUM,
        theme::TEXT_PRIMARY,
        NAME_MAX_WIDTH,
    );
    let unsaved = input.unsaved.then(|| {
        kit::galley(
            ui,
            "(unsaved)",
            size::BODY,
            REGULAR,
            theme::TEXT_SECONDARY,
            90.0,
        )
    });
    let content =
        name.size().x + unsaved.as_ref().map_or(0.0, |label| 4.0 + label.size().x) + 4.0 + 9.0;
    let rect = Rect::from_min_max(pos2(right - content - 24.0, y), pos2(right, y + BUBBLE));
    let help = if input.unsaved {
        format!("{} — unsaved changes", input.layout_name)
    } else {
        input.layout_name.to_owned()
    };
    let response = bubble_button(ui, rect, Id::new("toolbar-layout"), false).on_hover_text(&help);
    let response = crate::ui::accessibility::label(response, format!("Layout: {help}"));
    let mut x = rect.left() + 12.0;
    let name_width = name.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::TEXT_PRIMARY,
    );
    x += name_width + 4.0;
    if let Some(label) = unsaved {
        let width = label.size().x;
        ui.painter().galley(
            pos2(x, rect.center().y - label.size().y / 2.0),
            label,
            theme::TEXT_SECONDARY,
        );
        x += width + 4.0;
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(pos2(x, rect.center().y - 4.5), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::TEXT_SECONDARY,
    );

    egui::Popup::menu(&response)
        .frame(kit::menu_frame())
        .align(egui::RectAlign::BOTTOM_END)
        .gap(6.0)
        .show(|ui| {
            ui.set_width(230.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            if kit::menu_item(ui, Icon::Download, "Save layout", true, false) {
                actions.push(ToolbarAction::SaveLayout);
                ui.close();
            }
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::CopyDoc, "Make a copy", true, false) {
                actions.push(ToolbarAction::CopyLayout);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::Pencil,
                "Rename",
                input.active_layout.is_some(),
                false,
            ) {
                actions.push(ToolbarAction::RenameLayout);
                ui.close();
            }
            if !input.layouts.is_empty() {
                kit::menu_separator(ui);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(kit::text(
                        "Layouts",
                        size::CAPTION + 0.5,
                        SEMIBOLD,
                        theme::TEXT_TERTIARY,
                    ));
                });
                ui.add_space(4.0);
                for (id, name) in input.layouts {
                    match layout_row(ui, *id, name, input.active_layout == Some(*id)) {
                        Some(LayoutRow::Open) => {
                            actions.push(ToolbarAction::OpenLayout(*id));
                            ui.close();
                        }
                        Some(LayoutRow::Delete) => {
                            actions.push(ToolbarAction::DeleteLayout(*id));
                            ui.close();
                        }
                        None => {}
                    }
                }
            }
        });
}

enum LayoutRow {
    Open,
    Delete,
}

/// A saved layout in the menu: a check on the open one, the name, and a trash button on hover.
fn layout_row(ui: &mut Ui, id: u64, name: &str, current: bool) -> Option<LayoutRow> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let trash = Rect::from_min_size(
        pos2(rect.right() - 26.0, rect.top() + 2.0),
        Vec2::splat(20.0),
    );
    let row = ui.interact(rect, Id::new(("toolbar-layout-row", id)), Sense::click());
    let delete = ui
        .interact(
            trash,
            Id::new(("toolbar-layout-delete", id)),
            Sense::click(),
        )
        .on_hover_text("Delete this layout");
    let hovered = row.hovered() || delete.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 5.0, kit::ACCENT);
    }
    let color = if hovered {
        Color32::WHITE
    } else {
        theme::TEXT_PRIMARY
    };
    if current {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(11.0)),
            Icon::Check,
            color,
        );
    }
    let label = kit::galley(ui, name, size::BODY, REGULAR, color, width - 70.0);
    painter.galley(
        pos2(rect.left() + 32.0, rect.center().y - label.size().y / 2.0),
        label,
        color,
    );
    if hovered {
        if delete.hovered() {
            painter.rect_filled(trash, 5.0, Color32::from_white_alpha(40));
        }
        icons::paint(
            painter,
            Rect::from_center_size(trash.center(), Vec2::splat(11.0)),
            Icon::Trash,
            Color32::WHITE,
        );
    }
    if delete.clicked() {
        Some(LayoutRow::Delete)
    } else if row.clicked() {
        Some(LayoutRow::Open)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbar_draws_headlessly_with_and_without_a_focused_chart() {
        let layouts = vec![(1, "Morning".to_owned()), (2, "Swing trades".to_owned())];
        let ctx = egui::Context::default();
        for (log, unsaved, active) in [(None, false, None), (Some(true), true, Some(2))] {
            let input = ToolbarInput {
                timeframe: Timeframe::Day,
                log_scale: log,
                layout_name: "Morning",
                unsaved,
                layouts: &layouts,
                active_layout: active,
                watchlist_visible: true,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &input).is_empty(), "nothing was clicked");
            });
            output.textures_delta.clear();
        }
    }
}
