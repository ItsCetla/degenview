//! Add, edit or duplicate one portfolio transaction. A pure view over a `TransactionDraft`
//! (parsing, the price/total link, validation and the summary live in
//! `model::portfolio_editor`) that returns an outcome; the host saves it to the ledger and
//! reports a rejection back through `TransactionEditorState::failed`.

// Transient: the portfolio host opens the editor once it is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioCurrency, PortfolioTransaction, PortfolioTransactionType};
use crate::model::portfolio_editor::{Category, EditorField, Summary, TransactionDraft};
use crate::ui::icons::Icon;
use crate::ui::portfolio_transactions::{type_name, type_tint};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, RichText, Vec2};

pub const WIDTH: f32 = 460.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorMode {
    New,
    Edit,
    Duplicate,
}

impl EditorMode {
    pub fn title(self) -> &'static str {
        match self {
            Self::New => "Add Transaction",
            Self::Edit => "Edit Transaction",
            Self::Duplicate => "Duplicate Transaction",
        }
    }

    pub fn submit_title(self) -> &'static str {
        match self {
            Self::Edit => "Save Changes",
            Self::New | Self::Duplicate => "Add Transaction",
        }
    }
}

pub struct TransactionEditorState {
    pub mode: EditorMode,
    base: PortfolioTransaction,
    pub draft: TransactionDraft,
    pub submitting: bool,
    pub error: Option<String>,
    /// The asset's price now, in the transaction's currency; None until it arrives.
    pub market_price: Option<f64>,
    first_frame: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionEditorOutcome {
    /// Save it: `Edit` replaces the transaction with this id, the others add a new one.
    Save {
        transaction: Box<PortfolioTransaction>,
        mode: EditorMode,
    },
    Cancel,
}

impl TransactionEditorState {
    pub fn new(mode: EditorMode, transaction: PortfolioTransaction, utc_offset_secs: i64) -> Self {
        Self {
            mode,
            draft: TransactionDraft::new(&transaction, utc_offset_secs),
            base: transaction,
            submitting: false,
            error: None,
            market_price: None,
            first_frame: true,
        }
    }

    pub fn asset_ticker(&self) -> String {
        self.base.asset.display_ticker()
    }

    pub fn price_currency(&self) -> PortfolioCurrency {
        self.base.price_currency
    }

    /// The host looked the price up: a new transaction opens with it in the price field; an edit
    /// or a duplicate keeps its own price and only offers this through the field's Market button.
    pub fn set_market_price(&mut self, price: f64) {
        self.market_price = Some(price);
        if self.mode == EditorMode::New {
            self.draft.prefill_price(price);
        }
    }

    /// The ledger rejected the save: the sheet stays open and says why.
    pub fn failed(&mut self, message: impl Into<String>) {
        self.submitting = false;
        self.error = Some(message.into());
    }

    fn subtitle(&self) -> String {
        format!(
            "{} · {}",
            self.base.asset.display_ticker(),
            self.base.asset.source.display_name()
        )
    }
}

// MARK: Pure text

pub fn price_label(ticker: &str) -> String {
    format!("Price per {ticker}")
}

pub fn price_hint(requires_price: bool) -> &'static str {
    if requires_price {
        "Required"
    } else {
        "Optional"
    }
}

pub fn summary_rows(summary: &Summary, currency: PortfolioCurrency) -> Vec<(String, String, bool)> {
    let mut rows = vec![("Value".to_owned(), currency.format(summary.value), false)];
    if summary.fee > 0.0 {
        rows.push(("Fee".to_owned(), currency.format(summary.fee), false));
    }
    if let Some((label, amount)) = summary.net {
        rows.push((label.to_owned(), currency.format(amount), true));
    }
    rows
}

pub fn primary_color(kind: PortfolioTransactionType) -> Color32 {
    if kind == PortfolioTransactionType::Adjustment {
        theme::ACCENT
    } else {
        type_tint(kind)
    }
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut TransactionEditorState,
) -> Option<TransactionEditorOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("portfolio-transaction-editor"), WIDTH, |ui| {
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 120.0).clamp(240.0, 650.0))
            .show(ui, |ui| {
                ui.add_space(24.0);
                sheet::inset(ui, |ui| form(ui, state));
                ui.add_space(20.0);
            });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            footer(ui, state, &mut outcome);
            ui.add_space(8.0);
        });
    });
    state.first_frame = false;
    if outcome.is_none() && !state.submitting && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(TransactionEditorOutcome::Cancel);
    }
    outcome
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().strong().weak());
}

fn footnote(ui: &mut egui::Ui, error: Option<&String>, hint: Option<&str>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).small().color(theme::DOWN));
    } else if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().weak());
    }
}

fn field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    suffix: &str,
    error: bool,
    width: f32,
) -> egui::Response {
    ui.horizontal(|ui| {
        let mut edit = egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(width);
        if error {
            edit = edit.text_color(theme::DOWN);
        }
        let response = ui.add(edit);
        ui.label(RichText::new(suffix).small().weak());
        response
    })
    .inner
}

fn form(ui: &mut egui::Ui, state: &mut TransactionEditorState) {
    sheet::header(
        ui,
        Icon::Exchange,
        state.mode.title(),
        &state.subtitle(),
        theme::ACCENT,
    );
    ui.add_space(18.0);

    // Type: category tabs, then the types inside a multi-type category.
    ui.horizontal(|ui| {
        for category in Category::ALL {
            let selected = state.draft.category() == category;
            if ui.selectable_label(selected, category.title()).clicked() {
                state.draft.select(category);
            }
        }
    });
    let types = state.draft.category().types();
    if types.len() > 1 {
        ui.horizontal(|ui| {
            for kind in types {
                let selected = state.draft.kind == *kind;
                let tint = type_tint(*kind);
                let text = RichText::new(type_name(*kind))
                    .small()
                    .strong()
                    .color(if selected {
                        tint
                    } else {
                        Color32::from_gray(160)
                    });
                if ui.selectable_label(selected, text).clicked() {
                    state.draft.kind = *kind;
                }
            }
        });
    }
    ui.add_space(12.0);

    let issues = state.draft.issues();
    let ticker = state.asset_ticker();
    let currency = state.price_currency();

    label(ui, "Quantity");
    let quantity = field(
        ui,
        &mut state.draft.quantity_text,
        "0",
        &ticker,
        issues.contains_key(&EditorField::Quantity),
        260.0,
    );
    if std::mem::take(&mut state.first_frame) {
        quantity.request_focus();
    }
    footnote(ui, issues.get(&EditorField::Quantity), None);
    ui.add_space(10.0);

    if state.draft.uses_price() {
        price_fields(ui, state, &issues, currency, &ticker);
        ui.add_space(10.0);
    }

    ui.columns(2, |columns| {
        label(&mut columns[0], "Fee");
        field(
            &mut columns[0],
            &mut state.draft.fee_text,
            "0",
            state.base.fee_currency.code(),
            issues.contains_key(&EditorField::Fee),
            90.0,
        );
        footnote(&mut columns[0], issues.get(&EditorField::Fee), None);
        label(&mut columns[1], "Date & time");
        let mut date = state.draft.date_text.clone();
        let response = columns[1].add(
            egui::TextEdit::singleline(&mut date)
                .hint_text("YYYY-MM-DD HH:MM")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            state.draft.date_text = date;
        }
        footnote(&mut columns[1], issues.get(&EditorField::Date), None);
    });
    ui.add_space(10.0);

    label(ui, "Notes");
    ui.add(
        egui::TextEdit::multiline(&mut state.draft.notes)
            .hint_text("Add a note (optional)")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    ui.add_space(10.0);

    if let Some(summary) = state.draft.summary() {
        egui::Frame::new()
            .fill(Color32::from_white_alpha(10))
            .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (title, value, emphasized) in summary_rows(&summary, currency) {
                    if emphasized {
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        let name = RichText::new(title);
                        ui.label(if emphasized {
                            name.strong()
                        } else {
                            name.weak()
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let value = RichText::new(value).monospace();
                            ui.label(if emphasized { value.strong() } else { value });
                        });
                    });
                }
            });
    }
    if let Some(error) = &state.error {
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(theme::DOWN.gamma_multiply(0.14))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("⚠ Transaction not saved")
                        .strong()
                        .color(theme::DOWN),
                );
                ui.label(RichText::new(error).small());
            });
    }
    ui.add_space(8.0);
}

fn price_fields(
    ui: &mut egui::Ui,
    state: &mut TransactionEditorState,
    issues: &std::collections::HashMap<EditorField, String>,
    currency: PortfolioCurrency,
    ticker: &str,
) {
    ui.columns(2, |columns| {
        label(&mut columns[0], &price_label(ticker));
        let mut price = state.draft.displayed_price();
        let changed = field(
            &mut columns[0],
            &mut price,
            "0.00",
            currency.code(),
            issues.contains_key(&EditorField::Price),
            110.0,
        )
        .changed();
        if changed {
            state.draft.set_price(&price);
        }
        columns[0].horizontal(|ui| {
            if let Some(market) = state.market_price
                && ui.small_button("Market").clicked()
            {
                let text = state.draft.text_for_market_price(market);
                state.draft.set_price(&text);
            }
        });
        footnote(
            &mut columns[0],
            issues.get(&EditorField::Price),
            Some(price_hint(state.draft.requires_price())),
        );

        label(&mut columns[1], "Total");
        let mut total = state.draft.displayed_total();
        let changed = field(
            &mut columns[1],
            &mut total,
            "0.00",
            currency.code(),
            issues.contains_key(&EditorField::Total),
            110.0,
        )
        .changed();
        if changed {
            state.draft.set_total(&total);
        }
        footnote(&mut columns[1], issues.get(&EditorField::Total), None);
    });
}

fn footer(
    ui: &mut egui::Ui,
    state: &mut TransactionEditorState,
    outcome: &mut Option<TransactionEditorOutcome>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!state.submitting, egui::Button::new("Cancel"))
            .clicked()
        {
            *outcome = Some(TransactionEditorOutcome::Cancel);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if state.submitting {
                ui.spinner();
            }
            let button = egui::Button::new(
                RichText::new(state.mode.submit_title())
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(primary_color(state.draft.kind))
            .min_size(Vec2::new(130.0, 28.0));
            if ui
                .add_enabled(state.draft.can_submit() && !state.submitting, button)
                .clicked()
                && let Some(transaction) = state
                    .draft
                    .make_transaction(&state.base, crate::model::watchlist::now_ms())
            {
                state.submitting = true;
                state.error = None;
                *outcome = Some(TransactionEditorOutcome::Save {
                    transaction: Box::new(transaction),
                    mode: state.mode,
                });
            }
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource};
    use std::collections::HashMap;

    fn transaction(price: Option<f64>) -> PortfolioTransaction {
        PortfolioTransaction {
            id: 5,
            portfolio_id: 1,
            asset: PortfolioAsset {
                key: "Binance:BTCUSDT".into(),
                symbol: "BTC/USDT".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity: if price.is_some() { 0.5 } else { 0.0 },
            price,
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_709_649_000_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn modes_read_in_words() {
        assert_eq!(EditorMode::New.title(), "Add Transaction");
        assert_eq!(EditorMode::Edit.title(), "Edit Transaction");
        assert_eq!(EditorMode::Duplicate.title(), "Duplicate Transaction");
        assert_eq!(EditorMode::Edit.submit_title(), "Save Changes");
        assert_eq!(EditorMode::Duplicate.submit_title(), "Add Transaction");
        assert_eq!(price_label("BTC"), "Price per BTC");
        assert_eq!(price_hint(true), "Required");
        assert_eq!(price_hint(false), "Optional");
    }

    #[test]
    fn only_a_new_transaction_takes_the_market_price_as_its_own() {
        let mut new = TransactionEditorState::new(EditorMode::New, transaction(None), 0);
        new.set_market_price(67_432.19);
        assert_eq!(new.draft.displayed_price(), "67,432.19");
        assert_eq!(new.market_price, Some(67_432.19));
        let mut edit = TransactionEditorState::new(EditorMode::Edit, transaction(None), 0);
        edit.set_market_price(67_432.19);
        assert_eq!(
            edit.draft.displayed_price(),
            "",
            "offered through the Market button only"
        );
        assert_eq!(edit.market_price, Some(67_432.19));
        let mut duplicate =
            TransactionEditorState::new(EditorMode::Duplicate, transaction(Some(50.0)), 0);
        duplicate.set_market_price(1.0);
        assert_eq!(duplicate.draft.displayed_price(), "50.00");
    }

    #[test]
    fn a_rejected_save_reopens_the_form_with_the_reason() {
        let mut state = TransactionEditorState::new(EditorMode::Edit, transaction(Some(10.0)), 0);
        state.submitting = true;
        state.failed("Sell exceeds the holding.");
        assert!(!state.submitting);
        assert_eq!(state.error.as_deref(), Some("Sell exceeds the holding."));
        assert_eq!(state.subtitle(), "BTC · Binance");
        assert_eq!(state.asset_ticker(), "BTC");
    }

    #[test]
    fn the_summary_card_lists_value_fee_and_the_net() {
        let summary = Summary {
            value: 1_000.0,
            fee: 2.5,
            net: Some(("Total cost", 1_002.5)),
        };
        let rows = summary_rows(&summary, PortfolioCurrency::Usd);
        assert_eq!(
            rows,
            vec![
                ("Value".to_owned(), "$1,000.00".to_owned(), false),
                ("Fee".to_owned(), "$2.50".to_owned(), false),
                ("Total cost".to_owned(), "$1,002.50".to_owned(), true),
            ]
        );
        let bare = Summary {
            value: 5.0,
            fee: 0.0,
            net: None,
        };
        assert_eq!(summary_rows(&bare, PortfolioCurrency::Usd).len(), 1);
        assert_eq!(
            primary_color(PortfolioTransactionType::Adjustment),
            theme::ACCENT
        );
        assert_eq!(primary_color(PortfolioTransactionType::Sell), theme::DOWN);
    }
}
