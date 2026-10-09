//! The app's Settings window: a sidebar of badge-and-title rows beside the selected page.
//! Only the pages that exist in the Rust app are listed — Notifications (how alerts are
//! delivered, and the background agent's health) and Webhooks. Appearance, Alpaca,
//! CoinMarketCap and About slot in as `SettingsSection` variants when they are ported.
//!
//! A pure view: it reads settings values and returns `SettingsAction`s for the host.

// Transient: the app shell opens this window once the settings host is wired.
#![allow(dead_code)]

use crate::model::alert::{
    AlertNotificationPermission, AlertNotificationSettings, AlertRuntimeHealth, AlertServiceState,
};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::theme;
use crate::ui::webhook_settings::{
    self, WebhookSettingsAction, WebhookSettingsInput, WebhookSettingsState,
};
use eframe::egui::{self, Color32, RichText, Sense, Vec2};

pub const SIDEBAR_WIDTH: f32 = 200.0;
/// The window the Swift app gives Settings.
pub const WINDOW_SIZE: Vec2 = Vec2::new(800.0, 580.0);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SettingsSection {
    #[default]
    Notifications,
    Webhooks,
}

impl SettingsSection {
    pub const ALL: [Self; 2] = [Self::Notifications, Self::Webhooks];

    pub fn title(self) -> &'static str {
        match self {
            Self::Notifications => "Notifications",
            Self::Webhooks => "Webhooks",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Notifications => "🔔",
            Self::Webhooks => "⇪",
        }
    }

    /// The sidebar badge colour, as in the system Settings app.
    pub fn tint(self) -> Color32 {
        match self {
            Self::Notifications => Color32::from_rgb(255, 69, 58),
            Self::Webhooks => Color32::from_rgb(48, 176, 199),
        }
    }

    /// The stable name the selection is remembered under.
    pub fn key(self) -> &'static str {
        match self {
            Self::Notifications => "notifications",
            Self::Webhooks => "webhooks",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsAction {
    Webhook(WebhookSettingsAction),
    /// A switch on the Notifications page changed: the whole new settings value.
    UpdateNotifications(AlertNotificationSettings),
    RetryRegistration,
    OpenLoginItems,
    OpenNotificationSettings,
}

pub struct SettingsInput<'a> {
    pub notifications: &'a AlertNotificationSettings,
    pub health: &'a AlertRuntimeHealth,
    pub webhooks: WebhookSettingsInput<'a>,
    pub now_ms: i64,
}

#[derive(Default)]
pub struct SettingsState {
    pub section: SettingsSection,
    pub webhooks: WebhookSettingsState,
}

// MARK: Pure labels

/// The background agent's state in words.
pub fn service_label(state: AlertServiceState) -> &'static str {
    match state {
        AlertServiceState::Enabled => "Running in background",
        AlertServiceState::RequiresApproval => "Needs approval in Login Items",
        AlertServiceState::Disabled => "Foreground only",
        AlertServiceState::NotFound => "Agent not embedded",
        AlertServiceState::Failed => "Registration failed",
        AlertServiceState::Unavailable => "Unavailable",
    }
}

pub fn service_tone(state: AlertServiceState) -> Tone {
    match state {
        AlertServiceState::Enabled => Tone::Good,
        AlertServiceState::RequiresApproval => Tone::Warning,
        AlertServiceState::Failed | AlertServiceState::NotFound => Tone::Bad,
        AlertServiceState::Disabled | AlertServiceState::Unavailable => Tone::Neutral,
    }
}

pub fn permission_label(permission: AlertNotificationPermission) -> &'static str {
    match permission {
        AlertNotificationPermission::Authorized => "Authorized",
        AlertNotificationPermission::Provisional => "Provisional",
        AlertNotificationPermission::Ephemeral => "Ephemeral",
        AlertNotificationPermission::Denied => "Denied",
        AlertNotificationPermission::NotDetermined => "Not requested",
        AlertNotificationPermission::Unknown => "Unknown",
    }
}

pub fn permission_tone(permission: AlertNotificationPermission) -> Tone {
    match permission {
        AlertNotificationPermission::Authorized
        | AlertNotificationPermission::Provisional
        | AlertNotificationPermission::Ephemeral => Tone::Good,
        AlertNotificationPermission::Denied => Tone::Bad,
        AlertNotificationPermission::NotDetermined | AlertNotificationPermission::Unknown => {
            Tone::Neutral
        }
    }
}

/// A provider's line: its last error, or whether data has arrived yet.
pub fn provider_label(last_error: Option<&str>, has_quote: bool) -> String {
    match last_error {
        Some(error) => error.to_owned(),
        None if has_quote => "Healthy".to_owned(),
        None => "Waiting for data".to_owned(),
    }
}

/// "3 unreconciled data gaps".
pub fn gaps_title(count: usize) -> String {
    format!(
        "{count} unreconciled data {}",
        if count == 1 { "gap" } else { "gaps" }
    )
}

/// One switch on the Notifications page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationSwitch {
    Delivery,
    Sound,
    InAppBanners,
    MacosNotifications,
}

impl NotificationSwitch {
    pub const ALL: [Self; 4] = [
        Self::Delivery,
        Self::Sound,
        Self::InAppBanners,
        Self::MacosNotifications,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Delivery => "Price Alert Delivery",
            Self::Sound => "Sound",
            Self::InAppBanners => "In-App Banners",
            Self::MacosNotifications => "macOS Notifications",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Delivery => {
                "Evaluate armed alerts and deliver them when their conditions are met."
            }
            Self::Sound => "Play an alert sound when a notification is delivered.",
            Self::InAppBanners => "Show alerts inside DegenView while the app is open.",
            Self::MacosNotifications => {
                "Send alerts through Notification Center, including in the background."
            }
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Delivery => "🔔",
            Self::Sound => "🔊",
            Self::InAppBanners => "▭",
            Self::MacosNotifications => "⌘",
        }
    }

    pub fn get(self, settings: &AlertNotificationSettings) -> bool {
        match self {
            Self::Delivery => settings.delivery_enabled,
            Self::Sound => settings.sound_enabled,
            Self::InAppBanners => settings.in_app_banners_enabled,
            Self::MacosNotifications => settings.macos_notifications_enabled,
        }
    }

    /// The settings with this switch set; everything else is untouched.
    pub fn set(
        self,
        settings: &AlertNotificationSettings,
        value: bool,
    ) -> AlertNotificationSettings {
        let mut updated = settings.clone();
        match self {
            Self::Delivery => updated.delivery_enabled = value,
            Self::Sound => updated.sound_enabled = value,
            Self::InAppBanners => updated.in_app_banners_enabled = value,
            Self::MacosNotifications => updated.macos_notifications_enabled = value,
        }
        updated
    }
}

// MARK: View

fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Good => theme::UP,
        Tone::Warning => theme::REPLAY,
        Tone::Neutral => Color32::from_gray(150),
        Tone::Bad => theme::DOWN,
    }
}

fn status_badge(ui: &mut egui::Ui, text: &str, tone: Tone) {
    let color = tone_color(tone);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.label(RichText::new(text).small().strong().color(color));
            });
        });
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    input: &SettingsInput,
) -> Vec<SettingsAction> {
    let mut actions = Vec::new();
    ui.horizontal_top(|ui| {
        ui.allocate_ui(Vec2::new(SIDEBAR_WIDTH, ui.available_height()), |ui| {
            sidebar(ui, state);
        });
        ui.separator();
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 24.0);
                            match state.section {
                                SettingsSection::Notifications => {
                                    notifications_page(ui, input, &mut actions);
                                }
                                SettingsSection::Webhooks => {
                                    for action in webhook_settings::show(
                                        ui,
                                        &mut state.webhooks,
                                        &input.webhooks,
                                    ) {
                                        actions.push(SettingsAction::Webhook(action));
                                    }
                                }
                            }
                            ui.add_space(24.0);
                        });
                    });
                });
        });
    });
    actions
}

fn sidebar(ui: &mut egui::Ui, state: &mut SettingsState) {
    ui.add_space(10.0);
    for section in SettingsSection::ALL {
        let selected = state.section == section;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width() - 8.0, 32.0), Sense::click());
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, 8.0, theme::ACCENT.gamma_multiply(0.28));
        } else if response.hovered() {
            painter.rect_filled(rect, 8.0, Color32::from_white_alpha(14));
        }
        let badge = egui::Rect::from_center_size(
            egui::Pos2::new(rect.left() + 22.0, rect.center().y),
            Vec2::splat(24.0),
        );
        painter.rect_filled(badge, 6.0, section.tint());
        painter.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            section.glyph(),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
        painter.text(
            egui::Pos2::new(rect.left() + 44.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            section.title(),
            egui::FontId::proportional(13.5),
            ui.visuals().text_color(),
        );
        if response.clicked() {
            state.section = section;
        }
    }
}

fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).heading());
    ui.label(RichText::new(subtitle).weak());
    ui.add_space(12.0);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn notifications_page(ui: &mut egui::Ui, input: &SettingsInput, actions: &mut Vec<SettingsAction>) {
    page_header(
        ui,
        "Notifications",
        "Choose how DegenView delivers price alerts and check on background delivery.",
    );

    ui.label(RichText::new("Delivery").strong().size(15.0));
    ui.add_space(4.0);
    for switch in NotificationSwitch::ALL {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(switch.glyph())
                        .size(16.0)
                        .color(theme::ACCENT),
                );
                ui.vertical(|ui| {
                    ui.label(RichText::new(switch.title()).strong());
                    ui.label(RichText::new(switch.hint()).small().weak());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut value = switch.get(input.notifications);
                    if ui.checkbox(&mut value, "").changed() {
                        actions.push(SettingsAction::UpdateNotifications(
                            switch.set(input.notifications, value),
                        ));
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    ui.add_space(10.0);
    ui.label(RichText::new("Background agent").strong().size(15.0));
    ui.label(
        RichText::new("Keeps alerts evaluating while DegenView is closed.")
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    let health = input.health;
    card(ui, |ui| {
        row(ui, "Status", |ui| {
            status_badge(
                ui,
                service_label(health.service_state),
                service_tone(health.service_state),
            );
        });
        ui.separator();
        row(ui, "Notification permission", |ui| {
            status_badge(
                ui,
                permission_label(health.notification_permission),
                permission_tone(health.notification_permission),
            );
        });
        if let Some(heartbeat) = health.heartbeat_ms {
            ui.separator();
            row(ui, "Last heartbeat", |ui| {
                ui.label(RichText::new(relative_time(input.now_ms, heartbeat)).weak());
            });
        }
        for provider in &health.providers {
            ui.separator();
            row(ui, provider.source.display_name(), |ui| {
                status_badge(
                    ui,
                    &provider_label(
                        provider.last_error.as_deref(),
                        provider.last_successful_quote_ms.is_some(),
                    ),
                    if provider.last_error.is_none() {
                        Tone::Good
                    } else {
                        Tone::Bad
                    },
                );
            });
        }
    });
    if !health.unreconciled_gaps.is_empty() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("⚠ {}", gaps_title(health.unreconciled_gaps.len())))
                .strong()
                .color(theme::REPLAY),
        );
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Retry Registration").clicked() {
            actions.push(SettingsAction::RetryRegistration);
        }
        if ui.button("Login Items Settings").clicked() {
            actions.push(SettingsAction::OpenLoginItems);
        }
        if ui.button("Notification Settings").clicked() {
            actions.push(SettingsAction::OpenNotificationSettings);
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "If the background item is disabled or unavailable, alerts keep evaluating while DegenView is open.",
        )
        .small()
        .weak(),
    );
}

fn row(ui: &mut egui::Ui, title: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_remember_themselves_by_key() {
        for section in SettingsSection::ALL {
            assert_eq!(SettingsSection::from_key(section.key()), Some(section));
        }
        assert_eq!(
            SettingsSection::from_key("appearance"),
            None,
            "not ported yet"
        );
        assert_eq!(SettingsSection::default(), SettingsSection::Notifications);
        assert_eq!(SettingsSection::Webhooks.title(), "Webhooks");
        assert_ne!(
            SettingsSection::Notifications.tint(),
            SettingsSection::Webhooks.tint()
        );
    }

    #[test]
    fn the_agent_state_reads_in_words_and_tones() {
        use AlertServiceState::*;
        for (state, label, tone) in [
            (Enabled, "Running in background", Tone::Good),
            (
                RequiresApproval,
                "Needs approval in Login Items",
                Tone::Warning,
            ),
            (Disabled, "Foreground only", Tone::Neutral),
            (NotFound, "Agent not embedded", Tone::Bad),
            (Failed, "Registration failed", Tone::Bad),
            (Unavailable, "Unavailable", Tone::Neutral),
        ] {
            assert_eq!(service_label(state), label);
            assert_eq!(service_tone(state), tone);
        }
    }

    #[test]
    fn notification_permission_reads_in_words_and_tones() {
        use AlertNotificationPermission::*;
        for (permission, label, tone) in [
            (Authorized, "Authorized", Tone::Good),
            (Provisional, "Provisional", Tone::Good),
            (Ephemeral, "Ephemeral", Tone::Good),
            (Denied, "Denied", Tone::Bad),
            (NotDetermined, "Not requested", Tone::Neutral),
            (Unknown, "Unknown", Tone::Neutral),
        ] {
            assert_eq!(permission_label(permission), label);
            assert_eq!(permission_tone(permission), tone);
        }
    }

    #[test]
    fn provider_lines_say_why_or_that_all_is_well() {
        assert_eq!(provider_label(Some("Rate limited"), true), "Rate limited");
        assert_eq!(provider_label(None, true), "Healthy");
        assert_eq!(provider_label(None, false), "Waiting for data");
        assert_eq!(gaps_title(1), "1 unreconciled data gap");
        assert_eq!(gaps_title(3), "3 unreconciled data gaps");
    }

    #[test]
    fn each_switch_changes_only_its_own_setting() {
        let base = AlertNotificationSettings::default();
        for switch in NotificationSwitch::ALL {
            assert!(switch.get(&base), "{} starts on", switch.title());
            let off = switch.set(&base, false);
            assert!(!switch.get(&off));
            let others_on = NotificationSwitch::ALL
                .iter()
                .filter(|other| **other != switch)
                .all(|other| other.get(&off));
            assert!(
                others_on,
                "turning {} off touched another switch",
                switch.title()
            );
            assert_eq!(switch.set(&off, true), base);
        }
        assert!(NotificationSwitch::Delivery.hint().contains("armed alerts"));
        assert_eq!(
            NotificationSwitch::MacosNotifications.title(),
            "macOS Notifications"
        );
    }
}
