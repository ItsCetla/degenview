mod app;
mod format;
mod model;
mod native_bridge;
mod pine;
mod portfolio_host;
mod service;
mod store;
mod ui;
mod workspace;

use app::DegenViewApp;
use eframe::egui;

fn main() -> eframe::Result {
    let store = store::open_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DegenView")
            .with_app_id("com.degenview.rust")
            .with_inner_size([1.0, 1.0])
            .with_position([-10_000.0, -10_000.0]),
        ..Default::default()
    };

    eframe::run_native(
        "DegenView",
        options,
        Box::new(move |creation| Ok(Box::new(DegenViewApp::new(creation, store.clone())))),
    )
}
