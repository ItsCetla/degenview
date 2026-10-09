//! The Alerts window: every price alert by state, what has fired, and the script alerts.
//! A pure view: it reads rules, history and runtime health and returns
//! `AlertCenterAction`s; the host owns the alert store, the editor sheet and the
//! "seen" bookkeeping (`UnseenAlerts`).

// Transient: the Alerts window hosts this once the alert store is wired.
#![allow(dead_code)]

use crate::model::alert::AlertNotificationPermission;
use crate::model::alert::{
    AlertAsset, AlertCondition, AlertCurrency, AlertDeliveryState, AlertEventOrigin,
    AlertFrequency, AlertId, AlertRuntimeHealth, AlertServiceState, AlertState, AlertTriggerEvent,
    PriceAlert,
};
use crate::model::alert_feed::day_sections;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AlertTab {
    #[default]
    Active,
    Triggered,
    Paused,
    All,
    History,
    Scripts,
}

impl AlertTab {
    pub const ALL: [Self; 6] = [
        Self::Active,
        Self::Triggered,
        Self::Paused,
        Self::All,
        Self::History,
        Self::Scripts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Triggered => "Triggered",
            Self::Paused => "Paused",
            Self::All => "All",
            Self::History => "History",
            Self::Scripts => "Scripts",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Active => "🔔",
            Self::Triggered => "✔",
            Self::Paused => "⏸",
            Self::All => "☰",
            Self::History => "🕘",
            Self::Scripts => "{}",
        }
    }

    fn matches(self, alert: &PriceAlert) -> bool {
        match self {
            Self::Active => alert.state == AlertState::Active,
            Self::Triggered => alert.state == AlertState::Triggered,
            Self::Paused => alert.state == AlertState::Paused,
            Self::All => true,
            Self::History | Self::Scripts => false,
        }
    }
}

/// The market price an alert row compares against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuoteSummary {
    pub price: f64,
    pub currency: AlertCurrency,
    pub fresh: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AlertCenterAction {
    Edit(AlertId),
    /// Pause an active alert, resume a paused one, re-enable a triggered one.
    Toggle(AlertId),
    Duplicate(AlertId),
    Delete(AlertId),
    ClearHistory,
    OpenNotificationSettings,
}

pub struct AlertCenterInput<'a> {
    pub alerts: &'a [PriceAlert],
    pub history: &'a [AlertTriggerEvent],
    pub health: &'a AlertRuntimeHealth,
    /// By asset key (`AlertAsset::key`).
    pub quote: &'a dyn Fn(&str) -> Option<QuoteSummary>,
    /// `(delivered, total)` webhook results for a trigger, if it posted to any.
    pub webhook_summary: &'a dyn Fn(u64) -> Option<(usize, usize)>,
    pub script_alert_count: usize,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

#[derive(Default)]
pub struct AlertCenterState {
    pub tab: AlertTab,
    pub search: String,
    pending_delete: Option<AlertId>,
    confirming_clear: bool,
}

impl AlertCenterState {
    /// Switches to the Scripts tab and clears any search that would hide it.
    pub fn show_scripts(&mut self) {
        self.search.clear();
        self.tab = AlertTab::Scripts;
    }
}

// MARK: Pure derivations

/// The search matches the symbol, the name and the provider's own id, ignoring case.
pub fn matches_search(asset: &AlertAsset, search: &str) -> bool {
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    [
        asset.symbol.as_str(),
        asset.name.as_str(),
        asset.instrument.symbol.as_str(),
        asset.source().display_name(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(&needle))
}

/// Rules of a tab matching the search, newest-edited first.
pub fn rules_for<'a>(alerts: &'a [PriceAlert], tab: AlertTab, search: &str) -> Vec<&'a PriceAlert> {
    let mut rules: Vec<&PriceAlert> = alerts
        .iter()
        .filter(|alert| tab.matches(alert) && matches_search(&alert.asset, search))
        .collect();
    rules.sort_by_key(|alert| std::cmp::Reverse(alert.updated_at_ms));
    rules
}

pub fn history_for<'a>(
    history: &'a [AlertTriggerEvent],
    search: &str,
) -> Vec<&'a AlertTriggerEvent> {
    history
        .iter()
        .filter(|event| matches_search(&event.asset, search))
        .collect()
}

/// The count shown on a tab; `None` hides the badge.
pub fn tab_count(tab: AlertTab, input: &AlertCenterInput) -> usize {
    match tab {
        AlertTab::History => input.history.len(),
        AlertTab::Scripts => input.script_alert_count,
        _ => input
            .alerts
            .iter()
            .filter(|alert| tab.matches(alert))
            .count(),
    }
}

/// "2 active · 1 triggered", or "No price alerts yet".
pub fn summary(alerts: &[PriceAlert]) -> String {
    let count = |state| alerts.iter().filter(|alert| alert.state == state).count();
    let parts: Vec<String> = [
        (AlertState::Active, "active"),
        (AlertState::Triggered, "triggered"),
        (AlertState::Paused, "paused"),
    ]
    .into_iter()
    .filter_map(|(state, word)| {
        let n = count(state);
        (n > 0).then(|| format!("{n} {word}"))
    })
    .collect();
    if parts.is_empty() {
        "No price alerts yet".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// The newest trigger of each rule, shown on a triggered card.
pub fn last_fired_by_rule(history: &[AlertTriggerEvent]) -> HashMap<AlertId, &AlertTriggerEvent> {
    let mut latest: HashMap<AlertId, &AlertTriggerEvent> = HashMap::new();
    for event in history {
        match latest.get(&event.alert_id) {
            Some(existing) if existing.timestamp_ms >= event.timestamp_ms => {}
            _ => {
                latest.insert(event.alert_id, event);
            }
        }
    }
    latest
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Good,
    Warning,
    Neutral,
    Bad,
}

/// `(text, tone, help)` for a rule's status badge.
pub fn status(alert: &PriceAlert, quote_is_fresh: bool) -> (&'static str, Tone, &'static str) {
    match alert.state {
        AlertState::Active if !alert.armed => (
            "Re-arming",
            Tone::Warning,
            "Waiting for the price to move back across the level before it can fire again.",
        ),
        AlertState::Active if !quote_is_fresh => (
            "Waiting for data",
            Tone::Warning,
            "Waiting for current market data.",
        ),
        AlertState::Active => ("Active", Tone::Good, "Watching the market."),
        AlertState::Paused => (
            "Paused",
            Tone::Neutral,
            "Paused. It will not fire until resumed.",
        ),
        AlertState::Triggered => (
            "Triggered",
            Tone::Warning,
            "Fired. Re-enable it to watch again.",
        ),
        AlertState::Unsupported => (
            "Unsupported",
            Tone::Bad,
            "This rule type is not supported by this version.",
        ),
    }
}

pub fn toggle_title(state: AlertState) -> &'static str {
    match state {
        AlertState::Active => "Pause",
        AlertState::Triggered => "Re-enable",
        _ => "Resume",
    }
}

/// The market price, when it is fresh and already in the alert's currency.
pub fn live_price(alert: &PriceAlert, quote: Option<QuoteSummary>) -> Option<f64> {
    quote
        .filter(|quote| quote.fresh && quote.currency == alert.currency)
        .map(|quote| quote.price)
}

/// "0.8% away" for a live rule, "Fired 3 hr. ago" once it has.
pub fn secondary_text(
    alert: &PriceAlert,
    last_fired: Option<&AlertTriggerEvent>,
    live: Option<f64>,
    now_ms: i64,
) -> Option<String> {
    if alert.state == AlertState::Triggered
        && let Some(event) = last_fired
    {
        return Some(format!(
            "Fired {}",
            relative_time(now_ms, event.timestamp_ms)
        ));
    }
    if alert.state != AlertState::Active {
        return None;
    }
    let (price, target) = (live?, alert.condition.target()?);
    if price <= 0.0 {
        return None;
    }
    let away = (target - price).abs() / price * 100.0;
    Some(if away < 10.0 {
        format!("{away:.1}% away")
    } else {
        format!("{away:.0}% away")
    })
}

/// Abbreviated relative time: "now", "5 min. ago", "3 hr. ago", "2 days ago".
pub fn relative_time(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms) / 1_000;
    if secs < 0 {
        return "in the future".to_owned();
    }
    let (value, unit) = match secs {
        0..=4 => return "now".to_owned(),
        5..=59 => (secs, "sec."),
        60..=3_599 => (secs / 60, "min."),
        3_600..=86_399 => (secs / 3_600, "hr."),
        86_400..=604_799 => (
            secs / 86_400,
            if secs / 86_400 == 1 { "day" } else { "days" },
        ),
        604_800..=2_591_999 => (secs / 604_800, "wk."),
        2_592_000..=31_535_999 => (secs / 2_592_000, "mo."),
        _ => (secs / 31_536_000, "yr."),
    };
    format!("{value} {unit} ago")
}

/// "14:03" in the viewer's time zone.
pub fn clock_time(ms: i64, utc_offset_secs: i64) -> String {
    let seconds = (ms.div_euclid(1_000) + utc_offset_secs).rem_euclid(86_400);
    format!("{:02}:{:02}", seconds / 3_600, seconds % 3_600 / 60)
}

/// What an alert waits for as `(glyph, label, bullish)`: "Crosses above", "Falls 5%".
pub fn condition_chip(condition: &AlertCondition) -> (&'static str, String, Option<bool>) {
    let percent = |value: f64| {
        let text = format!("{value:.2}");
        format!("{}%", text.trim_end_matches('0').trim_end_matches('.'))
    };
    match condition {
        AlertCondition::CrossesAbove { .. } => ("↗", "Crosses above".to_owned(), Some(true)),
        AlertCondition::CrossesBelow { .. } => ("↘", "Crosses below".to_owned(), Some(false)),
        AlertCondition::RisesBy { percent: p, .. } => {
            ("↑", format!("Rises {}", percent(*p)), Some(true))
        }
        AlertCondition::FallsBy { percent: p, .. } => {
            ("↓", format!("Falls {}", percent(*p)), Some(false))
        }
        AlertCondition::Unsupported { .. } => ("?", "Unsupported".to_owned(), None),
    }
}

/// A trigger record carries no condition, so a deleted rule's direction is read off the prices.
pub fn crossed_chip(observed: f64, target: f64) -> (&'static str, String, Option<bool>) {
    if observed >= target {
        ("↗", "Crossed above".to_owned(), Some(true))
    } else {
        ("↘", "Crossed below".to_owned(), Some(false))
    }
}

/// The sentence under the history list.
pub fn history_note(count: usize) -> String {
    if count == 0 {
        "Every time an alert fires it is logged here.".to_owned()
    } else {
        format!(
            "{count} {} logged. Clearing history keeps your alerts.",
            if count == 1 { "trigger" } else { "triggers" }
        )
    }
}

pub fn runtime_badge(health: &AlertRuntimeHealth) -> (&'static str, Tone, &'static str) {
    if health.service_state == AlertServiceState::Enabled {
        (
            "Background agent active",
            Tone::Good,
            "Alerts keep being checked when DegenView is closed.",
        )
    } else {
        (
            "Foreground evaluation only",
            Tone::Warning,
            "Alerts are only checked while DegenView is open.",
        )
    }
}

pub fn empty_message(
    tab: AlertTab,
    searching: bool,
    any_in_tab: bool,
) -> (&'static str, &'static str) {
    if searching && any_in_tab {
        return ("No Results", "Nothing matches this search.");
    }
    match tab {
        AlertTab::Active => (
            "No Active Alerts",
            "Click the bell on a chart to alert on a price.",
        ),
        AlertTab::Triggered => (
            "Nothing Triggered",
            "An alert that fires once waits here until you re-enable it.",
        ),
        AlertTab::Paused => (
            "Nothing Paused",
            "Paused alerts wait here until you resume them.",
        ),
        AlertTab::All => (
            "No Alerts",
            "Click the bell on a chart to alert on a price.",
        ),
        AlertTab::History => (
            "No Trigger History",
            "Every time an alert fires it is logged here.",
        ),
        AlertTab::Scripts => (
            "No Script Alerts",
            "Alerts raised by your Pine scripts appear here.",
        ),
    }
}

// MARK: View

const CARD_HEIGHT: f32 = 56.0;
const GUTTER: f32 = 20.0;

fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Good => theme::UP,
        Tone::Warning => theme::REPLAY,
        Tone::Neutral => Color32::from_gray(150),
        Tone::Bad => theme::DOWN,
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
) -> Vec<AlertCenterAction> {
    let mut actions = Vec::new();
    header(ui, state, input, &mut actions);
    ui.separator();

    match state.tab {
        AlertTab::History => history_view(ui, state, input),
        AlertTab::Scripts => {
            let (title, detail) = empty_message(AlertTab::Scripts, false, false);
            centered(ui, title, detail);
        }
        tab => rules_view(ui, state, input, tab, &mut actions),
    }

    delete_dialog(ui.ctx(), state, input, &mut actions);
    clear_dialog(ui.ctx(), state, &mut actions);
    actions
}

fn header(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    actions: &mut Vec<AlertCenterAction>,
) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.vertical(|ui| {
            ui.label(RichText::new("Alerts").size(20.0).strong());
            ui.label(RichText::new(summary(input.alerts)).weak());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(GUTTER);
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search asset or name")
                    .desired_width(200.0),
            );
            let (text, tone, help) = runtime_badge(input.health);
            badge(ui, text, tone).on_hover_text(help);
        });
    });
    ui.add_space(8.0);

    if input.health.notification_permission == AlertNotificationPermission::Denied {
        ui.horizontal(|ui| {
            ui.add_space(GUTTER);
            egui::Frame::new()
                .fill(theme::REPLAY.gamma_multiply(0.10))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("🔕 Notifications are turned off").strong());
                        ui.label(
                            RichText::new(
                                "Alerts still fire and are logged here, but macOS will not show them.",
                            )
                            .small()
                            .weak(),
                        );
                        if ui.small_button("Open Settings").clicked() {
                            actions.push(AlertCenterAction::OpenNotificationSettings);
                        }
                    });
                });
        });
        ui.add_space(8.0);
    }

    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        for tab in AlertTab::ALL {
            let selected = state.tab == tab;
            let count = tab_count(tab, input);
            let label = if count > 0 {
                format!("{} {}  {count}", tab.glyph(), tab.title())
            } else {
                format!("{} {}", tab.glyph(), tab.title())
            };
            if ui
                .selectable_label(selected, RichText::new(label).strong())
                .clicked()
            {
                state.tab = tab;
            }
        }
    });
    ui.add_space(8.0);
}

fn badge(ui: &mut egui::Ui, text: &str, tone: Tone) -> egui::Response {
    let color = tone_color(tone);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().strong().color(color))
        })
        .response
}

fn centered(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(4.0);
        ui.label(RichText::new(detail).weak());
    });
}

fn rules_view(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    tab: AlertTab,
    actions: &mut Vec<AlertCenterAction>,
) {
    let rules = rules_for(input.alerts, tab, &state.search);
    if rules.is_empty() {
        let any_in_tab = input.alerts.iter().any(|alert| tab.matches(alert));
        let (title, detail) = empty_message(tab, !state.search.trim().is_empty(), any_in_tab);
        centered(ui, title, detail);
        return;
    }
    let fired = last_fired_by_rule(input.history);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add_space(4.0);
            for alert in rules {
                rule_row(
                    ui,
                    state,
                    input,
                    alert,
                    fired.get(&alert.id).copied(),
                    actions,
                );
            }
            ui.add_space(8.0);
        });
}

fn card_rect(ui: &mut egui::Ui) -> (Rect, egui::Response) {
    let width = ui.available_width() - GUTTER * 2.0;
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.allocate_exact_size(Vec2::new(width, CARD_HEIGHT), Sense::click())
    })
    .inner
}

fn asset_icon(ui: &egui::Ui, painter: &egui::Painter, at: Pos2, asset: &AlertAsset) {
    let rect = Rect::from_center_size(at, Vec2::splat(30.0));
    let source = asset.source();
    if let Some(url) = crate::service::icon::get_url(source, &asset.instrument.symbol) {
        egui::Image::new(url)
            .fit_to_exact_size(rect.size())
            .paint_at(ui, rect);
    } else {
        crate::service::icon::request(source, &asset.instrument.symbol);
        painter.circle_filled(rect.center(), 15.0, Color32::from_gray(70));
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            asset
                .symbol
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default(),
            FontId::proportional(13.0),
            Color32::from_gray(210),
        );
    }
    // The source's mark in the corner, so two markets of one asset read apart.
    let mark = rect.right_bottom() + Vec2::new(-2.0, -2.0);
    painter.circle(mark, 7.0, theme::SURFACE, Stroke::new(1.0, theme::BORDER));
    painter.text(
        mark,
        Align2::CENTER_CENTER,
        source.glyph(),
        FontId::proportional(8.5),
        Color32::from_gray(200),
    );
}

fn paint_card(painter: &egui::Painter, rect: Rect, hovered: bool) {
    painter.rect_filled(
        rect,
        10.0,
        Color32::from_white_alpha(if hovered { 18 } else { 9 }),
    );
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
}

fn chip(
    painter: &egui::Painter,
    right: f32,
    center_y: f32,
    chip: &(&str, String, Option<bool>),
) -> f32 {
    let color = match chip.2 {
        Some(true) => theme::UP,
        Some(false) => theme::DOWN,
        None => Color32::from_gray(150),
    };
    let text = format!("{} {}", chip.0, chip.1);
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), color);
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let rect = Rect::from_min_size(Pos2::new(right - size.x, center_y - size.y / 2.0), size);
    painter.rect_filled(rect, 999.0, color.gamma_multiply(0.16));
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, color);
    rect.left()
}

fn rule_row(
    ui: &mut egui::Ui,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    alert: &PriceAlert,
    last_fired: Option<&AlertTriggerEvent>,
    actions: &mut Vec<AlertCenterAction>,
) {
    let (rect, response) = card_rect(ui);
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    let dim = if alert.state == AlertState::Paused {
        0.7
    } else {
        1.0
    };
    paint_card(&painter, rect, hovered);
    let quote = (input.quote)(&alert.asset.key());
    let live = live_price(alert, quote);
    let text = ui.visuals().text_color().gamma_multiply(dim);
    let weak = ui.visuals().weak_text_color().gamma_multiply(dim);
    let center_y = rect.center().y;

    asset_icon(
        ui,
        &painter,
        Pos2::new(rect.left() + 28.0, center_y),
        &alert.asset,
    );

    // Right to left: actions, status, target, chip.
    let mut x = rect.right() - 12.0;
    let actions_left = x - 92.0;
    x = actions_left;

    let (status_text, tone, status_help) = status(alert, quote.is_some_and(|q| q.fresh));
    let status_color = tone_color(tone);
    let status_galley = painter.layout_no_wrap(
        status_text.to_owned(),
        FontId::proportional(11.0),
        status_color.gamma_multiply(dim),
    );
    let status_rect = Rect::from_min_size(
        Pos2::new(
            x - 136.0 + (136.0 - status_galley.size().x - 18.0),
            center_y - 10.0,
        ),
        Vec2::new(status_galley.size().x + 18.0, 20.0),
    );
    painter.rect_filled(status_rect, 999.0, status_color.gamma_multiply(0.16 * dim));
    painter.galley(
        status_rect.min + Vec2::new(9.0, 3.0),
        status_galley,
        status_color,
    );
    x -= 136.0 + 8.0;

    let target_right = x;
    let target_text = alert.condition.target().map_or_else(
        || "—".to_owned(),
        |target| alert.currency.format_price(target),
    );
    let target = painter.layout_no_wrap(target_text, FontId::monospace(13.5), text);
    let secondary = secondary_text(alert, last_fired, live, input.now_ms);
    let target_y = if secondary.is_some() {
        center_y - 9.0
    } else {
        center_y
    };
    painter.galley(
        Pos2::new(
            target_right - target.size().x,
            target_y - target.size().y / 2.0,
        ),
        target,
        text,
    );
    if let Some(secondary) = secondary {
        let galley = painter.layout_no_wrap(secondary, FontId::monospace(10.5), weak);
        painter.galley(
            Pos2::new(target_right - galley.size().x, center_y + 5.0),
            galley,
            weak,
        );
    }
    x -= 132.0 + 8.0;

    let chip_left = chip(&painter, x, center_y, &condition_chip(&alert.condition));

    // Identity fills what's left.
    let name_left = rect.left() + 52.0;
    let name_width = (chip_left - name_left - 12.0).max(40.0);
    let mut title = alert.asset.symbol.clone();
    title.push_str(if alert.frequency == AlertFrequency::Once {
        "  ①"
    } else {
        "  ↻"
    });
    if !alert.webhook_endpoint_ids.is_empty() {
        title.push_str("  ⇪");
    }
    let name = fitted(
        &painter,
        &title,
        FontId::proportional(14.0),
        text,
        name_width,
    );
    let detail_text = if alert.note.is_empty() {
        format!(
            "{} · {}",
            alert.asset.name,
            alert.asset.source().display_name()
        )
    } else {
        alert.note.clone()
    };
    let detail = fitted(
        &painter,
        &detail_text,
        FontId::proportional(11.0),
        weak,
        name_width,
    );
    let total = name.size().y + 2.0 + detail.size().y;
    let top = center_y - total / 2.0;
    painter.galley(Pos2::new(name_left, top), name.clone(), text);
    painter.galley(
        Pos2::new(name_left, top + name.size().y + 2.0),
        detail,
        weak,
    );

    // Hover actions.
    let button = |ui: &mut egui::Ui, at: f32, glyph: &str, tip: &str| -> bool {
        let rect = Rect::from_center_size(Pos2::new(at, center_y), Vec2::splat(26.0));
        let response = ui.interact(
            rect,
            Id::new(("alert-action", alert.id, glyph)),
            Sense::click(),
        );
        let visible = hovered || response.hovered();
        if visible {
            if response.hovered() {
                painter.rect_filled(rect, 6.0, Color32::from_white_alpha(20));
            }
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
        }
        visible && response.on_hover_text(tip).clicked()
    };
    if button(ui, actions_left + 13.0, "✎", "Edit") {
        actions.push(AlertCenterAction::Edit(alert.id));
    }
    if alert.state != AlertState::Unsupported
        && button(
            ui,
            actions_left + 43.0,
            if alert.state == AlertState::Active {
                "⏸"
            } else {
                "▶"
            },
            toggle_title(alert.state),
        )
    {
        actions.push(AlertCenterAction::Toggle(alert.id));
    }

    if response.double_clicked() {
        actions.push(AlertCenterAction::Edit(alert.id));
    }
    response.on_hover_text(status_help).context_menu(|ui| {
        if ui.button("Edit…").clicked() {
            actions.push(AlertCenterAction::Edit(alert.id));
            ui.close();
        }
        if ui
            .add_enabled(
                alert.state != AlertState::Unsupported,
                egui::Button::new(toggle_title(alert.state)),
            )
            .clicked()
        {
            actions.push(AlertCenterAction::Toggle(alert.id));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            actions.push(AlertCenterAction::Duplicate(alert.id));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Delete…").color(theme::DOWN))
            .clicked()
        {
            state.pending_delete = Some(alert.id);
            ui.close();
        }
    });
}

fn fitted(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = max_width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

fn history_view(ui: &mut egui::Ui, state: &mut AlertCenterState, input: &AlertCenterInput) {
    let events = history_for(input.history, &state.search);
    let footer_height = 44.0;
    let body_height = (ui.available_height() - footer_height).max(60.0);
    ui.allocate_ui(Vec2::new(ui.available_width(), body_height), |ui| {
        if events.is_empty() {
            let (title, detail) = empty_message(
                AlertTab::History,
                !state.search.trim().is_empty(),
                !input.history.is_empty(),
            );
            centered(ui, title, detail);
            return;
        }
        let rules: HashMap<AlertId, &PriceAlert> =
            input.alerts.iter().map(|alert| (alert.id, alert)).collect();
        let sections = day_sections(
            events,
            |event| event.timestamp_ms,
            input.now_ms,
            input.utc_offset_secs,
        );
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for section in sections {
                    ui.horizontal(|ui| {
                        ui.add_space(GUTTER);
                        ui.add_space(0.0);
                        ui.label(RichText::new(&section.title).strong());
                        ui.label(
                            RichText::new(section.items.len().to_string())
                                .small()
                                .weak(),
                        );
                    });
                    for event in section.items {
                        history_row(ui, input, event, rules.get(&event.alert_id).copied());
                    }
                }
                ui.add_space(8.0);
            });
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.label(
            RichText::new(history_note(input.history.len()))
                .small()
                .weak(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(GUTTER);
            if ui
                .add_enabled(
                    !input.history.is_empty(),
                    egui::Button::new("🗑 Clear History"),
                )
                .on_hover_text("Remove every logged trigger, not just the ones matching the search")
                .clicked()
            {
                state.confirming_clear = true;
            }
        });
    });
}

fn history_row(
    ui: &mut egui::Ui,
    input: &AlertCenterInput,
    event: &AlertTriggerEvent,
    rule: Option<&PriceAlert>,
) {
    let (rect, response) = card_rect(ui);
    let painter = ui.painter_at(rect);
    paint_card(&painter, rect, response.hovered());
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let center_y = rect.center().y;
    asset_icon(
        ui,
        &painter,
        Pos2::new(rect.left() + 28.0, center_y),
        &event.asset,
    );

    // Right to left: time, price, chip, webhook / delivery marks.
    let mut x = rect.right() - 12.0;
    let time = painter.layout_no_wrap(
        clock_time(event.timestamp_ms, input.utc_offset_secs),
        FontId::monospace(13.0),
        text,
    );
    painter.galley(Pos2::new(x - time.size().x, center_y - 15.0), time, text);
    let relative = painter.layout_no_wrap(
        relative_time(input.now_ms, event.timestamp_ms),
        FontId::proportional(10.5),
        weak,
    );
    painter.galley(
        Pos2::new(x - relative.size().x, center_y + 3.0),
        relative,
        weak,
    );
    x -= 92.0 + 8.0;

    let price = painter.layout_no_wrap(
        event.currency.format_price(event.observed_value),
        FontId::monospace(13.5),
        text,
    );
    painter.galley(Pos2::new(x - price.size().x, center_y - 15.0), price, text);
    let target = painter.layout_no_wrap(
        format!("Target {}", event.currency.format_price(event.target)),
        FontId::monospace(10.5),
        weak,
    );
    painter.galley(Pos2::new(x - target.size().x, center_y + 3.0), target, weak);
    x -= 132.0 + 8.0;

    let kind = match rule {
        Some(rule) => condition_chip(&rule.condition),
        None => crossed_chip(event.observed_value, event.target),
    };
    let mut left = chip(&painter, x, center_y, &kind);

    if let Some((delivered, total)) = (input.webhook_summary)(event.id) {
        let color = if delivered == total {
            theme::UP
        } else {
            theme::REPLAY
        };
        let galley = painter.layout_no_wrap(
            format!("Webhooks {delivered}/{total}"),
            FontId::proportional(10.5),
            color,
        );
        left -= galley.size().x + 10.0;
        painter.galley(
            Pos2::new(left, center_y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    if event.delivery.state == AlertDeliveryState::Failed {
        left -= 22.0;
        painter.text(
            Pos2::new(left + 8.0, center_y),
            Align2::CENTER_CENTER,
            "⚠",
            FontId::proportional(14.0),
            theme::DOWN,
        );
    }

    let name_left = rect.left() + 52.0;
    let name_width = (left - name_left - 12.0).max(40.0);
    let mut title = event.asset.symbol.clone();
    if event.origin == AlertEventOrigin::CatchUp {
        title.push_str("  ⏱ Delayed");
    }
    let name = fitted(
        &painter,
        &title,
        FontId::proportional(14.0),
        text,
        name_width,
    );
    let detail = fitted(
        &painter,
        &format!(
            "{} · {}",
            event.asset.name,
            event.asset.source().display_name()
        ),
        FontId::proportional(11.0),
        weak,
        name_width,
    );
    let total = name.size().y + 2.0 + detail.size().y;
    let top = center_y - total / 2.0;
    painter.galley(Pos2::new(name_left, top), name.clone(), text);
    painter.galley(
        Pos2::new(name_left, top + name.size().y + 2.0),
        detail,
        weak,
    );
    if event.origin == AlertEventOrigin::CatchUp {
        response.on_hover_text("Seen late: the price crossed while DegenView was not watching.");
    }
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut AlertCenterState,
    input: &AlertCenterInput,
    actions: &mut Vec<AlertCenterAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(alert) = input.alerts.iter().find(|alert| alert.id == id) else {
        state.pending_delete = None;
        return;
    };
    let mut done = false;
    egui::Window::new("Delete this alert?")
        .id(Id::new("alert-delete"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(format!(
                "The {} alert is removed. Its trigger history is kept.",
                alert.asset.symbol
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Delete").color(theme::DOWN))
                    .clicked()
                {
                    actions.push(AlertCenterAction::Delete(id));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.pending_delete = None;
    }
}

fn clear_dialog(
    ctx: &egui::Context,
    state: &mut AlertCenterState,
    actions: &mut Vec<AlertCenterAction>,
) {
    if !state.confirming_clear {
        return;
    }
    let mut done = false;
    egui::Window::new("Clear all trigger history?")
        .id(Id::new("alert-clear"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("This can't be undone. Your alerts are kept.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Clear History").color(theme::DOWN))
                    .clicked()
                {
                    actions.push(AlertCenterAction::ClearHistory);
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.confirming_clear = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertDeliveryRecord, AlertNotificationPermission};
    use crate::model::{DataSource, InstrumentId};

    fn asset(symbol: &str, name: &str) -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, format!("{symbol}USDT")),
            symbol,
            name,
        )
    }

    fn alert(symbol: &str, name: &str, state: AlertState, updated: i64) -> PriceAlert {
        let mut alert = PriceAlert::new(
            asset(symbol, name),
            AlertCondition::CrossesAbove { target: 100.0 },
            0,
        );
        alert.state = state;
        alert.updated_at_ms = updated;
        alert
    }

    fn event(alert: &PriceAlert, at: i64) -> AlertTriggerEvent {
        AlertTriggerEvent {
            id: at as u64,
            alert_id: alert.id,
            asset: alert.asset.clone(),
            observed_value: 101.0,
            target: 100.0,
            currency: AlertCurrency::Usd,
            timestamp_ms: at,
            quote_fingerprint: "f".into(),
            origin: AlertEventOrigin::Live,
            delivery: AlertDeliveryRecord::default(),
            candle: None,
        }
    }

    fn input<'a>(
        alerts: &'a [PriceAlert],
        history: &'a [AlertTriggerEvent],
        health: &'a AlertRuntimeHealth,
    ) -> AlertCenterInput<'a> {
        AlertCenterInput {
            alerts,
            history,
            health,
            quote: &|_| None,
            webhook_summary: &|_| None,
            script_alert_count: 3,
            now_ms: 10_000_000,
            utc_offset_secs: 0,
        }
    }

    #[test]
    fn tabs_filter_by_state_and_sort_newest_edit_first() {
        let alerts = vec![
            alert("BTC", "Bitcoin", AlertState::Active, 10),
            alert("ETH", "Ethereum", AlertState::Active, 30),
            alert("SOL", "Solana", AlertState::Paused, 20),
            alert("XRP", "Ripple", AlertState::Triggered, 5),
        ];
        let symbols = |tab| {
            rules_for(&alerts, tab, "")
                .iter()
                .map(|alert| alert.asset.symbol.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(symbols(AlertTab::Active), ["ETH", "BTC"]);
        assert_eq!(symbols(AlertTab::Paused), ["SOL"]);
        assert_eq!(symbols(AlertTab::Triggered), ["XRP"]);
        assert_eq!(symbols(AlertTab::All), ["ETH", "SOL", "BTC", "XRP"]);
        assert!(symbols(AlertTab::History).is_empty());
        assert!(symbols(AlertTab::Scripts).is_empty());
    }

    #[test]
    fn search_matches_symbol_name_provider_id_and_source() {
        let a = asset("BTC", "Bitcoin");
        for hit in ["btc", "BIT", " coin ", "btcusdt", "binance", ""] {
            assert!(matches_search(&a, hit), "{hit:?}");
        }
        for miss in ["eth", "coinbase", "xyz"] {
            assert!(!matches_search(&a, miss), "{miss:?}");
        }
    }

    #[test]
    fn counts_and_summary_follow_the_states() {
        let alerts = vec![
            alert("BTC", "Bitcoin", AlertState::Active, 0),
            alert("ETH", "Ethereum", AlertState::Active, 0),
            alert("SOL", "Solana", AlertState::Triggered, 0),
        ];
        let history = vec![event(&alerts[0], 1), event(&alerts[2], 2)];
        let health = AlertRuntimeHealth::default();
        let input = input(&alerts, &history, &health);
        assert_eq!(tab_count(AlertTab::Active, &input), 2);
        assert_eq!(tab_count(AlertTab::Triggered, &input), 1);
        assert_eq!(tab_count(AlertTab::Paused, &input), 0);
        assert_eq!(tab_count(AlertTab::All, &input), 3);
        assert_eq!(tab_count(AlertTab::History, &input), 2);
        assert_eq!(tab_count(AlertTab::Scripts, &input), 3);
        assert_eq!(summary(&alerts), "2 active · 1 triggered");
        assert_eq!(summary(&[]), "No price alerts yet");
    }

    #[test]
    fn the_newest_trigger_of_each_rule_wins() {
        let a = alert("BTC", "Bitcoin", AlertState::Triggered, 0);
        let b = alert("ETH", "Ethereum", AlertState::Triggered, 0);
        let history = vec![event(&a, 5), event(&a, 9), event(&b, 7), event(&a, 3)];
        let latest = last_fired_by_rule(&history);
        assert_eq!(latest[&a.id].timestamp_ms, 9);
        assert_eq!(latest[&b.id].timestamp_ms, 7);
    }

    #[test]
    fn status_badges_explain_why_a_rule_is_not_yet_firing() {
        let mut a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        assert_eq!(status(&a, true).0, "Active");
        assert_eq!(
            status(&a, false),
            (
                "Waiting for data",
                Tone::Warning,
                "Waiting for current market data."
            )
        );
        a.armed = false;
        assert_eq!(status(&a, true).0, "Re-arming");
        a.state = AlertState::Paused;
        assert_eq!(status(&a, true).1, Tone::Neutral);
        a.state = AlertState::Triggered;
        assert_eq!(status(&a, true).0, "Triggered");
        a.state = AlertState::Unsupported;
        assert_eq!(status(&a, true).1, Tone::Bad);
        assert_eq!(toggle_title(AlertState::Active), "Pause");
        assert_eq!(toggle_title(AlertState::Triggered), "Re-enable");
        assert_eq!(toggle_title(AlertState::Paused), "Resume");
    }

    #[test]
    fn the_live_price_is_used_only_when_fresh_and_in_the_alerts_currency() {
        let a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        let quote = QuoteSummary {
            price: 95.0,
            currency: AlertCurrency::Usd,
            fresh: true,
        };
        assert_eq!(live_price(&a, Some(quote)), Some(95.0));
        assert_eq!(
            live_price(
                &a,
                Some(QuoteSummary {
                    fresh: false,
                    ..quote
                })
            ),
            None
        );
        assert_eq!(
            live_price(
                &a,
                Some(QuoteSummary {
                    currency: AlertCurrency::Eur,
                    ..quote
                })
            ),
            None
        );
        assert_eq!(live_price(&a, None), None);
    }

    #[test]
    fn the_secondary_line_shows_distance_or_when_it_fired() {
        let active = alert("BTC", "Bitcoin", AlertState::Active, 0);
        assert_eq!(
            secondary_text(&active, None, Some(95.0), 0).as_deref(),
            Some("5.3% away")
        );
        assert_eq!(
            secondary_text(&active, None, Some(50.0), 0).as_deref(),
            Some("100% away")
        );
        assert_eq!(secondary_text(&active, None, None, 0), None);
        assert_eq!(secondary_text(&active, None, Some(0.0), 0), None);

        let mut triggered = alert("BTC", "Bitcoin", AlertState::Triggered, 0);
        let fired = event(&triggered, 1_000_000);
        assert_eq!(
            secondary_text(&triggered, Some(&fired), None, 1_000_000 + 3 * 3_600_000).as_deref(),
            Some("Fired 3 hr. ago")
        );
        triggered.state = AlertState::Paused;
        assert_eq!(
            secondary_text(&triggered, Some(&fired), Some(95.0), 0),
            None
        );
    }

    #[test]
    fn relative_times_are_abbreviated() {
        let now = 100_000_000_000;
        for (ago_secs, expected) in [
            (0, "now"),
            (4, "now"),
            (30, "30 sec. ago"),
            (120, "2 min. ago"),
            (3 * 3_600, "3 hr. ago"),
            (86_400, "1 day ago"),
            (3 * 86_400, "3 days ago"),
            (14 * 86_400, "2 wk. ago"),
            (90 * 86_400, "3 mo. ago"),
            (800 * 86_400, "2 yr. ago"),
        ] {
            assert_eq!(
                relative_time(now, now - ago_secs * 1_000),
                expected,
                "{ago_secs}"
            );
        }
        assert_eq!(relative_time(now, now + 5_000), "in the future");
    }

    #[test]
    fn clock_times_follow_the_viewers_zone() {
        // 2024-01-15 15:00 UTC.
        assert_eq!(clock_time(1_705_330_800_000, 0), "15:00");
        assert_eq!(clock_time(1_705_330_800_000, 2 * 3_600), "17:00");
        assert_eq!(clock_time(1_705_330_800_000, -8 * 3_600), "07:00");
        assert_eq!(clock_time(1_705_330_800_000 + 90_000, 0), "15:01");
    }

    #[test]
    fn condition_chips_name_the_direction() {
        assert_eq!(
            condition_chip(&AlertCondition::CrossesAbove { target: 1.0 }).1,
            "Crosses above"
        );
        assert_eq!(
            condition_chip(&AlertCondition::CrossesBelow { target: 1.0 }).2,
            Some(false)
        );
        assert_eq!(
            condition_chip(&AlertCondition::rises_by(5.0, 100.0)).1,
            "Rises 5%"
        );
        assert_eq!(
            condition_chip(&AlertCondition::falls_by(2.5, 100.0)).1,
            "Falls 2.5%"
        );
        assert_eq!(
            condition_chip(&AlertCondition::Unsupported { tag: "x".into() }).2,
            None
        );
        assert_eq!(crossed_chip(101.0, 100.0).1, "Crossed above");
        assert_eq!(crossed_chip(100.0, 100.0).1, "Crossed above");
        assert_eq!(crossed_chip(99.0, 100.0).1, "Crossed below");
    }

    #[test]
    fn notes_badges_and_empty_states_read_right() {
        assert_eq!(
            history_note(0),
            "Every time an alert fires it is logged here."
        );
        assert_eq!(
            history_note(1),
            "1 trigger logged. Clearing history keeps your alerts."
        );
        assert_eq!(
            history_note(7),
            "7 triggers logged. Clearing history keeps your alerts."
        );

        let mut health = AlertRuntimeHealth::default();
        assert_eq!(runtime_badge(&health).0, "Foreground evaluation only");
        health.service_state = AlertServiceState::Enabled;
        assert_eq!(runtime_badge(&health).1, Tone::Good);

        assert_eq!(empty_message(AlertTab::Active, true, true).0, "No Results");
        assert_eq!(
            empty_message(AlertTab::Active, true, false).0,
            "No Active Alerts"
        );
        assert_eq!(
            empty_message(AlertTab::History, false, false).0,
            "No Trigger History"
        );
        assert_eq!(
            empty_message(AlertTab::Paused, false, false).0,
            "Nothing Paused"
        );
        let denied = AlertRuntimeHealth {
            notification_permission: AlertNotificationPermission::Denied,
            ..Default::default()
        };
        assert_eq!(
            denied.notification_permission,
            AlertNotificationPermission::Denied
        );
    }

    #[test]
    fn history_search_filters_by_asset_and_keeps_order() {
        let a = alert("BTC", "Bitcoin", AlertState::Active, 0);
        let b = alert("ETH", "Ethereum", AlertState::Active, 0);
        let history = vec![event(&a, 1), event(&b, 2), event(&a, 3)];
        let found = history_for(&history, "eth");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].asset.symbol, "ETH");
        assert_eq!(history_for(&history, "").len(), 3);
    }

    #[test]
    fn showing_scripts_clears_a_search_that_would_hide_them() {
        let mut state = AlertCenterState {
            search: "btc".into(),
            ..AlertCenterState::default()
        };
        state.show_scripts();
        assert!(state.search.is_empty());
        assert_eq!(state.tab, AlertTab::Scripts);
    }
}
