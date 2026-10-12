// Release builds on Windows are GUI-subsystem apps: no console window behind the UI.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = degenview::run_gui() {
        report_fatal(&error.to_string());
        std::process::exit(1);
    }
}

/// A GUI-subsystem build has no console, so a startup failure would vanish; show it in a dialog.
#[cfg(windows)]
fn report_fatal(message: &str) {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut (), text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    eprintln!("Error: {message}");
    let wide = |text: &str| {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let text = wide(message);
    let caption = wide("DegenView failed to start");
    // MB_OK | MB_ICONERROR
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10) };
}

#[cfg(not(windows))]
fn report_fatal(message: &str) {
    eprintln!("Error: {message}");
}
