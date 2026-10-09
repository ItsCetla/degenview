//! The paper order ticket: opened from a chart's BUY / SELL buttons. A pure view over a
//! `TicketDraft` (all the parsing, validation and figures live in `model::paper_ticket`) that
//! returns `PaperTicketAction`s; the host owns the store, refreshes the market figures each frame
//! and reports back whether the submit worked.

// Transient: the paper-trading host opens the ticket once it is wired.
#![allow(dead_code)]

use crate::model::paper::{
    PaperCurrency, PaperOrderRequest, PaperOrderSide, PaperOrderType, PaperTimeInForce,
};
use crate::model::paper_format as fmt;
use crate::model::paper_ticket::{self as ticket, TicketDraft, TicketField, decimal_text};
use crate::ui::paper_header;
use crate::ui::paper_tables::side_tint;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Vec2};

pub const WIDTH: f32 = 460.0;

/// The quick-fill shortcuts under the quantity: a share of the available funds.
pub const QUICK_FILLS: [(f64, &str); 4] =
    [(0.25, "25%"), (0.5, "50%"), (0.75, "75%"), (1.0, "Max")];

pub struct PaperTicketState {
    pub draft: TicketDraft,
    pub shows_protection: bool,
    /// True between pressing Submit and the host calling `finish`.
    pub submitting: bool,
}

impl PaperTicketState {
    pub fn new(draft: TicketDraft) -> Self {
        Self {
            draft,
            shows_protection: false,
            submitting: false,
        }
    }

    /// The host's answer to a `Submit`: a success closes the sheet (the host drops the state), a
    /// failure re-enables the button and the host shows the error through `PaperTicketInput`.
    pub fn finish(&mut self) {
        self.submitting = false;
    }

    /// Fresh market figures for the open ticket; typed text is untouched.
    pub fn refresh_market(
        &mut self,
        bid: Option<f64>,
        ask: Option<f64>,
        last: Option<f64>,
        available_funds: Option<f64>,
        existing_signed_quantity: f64,
    ) {
        self.draft.bid = bid;
        self.draft.ask = ask;
        if last.is_some() {
            self.draft.last = last;
        }
        self.draft.available_funds = available_funds;
        self.draft.existing_signed_quantity = existing_signed_quantity;
    }
}

pub struct PaperTicketInput<'a> {
    pub account_id: Option<u64>,
    pub currency: PaperCurrency,
    /// The store's last error, shown under the form as "Order not placed".
    pub error: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperTicketAction {
    Submit(Box<PaperOrderRequest>),
    Cancel,
}

// MARK: Pure figures

/// One cell of the quote strip across the top.
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteCell {
    pub title: &'static str,
    pub value: String,
    /// The side whose colour the cell takes (the price that side would trade at).
    pub highlight: Option<PaperOrderSide>,
}

const NO_VALUE: &str = "—";

/// Last is always shown; Bid, Ask and Spread only for markets whose stream supplies them.
pub fn quote_cells(draft: &TicketDraft) -> Vec<QuoteCell> {
    let price = |value: Option<f64>| {
        value.map_or_else(
            || NO_VALUE.to_owned(),
            |value| fmt::price(value, &draft.instrument),
        )
    };
    let mut cells = Vec::new();
    if draft.has_book() {
        cells.push(QuoteCell {
            title: "Bid",
            value: price(draft.bid),
            highlight: (draft.side == PaperOrderSide::Sell).then_some(PaperOrderSide::Sell),
        });
    }
    cells.push(QuoteCell {
        title: "Last",
        value: price(draft.last),
        highlight: None,
    });
    if draft.has_book() {
        cells.push(QuoteCell {
            title: "Ask",
            value: price(draft.ask),
            highlight: (draft.side == PaperOrderSide::Buy).then_some(PaperOrderSide::Buy),
        });
        if let Some(spread) = draft.spread() {
            cells.push(QuoteCell {
                title: "Spread",
                value: price(Some(spread)),
                highlight: None,
            });
        }
    }
    cells
}

pub const NO_BOOK_NOTE: &str =
    "Best bid and ask aren't available for this market, so orders fill at the last price.";

/// One line of the summary card.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryRow {
    pub title: &'static str,
    pub value: String,
    /// Drawn red: the order needs more than the account has free.
    pub alert: bool,
}

pub fn summary_rows(draft: &TicketDraft, currency: PaperCurrency) -> Vec<SummaryRow> {
    let money = |value: Option<f64>| {
        value.map_or_else(|| NO_VALUE.to_owned(), |value| fmt::money(value, currency))
    };
    let exceeds = draft.exceeds_funds();
    let mut rows = vec![
        SummaryRow {
            title: "Order value",
            value: money(draft.notional()),
            alert: false,
        },
        SummaryRow {
            title: "Required margin",
            value: money(draft.required_margin()),
            alert: exceeds,
        },
    ];
    if let Some(fee) = draft.estimated_fee() {
        rows.push(SummaryRow {
            title: "Estimated fee",
            value: money(Some(fee)),
            alert: false,
        });
    }
    rows.push(SummaryRow {
        title: "Available funds",
        value: money(draft.available_funds),
        alert: exceeds,
    });
    rows
}

pub fn shortfall_text(draft: &TicketDraft, currency: PaperCurrency) -> Option<String> {
    draft.shortfall().map(|shortfall| {
        format!(
            "Short by {}. Lower the quantity or raise leverage.",
            fmt::money(shortfall, currency)
        )
    })
}

/// The label for an order type's tab.
pub fn type_title(kind: PaperOrderType) -> &'static str {
    kind.label()
}

pub fn stop_label(kind: PaperOrderType) -> &'static str {
    if kind == PaperOrderType::StopLimit {
        "Stop trigger"
    } else {
        "Stop price"
    }
}

pub const MARKET_HINT: &str = "Fills immediately at the best available price.";

/// Sets the quantity to `fraction` of the funds, when that is possible. Returns whether it did.
pub fn apply_quick_fill(draft: &mut TicketDraft, fraction: f64) -> bool {
    match draft.quantity_for_funds_fraction(fraction) {
        Some(quantity) => {
            draft.quantity_text = decimal_text(quantity, draft.comma_locale);
            true
        }
        None => false,
    }
}

/// Fills a price field with the current market price. Returns whether there was one.
pub fn fill_market_price(draft: &mut TicketDraft, field: TicketField) -> bool {
    let Some(price) = draft.market_price() else {
        return false;
    };
    let text = decimal_text(price, draft.comma_locale);
    match field {
        TicketField::LimitPrice => draft.limit_price_text = text,
        TicketField::StopPrice => draft.stop_price_text = text,
        _ => return false,
    }
    true
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut PaperTicketState,
    input: &PaperTicketInput,
) -> Vec<PaperTicketAction> {
    let mut actions = Vec::new();
    egui::Window::new(state.draft.instrument.display_name.clone())
        .id(Id::new("paper-order-ticket"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(WIDTH, 0.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 160.0).max(240.0))
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    form(ui, state, input);
                });
            ui.separator();
            footer(ui, state, input, &mut actions);
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(PaperTicketAction::Cancel);
    }
    actions
}

fn form(ui: &mut egui::Ui, state: &mut PaperTicketState, input: &PaperTicketInput) {
    let instrument = state.draft.instrument.clone();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(&instrument.display_name).heading());
            ui.label(
                RichText::new(format!(
                    "{} · Paper order",
                    instrument.source.display_name()
                ))
                .weak(),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            paper_header::badge(ui);
        });
    });
    ui.add_space(12.0);

    quote_strip(ui, &state.draft);
    ui.add_space(12.0);
    side_selector(ui, &mut state.draft.side);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        for kind in PaperOrderType::ALL {
            let selected = state.draft.kind == kind;
            if ui.selectable_label(selected, type_title(kind)).clicked() {
                state.draft.kind = kind;
            }
        }
    });
    ui.add_space(12.0);

    let issues = state.draft.issues();
    quantity_field(ui, &mut state.draft, &issues);
    ui.add_space(12.0);
    price_fields(ui, &mut state.draft, &issues);
    protection(ui, state, &issues);
    if state.draft.kind != PaperOrderType::Market {
        ui.add_space(12.0);
        field_label(ui, "Time in force");
        ui.horizontal(|ui| {
            for tif in PaperTimeInForce::ALL {
                let selected = state.draft.time_in_force == tif;
                if ui.selectable_label(selected, tif.label()).clicked() {
                    state.draft.time_in_force = tif;
                }
            }
        });
    }
    ui.add_space(12.0);
    summary_card(ui, &state.draft, input.currency);
    if let Some(error) = input.error {
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(theme::DOWN.gamma_multiply(0.14))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("⚠ Order not placed")
                        .strong()
                        .color(theme::DOWN),
                );
                ui.label(RichText::new(error).small());
            });
    }
    ui.add_space(8.0);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn quote_strip(ui: &mut egui::Ui, draft: &TicketDraft) {
    card(ui, |ui| {
        let cells = quote_cells(draft);
        ui.columns(cells.len(), |columns| {
            for (column, cell) in columns.iter_mut().zip(&cells) {
                column.vertical_centered(|ui| {
                    ui.label(RichText::new(cell.title).small().weak());
                    let color = match cell.highlight {
                        Some(side) => side_tint(side),
                        None if cell.value == NO_VALUE => Color32::from_gray(150),
                        None => ui.visuals().text_color(),
                    };
                    ui.label(RichText::new(&cell.value).monospace().color(color));
                });
            }
        });
    });
    if !draft.has_book() {
        ui.add_space(4.0);
        ui.label(RichText::new(NO_BOOK_NOTE).small().weak());
    }
}

fn side_selector(ui: &mut egui::Ui, side: &mut PaperOrderSide) {
    ui.columns(2, |columns| {
        for (column, option) in columns
            .iter_mut()
            .zip([PaperOrderSide::Buy, PaperOrderSide::Sell])
        {
            let selected = *side == option;
            let text = RichText::new(option.label())
                .size(14.0)
                .strong()
                .color(if selected {
                    Color32::WHITE
                } else {
                    Color32::from_gray(160)
                });
            let mut button = egui::Button::new(text).min_size(Vec2::new(0.0, 32.0));
            button = if selected {
                button.fill(side_tint(option).gamma_multiply(0.92))
            } else {
                button.fill(Color32::from_white_alpha(10))
            };
            if column
                .add_sized([column.available_width(), 32.0], button)
                .clicked()
            {
                *side = option;
            }
        }
    });
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().strong().weak());
}

fn field_footer(ui: &mut egui::Ui, error: Option<&String>, hint: Option<&str>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).small().color(theme::DOWN));
    } else if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().weak());
    }
}

/// A text box with a trailing unit and an optional accessory button; returns the button click.
fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    placeholder: &str,
    suffix: &str,
    is_error: bool,
    accessory: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 56.0 - accessory.map_or(0.0, |_| 52.0)).max(60.0);
        let mut edit = egui::TextEdit::singleline(text)
            .hint_text(placeholder)
            .desired_width(width);
        if is_error {
            edit = edit.text_color(theme::DOWN);
        }
        ui.add(edit);
        ui.label(RichText::new(suffix).small().weak());
        if let Some(title) = accessory {
            clicked = ui.small_button(title).clicked();
        }
    });
    clicked
}

fn quantity_field(
    ui: &mut egui::Ui,
    draft: &mut TicketDraft,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    field_label(ui, "Quantity");
    let error = issues.get(&TicketField::Quantity);
    let symbol = ticket::base_symbol(&draft.instrument);
    text_field(
        ui,
        &mut draft.quantity_text,
        "0",
        &symbol,
        error.is_some(),
        None,
    );
    ui.horizontal(|ui| {
        for (fraction, title) in QUICK_FILLS {
            let possible = draft.quantity_for_funds_fraction(fraction).is_some();
            if ui
                .add_enabled(
                    possible,
                    egui::Button::new(RichText::new(title).small().strong()),
                )
                .clicked()
            {
                apply_quick_fill(draft, fraction);
            }
        }
        ui.label(RichText::new("of available funds").small().weak());
    });
    let hint = (draft.kind == PaperOrderType::Market).then_some(MARKET_HINT);
    field_footer(ui, error, hint);
}

fn price_fields(
    ui: &mut egui::Ui,
    draft: &mut TicketDraft,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    if draft.kind == PaperOrderType::Market {
        return;
    }
    let currency = draft.instrument.quote_currency.code();
    let has_market = draft.market_price().is_some();
    let accessory = has_market.then_some("Market");
    ui.columns(
        if draft.uses_stop_price() && draft.uses_limit_price() {
            2
        } else {
            1
        },
        |columns| {
            let mut index = 0;
            if draft.uses_stop_price() {
                let column = &mut columns[index];
                index += 1;
                field_label(column, stop_label(draft.kind));
                let error = issues.get(&TicketField::StopPrice);
                if text_field(
                    column,
                    &mut draft.stop_price_text,
                    "0",
                    currency,
                    error.is_some(),
                    accessory,
                ) {
                    fill_market_price(draft, TicketField::StopPrice);
                }
                field_footer(column, error, None);
            }
            if draft.uses_limit_price() {
                let column = &mut columns[index];
                field_label(column, "Limit price");
                let error = issues.get(&TicketField::LimitPrice);
                if text_field(
                    column,
                    &mut draft.limit_price_text,
                    "0",
                    currency,
                    error.is_some(),
                    accessory,
                ) {
                    fill_market_price(draft, TicketField::LimitPrice);
                }
                field_footer(column, error, None);
            }
        },
    );
}

fn protection(
    ui: &mut egui::Ui,
    state: &mut PaperTicketState,
    issues: &std::collections::HashMap<TicketField, String>,
) {
    ui.add_space(8.0);
    let arrow = if state.shows_protection { "⏷" } else { "⏵" };
    let response = ui.horizontal(|ui| {
        let clicked = ui
            .add(
                egui::Label::new(
                    RichText::new(format!("{arrow} Take profit & stop loss")).strong(),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        ui.label(RichText::new("Optional").small().weak());
        clicked
    });
    if response.inner {
        state.shows_protection = !state.shows_protection;
    }
    if !state.shows_protection {
        return;
    }
    ui.add_space(4.0);
    let currency = state.draft.instrument.quote_currency.code();
    ui.columns(2, |columns| {
        field_label(&mut columns[0], "Take profit");
        let error = issues.get(&TicketField::TakeProfit);
        text_field(
            &mut columns[0],
            &mut state.draft.take_profit_text,
            "Price",
            currency,
            error.is_some(),
            None,
        );
        field_footer(&mut columns[0], error, None);
        field_label(&mut columns[1], "Stop loss");
        let error = issues.get(&TicketField::StopLoss);
        text_field(
            &mut columns[1],
            &mut state.draft.stop_loss_text,
            "Price",
            currency,
            error.is_some(),
            None,
        );
        field_footer(&mut columns[1], error, None);
    });
}

fn summary_card(ui: &mut egui::Ui, draft: &TicketDraft, currency: PaperCurrency) {
    card(ui, |ui| {
        for row in summary_rows(draft, currency) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(row.title).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if row.alert {
                        theme::DOWN
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.label(RichText::new(&row.value).monospace().color(color));
                });
            });
        }
        if let Some(text) = shortfall_text(draft, currency) {
            ui.label(
                RichText::new(format!("⚠ {text}"))
                    .small()
                    .color(theme::DOWN),
            );
        }
        if let Some(note) = draft.position_note() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Position").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(note);
                });
            });
        }
    });
}

fn footer(
    ui: &mut egui::Ui,
    state: &mut PaperTicketState,
    input: &PaperTicketInput,
    actions: &mut Vec<PaperTicketAction>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            actions.push(PaperTicketAction::Cancel);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled =
                state.draft.can_submit() && !state.submitting && input.account_id.is_some();
            let label = if state.submitting {
                format!("… {}", state.draft.submit_title())
            } else {
                state.draft.submit_title()
            };
            let button = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
                .fill(side_tint(state.draft.side))
                .min_size(Vec2::new(150.0, 30.0));
            if ui.add_enabled(enabled, button).clicked()
                && let Some(account_id) = input.account_id
                && let Some(request) = state.draft.request(account_id)
            {
                state.submitting = true;
                actions.push(PaperTicketAction::Submit(Box::new(request)));
            }
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;

    fn draft() -> TicketDraft {
        let mut instrument = PaperInstrument::chart("BTCUSDT", "BTC/USDT", DataSource::Binance);
        instrument.tick_size = 0.01;
        instrument.minimum_quantity = 0.001;
        instrument.quantity_increment = 0.001;
        let mut draft = TicketDraft::new(instrument);
        draft.bid = Some(99.0);
        draft.ask = Some(101.0);
        draft.last = Some(100.0);
        draft.available_funds = Some(10_000.0);
        draft
    }

    use crate::model::paper::PaperInstrument;

    #[test]
    fn the_strip_shows_the_book_only_when_there_is_one() {
        let mut ticket = draft();
        let titles = |cells: Vec<QuoteCell>| cells.iter().map(|c| c.title).collect::<Vec<_>>();
        assert_eq!(
            titles(quote_cells(&ticket)),
            ["Bid", "Last", "Ask", "Spread"]
        );
        ticket.bid = None;
        assert_eq!(titles(quote_cells(&ticket)), ["Last"]);
        ticket.last = None;
        assert_eq!(quote_cells(&ticket)[0].value, "—");
    }

    #[test]
    fn the_price_the_side_trades_at_takes_its_colour() {
        let mut ticket = draft();
        let highlighted = |ticket: &TicketDraft| {
            quote_cells(ticket)
                .iter()
                .filter(|cell| cell.highlight.is_some())
                .map(|cell| cell.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(highlighted(&ticket), ["Ask"]);
        ticket.side = PaperOrderSide::Sell;
        assert_eq!(highlighted(&ticket), ["Bid"]);
        let cells = quote_cells(&ticket);
        assert_eq!(cells[3].value, "2", "spread");
    }

    #[test]
    fn the_summary_turns_red_and_names_the_shortfall() {
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        let titles: Vec<_> = rows.iter().map(|row| row.title).collect();
        assert_eq!(
            titles,
            ["Order value", "Required margin", "Available funds"]
        );
        assert_eq!(rows[0].value, "$101.00");
        assert!(rows.iter().all(|row| !row.alert));
        assert_eq!(shortfall_text(&ticket, PaperCurrency::Usd), None);

        ticket.available_funds = Some(60.0);
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        assert!(rows[1].alert && rows[2].alert && !rows[0].alert);
        assert_eq!(
            shortfall_text(&ticket, PaperCurrency::Usd).as_deref(),
            Some("Short by $41.00. Lower the quantity or raise leverage.")
        );

        ticket.quantity_text.clear();
        assert_eq!(summary_rows(&ticket, PaperCurrency::Usd)[0].value, "—");
    }

    #[test]
    fn a_fee_row_appears_only_with_a_commission() {
        use crate::model::paper::PaperCommissionConfiguration;
        let mut ticket = draft();
        ticket.quantity_text = "1".into();
        assert_eq!(summary_rows(&ticket, PaperCurrency::Usd).len(), 3);
        ticket.commission = PaperCommissionConfiguration::FixedPerOrder { value: 2.0 };
        let rows = summary_rows(&ticket, PaperCurrency::Usd);
        assert_eq!(rows[2].title, "Estimated fee");
        assert_eq!(rows[2].value, "$2.00");
    }

    #[test]
    fn quick_fills_and_market_buttons_write_the_fields() {
        let mut ticket = draft();
        assert!(apply_quick_fill(&mut ticket, 0.5));
        assert_eq!(ticket.quantity_text, "49.504");
        ticket.available_funds = None;
        assert!(!apply_quick_fill(&mut ticket, 1.0));
        assert_eq!(
            ticket.quantity_text, "49.504",
            "an impossible fill leaves the field"
        );

        assert!(fill_market_price(&mut ticket, TicketField::LimitPrice));
        assert_eq!(ticket.limit_price_text, "101");
        ticket.side = PaperOrderSide::Sell;
        assert!(fill_market_price(&mut ticket, TicketField::StopPrice));
        assert_eq!(ticket.stop_price_text, "99");
        assert!(!fill_market_price(&mut ticket, TicketField::Quantity));
        ticket.bid = None;
        ticket.ask = None;
        ticket.last = None;
        assert!(!fill_market_price(&mut ticket, TicketField::LimitPrice));
    }

    #[test]
    fn the_stop_field_is_named_for_its_role() {
        assert_eq!(stop_label(PaperOrderType::Stop), "Stop price");
        assert_eq!(stop_label(PaperOrderType::StopLimit), "Stop trigger");
    }

    #[test]
    fn refreshing_market_data_keeps_what_was_typed() {
        let mut state = PaperTicketState::new(draft());
        state.draft.quantity_text = "0.5".into();
        state.refresh_market(Some(98.0), Some(100.0), Some(99.0), Some(500.0), -1.0);
        assert_eq!(state.draft.quantity_text, "0.5");
        assert_eq!(state.draft.bid, Some(98.0));
        assert_eq!(state.draft.existing_signed_quantity, -1.0);
        state.refresh_market(None, None, None, None, 0.0);
        assert_eq!(
            state.draft.last,
            Some(99.0),
            "a missing tick keeps the last price"
        );
        state.submitting = true;
        state.finish();
        assert!(!state.submitting);
    }
}
