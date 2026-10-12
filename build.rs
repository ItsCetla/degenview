fn main() {
    // Embed the app icon in the Windows executable (Explorer, taskbar, shortcuts).
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/brand/DegenView.ico");
        winresource::WindowsResource::new()
            .set_icon("assets/brand/DegenView.ico")
            .compile()
            .expect("embed Windows icon");
    }
}
