//! Settings ▸ About (Swift `AboutSettingsView`): the app icon, name and version, the license and
//! a copyable build summary, the project links, and the copyright line.

use crate::format::civil_from_days;
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::time::{Duration, UNIX_EPOCH};

pub const NAME: &str = "DegenView";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LICENSE: &str = "GPL-3.0-only";
pub const REPOSITORY_URL: &str = "https://github.com/degenview/degenview-rust";
pub const DONATION_URL: &str = "https://buymeacoffee.com/degenview";
const COPYRIGHT: &str = "Copyright © 2026 Nico Oelgart. Licensed under GPL-3.0-only.";

const MAX_WIDTH: f32 = 480.0;
const ROW_HEIGHT: f32 = 44.0;
const ROW_PADDING: f32 = 14.0;
const BADGE: f32 = 26.0;
const BADGE_SPACING: f32 = 12.0;
/// How long "Copied" shows after the build summary goes to the clipboard.
const COPIED_FOR: Duration = Duration::from_millis(1_600);
const UPDATE_CHECK_DELAY: Duration = Duration::from_millis(900);
const UPDATE_CHECK_ID: &str = "settings-about-update-check";

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// When the running executable was built (its modification time), as epoch milliseconds.
fn build_time_ms() -> Option<i64> {
    let modified = std::env::current_exe()
        .ok()?
        .metadata()
        .ok()?
        .modified()
        .ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// "11 Oct 2026" in the viewer's time zone.
pub fn date_label(ms: i64) -> String {
    let local = ms + crate::service::timezone::local_utc_offset_secs(ms) * 1_000;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400_000));
    format!(
        "{day} {} {year}",
        MONTHS[(month as usize).saturating_sub(1).min(11)]
    )
}

/// The header's second line: "Version 0.1.0 - 11 Oct 2026".
pub fn header_subtitle(build_ms: Option<i64>) -> String {
    match build_ms {
        Some(ms) => format!("Version {VERSION} - {}", date_label(ms)),
        None => format!("Version {VERSION}"),
    }
}

/// One line for bug reports: "DegenView 0.1.0 · built 11 Oct 2026 at 02:14".
pub fn clipboard_summary(build_ms: Option<i64>) -> String {
    match build_ms {
        Some(ms) => {
            let local = ms + crate::service::timezone::local_utc_offset_secs(ms) * 1_000;
            let seconds = local.div_euclid(1_000).rem_euclid(86_400);
            format!(
                "{NAME} {VERSION} · built {} at {:02}:{:02}",
                date_label(ms),
                seconds / 3_600,
                seconds % 3_600 / 60
            )
        }
        None => format!("{NAME} {VERSION}"),
    }
}

pub fn show(ui: &mut Ui) {
    let build = build_time_ms();
    let area = ui.max_rect();
    egui::ScrollArea::vertical()
        .id_salt("settings-about")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = (area.width() - 56.0).min(MAX_WIDTH);
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        header(ui, build);
                        ui.add_space(20.0);
                        update_status_card(ui);
                        ui.add_space(24.0);
                        group(ui, |ui| {
                            row(
                                ui,
                                Icon::DocText,
                                Color32::from_rgb(48, 176, 199),
                                "License",
                                |ui| {
                                    ui.label(kit::text(
                                        LICENSE,
                                        kit::size::SUBHEADLINE + 1.0,
                                        kit::REGULAR,
                                        theme::text_secondary(),
                                    ));
                                },
                            );
                            divider(ui);
                            row(
                                ui,
                                Icon::InfoCircle,
                                Color32::from_rgb(94, 92, 230),
                                "Build Info",
                                |ui| {
                                    copy_button(ui, build);
                                },
                            );
                        });
                        ui.add_space(24.0);
                        group(ui, |ui| {
                            link_row(
                                ui,
                                egui::include_image!("../../assets/brand/github.svg"),
                                Color32::from_rgb(24, 23, 23),
                                "GitHub",
                                "Source code, issues and releases",
                                REPOSITORY_URL,
                            );
                            divider(ui);
                            link_row(
                                ui,
                                egui::include_image!("../../assets/brand/buymeacoffee.png"),
                                Color32::WHITE,
                                "Support DegenView",
                                "Buy me a coffee — it keeps development going",
                                DONATION_URL,
                            );
                        });
                        ui.add_space(24.0);
                        let copyright = kit::galley(
                            ui,
                            COPYRIGHT,
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_tertiary(),
                            width,
                        );
                        let (line, _) =
                            ui.allocate_exact_size(vec2(width, copyright.size().y), Sense::hover());
                        ui.painter().galley(
                            pos2(line.center().x - copyright.size().x / 2.0, line.top()),
                            copyright,
                            theme::text_tertiary(),
                        );
                    },
                );
            });
            ui.add_space(28.0);
        });
}

#[derive(Clone, Copy)]
enum UpdateStatus {
    Checking { started_at: f64 },
    UpToDate { checked_at: f64 },
}

/// The release checker is still a placeholder: show the same short checking state as Swift, then
/// report the running build as current without making a network request.
fn update_status_card(ui: &mut Ui) {
    let id = egui::Id::new(UPDATE_CHECK_ID);
    let now = ui.input(|input| input.time);
    let mut status = ui.ctx().data(|data| data.get_temp::<UpdateStatus>(id));
    if status.is_none() {
        status = Some(UpdateStatus::Checking { started_at: now });
    }
    if let Some(UpdateStatus::Checking { started_at }) = status
        && now - started_at >= UPDATE_CHECK_DELAY.as_secs_f64()
    {
        status = Some(UpdateStatus::UpToDate { checked_at: now });
    }
    let status = status.expect("update status initialized");

    if let UpdateStatus::Checking { started_at } = status {
        ui.ctx().data_mut(|data| {
            data.insert_temp(id, UpdateStatus::Checking { started_at });
        });
        ui.ctx().request_repaint_after(
            UPDATE_CHECK_DELAY.saturating_sub(Duration::from_secs_f64((now - started_at).max(0.0))),
        );
    } else {
        ui.ctx().data_mut(|data| data.insert_temp(id, status));
    }

    let (headline, detail, tint, checking) = match status {
        UpdateStatus::Checking { .. } => (
            "Checking for updates…",
            format!("You're on version {VERSION}."),
            Color32::from_rgb(142, 142, 147),
            true,
        ),
        UpdateStatus::UpToDate { checked_at } => (
            "DegenView is up to date",
            format!(
                "You're on the latest version, {VERSION}. Checked {}.",
                relative_check_time(now - checked_at)
            ),
            Color32::from_rgb(48, 176, 112),
            false,
        ),
    };

    egui::Frame::new()
        .fill(tint.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, tint.gamma_multiply(0.28)))
        .corner_radius(14.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                ui.painter().circle_filled(badge.center(), 22.0, tint);
                if checking {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(badge), |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.spinner();
                        });
                    });
                } else {
                    ui.painter().text(
                        badge.center(),
                        egui::Align2::CENTER_CENTER,
                        "✓",
                        egui::FontId::proportional(22.0),
                        Color32::WHITE,
                    );
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = ui.add_enabled(!checking, egui::Button::new("Check Now"));
                    if button.clicked() {
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(id, UpdateStatus::Checking { started_at: now });
                        });
                        ui.ctx().request_repaint_after(UPDATE_CHECK_DELAY);
                    }
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(headline).strong());
                        ui.label(egui::RichText::new(detail).color(theme::text_secondary()));
                    });
                });
            });
        });
}

fn relative_check_time(elapsed: f64) -> &'static str {
    if elapsed < 60.0 {
        "just now"
    } else if elapsed < 3_600.0 {
        "less than an hour ago"
    } else {
        "earlier"
    }
}

fn header(ui: &mut Ui, build: Option<i64>) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
    let icon = Rect::from_min_size(row.min, Vec2::splat(72.0));
    ui.painter().add(
        egui::Shadow {
            offset: [0, 3],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(56),
        }
        .as_shape(icon.shrink(8.0), 14.0),
    );
    // Rounded like a macOS app icon.
    egui::Image::new(egui::include_image!("../../assets/brand/app_icon.png"))
        .fit_to_exact_size(icon.size())
        .corner_radius(16.0)
        .paint_at(ui, icon);
    let left = icon.right() + 16.0;
    let name = kit::galley(
        ui,
        NAME,
        28.0,
        kit::BOLD,
        theme::text_primary(),
        row.right() - left,
    );
    let subtitle = kit::galley(
        ui,
        &header_subtitle(build),
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        row.right() - left,
    );
    let top = row.center().y - (name.size().y + 3.0 + subtitle.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + name_height + 3.0),
        subtitle,
        theme::text_secondary(),
    );
}

/// A rounded, inset-list card holding rows separated by dividers.
fn group(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .stroke(Stroke::new(1.0, kit::hairline()))
        .corner_radius(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            content(ui);
        });
}

/// The hairline between rows, inset to where their text begins.
fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        (rect.left() + ROW_PADDING + BADGE + BADGE_SPACING)..=rect.right(),
        rect.center().y,
        Stroke::new(1.0, kit::hairline()),
    );
}

fn badge(ui: &Ui, rect: Rect, tint: Color32) {
    ui.painter().rect_filled(rect, 7.0, tint);
    ui.painter().rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.center().y)),
        egui::CornerRadius {
            nw: 7,
            ne: 7,
            sw: 0,
            se: 0,
        },
        Color32::from_white_alpha(28),
    );
}

/// One line: a tinted badge, a title, and `trailing` at the right.
fn row(ui: &mut Ui, icon: Icon, tint: Color32, title: &str, trailing: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let tile = Rect::from_min_size(
        pos2(rect.left() + ROW_PADDING, rect.center().y - BADGE / 2.0),
        Vec2::splat(BADGE),
    );
    badge(ui, tile, tint);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(tile.center(), Vec2::splat(13.0)),
        icon,
        Color32::WHITE,
    );
    let label = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_primary(),
        200.0,
    );
    ui.painter().galley(
        pos2(
            tile.right() + BADGE_SPACING,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(ROW_PADDING, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    trailing(&mut right);
}

/// "Copy" the build summary for a bug report; it reads "Copied" for a moment after.
fn copy_button(ui: &mut Ui, build: Option<i64>) {
    let copied_at = egui::Id::new("about-copied-at");
    let now = ui.input(|input| input.time);
    let copied = ui
        .data(|data| data.get_temp::<f64>(copied_at))
        .is_some_and(|at| now - at < COPIED_FOR.as_secs_f64());
    let (icon, label, color) = if copied {
        (Icon::Check, "Copied", kit::GREEN)
    } else {
        (Icon::CopyDoc, "Copy", kit::ACCENT)
    };
    let text = kit::galley(
        ui,
        label,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        color,
        80.0,
    );
    let size = vec2(12.0 + 5.0 + text.size().x + 12.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = crate::ui::accessibility::label(response, "Copy build info")
        .on_hover_text("Copy the version and build date for a bug report")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, kit::primary(0.06));
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(rect.left() + 6.0, rect.center().y - 6.0),
            Vec2::splat(12.0),
        ),
        icon,
        color,
    );
    ui.painter().galley(
        pos2(rect.left() + 23.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    if response.clicked() {
        ui.ctx().copy_text(clipboard_summary(build));
        ui.data_mut(|data| data.insert_temp(copied_at, now));
        ui.ctx().request_repaint_after(COPIED_FOR);
    } else if copied {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// A whole-row link: the brand logo on its tile, title and hint, highlighted on hover.
fn link_row(
    ui: &mut Ui,
    logo: egui::ImageSource<'static>,
    tile: Color32,
    title: &str,
    hint: &str,
    url: &str,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 6.0), Sense::click());
    let response = crate::ui::accessibility::label(response, format!("{title}, {hint}"))
        .on_hover_text(url)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 0.0, theme::quaternary(0.7));
    }
    let badge_rect = Rect::from_min_size(
        pos2(rect.left() + ROW_PADDING, rect.center().y - BADGE / 2.0),
        Vec2::splat(BADGE),
    );
    ui.painter().rect_filled(badge_rect, 7.0, tile);
    ui.painter().rect_stroke(
        badge_rect,
        7.0,
        Stroke::new(0.5, kit::primary(0.2)),
        StrokeKind::Inside,
    );
    egui::Image::new(logo)
        .fit_to_exact_size(badge_rect.shrink(4.0).size())
        .paint_at(ui, badge_rect.shrink(4.0));
    let left = badge_rect.right() + BADGE_SPACING;
    let width = rect.right() - 40.0 - left;
    let name = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_primary(),
        width,
    );
    let detail = kit::galley(
        ui,
        hint,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = rect.center().y - (name.size().y + 1.0 + detail.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + name_height + 1.0),
        detail,
        theme::text_secondary(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - ROW_PADDING - 5.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ArrowUpRight,
        if hovered {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
    );
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_lines_read_like_swift() {
        // 2026-10-11 02:14 UTC; tests run in UTC.
        let ms = 1_791_684_840_000;
        assert_eq!(date_label(ms), "11 Oct 2026");
        assert_eq!(
            header_subtitle(Some(ms)),
            format!("Version {VERSION} - 11 Oct 2026")
        );
        assert_eq!(header_subtitle(None), format!("Version {VERSION}"));
        assert_eq!(
            clipboard_summary(Some(ms)),
            format!("DegenView {VERSION} · built 11 Oct 2026 at 02:14")
        );
    }

    #[test]
    fn the_page_draws_headlessly() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), show);
        output.textures_delta.clear();
    }
}
