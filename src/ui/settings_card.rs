//! The building blocks of Swift's settings pages: a titled section, a `SettingsCardRow` (icon
//! badge, title and hint on the left, the control anchored right) and the macOS switch.

use super::icons::{self, Icon};
use super::theme;
use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Margin, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

/// Width kept for a card's control when its text wraps.
const CONTROL_RESERVE: f32 = 130.0;

/// A section heading: `.title3` semibold with an optional `.subheadline` secondary line.
pub fn section_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.label(super::kit::text(
        title,
        15.0,
        super::kit::SEMIBOLD,
        theme::text_primary(),
    ));
    if let Some(subtitle) = subtitle {
        ui.label(super::kit::text(
            subtitle,
            11.5,
            super::kit::REGULAR,
            theme::text_secondary(),
        ));
    }
    ui.add_space(8.0);
}

/// One setting as a card: the control sits on the right at its natural size. Returns what
/// `control` returns.
pub fn card<R>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    hint: Option<&str>,
    control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let width = ui.available_width();
    let mut result = None;
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .stroke(Stroke::new(
            1.0,
            theme::with_alpha(theme::separator(), 0.45),
        ))
        .corner_radius(12.0)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(width - 28.0 - 2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                ui.painter()
                    .rect_filled(badge, 8.0, theme::with_alpha(theme::SYSTEM_ACCENT, 0.12));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(badge.center(), Vec2::splat(15.0)),
                    icon,
                    theme::SYSTEM_ACCENT,
                );
                ui.vertical(|ui| {
                    // The hint wraps within what the control leaves: a card's width never grows.
                    ui.set_max_width(
                        (width - 30.0 - 30.0 - 14.0 * 2.0 - CONTROL_RESERVE).max(120.0),
                    );
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(super::kit::text(
                        title,
                        12.0,
                        super::kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    if let Some(hint) = hint {
                        ui.add(
                            egui::Label::new(super::kit::text(
                                hint,
                                10.5,
                                super::kit::REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    result = Some(control(ui));
                });
            });
        });
    result.expect("the control ran")
}

/// A macOS-style switch: a 36×20 capsule with a sliding knob. Returns whether it was toggled.
pub fn switch(ui: &mut Ui, on: &mut bool) -> bool {
    switch_enabled(ui, on, true)
}

pub fn switch_enabled(ui: &mut Ui, on: &mut bool, enabled: bool) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 20.0), sense);
    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
    let off = theme::pick(
        Color32::from_rgb(72, 82, 88),
        Color32::from_rgb(222, 222, 226),
    );
    let channel =
        |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * t) as u8;
    // On is the accent colour, as a macOS `.switch` toggle draws it.
    let on_color = super::kit::ACCENT;
    let mut color = Color32::from_rgb(
        channel(off.r(), on_color.r()),
        channel(off.g(), on_color.g()),
        channel(off.b(), on_color.b()),
    );
    if !enabled {
        color = color.gamma_multiply(0.45);
    }
    ui.painter().rect_filled(rect, 10.0, color);
    let knob_x = rect.min.x + 10.0 + t * 16.0;
    ui.painter()
        .circle_filled(pos2(knob_x, rect.center().y), 8.0, Color32::WHITE);
    let clicked = enabled && response.on_hover_cursor(CursorIcon::PointingHand).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_runs_its_control_and_the_switch_toggles() {
        let ctx = egui::Context::default();
        let mut on = false;
        let mut ran = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            section_title(ui, "Indicators", Some("Add context."));
            card(
                ui,
                Icon::Hash,
                "Volume",
                Some("Turnover per candle."),
                |ui| {
                    ran = true;
                    switch(ui, &mut on);
                },
            );
        });
        output.textures_delta.clear();
        assert!(ran && !on, "drawn, and not toggled without a click");
    }
}
