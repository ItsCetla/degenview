mod app;
mod format;
mod model;
mod service;
mod ui;

use app::DegenViewApp;
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DegenView")
            .with_app_id("com.degenview.rust")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "DegenView",
        options,
        Box::new(|creation| Ok(Box::new(DegenViewApp::new(creation)))),
    )
}
