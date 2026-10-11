//! Create a paper account, or reset the selected one. A reset keeps the account but wipes its
//! positions, orders, fills and journal, so it opens on a red warning and starts from the
//! account's current settings rather than defaults. A pure view: it returns the chosen values and
//! the host applies them to the store.

// Transient: the paper-trading host opens this once it is wired.
#![allow(dead_code)]

use crate::model::paper::{
    PaperAccount, PaperAccountSettings, PaperCommissionConfiguration, PaperCurrency,
    PaperLeverageConfiguration,
};
use crate::model::paper_format as fmt;
use crate::model::paper_ticket::{decimal_text, parse_decimal};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, RichText};

pub const WIDTH: f32 = 460.0;
pub const LEVERAGE_PRESETS: [f64; 7] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];
pub const MAX_LEVERAGE: f64 = 1000.0;
pub const DEFAULT_NAME: &str = "Paper Trading";
pub const DEFAULT_BALANCE: f64 = 100_000.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommissionKind {
    None,
    Fixed,
    Percentage,
    PerContract,
}

impl CommissionKind {
    pub const ALL: [Self; 4] = [Self::None, Self::Fixed, Self::Percentage, Self::PerContract];

    pub fn title(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Fixed => "Fixed",
            Self::Percentage => "Percent",
            Self::PerContract => "Per contract",
        }
    }

    /// The unit beside the amount.
    pub fn unit(self, currency: PaperCurrency) -> String {
        match self {
            Self::None => String::new(),
            Self::Fixed => format!("{} per order", currency.code()),
            Self::Percentage => "% of value".to_owned(),
            Self::PerContract => format!("{} per contract", currency.code()),
        }
    }

    fn of(configuration: PaperCommissionConfiguration) -> (Self, f64) {
        match configuration {
            PaperCommissionConfiguration::None => (Self::None, 0.0),
            PaperCommissionConfiguration::FixedPerOrder { value } => (Self::Fixed, value),
            PaperCommissionConfiguration::Percentage { value } => (Self::Percentage, value),
            PaperCommissionConfiguration::PerContract { value } => (Self::PerContract, value),
        }
    }

    fn configuration(self, value: f64) -> PaperCommissionConfiguration {
        match self {
            Self::None => PaperCommissionConfiguration::None,
            Self::Fixed => PaperCommissionConfiguration::FixedPerOrder { value },
            Self::Percentage => PaperCommissionConfiguration::Percentage { value },
            Self::PerContract => PaperCommissionConfiguration::PerContract { value },
        }
    }
}

/// What the sheet hands the host on Create / Reset.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountSheetResult {
    pub name: String,
    pub currency: PaperCurrency,
    pub balance: f64,
    pub settings: PaperAccountSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AccountSheetOutcome {
    Submit(Box<AccountSheetResult>),
    Cancel,
}

pub struct PaperAccountSheetState {
    pub is_reset: bool,
    /// The settings a reset keeps (slippage, chart toggles); a new account starts from defaults.
    base_settings: PaperAccountSettings,
    pub name: String,
    pub balance_text: String,
    pub currency: PaperCurrency,
    pub leverage_text: String,
    pub commission_kind: CommissionKind,
    pub commission_text: String,
}

impl PaperAccountSheetState {
    pub fn create() -> Self {
        Self {
            is_reset: false,
            base_settings: PaperAccountSettings::default(),
            name: DEFAULT_NAME.to_owned(),
            balance_text: decimal_text(DEFAULT_BALANCE, false),
            currency: PaperCurrency::Usd,
            leverage_text: "1".to_owned(),
            commission_kind: CommissionKind::None,
            commission_text: "0".to_owned(),
        }
    }

    /// Starts from the account's current settings, not the defaults.
    pub fn reset(account: &PaperAccount) -> Self {
        let (commission_kind, commission) = CommissionKind::of(account.settings.commission);
        Self {
            is_reset: true,
            base_settings: account.settings.clone(),
            name: account.name.clone(),
            balance_text: decimal_text(account.initial_balance, false),
            currency: account.base_currency,
            leverage_text: decimal_text(account.settings.leverage.crypto, false),
            commission_kind,
            commission_text: decimal_text(commission, false),
        }
    }

    pub fn balance(&self) -> Option<f64> {
        parse_decimal(&self.balance_text, false)
    }

    pub fn leverage(&self) -> Option<f64> {
        parse_decimal(&self.leverage_text, false)
    }

    pub fn commission_value(&self) -> Option<f64> {
        parse_decimal(&self.commission_text, false)
    }

    pub fn balance_error(&self) -> Option<&'static str> {
        match self.balance() {
            None => Some("Enter a number."),
            Some(balance) if balance > 0.0 => None,
            Some(_) => Some("Must be greater than zero."),
        }
    }

    pub fn leverage_error(&self) -> Option<&'static str> {
        match self.leverage() {
            None => Some("Enter a number."),
            Some(leverage) if (1.0..=MAX_LEVERAGE).contains(&leverage) => None,
            Some(_) => Some("Must be between 1× and 1000×."),
        }
    }

    pub fn commission_error(&self) -> Option<&'static str> {
        if self.commission_kind == CommissionKind::None {
            return None;
        }
        match self.commission_value() {
            None => Some("Enter a number."),
            Some(value) if value >= 0.0 => None,
            Some(_) => Some("Can't be negative."),
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.name.trim().is_empty()
            && self.balance_error().is_none()
            && self.leverage_error().is_none()
            && self.commission_error().is_none()
    }

    /// The balance error, once something has been typed.
    pub fn visible_balance_error(&self) -> Option<&'static str> {
        (!self.balance_text.is_empty())
            .then(|| self.balance_error())
            .flatten()
    }

    pub fn visible_leverage_error(&self) -> Option<&'static str> {
        (!self.leverage_text.is_empty())
            .then(|| self.leverage_error())
            .flatten()
    }

    pub fn title(&self) -> &'static str {
        if self.is_reset {
            "Reset Paper Account"
        } else {
            "New Paper Account"
        }
    }

    pub fn subtitle(&self) -> &'static str {
        if self.is_reset {
            "Start over with a fresh balance."
        } else {
            "Practice with simulated money."
        }
    }

    pub fn submit_title(&self) -> &'static str {
        if self.is_reset {
            "Reset Account"
        } else {
            "Create Account"
        }
    }

    /// "Balance resets to $100,000.00." for a reset with a valid balance.
    pub fn reset_note(&self) -> Option<String> {
        if !self.is_reset {
            return None;
        }
        self.balance()
            .map(|balance| format!("Balance resets to {}.", fmt::money(balance, self.currency)))
    }

    /// One leverage applies to every market, as in the Swift sheet.
    pub fn result(&self) -> Option<AccountSheetResult> {
        if !self.can_submit() {
            return None;
        }
        let (balance, leverage) = (self.balance()?, self.leverage()?);
        let fee = self.commission_value().unwrap_or(0.0);
        let mut settings = self.base_settings.clone();
        settings.commission = self.commission_kind.configuration(fee);
        settings.leverage = PaperLeverageConfiguration {
            stocks: leverage,
            crypto: leverage,
            forex: leverage,
            futures: leverage,
            prediction: leverage,
        };
        Some(AccountSheetResult {
            name: self.name.trim().to_owned(),
            currency: self.currency,
            balance,
            settings,
        })
    }
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut PaperAccountSheetState,
) -> Option<AccountSheetOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("paper-account-sheet"), WIDTH, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            form(ui, state);
            ui.add_space(18.0);
            footer(ui, state, &mut outcome);
        });
        ui.add_space(24.0);
    });
    if outcome.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(AccountSheetOutcome::Cancel);
    }
    outcome
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(kit::text(
        text,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        theme::text_secondary(),
    ));
}

fn error_line(ui: &mut egui::Ui, error: Option<&str>, hint: Option<&str>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).small().color(theme::DOWN));
    } else if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().weak());
    }
}

fn amount_field(
    ui: &mut egui::Ui,
    id: &str,
    text: &mut String,
    placeholder: &str,
    suffix: &str,
    error: bool,
    width: f32,
) {
    let id = ui.make_persistent_id(id);
    let focused = ui.memory(|memory| memory.has_focus(id));
    egui::Frame::new()
        .fill(theme::quaternary(0.5))
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(
            if error || focused { 1.5 } else { 1.0 },
            if error {
                theme::DOWN
            } else if focused {
                theme::SYSTEM_ACCENT
            } else {
                theme::separator().gamma_multiply(0.6)
            },
        ))
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width((width - 20.0).max(60.0));
            ui.horizontal(|ui| {
                let trailing = suffix.chars().count() as f32 * 7.0 + 8.0;
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text(placeholder)
                        .desired_width((ui.available_width() - trailing).max(30.0)),
                );
                ui.label(kit::text(
                    suffix,
                    kit::size::SUBHEADLINE + 1.0,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
            });
        });
}

fn form(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    let tint = if state.is_reset {
        theme::DOWN
    } else {
        theme::ACCENT
    };
    sheet::header(
        ui,
        if state.is_reset {
            Icon::Cycle
        } else {
            Icon::Plus
        },
        state.title(),
        state.subtitle(),
        tint,
    );
    ui.add_space(18.0);

    if state.is_reset {
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            theme::DOWN,
            "This can't be undone",
            Some(
                "Every position, order, fill, history and journal entry in this account is removed.",
            ),
            None,
        );
        ui.add_space(14.0);
    }

    label(ui, "Account name");
    ui.add_enabled_ui(!state.is_reset, |ui| {
        let width = ui.available_width();
        amount_field(
            ui,
            "paper-account-name",
            &mut state.name,
            DEFAULT_NAME,
            "",
            false,
            width,
        );
    });
    ui.add_space(14.0);

    ui.columns(2, |columns| {
        label(&mut columns[0], "Starting balance");
        let error = state.visible_balance_error();
        let code = state.currency.code();
        let width = columns[0].available_width();
        amount_field(
            &mut columns[0],
            "paper-account-balance",
            &mut state.balance_text,
            "100000",
            code,
            error.is_some(),
            width,
        );
        error_line(&mut columns[0], error, None);
        label(&mut columns[1], "Currency");
        egui::ComboBox::from_id_salt("paper-account-currency")
            .selected_text(state.currency.code())
            .show_ui(&mut columns[1], |ui| {
                for currency in PaperCurrency::ALL {
                    ui.selectable_value(&mut state.currency, currency, currency.code());
                }
            });
    });
    ui.add_space(14.0);

    leverage_field(ui, state);
    ui.add_space(14.0);
    commission_field(ui, state);

    if let Some(note) = state.reset_note() {
        ui.add_space(10.0);
        ui.label(RichText::new(format!("↺ {note}")).small().weak());
    }
}

fn footer(
    ui: &mut egui::Ui,
    state: &PaperAccountSheetState,
    outcome: &mut Option<AccountSheetOutcome>,
) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fill = if state.is_reset {
                theme::DOWN
            } else {
                theme::ACCENT
            };
            let button = egui::Button::new(
                RichText::new(state.submit_title())
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(fill)
            .min_size(egui::Vec2::new(130.0, 30.0));
            if ui.add_enabled(state.can_submit(), button).clicked()
                && let Some(result) = state.result()
            {
                *outcome = Some(AccountSheetOutcome::Submit(Box::new(result)));
            }
            if ui.button("Cancel").clicked() {
                *outcome = Some(AccountSheetOutcome::Cancel);
            }
        });
    });
}

fn leverage_field(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    label(ui, "Leverage");
    let error = state.visible_leverage_error();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        amount_field(
            ui,
            "paper-account-leverage",
            &mut state.leverage_text,
            "1",
            "×",
            error.is_some(),
            110.0,
        );
        let current = state.leverage();
        for preset in LEVERAGE_PRESETS {
            let selected = current == Some(preset);
            if leverage_preset(ui, preset, selected).clicked() {
                state.leverage_text = decimal_text(preset, false);
            }
        }
    });
    error_line(
        ui,
        error,
        Some("Applies to every market. 1× means no borrowing."),
    );
}

fn leverage_preset(ui: &mut egui::Ui, preset: f64, selected: bool) -> egui::Response {
    let label = format!("{preset}×");
    let color = if selected {
        theme::ACCENT
    } else {
        theme::text_secondary()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, egui::FontId::proportional(11.0), color);
    let (rect, response) =
        ui.allocate_exact_size(galley.size() + egui::vec2(14.0, 10.0), egui::Sense::click());
    ui.painter().rect_filled(
        rect,
        rect.height() / 2.0,
        if selected {
            theme::with_alpha(theme::ACCENT, 0.14)
        } else {
            theme::quaternary(0.5)
        },
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn commission_field(ui: &mut egui::Ui, state: &mut PaperAccountSheetState) {
    label(ui, "Commission");
    kit::segmented(
        ui,
        Id::new("paper-account-commission-kind"),
        &CommissionKind::ALL.map(|kind| (kind, kind.title())),
        &mut state.commission_kind,
    );
    if state.commission_kind != CommissionKind::None {
        ui.add_space(8.0);
        let error = state.commission_error();
        let unit = state.commission_kind.unit(state.currency);
        let width = ui.available_width();
        amount_field(
            ui,
            "paper-account-commission",
            &mut state.commission_text,
            "0",
            &unit,
            error.is_some(),
            width,
        );
        error_line(ui, error, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> PaperAccount {
        let mut account = PaperAccount::new(3, "Swing", 0);
        account.base_currency = PaperCurrency::Eur;
        account.initial_balance = 25_000.5;
        account.settings.slippage_ticks = 2.0;
        account.settings.show_orders_on_chart = false;
        account.settings.leverage.crypto = 5.0;
        account.settings.commission = PaperCommissionConfiguration::Percentage { value: 0.1 };
        account
    }

    #[test]
    fn a_new_account_starts_from_the_defaults() {
        let state = PaperAccountSheetState::create();
        assert!(!state.is_reset && state.can_submit());
        assert_eq!(state.title(), "New Paper Account");
        assert_eq!(state.submit_title(), "Create Account");
        let result = state.result().unwrap();
        assert_eq!(result.name, "Paper Trading");
        assert_eq!(result.balance, 100_000.0);
        assert_eq!(result.currency, PaperCurrency::Usd);
        assert_eq!(result.settings, PaperAccountSettings::default());
        assert_eq!(state.reset_note(), None);
    }

    #[test]
    fn a_reset_starts_from_the_accounts_own_settings() {
        let state = PaperAccountSheetState::reset(&account());
        assert!(state.is_reset);
        assert_eq!(state.name, "Swing");
        assert_eq!(state.balance_text, "25000.5");
        assert_eq!(state.leverage_text, "5");
        assert_eq!(state.commission_kind, CommissionKind::Percentage);
        assert_eq!(state.commission_text, "0.1");
        assert_eq!(state.currency, PaperCurrency::Eur);
        assert_eq!(
            state.reset_note().as_deref(),
            Some("Balance resets to €25,000.50.")
        );
        assert_eq!(state.submit_title(), "Reset Account");
        let result = state.result().unwrap();
        assert_eq!(
            result.settings.slippage_ticks, 2.0,
            "other settings survive a reset"
        );
        assert!(!result.settings.show_orders_on_chart);
        assert_eq!(
            result.settings.leverage.stocks, 5.0,
            "one leverage for every market"
        );
        assert_eq!(
            result.settings.commission,
            PaperCommissionConfiguration::Percentage { value: 0.1 }
        );
    }

    #[test]
    fn each_field_has_its_own_rule() {
        let mut state = PaperAccountSheetState::create();
        state.balance_text = "abc".into();
        assert_eq!(state.balance_error(), Some("Enter a number."));
        state.balance_text = "0".into();
        assert_eq!(state.balance_error(), Some("Must be greater than zero."));
        assert!(!state.can_submit());
        state.balance_text.clear();
        assert_eq!(
            state.visible_balance_error(),
            None,
            "an empty field is not red yet"
        );
        assert!(!state.can_submit(), "but it still blocks the button");
        state.balance_text = "50,000".into();
        assert!(state.balance_error().is_none());

        for (text, error) in [
            ("0.5", Some("Must be between 1× and 1000×.")),
            ("1001", Some("Must be between 1× and 1000×.")),
            ("x", Some("Enter a number.")),
            ("1000", None),
            ("1", None),
        ] {
            state.leverage_text = text.into();
            assert_eq!(state.leverage_error(), error, "{text}");
        }
        state.leverage_text = "1".into();

        state.commission_text = "-1".into();
        assert_eq!(
            state.commission_error(),
            None,
            "ignored while commission is None"
        );
        state.commission_kind = CommissionKind::Fixed;
        assert_eq!(state.commission_error(), Some("Can't be negative."));
        state.commission_text = "".into();
        assert_eq!(state.commission_error(), Some("Enter a number."));
        state.commission_text = "2".into();
        assert_eq!(state.commission_error(), None);

        state.name = "   ".into();
        assert!(!state.can_submit());
        assert_eq!(state.result(), None);
    }

    #[test]
    fn the_result_trims_the_name_and_builds_the_commission() {
        let mut state = PaperAccountSheetState::create();
        state.name = "  Scalping ".into();
        state.commission_kind = CommissionKind::PerContract;
        state.commission_text = "1,5".into();
        state.leverage_text = "10".into();
        let result = state.result().unwrap();
        assert_eq!(result.name, "Scalping");
        assert_eq!(
            result.settings.commission,
            PaperCommissionConfiguration::PerContract { value: 1.5 }
        );
        assert_eq!(result.settings.leverage.prediction, 10.0);
        state.commission_kind = CommissionKind::None;
        assert_eq!(
            state.result().unwrap().settings.commission,
            PaperCommissionConfiguration::None
        );
    }

    #[test]
    fn commission_units_follow_the_kind_and_currency() {
        assert_eq!(CommissionKind::None.unit(PaperCurrency::Usd), "");
        assert_eq!(
            CommissionKind::Fixed.unit(PaperCurrency::Gbp),
            "GBP per order"
        );
        assert_eq!(
            CommissionKind::Percentage.unit(PaperCurrency::Usd),
            "% of value"
        );
        assert_eq!(
            CommissionKind::PerContract.unit(PaperCurrency::Usdt),
            "USDT per contract"
        );
        let titles: Vec<_> = CommissionKind::ALL
            .iter()
            .map(|kind| kind.title())
            .collect();
        assert_eq!(titles, ["None", "Fixed", "Percent", "Per contract"]);
    }
}
