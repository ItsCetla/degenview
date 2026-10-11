mod alert_host;
mod app;
mod format;
mod model;
mod native_bridge;
mod paper_host;
mod pine;
mod pine_host;
mod pine_subscription_host;
mod portfolio_host;
mod portfolio_import_flow;
mod script_manager;
mod service;
mod store;
mod ui;
mod webhook_host;
mod workspace;

use app::DegenViewApp;
use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = store::open_default()?;
    // eframe applies the viewport icon to the Dock itself and, with none given, its own default
    // after startup, so the icon has to be handed over here.
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand/DegenView-512.png"))?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DegenView")
            .with_app_id("com.degenview.rust")
            .with_icon(icon)
            .with_inner_size([1.0, 1.0])
            .with_position([-10_000.0, -10_000.0]),
        ..Default::default()
    };

    Ok(eframe::run_native(
        "DegenView",
        options,
        Box::new(move |creation| Ok(Box::new(DegenViewApp::new(creation, store.clone())))),
    )?)
}
