use eframe::egui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCommand {
    NewTab,
    OpenSettings,
}

#[cfg(target_os = "macos")]
mod platform {
    use super::NativeCommand;
    use eframe::egui;
    use objc2::ffi;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, ProtocolObject, Sel};
    use objc2::sel;
    use objc2_app_kit::{
        NSApplication, NSApplicationDelegate, NSMenu, NSMenuItem, NSWindow, NSWindowOrderingMode,
        NSWindowTabbingMode,
    };
    use objc2_foundation::{MainThreadMarker, NSString, ns_string};
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    const WINDOW_TITLE_PREFIX: &str = "DegenView — ";

    static COMMANDS: Mutex<Vec<NativeCommand>> = Mutex::new(Vec::new());
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static SEEN_WINDOWS: Mutex<Option<HashSet<usize>>> = Mutex::new(None);
    static EXPECTED_WINDOWS: AtomicUsize = AtomicUsize::new(0);

    extern "C-unwind" fn new_window_for_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::NewTab);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    extern "C-unwind" fn open_settings(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::OpenSettings);
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    extern "C-unwind" fn move_tab_to_new_window(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        with_key_window(|window| window.moveTabToNewWindow(None));
    }

    extern "C-unwind" fn merge_all_windows(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        with_key_window(|window| window.mergeAllWindows(None));
    }

    extern "C-unwind" fn select_next_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        with_key_window(|window| window.selectNextTab(None));
    }

    extern "C-unwind" fn select_previous_tab(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        with_key_window(|window| window.selectPreviousTab(None));
    }

    fn with_key_window(action: impl FnOnce(&NSWindow)) {
        let mtm = MainThreadMarker::new().expect("AppKit action must run on the main thread");
        if let Some(window) = NSApplication::sharedApplication(mtm).keyWindow() {
            action(&window);
        }
    }

    pub fn install(ctx: &egui::Context) {
        let _ = CONTEXT.set(ctx.clone());
        INSTALLED.get_or_init(|| {
            let mtm = MainThreadMarker::new().expect("AppKit setup must run on the main thread");
            let app = NSApplication::sharedApplication(mtm);
            let delegate = app
                .delegate()
                .expect("winit must install its application delegate before app creation");
            let delegate_protocol: &ProtocolObject<dyn NSApplicationDelegate> = &delegate;
            let delegate_object: &AnyObject = delegate_protocol.as_ref();
            install_delegate_actions(delegate_object.class());
            install_menus(&app, mtm, delegate_object);
            NSWindow::setAllowsAutomaticWindowTabbing(true, mtm);
        });
    }

    fn install_delegate_actions(delegate_class: &'static AnyClass) {
        install_action(delegate_class, sel!(newWindowForTab:), new_window_for_tab);
        install_action(delegate_class, sel!(openSettings:), open_settings);
        install_action(
            delegate_class,
            sel!(moveTabToNewWindow:),
            move_tab_to_new_window,
        );
        install_action(delegate_class, sel!(mergeAllWindows:), merge_all_windows);
        install_action(delegate_class, sel!(selectNextTab:), select_next_tab);
        install_action(
            delegate_class,
            sel!(selectPreviousTab:),
            select_previous_tab,
        );
    }

    fn install_action(
        delegate_class: &'static AnyClass,
        selector: Sel,
        action: extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>),
    ) {
        let implementation = unsafe {
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, Option<&AnyObject>),
                objc2::runtime::Imp,
            >(action)
        };
        let added = unsafe {
            ffi::class_addMethod(
                delegate_class as *const AnyClass as *mut AnyClass,
                selector,
                implementation,
                c"v@:@".as_ptr(),
            )
        };
        assert!(added.as_bool(), "delegate already implements {selector}");
    }

    pub fn take_commands() -> Vec<NativeCommand> {
        COMMANDS
            .lock()
            .map(|mut commands| std::mem::take(&mut *commands))
            .unwrap_or_default()
    }

    pub fn expect_new_windows(count: usize) {
        EXPECTED_WINDOWS.fetch_add(count, Ordering::Release);
    }

    pub fn reconcile_tabs() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let windows: Vec<Retained<NSWindow>> = app
            .windows()
            .iter()
            .filter(|window| window.title().to_string().starts_with(WINDOW_TITLE_PREFIX))
            .collect();
        if windows.is_empty() {
            return;
        }

        let active_windows: HashSet<usize> =
            windows.iter().map(|window| window_id(window)).collect();
        let mut seen_guard = match SEEN_WINDOWS.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let seen = seen_guard.get_or_insert_with(HashSet::new);
        seen.retain(|identity| active_windows.contains(identity));

        let group_identifier = ns_string!("com.degenview.rust.workspace");
        for window in &windows {
            window.setTabbingIdentifier(group_identifier);
            window.setTabbingMode(NSWindowTabbingMode::Preferred);
        }

        let mut anchor = windows
            .iter()
            .find(|window| seen.contains(&window_id(window)))
            .cloned()
            .or_else(|| {
                app.keyWindow()
                    .filter(|window| window.title().to_string().starts_with(WINDOW_TITLE_PREFIX))
            })
            .unwrap_or_else(|| windows[0].clone());

        let new_windows: Vec<_> = windows
            .iter()
            .filter(|window| !seen.contains(&window_id(window)))
            .cloned()
            .collect();

        for window in &new_windows {
            let expected = EXPECTED_WINDOWS
                .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    count.checked_sub(1)
                })
                .is_ok();
            if expected && !std::ptr::eq(&*anchor, &**window) && !same_tab_group(&anchor, window) {
                anchor.addTabbedWindow_ordered(window, NSWindowOrderingMode::Above);
            }
            seen.insert(window_id(window));
            anchor = anchor
                .tabGroup()
                .and_then(|group| group.selectedWindow())
                .unwrap_or_else(|| anchor.clone());
        }

        for window in new_windows.iter().chain(std::iter::once(&anchor)) {
            let tab_bar_visible = window
                .tabGroup()
                .is_some_and(|group| group.isTabBarVisible());
            if !tab_bar_visible {
                window.toggleTabBar(None);
            }
        }
    }

    fn same_tab_group(lhs: &NSWindow, rhs: &NSWindow) -> bool {
        lhs.tabGroup().is_some_and(|group| {
            group
                .windows()
                .iter()
                .any(|window| std::ptr::eq(&*window, rhs))
        })
    }

    fn window_id(window: &NSWindow) -> usize {
        std::ptr::from_ref(window).addr()
    }

    fn install_menus(app: &NSApplication, mtm: MainThreadMarker, delegate: &AnyObject) {
        let menubar = app.mainMenu().unwrap_or_else(|| NSMenu::new(mtm));

        // App menu: standard macOS placement for "Settings…" (⌘,).
        let app_item = NSMenuItem::new(mtm);
        let app_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("DegenView"));
        let settings = menu_item(
            mtm,
            ns_string!("Settings…"),
            Some(sel!(openSettings:)),
            ns_string!(","),
        );
        unsafe { settings.setTarget(Some(delegate)) };
        app_menu.addItem(&settings);
        app_item.setSubmenu(Some(&app_menu));
        menubar.addItem(&app_item);

        let file_item = NSMenuItem::new(mtm);
        let file_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("File"));
        let new_tab = menu_item(
            mtm,
            ns_string!("New Tab"),
            Some(sel!(newWindowForTab:)),
            ns_string!("t"),
        );
        unsafe { new_tab.setTarget(Some(delegate)) };
        file_menu.addItem(&new_tab);
        file_item.setSubmenu(Some(&file_menu));
        menubar.addItem(&file_item);

        let window_item = NSMenuItem::new(mtm);
        let window_menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("Window"));
        window_menu.addItem(&menu_item(
            mtm,
            ns_string!("Minimize"),
            Some(sel!(performMiniaturize:)),
            ns_string!("m"),
        ));
        window_menu.addItem(&NSMenuItem::separatorItem(mtm));
        let move_tab = menu_item(
            mtm,
            ns_string!("Move Tab to New Window"),
            Some(sel!(moveTabToNewWindow:)),
            ns_string!(""),
        );
        unsafe { move_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&move_tab);
        let merge_all = menu_item(
            mtm,
            ns_string!("Merge All Windows"),
            Some(sel!(mergeAllWindows:)),
            ns_string!(""),
        );
        unsafe { merge_all.setTarget(Some(delegate)) };
        window_menu.addItem(&merge_all);
        let next_tab = menu_item(
            mtm,
            ns_string!("Show Next Tab"),
            Some(sel!(selectNextTab:)),
            ns_string!(""),
        );
        unsafe { next_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&next_tab);
        let previous_tab = menu_item(
            mtm,
            ns_string!("Show Previous Tab"),
            Some(sel!(selectPreviousTab:)),
            ns_string!(""),
        );
        unsafe { previous_tab.setTarget(Some(delegate)) };
        window_menu.addItem(&previous_tab);
        window_item.setSubmenu(Some(&window_menu));
        menubar.addItem(&window_item);
        app.setWindowsMenu(Some(&window_menu));
        app.setMainMenu(Some(&menubar));
    }

    fn menu_item(
        mtm: MainThreadMarker,
        title: &NSString,
        selector: Option<Sel>,
        key: &NSString,
    ) -> Retained<NSMenuItem> {
        unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), title, selector, key) }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::NativeCommand;
    use eframe::egui;

    pub fn install(_ctx: &egui::Context) {}

    pub fn take_commands() -> Vec<NativeCommand> {
        Vec::new()
    }

    pub fn expect_new_windows(_count: usize) {}

    pub fn reconcile_tabs() {}
}

pub fn install(ctx: &egui::Context) {
    platform::install(ctx);
}

pub fn take_commands() -> Vec<NativeCommand> {
    platform::take_commands()
}

pub fn expect_new_windows(count: usize) {
    platform::expect_new_windows(count);
}

pub fn reconcile_tabs() {
    platform::reconcile_tabs();
}
