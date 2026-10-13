//! The stateful bridge between the tab-window model and the window callbacks.
//!
//! Every OS window shows one `TabWindow`: a tab strip over the active tab. The window callbacks run
//! as deferred viewports with `'static` closures, so this is a cheap-to-clone handle on shared
//! state. Pure edits (select, reorder, move) apply here at once; anything that needs the app's
//! tab contents (new tab, close with an unsaved-script prompt) is queued as a [`TabRequest`] for
//! `DegenViewApp::process_commands`.

use crate::model::tab_windows::{TabId, TabWindow, TabWindows, WindowId};
use crate::ui::tab_bar::TabBarEvent;
use eframe::egui::{Pos2, Vec2, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// Where and how big a window first opens. Constant for the window's lifetime so repeated
/// `ViewportBuilder`s never re-position or re-size it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Option<Pos2>,
    pub size: Vec2,
}

impl Placement {
    pub const DEFAULT_SIZE: Vec2 = vec2(1440.0, 900.0);
}

/// What the app has to do for a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabRequest {
    /// The `+`: a new empty tab in this window.
    New(WindowId),
    /// Close one tab (a Script Manager with edits asks first).
    Close(TabId),
    /// The OS close button: close every tab of the window.
    CloseWindow(WindowId),
}

#[derive(Default)]
struct State {
    windows: TabWindows,
    placement: HashMap<WindowId, Placement>,
    /// Windows by most recent focus, newest first.
    focus: Vec<WindowId>,
    closed: Vec<TabId>,
    requests: Vec<TabRequest>,
}

impl State {
    fn place(&mut self, window: WindowId, position: Option<Pos2>, size: Vec2) {
        self.placement
            .entry(window)
            .or_insert(Placement { position, size });
    }

    /// Drops state of windows the model no longer has.
    fn forget_gone_windows(&mut self) {
        let windows = &self.windows;
        self.placement.retain(|id, _| windows.window(*id).is_some());
        self.focus.retain(|id| windows.window(*id).is_some());
    }

    fn size_of(&self, window: WindowId) -> Vec2 {
        self.placement
            .get(&window)
            .map_or(Placement::DEFAULT_SIZE, |placement| placement.size)
    }

    fn focused(&self) -> Option<WindowId> {
        self.focus
            .iter()
            .copied()
            .find(|id| self.windows.window(*id).is_some())
            .or_else(|| self.windows.windows().first().map(|window| window.id))
    }
}

#[derive(Clone, Default)]
pub struct TabWindowHost {
    state: Arc<Mutex<State>>,
}

impl TabWindowHost {
    /// Every tab in one window, in this order: how a launch restores the session.
    pub fn restore(tabs: impl IntoIterator<Item = TabId>) -> Self {
        let host = Self::default();
        {
            let mut state = host.lock();
            state.windows = TabWindows::from_tabs(tabs);
            let ids: Vec<WindowId> = state.windows.windows().iter().map(|w| w.id).collect();
            for id in ids {
                state.place(id, None, Placement::DEFAULT_SIZE);
            }
        }
        host
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn snapshot(&self) -> Vec<(TabWindow, Placement)> {
        let state = self.lock();
        state
            .windows
            .windows()
            .iter()
            .map(|window| {
                let placement = state
                    .placement
                    .get(&window.id)
                    .copied()
                    .unwrap_or(Placement {
                        position: None,
                        size: Placement::DEFAULT_SIZE,
                    });
                (window.clone(), placement)
            })
            .collect()
    }

    pub fn window(&self, id: WindowId) -> Option<TabWindow> {
        self.lock().windows.window(id).cloned()
    }

    pub fn window_of(&self, tab: TabId) -> Option<WindowId> {
        self.lock().windows.window_of(tab)
    }

    pub fn ordered_tabs(&self) -> Vec<TabId> {
        self.lock().windows.ordered_tabs()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().windows.is_empty()
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.lock().focused()
    }

    /// Records that `window` has keyboard focus.
    pub fn note_focused(&self, window: WindowId) {
        let mut state = self.lock();
        if state.focus.first() != Some(&window) {
            state.focus.retain(|id| *id != window);
            state.focus.insert(0, window);
        }
    }

    /// A new tab joins the window in front, else the first one, else opens a window.
    pub fn add_tab(&self, tab: TabId) -> WindowId {
        let mut state = self.lock();
        let target = state.focused();
        let window = state.windows.add_tab(target, tab);
        state.place(window, None, Placement::DEFAULT_SIZE);
        window
    }

    pub fn add_tab_to(&self, window: WindowId, tab: TabId) {
        self.lock().windows.add_tab(Some(window), tab);
    }

    /// Shows `tab` in its window and returns that window.
    pub fn select_tab(&self, tab: TabId) -> Option<WindowId> {
        let mut state = self.lock();
        if state.windows.select(tab) {
            state.windows.window_of(tab)
        } else {
            None
        }
    }

    /// Removes the tab without asking anyone; the app drops its content next frame.
    pub fn close_tab(&self, tab: TabId) {
        let mut state = self.lock();
        if state.windows.close_tab(tab).is_some() {
            state.closed.push(tab);
            state.forget_gone_windows();
        }
    }

    pub fn take_requests(&self) -> Vec<TabRequest> {
        std::mem::take(&mut self.lock().requests)
    }

    pub fn take_closed(&self) -> Vec<TabId> {
        std::mem::take(&mut self.lock().closed)
    }

    pub fn request_close_window(&self, window: WindowId) {
        self.lock().requests.push(TabRequest::CloseWindow(window));
    }

    /// The tabs of `window`, for closing them one by one.
    pub fn tabs_of(&self, window: WindowId) -> Vec<TabId> {
        self.lock()
            .windows
            .window(window)
            .map(|window| window.tabs.clone())
            .unwrap_or_default()
    }

    /// Applies what a window's strip reported.
    pub fn apply(&self, window: WindowId, events: Vec<TabBarEvent>) {
        let mut state = self.lock();
        for event in events {
            match event {
                TabBarEvent::Select(tab) => {
                    state.windows.select(tab);
                }
                TabBarEvent::Close(tab) => state.requests.push(TabRequest::Close(tab)),
                TabBarEvent::New => state.requests.push(TabRequest::New(window)),
                TabBarEvent::MoveToNewWindow(tab) => {
                    Self::detach(&mut state, tab, None);
                }
                TabBarEvent::MoveToWindow { tab, window } => {
                    let end = state.windows.window(window).map_or(0, |w| w.tabs.len());
                    state.windows.move_tab(tab, window, end);
                    state.forget_gone_windows();
                }
                TabBarEvent::DragStarted { .. } => {}
            }
        }
    }

    fn detach(state: &mut State, tab: TabId, position: Option<Pos2>) -> Option<WindowId> {
        let source = state.windows.window_of(tab)?;
        let size = state.size_of(source);
        let window = state.windows.detach(tab)?;
        state.place(window, position, size);
        Some(window)
    }

    /// The Window menu's Move Tab to New Window, for the window in front.
    pub fn detach_active(&self) {
        let mut state = self.lock();
        if let Some(window) = state.focused().and_then(|id| state.windows.window(id).cloned()) {
            Self::detach(&mut state, window.active, None);
        }
    }

    /// The Window menu's Merge All Windows, into the window in front.
    pub fn merge_all(&self) {
        let mut state = self.lock();
        if let Some(target) = state.focused() {
            state.windows.merge_all(target);
            state.forget_gone_windows();
        }
    }

    /// Show Next / Previous Tab in the window in front; returns the tab now showing.
    pub fn cycle(&self, forward: bool) -> Option<TabId> {
        let mut state = self.lock();
        let window = state.focused()?;
        state.windows.cycle(window, forward)
    }

    /// Show Next / Previous Tab in a specific window.
    pub fn cycle_window(&self, window: WindowId, forward: bool) -> Option<TabId> {
        self.lock().windows.cycle(window, forward)
    }

    /// The tab showing in the window in front.
    pub fn focused_tab(&self) -> Option<TabId> {
        let state = self.lock();
        let window = state.focused()?;
        state.windows.window(window).map(|window| window.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_opens_one_window_with_every_tab() {
        let host = TabWindowHost::restore([4, 2, 7]);
        let windows = host.snapshot();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0.tabs, vec![4, 2, 7]);
        assert_eq!(windows[0].0.active, 4);
    }

    #[test]
    fn new_tabs_join_the_window_in_front() {
        let host = TabWindowHost::restore([1, 2]);
        host.apply(
            host.focused_window().unwrap(),
            vec![TabBarEvent::MoveToNewWindow(2)],
        );
        let second = host.window_of(2).unwrap();
        host.note_focused(second);
        host.add_tab(3);
        assert_eq!(host.window_of(3), Some(second));
        assert_eq!(host.ordered_tabs(), vec![1, 2, 3]);
    }

    #[test]
    fn strip_events_edit_or_queue() {
        let host = TabWindowHost::restore([1, 2, 3]);
        let window = host.focused_window().unwrap();
        host.apply(
            window,
            vec![
                TabBarEvent::Select(3),
                TabBarEvent::Close(2),
                TabBarEvent::New,
            ],
        );
        assert_eq!(host.window(window).unwrap().active, 3);
        // Closing waits for the app (an unsaved script may object); the tab is still there.
        assert_eq!(host.ordered_tabs(), vec![1, 2, 3]);
        assert_eq!(
            host.take_requests(),
            vec![TabRequest::Close(2), TabRequest::New(window)]
        );
        assert!(host.take_requests().is_empty());
    }

    #[test]
    fn closing_a_tab_reports_it_once_and_forgets_an_emptied_window() {
        let host = TabWindowHost::restore([1, 2]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(2)]);
        let other = host.window_of(2).unwrap();
        host.note_focused(other);
        host.close_tab(2);
        assert_eq!(host.take_closed(), vec![2]);
        assert!(host.take_closed().is_empty());
        assert_eq!(host.focused_window(), Some(home));
        assert!(host.window(other).is_none());
    }

    #[test]
    fn move_to_window_appends_and_merge_all_gathers() {
        let host = TabWindowHost::restore([1, 2, 3]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(3)]);
        let other = host.window_of(3).unwrap();
        host.apply(
            home,
            vec![TabBarEvent::MoveToWindow {
                tab: 1,
                window: other,
            }],
        );
        assert_eq!(host.window(other).unwrap().tabs, vec![3, 1]);
        host.note_focused(home);
        host.merge_all();
        assert_eq!(host.snapshot().len(), 1);
        assert_eq!(host.ordered_tabs(), vec![2, 3, 1]);
    }

    #[test]
    fn cycling_and_active_tab_follow_the_window_in_front() {
        let host = TabWindowHost::restore([1, 2, 3]);
        assert_eq!(host.focused_tab(), Some(1));
        assert_eq!(host.cycle(true), Some(2));
        assert_eq!(host.focused_tab(), Some(2));
        assert_eq!(host.cycle(false), Some(1));
    }

    #[test]
    fn a_detached_window_opens_at_its_source_size() {
        let host = TabWindowHost::restore([1, 2]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(2)]);
        let detached = host.window_of(2).unwrap();
        let placement = host
            .snapshot()
            .into_iter()
            .find(|(window, _)| window.id == detached)
            .unwrap()
            .1;
        assert_eq!(placement.size, Placement::DEFAULT_SIZE);
    }
}
