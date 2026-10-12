use eframe::egui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCommand {
    NewTab,
    OpenSettings,
    SaveCurrentLayout,
}

/// The glyph a tab shows before its title, as the Swift app's `WindowTabIcon`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabIcon {
    Charts,
    Portfolio,
    ScriptManager,
}

impl TabIcon {
    /// The SF Symbol name.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Charts => "chart.xyaxis.line",
            Self::Portfolio => "briefcase",
            Self::ScriptManager => "curlybraces",
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{NativeCommand, TabIcon};
    use crate::ui::theme;
    use eframe::egui;
    use objc2::ffi;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, ProtocolObject, Sel};
    use objc2::sel;
    use objc2::{class, msg_send};
    use objc2_app_kit::{
        NSAccessibility, NSApplication, NSApplicationDelegate, NSMenu, NSMenuItem, NSWindow,
        NSWindowOrderingMode, NSWindowTabbingMode,
    };
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSPoint, NSRange, NSRect, NSSize, NSString, ns_string,
    };
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    /// Opaque `CGColorRef` pointee, so `-[NSColor CGColor]` and `-[CALayer setBackgroundColor:]`
    /// pass their type-checked encoding.
    #[repr(C)]
    struct CGColor {
        _private: [u8; 0],
    }

    // SAFETY: matches the `{CGColor=}` struct that CoreGraphics declares.
    unsafe impl objc2::encode::RefEncode for CGColor {
        const ENCODING_REF: objc2::encode::Encoding =
            objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("CGColor", &[]));
    }

    static COMMANDS: Mutex<Vec<NativeCommand>> = Mutex::new(Vec::new());
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();
    static SEEN_WINDOWS: Mutex<Option<HashSet<usize>>> = Mutex::new(None);
    static EXPECTED_WINDOWS: AtomicUsize = AtomicUsize::new(0);
    /// Tab icon by exact window title, replaced by the app every frame.
    static TAB_ICONS: Mutex<Option<HashMap<String, TabIcon>>> = Mutex::new(None);

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

    extern "C-unwind" fn save_current_layout(
        _delegate: &AnyObject,
        _selector: Sel,
        _sender: Option<&AnyObject>,
    ) {
        if let Ok(mut commands) = COMMANDS.lock() {
            commands.push(NativeCommand::SaveCurrentLayout);
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
            sel!(saveCurrentLayout:),
            save_current_layout,
        );
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

    pub fn set_tab_icons(icons: Vec<(String, TabIcon)>) {
        if let Ok(mut guard) = TAB_ICONS.lock() {
            *guard = Some(icons.into_iter().collect());
        }
    }

    /// The tab label: the SF Symbol, a small gap, then the title, in the small system font. The
    /// system tab bar offers no image slot, so the icon travels as a text attachment.
    fn tab_label(title: &str, symbol: &str) -> Option<Retained<AnyObject>> {
        const FONT_SIZE: f64 = 11.0;
        const WEIGHT_MEDIUM: f64 = 0.23;
        // SAFETY: plain AppKit/Foundation messages with the documented selectors and argument
        // types, sent on the main thread (callers hold a `MainThreadMarker`).
        unsafe {
            let name = NSString::from_str(symbol);
            let image: Retained<AnyObject> = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: &*name,
                accessibilityDescription: Option::<&NSString>::None
            ];
            let configuration: Retained<AnyObject> = msg_send![
                class!(NSImageSymbolConfiguration),
                configurationWithPointSize: FONT_SIZE,
                weight: WEIGHT_MEDIUM
            ];
            let sized: Option<Retained<AnyObject>> =
                msg_send![&*image, imageWithSymbolConfiguration: &*configuration];
            let image = sized.unwrap_or(image);
            let size: NSSize = msg_send![&*image, size];

            let attachment: Retained<AnyObject> = msg_send![class!(NSTextAttachment), new];
            let _: () = msg_send![&*attachment, setImage: &*image];
            // Sits on the text baseline; symbols are drawn slightly above it otherwise.
            let bounds = NSRect::new(NSPoint::new(0.0, -2.0), size);
            let _: () = msg_send![&*attachment, setBounds: bounds];
            let icon: Retained<AnyObject> =
                msg_send![class!(NSAttributedString), attributedStringWithAttachment: &*attachment];

            let empty = NSString::from_str("");
            let allocated: Allocated<AnyObject> =
                msg_send![class!(NSMutableAttributedString), alloc];
            let label: Retained<AnyObject> = msg_send![allocated, initWithString: &*empty];
            let _: () = msg_send![&*label, appendAttributedString: &*icon];
            let rest = NSString::from_str(&format!(" {title}"));
            let allocated: Allocated<AnyObject> = msg_send![class!(NSAttributedString), alloc];
            let text: Retained<AnyObject> = msg_send![allocated, initWithString: &*rest];
            let _: () = msg_send![&*label, appendAttributedString: &*text];

            let font: Retained<AnyObject> = msg_send![class!(NSFont), systemFontOfSize: FONT_SIZE];
            let length: usize = msg_send![&*label, length];
            let _: () = msg_send![
                &*label,
                addAttribute: &*ns_string!("NSFont"),
                value: &*font,
                range: NSRange::new(0, length)
            ];
            Some(label)
        }
    }

    /// Writes the icon into `window`'s tab label, unless it is already there (rewriting an equal
    /// label only makes the tab bar relayout).
    fn decorate_tab(window: &NSWindow, icon: TabIcon) {
        let title = window.title().to_string();
        let Some(label) = tab_label(&title, icon.symbol()) else {
            return;
        };
        // SAFETY: `tab`, `attributedTitle`, `isEqualToAttributedString:` and
        // `setAttributedTitle:` are the documented NSWindowTab / NSAttributedString messages.
        unsafe {
            let tab: Retained<AnyObject> = msg_send![window, tab];
            let current: Option<Retained<AnyObject>> = msg_send![&*tab, attributedTitle];
            let same = current.is_some_and(|current| {
                let equal: bool = msg_send![&*current, isEqualToAttributedString: &*label];
                equal
            });
            if !same {
                let _: () = msg_send![&*tab, setAttributedTitle: &*label];
            }
        }
    }

    /// A window is a tab when its title is one the app registered through `set_tab_icons`;
    /// Settings, Alerts and the hidden coordinator window never are.
    fn is_tab_window(window: &NSWindow) -> bool {
        TAB_ICONS.lock().is_ok_and(|guard| {
            guard
                .as_ref()
                .is_some_and(|icons| icons.contains_key(&window.title().to_string()))
        })
    }

    fn decorate_tabs(windows: &[Retained<NSWindow>]) {
        let Ok(guard) = TAB_ICONS.lock() else {
            return;
        };
        let Some(icons) = guard.as_ref() else {
            return;
        };
        for window in windows {
            if let Some(icon) = icons.get(&window.title().to_string()) {
                decorate_tab(window, *icon);
            }
        }
    }

    pub fn reconcile_tabs() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let windows: Vec<Retained<NSWindow>> = app
            .windows()
            .iter()
            .filter(|window| is_tab_window(window))
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
            // VoiceOver reads the accessibility title; keep it in lockstep with the window title
            // (which egui updates per tab).
            let title = window.title();
            window.setAccessibilityTitle(Some(&*title));
        }

        let mut anchor = windows
            .iter()
            .find(|window| seen.contains(&window_id(window)))
            .cloned()
            .or_else(|| app.keyWindow().filter(|window| is_tab_window(window)))
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

        // The tab bar builds its items a beat after a window joins a group and forgets a label
        // set before that, and a title change resets it, so the icons are written every pass.
        decorate_tabs(&windows);
        for window in &windows {
            tint_tab_bar(window);
        }
    }

    /// Paints the system tab bar in the app's chrome colour and gives its window the app's
    /// appearance, so the bar follows Settings ▸ Appearance instead of the Mac's.
    fn tint_tab_bar(window: &NSWindow) {
        let chrome = theme::chrome();
        let name = if theme::is_light() {
            ns_string!("NSAppearanceNameAqua")
        } else {
            ns_string!("NSAppearanceNameDarkAqua")
        };
        // SAFETY: documented NSAppearance / NSColor / NSView / CALayer messages on the main
        // thread; the views are only read and given a layer background.
        unsafe {
            let appearance: Option<Retained<AnyObject>> =
                msg_send![class!(NSAppearance), appearanceNamed: name];
            let _: () = msg_send![window, setAppearance: appearance.as_deref()];
            let color: Retained<AnyObject> = msg_send![
                class!(NSColor),
                colorWithSRGBRed: f64::from(chrome.r()) / 255.0,
                green: f64::from(chrome.g()) / 255.0,
                blue: f64::from(chrome.b()) / 255.0,
                alpha: 1.0f64
            ];
            let Some(content) = window.contentView() else {
                return;
            };
            let frame: Option<Retained<AnyObject>> = msg_send![&*content, superview];
            if let Some(frame) = frame {
                paint_tab_bar(&frame, &color);
            }
        }
    }

    unsafe fn paint_tab_bar(view: &AnyObject, color: &AnyObject) {
        // SAFETY: see `tint_tab_bar`.
        unsafe {
            let class_name = view.class().name().to_string_lossy().into_owned();
            if class_name == "NSTitlebarAccessoryContainerView" {
                let _: () = msg_send![view, setWantsLayer: true];
                let layer: Option<Retained<AnyObject>> = msg_send![view, layer];
                if let Some(layer) = layer {
                    let cg: *const CGColor = msg_send![color, CGColor];
                    let _: () = msg_send![&*layer, setBackgroundColor: cg];
                }
            }
            let subviews: Retained<NSArray<AnyObject>> = msg_send![view, subviews];
            for subview in subviews.iter() {
                paint_tab_bar(&subview, color);
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

        // winit creates the standard application menu. Add Settings there instead of appending
        // another top-level DegenView menu next to it.
        let app_menu =
            if let Some(existing) = menubar.itemAtIndex(0).and_then(|item| item.submenu()) {
                existing
            } else {
                let item = NSMenuItem::new(mtm);
                let menu = NSMenu::initWithTitle(mtm.alloc(), ns_string!("DegenView"));
                item.setSubmenu(Some(&menu));
                menubar.insertItem_atIndex(&item, 0);
                menu
            };
        remove_about_item(&app_menu);
        let settings = menu_item(
            mtm,
            ns_string!("Settings…"),
            Some(sel!(openSettings:)),
            ns_string!(","),
        );
        unsafe { settings.setTarget(Some(delegate)) };
        if let Some(existing) = app_menu.itemWithTitle(ns_string!("Settings…")) {
            unsafe {
                existing.setAction(Some(sel!(openSettings:)));
                existing.setTarget(Some(delegate));
            }
            existing.setKeyEquivalent(ns_string!(","));
        } else {
            app_menu.insertItem_atIndex(&settings, 0);
        }

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
        let save_layout = menu_item(
            mtm,
            ns_string!("Save Layout"),
            Some(sel!(saveCurrentLayout:)),
            ns_string!("s"),
        );
        unsafe { save_layout.setTarget(Some(delegate)) };
        file_menu.addItem(&save_layout);
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

    /// Drops winit's "About DegenView": the stock panel has nothing to show, About lives in
    /// Settings.
    fn remove_about_item(menu: &NSMenu) {
        let about = (0..menu.numberOfItems()).find(|index| {
            menu.itemAtIndex(*index)
                .is_some_and(|item| item.title().to_string().starts_with("About"))
        });
        if let Some(index) = about {
            menu.removeItemAtIndex(index);
        }
    }

    fn menu_item(
        mtm: MainThreadMarker,
        title: &NSString,
        selector: Option<Sel>,
        key: &NSString,
    ) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), title, selector, key)
        };
        // VoiceOver names menu items from their title; make it explicit.
        item.setAccessibilityTitle(Some(title));
        item
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

    pub fn set_tab_icons(_icons: Vec<(String, super::TabIcon)>) {}
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

/// Tells the bridge which icon each window title gets in the native tab bar.
pub fn set_tab_icons(icons: Vec<(String, TabIcon)>) {
    platform::set_tab_icons(icons);
}

#[cfg(test)]
mod tests {
    use super::TabIcon;

    #[test]
    fn each_tab_kind_has_the_swift_symbol() {
        assert_eq!(TabIcon::Charts.symbol(), "chart.xyaxis.line");
        assert_eq!(TabIcon::Portfolio.symbol(), "briefcase");
        assert_eq!(TabIcon::ScriptManager.symbol(), "curlybraces");
    }
}
