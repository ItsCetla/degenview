//! Create or edit one price alert: what to watch for, at what level, how often, a note
//! and the webhooks it posts to. A pure view over a `PriceAlertEditorState`; all the
//! parsing, hints and summary live on the state so they are testable and the host (which
//! owns the quote layer and the alert store) only feeds prices in and takes the alert out.

// Transient: the Alerts window and the chart-card bell host this once the alert store is wired.
#![allow(dead_code)]

use crate::model::alert::{
    AlertAsset, AlertCondition, AlertCurrency, AlertFrequency, AlertState, PriceAlert,
};
use crate::model::alert_message::{
    AlertMessageContext, DEFAULT_PRICE_ALERT_TEMPLATE, SUPPORTED_PLACEHOLDERS, render,
};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::model::webhook_policy;
use crate::ui::theme;
use eframe::egui::{self, Align2, Id, RichText, Vec2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Above,
    Below,
    Rises,
    Falls,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Above, Self::Below, Self::Rises, Self::Falls];

    pub fn title(self) -> &'static str {
        match self {
            Self::Above => "Above",
            Self::Below => "Below",
            Self::Rises => "Rises",
            Self::Falls => "Falls",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Above => "↗",
            Self::Below => "↘",
            Self::Rises => "↑",
            Self::Falls => "↓",
        }
    }

    pub fn is_percent(self) -> bool {
        matches!(self, Self::Rises | Self::Falls)
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Above => "Notify me when the price crosses above a level.",
            Self::Below => "Notify me when the price crosses below a level.",
            Self::Rises => "Notify me when the price climbs by a percentage from now.",
            Self::Falls => "Notify me when the price drops by a percentage from now.",
        }
    }
}

/// A quick target a few percent from the current price.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetOffset {
    pub percent: i32,
    pub target_text: String,
}

impl TargetOffset {
    /// "+5%" / "−5%".
    pub fn label(&self) -> String {
        format!(
            "{}{}%",
            if self.percent > 0 { "+" } else { "−" },
            self.percent.abs()
        )
    }
}

pub const PERCENT_OPTIONS: [u32; 5] = [1, 2, 5, 10, 20];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HintTone {
    Warning,
    Neutral,
}

#[derive(Clone, Debug)]
pub struct PriceAlertEditorState {
    asset: AlertAsset,
    existing: Option<PriceAlert>,
    pub direction: Direction,
    pub target: String,
    pub percent: String,
    currency: AlertCurrency,
    pub frequency: AlertFrequency,
    pub note: String,
    pub sends_to_webhooks: bool,
    pub webhook_ids: Vec<EndpointId>,
    pub webhook_message: String,
    /// The price a percentage alert counts from; fixed when the sheet opens (or kept from
    /// the saved alert).
    reference: Option<f64>,
    /// The market now, in the chosen currency; the host refreshes it while the sheet is open.
    current_price: Option<f64>,
    /// Set by the host when an identical alert already exists.
    pub identical: bool,
    price_requested: bool,
    focus_value: bool,
}

impl PriceAlertEditorState {
    /// `default_currency` is the asset's own quote currency when it is one an alert can use.
    pub fn new(
        asset: AlertAsset,
        existing: Option<PriceAlert>,
        default_currency: Option<AlertCurrency>,
    ) -> Self {
        let mut state = Self {
            asset,
            direction: Direction::Above,
            target: String::new(),
            percent: String::new(),
            currency: default_currency.unwrap_or(AlertCurrency::Usd),
            frequency: AlertFrequency::Once,
            note: String::new(),
            sends_to_webhooks: false,
            webhook_ids: Vec::new(),
            webhook_message: String::new(),
            reference: None,
            current_price: None,
            identical: false,
            price_requested: true,
            focus_value: true,
            existing: None,
        };
        if let Some(alert) = existing {
            state.currency = alert.currency;
            state.frequency = alert.frequency;
            state.note = alert.note.clone();
            state.sends_to_webhooks = !alert.webhook_endpoint_ids.is_empty();
            state.webhook_ids = alert.webhook_endpoint_ids.clone();
            state.webhook_message = alert.webhook_message.clone().unwrap_or_default();
            match &alert.condition {
                AlertCondition::CrossesAbove { target } => state.target = plain(*target),
                AlertCondition::CrossesBelow { target } => {
                    state.direction = Direction::Below;
                    state.target = plain(*target);
                }
                AlertCondition::RisesBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Rises;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::FallsBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Falls;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::Unsupported { .. } => {}
            }
            state.existing = Some(alert);
        }
        state
    }

    pub fn is_new(&self) -> bool {
        self.existing.is_none()
    }

    pub fn asset(&self) -> &AlertAsset {
        &self.asset
    }

    pub fn currency(&self) -> AlertCurrency {
        self.currency
    }

    pub fn current_price(&self) -> Option<f64> {
        self.current_price
    }

    pub fn reference(&self) -> Option<f64> {
        self.reference
    }

    /// Picks the currency; the host is asked for a fresh price in it.
    pub fn set_currency(&mut self, currency: AlertCurrency) {
        if currency != self.currency {
            self.currency = currency;
            self.current_price = None;
            self.price_requested = true;
        }
    }

    /// The currency to look a price up in, once, after the sheet opens or the currency
    /// changes.
    pub fn take_price_request(&mut self) -> Option<AlertCurrency> {
        std::mem::take(&mut self.price_requested).then_some(self.currency)
    }

    /// Feeds the market price (the host refreshes it every few seconds). A percentage alert
    /// counts from the first price it sees, and again after a currency change.
    pub fn set_current_price(&mut self, price: Option<f64>) {
        if let Some(price) = price {
            self.current_price = Some(price);
            if self.reference.is_none() {
                self.reference = Some(price);
            }
        }
    }

    /// After a currency change the old reference is in the wrong currency.
    pub fn reset_reference_to_current(&mut self) {
        self.reference = self.current_price;
    }

    // MARK: Parsing and hints

    fn parse_positive(text: &str) -> Option<f64> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    }

    pub fn parsed_target(&self) -> Option<f64> {
        Self::parse_positive(&self.target)
    }

    pub fn parsed_percent(&self) -> Option<f64> {
        Self::parse_positive(&self.percent)
    }

    /// Targets a few percent away from the current price, on the side the alert watches.
    pub fn offset_options(&self) -> Vec<TargetOffset> {
        let Some(price) = self.current_price else {
            return Vec::new();
        };
        let sign = if self.direction == Direction::Below {
            -1
        } else {
            1
        };
        [1, 2, 5, 10]
            .into_iter()
            .map(|step| {
                let percent = step * sign;
                let level = price * (1.0 + f64::from(percent) / 100.0);
                TargetOffset {
                    percent,
                    target_text: self.currency.plain_price_text(level),
                }
            })
            .collect()
    }

    /// How the typed level sits against the market, and a warning when the alert could not
    /// fire soon.
    pub fn target_hint(&self) -> Option<(String, HintTone)> {
        let level = self.parsed_target()?;
        let price = self.current_price.filter(|price| *price > 0.0)?;
        let change = (level - price) / price * 100.0;
        let signed = format!(
            "{}{:.1}%",
            if change >= 0.0 { "+" } else { "−" },
            change.abs()
        );
        if self.direction == Direction::Above && level <= price {
            return Some((
                format!(
                    "{signed} from now. The price is already above this level; it fires after dropping below and crossing back up."
                ),
                HintTone::Warning,
            ));
        }
        if self.direction == Direction::Below && level >= price {
            return Some((
                format!(
                    "{signed} from now. The price is already below this level; it fires after rising above and crossing back down."
                ),
                HintTone::Warning,
            ));
        }
        Some((
            format!("{signed} from the current price"),
            HintTone::Neutral,
        ))
    }

    /// The rule being built, or `None` while the sheet is not complete.
    pub fn condition(&self) -> Option<AlertCondition> {
        match self.direction {
            Direction::Above => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesAbove { target }),
            Direction::Below => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesBelow { target }),
            Direction::Rises => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::rises_by(percent, reference))
            }
            Direction::Falls => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::falls_by(percent, reference))
            }
        }
    }

    pub fn can_save(&self) -> bool {
        self.condition().is_some()
    }

    /// "BTC crosses above $70,000.00 · once", shown beside the Save button.
    pub fn summary(&self) -> Option<String> {
        let condition = self.condition()?;
        let ticker = &self.asset.symbol;
        let price = |value: f64| self.currency.format_price(value);
        let what = match &condition {
            AlertCondition::CrossesAbove { target } => {
                format!("{ticker} crosses above {}", price(*target))
            }
            AlertCondition::CrossesBelow { target } => {
                format!("{ticker} crosses below {}", price(*target))
            }
            AlertCondition::RisesBy {
                percent, target, ..
            } => {
                format!("{ticker} rises {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::FallsBy {
                percent, target, ..
            } => {
                format!("{ticker} falls {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::Unsupported { .. } => return None,
        };
        Some(format!(
            "{what} · {}",
            if self.frequency == AlertFrequency::Once {
                "once"
            } else {
                "every time"
            }
        ))
    }

    /// The alert as it would be saved, for the host's duplicate check.
    pub fn candidate(&self) -> Option<PriceAlert> {
        let condition = self.condition()?;
        let mut alert = PriceAlert::new(self.asset.clone(), condition, 0);
        if let Some(existing) = &self.existing {
            alert.id = existing.id;
        }
        alert.currency = self.currency;
        alert.frequency = self.frequency;
        Some(alert)
    }

    /// The saved form: the existing alert updated in place (id, creation time kept), or a
    /// new one; always active. Webhooks off means none, whatever was picked before.
    pub fn result(&self, now_ms: i64) -> Option<PriceAlert> {
        let condition = self.condition()?;
        let mut alert = self
            .existing
            .clone()
            .unwrap_or_else(|| PriceAlert::new(self.asset.clone(), condition.clone(), now_ms));
        alert.condition = condition;
        alert.currency = self.currency;
        alert.frequency = self.frequency;
        alert.note = self.note.clone();
        alert.webhook_endpoint_ids = if self.sends_to_webhooks {
            self.webhook_ids.clone()
        } else {
            Vec::new()
        };
        let message = self.webhook_message.trim();
        alert.webhook_message = (!message.is_empty()).then(|| self.webhook_message.clone());
        alert.state = AlertState::Active;
        Some(alert)
    }

    pub fn toggle_webhook(&mut self, endpoint: EndpointId, on: bool) {
        self.webhook_ids.retain(|id| *id != endpoint);
        if on {
            self.webhook_ids.push(endpoint);
        }
    }

    /// Example values for the webhook message preview: what this alert would send for its
    /// own market.
    pub fn example_context(&self, now_ms: i64) -> AlertMessageContext {
        AlertMessageContext {
            ticker: Some(self.asset.instrument.api_symbol()),
            exchange: Some(self.asset.source().display_name().to_owned()),
            interval: Some("1".to_owned()),
            close: self.current_price,
            time_ms: Some(now_ms),
            now_ms,
            ..AlertMessageContext::default()
        }
    }

    /// The message the webhook would post right now, with example values.
    pub fn webhook_preview(&self, now_ms: i64) -> String {
        let template = if self.webhook_message.trim().is_empty() {
            DEFAULT_PRICE_ALERT_TEMPLATE
        } else {
            &self.webhook_message
        };
        render(template, &self.example_context(now_ms))
    }
}

/// A plain, locale-independent number: the fields are parsed with `str::parse`.
fn plain(value: f64) -> String {
    format!("{value}")
}

// MARK: View

#[derive(Clone, Debug, PartialEq)]
pub enum PriceAlertEditorOutcome {
    Save(Box<PriceAlert>),
    Cancel,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut PriceAlertEditorState,
    endpoints: &[WebhookEndpoint],
    now_ms: i64,
) -> Option<PriceAlertEditorOutcome> {
    let mut outcome = None;
    let title = if state.is_new() {
        "Create Price Alert"
    } else {
        "Edit Price Alert"
    };
    egui::Window::new(title)
        .id(Id::new("price-alert-editor"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(480.0, 0.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} · {}",
                    state.asset.name,
                    state.asset.source().display_name()
                ))
                .small()
                .weak(),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .max_height(460.0)
                .auto_shrink([false, true])
                .show(ui, |ui| form(ui, state, endpoints, now_ms));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(
                        state
                            .summary()
                            .unwrap_or_else(|| "Set a level to create the alert.".to_owned()),
                    )
                    .small()
                    .weak(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if state.is_new() {
                        "Create Alert"
                    } else {
                        "Save Changes"
                    };
                    if ui
                        .add_enabled(state.can_save(), egui::Button::new(label))
                        .clicked()
                    {
                        outcome = state
                            .result(now_ms)
                            .map(|alert| PriceAlertEditorOutcome::Save(Box::new(alert)));
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(PriceAlertEditorOutcome::Cancel);
                    }
                });
            });
        });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(PriceAlertEditorOutcome::Cancel);
    }
    outcome
}

fn form(
    ui: &mut egui::Ui,
    state: &mut PriceAlertEditorState,
    endpoints: &[WebhookEndpoint],
    now_ms: i64,
) {
    ui.spacing_mut().item_spacing.y = 6.0;

    // Current price.
    egui::Frame::new()
        .fill(egui::Color32::from_white_alpha(10))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Current price").small().weak());
                    ui.label(
                        RichText::new(state.current_price.map_or_else(
                            || "—".to_owned(),
                            |price| state.currency.format_price(price),
                        ))
                        .size(20.0)
                        .monospace()
                        .strong(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (text, color) = if state.current_price.is_some() {
                        ("Live", theme::UP)
                    } else {
                        ("No price yet", theme::REPLAY)
                    };
                    ui.label(RichText::new(text).small().strong().color(color));
                });
            });
        });
    ui.add_space(6.0);

    ui.label(RichText::new("Alert me when the price").strong());
    ui.horizontal(|ui| {
        for direction in Direction::ALL {
            let selected = state.direction == direction;
            if ui
                .selectable_label(
                    selected,
                    format!("{} {}", direction.glyph(), direction.title()),
                )
                .clicked()
            {
                state.direction = direction;
            }
        }
    });
    ui.label(RichText::new(state.direction.blurb()).small().weak());
    ui.add_space(4.0);

    value_section(ui, state);

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("How often").weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for frequency in AlertFrequency::ALL.into_iter().rev() {
                ui.selectable_value(&mut state.frequency, frequency, frequency.title());
            }
        });
    })
    .response
    .on_hover_text(if state.frequency == AlertFrequency::Once {
        "Fires once, then waits until you turn it back on."
    } else {
        "Fires on each crossing, re-arming in between."
    });

    ui.add(
        egui::TextEdit::singleline(&mut state.note)
            .hint_text("Add a note (optional)")
            .desired_width(f32::INFINITY),
    );

    webhook_section(ui, state, endpoints, now_ms);

    if state.identical {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(theme::REPLAY.gamma_multiply(0.10))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.label(RichText::new("⚠ An identical alert already exists").strong());
                ui.label(
                    RichText::new("Saving adds a second one that fires at the same time.")
                        .small()
                        .weak(),
                );
            });
    }
}

fn value_section(ui: &mut egui::Ui, state: &mut PriceAlertEditorState) {
    if state.direction.is_percent() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("By").weak());
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.percent)
                    .hint_text("0")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(120.0),
            );
            if std::mem::take(&mut state.focus_value) {
                response.request_focus();
            }
            ui.label("%");
        });
        ui.horizontal(|ui| {
            for option in PERCENT_OPTIONS {
                let text = option.to_string();
                let selected = state.percent == text;
                if ui
                    .selectable_label(selected, RichText::new(format!("{option}%")).monospace())
                    .clicked()
                {
                    state.percent = text;
                }
            }
        });
        if state.reference.is_none() {
            ui.label(
                RichText::new("A fresh current price is required for percentage alerts.")
                    .small()
                    .color(theme::REPLAY),
            );
        } else if let Some(condition) = state.condition()
            && let Some(level) = condition.target()
        {
            ui.label(
                RichText::new(format!("Fires at {}", state.currency.format_price(level)))
                    .small()
                    .weak(),
            );
        }
        return;
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new("Target price").weak());
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.target)
                .hint_text("0.00")
                .font(egui::TextStyle::Monospace)
                .desired_width(150.0),
        );
        if std::mem::take(&mut state.focus_value) {
            response.request_focus();
        }
        let mut currency = state.currency;
        egui::ComboBox::from_id_salt("alert-currency")
            .selected_text(currency.code())
            .width(60.0)
            .show_ui(ui, |ui| {
                for option in AlertCurrency::ALL {
                    ui.selectable_value(&mut currency, option, option.code());
                }
            })
            .response
            .on_hover_text("Currency the alert is checked in");
        state.set_currency(currency);
    });
    let offsets = state.offset_options();
    if !offsets.is_empty() {
        ui.horizontal(|ui| {
            for offset in offsets {
                let selected = state.target == offset.target_text;
                if ui
                    .selectable_label(selected, RichText::new(offset.label()).monospace())
                    .clicked()
                {
                    state.target = offset.target_text;
                }
            }
        });
    }
    if let Some((text, tone)) = state.target_hint() {
        let rich = RichText::new(text).small();
        ui.label(match tone {
            HintTone::Warning => rich.color(theme::REPLAY),
            HintTone::Neutral => rich.weak(),
        });
    }
}

fn webhook_section(
    ui: &mut egui::Ui,
    state: &mut PriceAlertEditorState,
    endpoints: &[WebhookEndpoint],
    now_ms: i64,
) {
    ui.add_space(4.0);
    ui.checkbox(&mut state.sends_to_webhooks, "Also post to webhooks");
    if !state.sends_to_webhooks {
        return;
    }
    if endpoints.is_empty() {
        ui.label(
            RichText::new("No webhooks yet. Add one in Settings ▸ Webhooks.")
                .small()
                .weak(),
        );
    }
    for endpoint in endpoints {
        let mut on = state.webhook_ids.contains(&endpoint.id);
        let label = format!(
            "{}  {}{}",
            endpoint.name,
            webhook_policy::redact(&endpoint.url),
            if endpoint.is_enabled { "" } else { "  (off)" }
        );
        if ui.checkbox(&mut on, label).changed() {
            state.toggle_webhook(endpoint.id, on);
        }
    }
    ui.label(RichText::new("Message").small().weak());
    ui.add(
        egui::TextEdit::multiline(&mut state.webhook_message)
            .hint_text(DEFAULT_PRICE_ALERT_TEMPLATE)
            .font(egui::TextStyle::Monospace)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    ui.label(
        RichText::new(format!("Sends: {}", state.webhook_preview(now_ms)))
            .small()
            .monospace()
            .weak(),
    );
    ui.label(
        RichText::new(format!(
            "Placeholders: {}",
            SUPPORTED_PLACEHOLDERS
                .iter()
                .map(|name| format!("{{{{{name}}}}}"))
                .collect::<Vec<_>>()
                .join(" ")
        ))
        .small()
        .weak(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, InstrumentId};

    fn asset() -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        )
    }

    fn state() -> PriceAlertEditorState {
        PriceAlertEditorState::new(asset(), None, None)
    }

    #[test]
    fn an_empty_sheet_cannot_be_saved_and_asks_for_a_price_once() {
        let mut state = state();
        assert!(state.is_new() && !state.can_save());
        assert_eq!(state.summary(), None);
        assert_eq!(state.take_price_request(), Some(AlertCurrency::Usd));
        assert_eq!(state.take_price_request(), None, "only once");
    }

    #[test]
    fn targets_parse_as_positive_finite_numbers() {
        let mut state = state();
        for (text, expected) in [
            ("70000", Some(70_000.0)),
            ("  0.5 ", Some(0.5)),
            ("1e5", Some(100_000.0)),
            ("0", None),
            ("-3", None),
            ("abc", None),
            ("", None),
            ("NaN", None),
            ("inf", None),
        ] {
            state.target = text.into();
            assert_eq!(state.parsed_target(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_price_level_builds_a_crossing_condition_and_a_summary() {
        let mut state = state();
        state.target = "70000".into();
        assert_eq!(
            state.condition(),
            Some(AlertCondition::CrossesAbove { target: 70_000.0 })
        );
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC crosses above $70,000.00 · once")
        );
        state.direction = Direction::Below;
        state.frequency = AlertFrequency::EveryTime;
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC crosses below $70,000.00 · every time")
        );
    }

    #[test]
    fn percentage_alerts_need_a_reference_price() {
        let mut state = state();
        state.direction = Direction::Rises;
        state.percent = "10".into();
        assert!(!state.can_save(), "no price yet, nothing to count from");
        state.set_current_price(Some(100.0));
        assert_eq!(state.reference(), Some(100.0));
        let condition = state.condition().unwrap();
        assert!((condition.target().unwrap() - 110.0).abs() < 1e-9);
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC rises 10% to $110.00 · once")
        );
        // The reference is fixed when first seen; later prices do not move it.
        state.set_current_price(Some(200.0));
        assert_eq!(state.reference(), Some(100.0));
        assert_eq!(state.current_price(), Some(200.0));
        state.direction = Direction::Falls;
        state.percent = "25".into();
        assert!((state.condition().unwrap().target().unwrap() - 75.0).abs() < 1e-9);
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC falls 25% to $75.00 · once")
        );
    }

    #[test]
    fn a_saved_alert_reopens_exactly_as_it_was_made() {
        let mut saved = PriceAlert::new(asset(), AlertCondition::rises_by(5.0, 80.0), 7);
        saved.currency = AlertCurrency::Eur;
        saved.frequency = AlertFrequency::EveryTime;
        saved.note = "swing".into();
        saved.webhook_endpoint_ids = vec![3, 4];
        saved.webhook_message = Some("{{ticker}} up".into());
        let state = PriceAlertEditorState::new(asset(), Some(saved.clone()), None);
        assert!(!state.is_new());
        assert_eq!(state.direction, Direction::Rises);
        assert_eq!(state.percent, "5");
        assert_eq!(
            state.reference(),
            Some(80.0),
            "a saved percentage alert keeps its price"
        );
        assert_eq!(state.currency(), AlertCurrency::Eur);
        assert!(state.sends_to_webhooks);
        assert_eq!(state.webhook_ids, vec![3, 4]);
        let result = state.result(99).unwrap();
        assert_eq!(result.id, saved.id);
        assert_eq!(result.created_at_ms, 7, "editing keeps the creation time");
        assert_eq!(result.condition, saved.condition);
        assert_eq!(result.note, "swing");
        assert_eq!(
            result.state,
            AlertState::Active,
            "saving re-enables a triggered alert"
        );
    }

    #[test]
    fn webhooks_off_means_none_even_if_some_were_picked() {
        let mut state = state();
        state.target = "100".into();
        state.sends_to_webhooks = true;
        state.toggle_webhook(5, true);
        state.toggle_webhook(6, true);
        state.toggle_webhook(5, false);
        state.webhook_message = "  hello  ".into();
        let on = state.result(1).unwrap();
        assert_eq!(on.webhook_endpoint_ids, vec![6]);
        assert_eq!(
            on.webhook_message.as_deref(),
            Some("  hello  "),
            "kept as typed"
        );
        state.sends_to_webhooks = false;
        assert!(state.result(1).unwrap().webhook_endpoint_ids.is_empty());
        state.sends_to_webhooks = true;
        state.webhook_message = "   ".into();
        assert_eq!(state.result(1).unwrap().webhook_message, None);
    }

    #[test]
    fn quick_targets_sit_on_the_side_the_alert_watches() {
        let mut state = state();
        assert!(
            state.offset_options().is_empty(),
            "no price, no suggestions"
        );
        state.set_current_price(Some(1_000.0));
        let up = state.offset_options();
        assert_eq!(
            up.iter().map(TargetOffset::label).collect::<Vec<_>>(),
            ["+1%", "+2%", "+5%", "+10%"]
        );
        assert_eq!(up[0].target_text, "1010.00");
        assert_eq!(up[3].target_text, "1100.00");
        state.direction = Direction::Below;
        let down = state.offset_options();
        assert_eq!(down[0].label(), "−1%");
        assert_eq!(down[0].target_text, "990.00");
        // Every suggestion parses back as a usable target.
        state.target = down[2].target_text.clone();
        assert!((state.parsed_target().unwrap() - 950.0).abs() < 1e-9);
    }

    #[test]
    fn the_hint_warns_when_the_price_is_already_past_the_level() {
        let mut state = state();
        state.set_current_price(Some(100.0));
        state.target = "120".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Neutral);
        assert_eq!(text, "+20.0% from the current price");

        state.target = "90".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Warning);
        assert!(text.starts_with("−10.0% from now. The price is already above this level"));

        state.direction = Direction::Below;
        state.target = "110".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Warning);
        assert!(text.contains("already below this level"));

        state.target = "".into();
        assert_eq!(state.target_hint(), None);
        state.target = "90".into();
        state.set_current_price(None);
        state.current_price = None;
        assert_eq!(state.target_hint(), None, "no price, no hint");
    }

    #[test]
    fn changing_currency_asks_for_a_new_price_and_drops_the_old_one() {
        let mut state = state();
        state.take_price_request();
        state.set_current_price(Some(100.0));
        state.set_currency(AlertCurrency::Usd);
        assert_eq!(
            state.take_price_request(),
            None,
            "same currency, nothing to ask"
        );
        state.set_currency(AlertCurrency::Eur);
        assert_eq!(state.current_price(), None);
        assert_eq!(state.take_price_request(), Some(AlertCurrency::Eur));
        state.set_current_price(Some(92.0));
        state.reset_reference_to_current();
        assert_eq!(state.reference(), Some(92.0));
    }

    #[test]
    fn the_default_currency_is_the_assets_own_when_alertable() {
        let eur = PriceAlertEditorState::new(asset(), None, Some(AlertCurrency::Eur));
        assert_eq!(eur.currency(), AlertCurrency::Eur);
        assert_eq!(state().currency(), AlertCurrency::Usd);
    }

    #[test]
    fn the_candidate_carries_the_existing_id_for_the_duplicate_check() {
        let saved = PriceAlert::new(asset(), AlertCondition::CrossesAbove { target: 5.0 }, 0);
        let editing = PriceAlertEditorState::new(asset(), Some(saved.clone()), None);
        assert_eq!(editing.candidate().unwrap().id, saved.id);
        assert!(state().candidate().is_none());
    }

    #[test]
    fn the_webhook_preview_renders_with_this_alerts_own_market() {
        let mut state = state();
        state.set_current_price(Some(67_432.5));
        assert_eq!(
            state.webhook_preview(1_714_571_130_000),
            "BTCUSDT alert: 67432.5"
        );
        state.webhook_message = "{{exchange}} {{ticker}} {{interval}} {{time}} {{open}}".into();
        assert_eq!(
            state.webhook_preview(1_714_571_130_000),
            "Binance BTCUSDT 1 2024-05-01T13:45:30Z {{open}}",
            "a value the alert does not have stays a placeholder"
        );
    }

    #[test]
    fn directions_describe_themselves() {
        assert!(Direction::Rises.is_percent() && !Direction::Above.is_percent());
        assert_eq!(Direction::Below.title(), "Below");
        assert!(Direction::Falls.blurb().contains("drops by a percentage"));
        assert_eq!(Direction::Above.glyph(), "↗");
    }
}
