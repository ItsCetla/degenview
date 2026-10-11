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
    let mut icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand/DegenView-512.png"))?;
    #[cfg(target_os = "macos")]
    round_macos_icon_corners(&mut icon);
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

/// eframe installs viewport icon data directly as NSApplication's Dock icon. macOS normally
/// applies its rounded app-icon mask to bundle icons, but not to that runtime override.
#[cfg(target_os = "macos")]
fn round_macos_icon_corners(icon: &mut egui::IconData) {
    let width = icon.width as usize;
    let height = icon.height as usize;
    if width == 0 || height == 0 || icon.rgba.len() != width * height * 4 {
        return;
    }

    let half_width = width as f32 / 2.0;
    let half_height = height as f32 / 2.0;
    let radius = (width.min(height) as f32 * 0.223)
        .min(half_width)
        .min(half_height);
    for y in 0..height {
        let qy = ((y as f32 + 0.5) - half_height).abs() - (half_height - radius);
        for x in 0..width {
            let qx = ((x as f32 + 0.5) - half_width).abs() - (half_width - radius);
            let outside_x = qx.max(0.0);
            let outside_y = qy.max(0.0);
            let distance = outside_x.hypot(outside_y) + qx.max(qy).min(0.0) - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            let alpha = &mut icon.rgba[(y * width + x) * 4 + 3];
            *alpha = (*alpha as f32 * coverage).round() as u8;
        }
    }
}
