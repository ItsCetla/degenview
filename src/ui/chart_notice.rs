//! A short message over the bottom of the window: what a chart command did, or why it did not.
//! It never takes the pointer, and the workspace that raised it drops it after a few seconds.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{self, Align2, Context, Id, Margin, Order, vec2};

pub fn show(ctx: &Context, id: Id, text: &str, error: bool) {
    let (icon, color) = if error {
        (Icon::TriangleFill, kit::ORANGE)
    } else {
        (Icon::Check, theme::text_secondary())
    };
    egui::Area::new(id)
        .order(Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -32.0))
        .interactable(false)
        .show(ctx, |ui| {
            kit::menu_frame()
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icons::inline(ui, icon, 12.0, color);
                        let response =
                            ui.label(kit::text(text, size::BODY, REGULAR, theme::text_primary()));
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text)
                        });
                    });
                });
        });
}
