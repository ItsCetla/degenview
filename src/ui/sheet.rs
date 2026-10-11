//! Shared shell for the compact macOS sheets used throughout the app.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, Ui, Vec2};

pub const INSET: i8 = 24;

pub fn show<R>(ctx: &egui::Context, id: Id, width: f32, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(
            egui::Frame::new()
                .fill(theme::chrome())
                .corner_radius(12.0)
                .stroke(Stroke::new(1.0, theme::ink(22)))
                .inner_margin(egui::Margin::ZERO)
                .shadow(egui::Shadow {
                    offset: [0, 12],
                    blur: 40,
                    spread: 0,
                    color: Color32::from_black_alpha(130),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui)
        })
        .inner
}

pub fn header(ui: &mut Ui, icon: Icon, title: &str, subtitle: &str, tint: Color32) {
    header_with(ui, title, subtitle, |ui| {
        let (badge, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        ui.painter()
            .rect_filled(badge, 12.0, theme::with_alpha(tint, 0.14));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(badge.center(), Vec2::splat(20.0)),
            icon,
            tint,
        );
    });
}

/// `SheetHeader` with any badge: `badge` paints a 46-point square, then the title and a
/// one-line description follow it.
pub fn header_with(ui: &mut Ui, title: &str, subtitle: &str, badge: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        badge(ui);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(kit::text(
                title,
                size::TITLE2 + 1.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add(
                egui::Label::new(kit::text(
                    subtitle,
                    size::SUBHEADLINE + 1.0,
                    REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
        });
    });
}

pub fn inset<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(INSET, 0))
        .show(ui, body)
        .inner
}
