//! The tab strip at the top of every window: one tab per open tab, a `+`, and the events the
//! window host turns into model edits. Dragging is only *reported* here (`DragStarted`); the host
//! drives the rest from raw pointer state because a dragged tab leaves this strip.

use super::icons::{self, Icon};
use super::theme;
use crate::model::tab_windows::{TabId, WindowId};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};

/// Strip height, below the native titlebar.
pub const HEIGHT: f32 = 34.0;
const MIN_TAB_WIDTH: f32 = 96.0;
const MAX_TAB_WIDTH: f32 = 220.0;
const NEW_BUTTON_WIDTH: f32 = 34.0;
const TAB_GAP: f32 = 2.0;
const SIDE_PADDING: f32 = 6.0;
const CLOSE_SIZE: f32 = 18.0;
const ICON_SIZE: f32 = 13.0;

/// What a tab shows.
#[derive(Clone, Debug)]
pub struct TabView {
    pub id: TabId,
    pub title: String,
    pub icon: Icon,
}

/// A window a tab can be sent to from the context menu.
#[derive(Clone, Debug)]
pub struct WindowChoice {
    pub id: WindowId,
    pub title: String,
}

pub struct TabBar<'a> {
    pub window: WindowId,
    pub tabs: &'a [TabView],
    pub active: TabId,
    /// The tab being dragged by the user right now, if it is in this strip.
    pub dragging: Option<TabId>,
    /// Where a tab dragged in from another window would land (0..=tabs.len()).
    pub drop_index: Option<usize>,
    pub other_windows: &'a [WindowChoice],
}

#[derive(Clone, Debug, PartialEq)]
pub enum TabBarEvent {
    Select(TabId),
    Close(TabId),
    New,
    /// The pointer pressed on `tab` and started dragging; `grab` is its offset inside the tab.
    DragStarted {
        tab: TabId,
        grab: Vec2,
    },
    MoveToNewWindow(TabId),
    MoveToWindow {
        tab: TabId,
        window: WindowId,
    },
}

/// What one frame of the strip produced.
pub struct TabBarOutput {
    pub events: Vec<TabBarEvent>,
    /// The strip in the window's own coordinates (where the dragged tab can be dropped).
    pub strip: Rect,
}

/// Width of each tab: they share the room left of the `+`, between the min and max.
pub fn tab_width(count: usize, available: f32) -> f32 {
    if count == 0 {
        return MAX_TAB_WIDTH;
    }
    let room = (available - NEW_BUTTON_WIDTH - SIDE_PADDING * 2.0).max(0.0);
    let gaps = TAB_GAP * (count - 1) as f32;
    ((room - gaps) / count as f32).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH)
}

/// Left edge of tab `index`, relative to the strip's left edge.
pub fn tab_left(index: usize, width: f32) -> f32 {
    SIDE_PADDING + index as f32 * (width + TAB_GAP)
}

/// The slot under `x` (relative to the strip's left edge), for reordering an existing tab.
pub fn slot_at(x: f32, count: usize, width: f32) -> usize {
    if count == 0 {
        return 0;
    }
    let slot = ((x - SIDE_PADDING) / (width + TAB_GAP)).floor();
    (slot.max(0.0) as usize).min(count - 1)
}

/// The gap nearest `x`, for dropping a tab from elsewhere: 0 is before the first tab and
/// `count` after the last.
pub fn insertion_at(x: f32, count: usize, width: f32) -> usize {
    let gap = ((x - SIDE_PADDING) / (width + TAB_GAP)).round();
    (gap.max(0.0) as usize).min(count)
}

impl TabBar<'_> {
    pub fn show(&self, ui: &mut Ui) -> TabBarOutput {
        let mut events = Vec::new();
        let strip = ui.max_rect();
        let width = tab_width(self.tabs.len(), strip.width());
        let painter = ui.painter().clone();
        painter.rect_filled(strip, 0.0, theme::chrome());
        painter.hline(
            strip.x_range(),
            strip.bottom() - 0.5,
            Stroke::new(1.0, theme::separator()),
        );

        for (index, tab) in self.tabs.iter().enumerate() {
            let rect = Rect::from_min_size(
                pos2(strip.left() + tab_left(index, width), strip.top() + 4.0),
                vec2(width, strip.height() - 4.0),
            );
            self.show_tab(ui, rect, tab, &mut events);
        }

        let new_left = strip.left() + tab_left(self.tabs.len(), width);
        let new_rect = Rect::from_min_size(
            pos2(new_left, strip.top() + 5.0),
            vec2(NEW_BUTTON_WIDTH - 4.0, strip.height() - 8.0),
        );
        let new_tab = ui.interact(new_rect, self.id("new"), Sense::click());
        let new_tab = super::accessibility::label(new_tab, "New tab").on_hover_text("New tab");
        if new_tab.hovered() {
            painter.rect_filled(new_rect, 6.0, theme::ink(18));
        }
        icons::paint(
            &painter,
            Rect::from_center_size(new_rect.center(), Vec2::splat(13.0)),
            Icon::Plus,
            theme::text_secondary(),
        );
        if new_tab.clicked() {
            events.push(TabBarEvent::New);
        }

        if let Some(index) = self.drop_index {
            let x = strip.left() + tab_left(index, width) - TAB_GAP / 2.0 - 1.0;
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(x, strip.top() + 6.0),
                    vec2(2.0, strip.height() - 10.0),
                ),
                1.0,
                theme::text_primary(),
            );
        }

        TabBarOutput { events, strip }
    }

    fn id(&self, name: &str) -> Id {
        Id::new(("window-tab-bar", self.window, name))
    }

    fn show_tab(&self, ui: &mut Ui, rect: Rect, tab: &TabView, events: &mut Vec<TabBarEvent>) {
        let active = tab.id == self.active;
        let dragging = self.dragging == Some(tab.id);
        let close_rect = Rect::from_center_size(
            pos2(rect.right() - 6.0 - CLOSE_SIZE / 2.0, rect.center().y + 1.0),
            Vec2::splat(CLOSE_SIZE),
        );

        let response = ui.interact(rect, self.id("tab").with(tab.id), Sense::click_and_drag());
        let close = ui.interact(close_rect, self.id("close").with(tab.id), Sense::click());
        let response = super::accessibility::label_as(
            response,
            egui::WidgetType::SelectableLabel,
            tab.title.clone(),
        );
        let close = super::accessibility::label(close, format!("Close {}", tab.title));

        let painter = ui.painter();
        let fill = if active || dragging {
            theme::background()
        } else if response.hovered() {
            theme::ink(14)
        } else {
            Color32::TRANSPARENT
        };
        let corners = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(rect, corners, fill);
        if active || dragging {
            painter.rect_stroke(
                rect,
                corners,
                Stroke::new(1.0, theme::separator()),
                StrokeKind::Inside,
            );
            // Joins the tab to the content below by hiding the strip's bottom rule under it.
            painter.hline(
                rect.x_range().shrink(1.0),
                rect.bottom() - 0.5,
                Stroke::new(1.5, theme::background()),
            );
        }

        let color = if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let icon_rect = Rect::from_center_size(
            pos2(rect.left() + 12.0 + ICON_SIZE / 2.0, rect.center().y + 1.0),
            Vec2::splat(ICON_SIZE),
        );
        icons::paint(painter, icon_rect, tab.icon, color);

        let text_left = icon_rect.right() + 7.0;
        let text_clip = Rect::from_min_max(
            pos2(text_left, rect.top()),
            pos2(close_rect.left() - 2.0, rect.bottom()),
        );
        painter.with_clip_rect(text_clip).text(
            pos2(text_left, rect.center().y + 1.0),
            Align2::LEFT_CENTER,
            &tab.title,
            FontId::proportional(12.0),
            color,
        );

        if response.hovered() || close.hovered() || active {
            if close.hovered() {
                painter.rect_filled(close_rect, 5.0, theme::ink(26));
            }
            icons::paint(
                painter,
                Rect::from_center_size(close_rect.center(), Vec2::splat(9.0)),
                Icon::Xmark,
                color,
            );
        }

        if close.clicked() || response.middle_clicked() {
            events.push(TabBarEvent::Close(tab.id));
        } else if response.clicked() {
            events.push(TabBarEvent::Select(tab.id));
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            events.push(TabBarEvent::Select(tab.id));
            events.push(TabBarEvent::DragStarted {
                tab: tab.id,
                grab: pointer - rect.min,
            });
        }

        response.context_menu(|ui| {
            if ui.button("Close Tab").clicked() {
                events.push(TabBarEvent::Close(tab.id));
                ui.close();
            }
            if self.tabs.len() > 1 && ui.button("Move Tab to New Window").clicked() {
                events.push(TabBarEvent::MoveToNewWindow(tab.id));
                ui.close();
            }
            if !self.other_windows.is_empty() {
                ui.menu_button("Move Tab to Window", |ui| {
                    for window in self.other_windows {
                        if ui.button(&window.title).clicked() {
                            events.push(TabBarEvent::MoveToWindow {
                                tab: tab.id,
                                window: window.id,
                            });
                            ui.close();
                        }
                    }
                });
            }
        });
    }
}

/// Where the pointer is relative to the strip, for deciding between reordering and tearing off.
pub fn is_outside_tear_off_zone(pointer: Pos2, strip: Rect) -> bool {
    pointer.y < strip.top() - HEIGHT
        || pointer.y > strip.bottom() + HEIGHT
        || pointer.x < strip.left() - HEIGHT
        || pointer.x > strip.right() + HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_room_between_the_limits() {
        assert_eq!(tab_width(1, 2000.0), MAX_TAB_WIDTH);
        assert_eq!(tab_width(40, 800.0), MIN_TAB_WIDTH);
        let width = tab_width(4, 800.0);
        assert!(width > MIN_TAB_WIDTH && width < MAX_TAB_WIDTH);
        // Four tabs and the + fit the strip.
        let used = tab_left(4, width) + NEW_BUTTON_WIDTH;
        assert!(used <= 800.0 + 0.01, "{used}");
    }

    #[test]
    fn slot_at_clamps_to_the_strip() {
        let width = 100.0;
        assert_eq!(slot_at(-50.0, 3, width), 0);
        assert_eq!(slot_at(SIDE_PADDING + 50.0, 3, width), 0);
        assert_eq!(slot_at(SIDE_PADDING + 150.0, 3, width), 1);
        assert_eq!(slot_at(9999.0, 3, width), 2);
        assert_eq!(slot_at(10.0, 0, width), 0);
    }

    #[test]
    fn insertion_picks_the_nearest_gap() {
        let width = 100.0;
        assert_eq!(insertion_at(-30.0, 3, width), 0);
        assert_eq!(insertion_at(SIDE_PADDING + 40.0, 3, width), 0);
        assert_eq!(insertion_at(SIDE_PADDING + 60.0, 3, width), 1);
        assert_eq!(insertion_at(9999.0, 3, width), 3);
    }

    #[test]
    fn tearing_off_needs_a_clear_exit_from_the_strip() {
        let strip = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, HEIGHT));
        assert!(!is_outside_tear_off_zone(pos2(400.0, 20.0), strip));
        assert!(!is_outside_tear_off_zone(pos2(400.0, HEIGHT + 10.0), strip));
        assert!(is_outside_tear_off_zone(pos2(400.0, HEIGHT * 2.5), strip));
        assert!(is_outside_tear_off_zone(pos2(-HEIGHT - 1.0, 10.0), strip));
    }
}
