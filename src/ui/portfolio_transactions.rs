//! Every transaction in the selected portfolio, searchable, filterable and sortable.
//!
//! Rows come from the unprojected ledger (so Edit opens exactly what was entered); the price
//! and fee shown are the reporting-currency projection of the same transaction.

// Transient: the Portfolio window hosts this once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioTransaction, PortfolioTransactionType};
use crate::model::portfolio_format::{PortfolioValueFormatter, QuantitySign};
use crate::ui::alert_center::clock_time;
use crate::ui::portfolio_table::{
    self as table, HEADER_HEIGHT, ROW_HEIGHT, footer, header_cell, numeric_cell, pill, search_field,
};
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionColumn {
    Date,
    Asset,
    Type,
    Quantity,
    Price,
    Total,
    Fee,
    Notes,
}

impl TransactionColumn {
    pub const ALL: [Self; 8] = [
        Self::Date,
        Self::Asset,
        Self::Type,
        Self::Quantity,
        Self::Price,
        Self::Total,
        Self::Fee,
        Self::Notes,
    ];

    /// Money columns name the reporting currency.
    pub fn title(self, currency_code: &str) -> String {
        match self {
            Self::Date => "Date".to_owned(),
            Self::Asset => "Asset".to_owned(),
            Self::Type => "Type".to_owned(),
            Self::Quantity => "Quantity".to_owned(),
            Self::Price => format!("Price ({currency_code})"),
            Self::Total => format!("Total ({currency_code})"),
            Self::Fee => "Fee".to_owned(),
            Self::Notes => "Notes".to_owned(),
        }
    }

    /// The fee column does not sort, as in the Swift table.
    pub fn is_sortable(self) -> bool {
        self != Self::Fee
    }

    fn is_numeric(self) -> bool {
        matches!(self, Self::Quantity | Self::Price | Self::Total | Self::Fee)
    }

    /// A first click sorts text A to Z and everything else (dates, figures) newest/largest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, Self::Asset | Self::Type | Self::Notes)
    }
}

/// The reporting-currency view of one transaction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub price: Option<f64>,
    pub fee: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransactionRow {
    pub transaction: PortfolioTransaction,
    pub price: Option<f64>,
    pub fee: f64,
}

impl TransactionRow {
    pub fn ticker(&self) -> String {
        self.transaction.asset.display_ticker()
    }

    pub fn total(&self) -> Option<f64> {
        self.price.map(|price| price * self.transaction.quantity)
    }

    pub fn type_name(&self) -> &'static str {
        type_name(self.transaction.kind)
    }

    pub fn source_name(&self) -> &'static str {
        use crate::model::portfolio::PortfolioSource::*;
        match self.transaction.source {
            Manual => "Manual",
            Wallet => "Wallet",
            Exchange => "Exchange",
            Csv => "CSV",
            CoinMarketCap => "CoinMarketCap",
        }
    }
}

pub fn type_name(kind: PortfolioTransactionType) -> &'static str {
    use PortfolioTransactionType::*;
    match kind {
        Buy => "Buy",
        Sell => "Sell",
        TransferIn => "Transfer In",
        TransferOut => "Transfer Out",
        Reward => "Reward",
        StakingReward => "Staking Reward",
        Airdrop => "Airdrop",
        Mining => "Mining",
        Interest => "Interest",
        Fee => "Fee",
        Adjustment => "Adjustment",
    }
}

pub const ALL_TYPES: [PortfolioTransactionType; 11] = {
    use PortfolioTransactionType::*;
    [
        Buy,
        Sell,
        TransferIn,
        TransferOut,
        Reward,
        StakingReward,
        Airdrop,
        Mining,
        Interest,
        Fee,
        Adjustment,
    ]
};

/// Buys and sells read at a glance in a long table.
pub fn type_tint(kind: PortfolioTransactionType) -> Color32 {
    use PortfolioTransactionType::*;
    match kind {
        Buy => theme::UP,
        Sell => theme::DOWN,
        TransferIn | TransferOut => Color32::from_rgb(10, 132, 255),
        Reward | StakingReward | Airdrop | Mining | Interest => Color32::from_rgb(191, 90, 242),
        Fee => theme::REPLAY,
        Adjustment => Color32::from_gray(150),
    }
}

/// The sign shown on a quantity: inflows "+", outflows "−".
pub fn quantity_sign(kind: PortfolioTransactionType) -> QuantitySign {
    if kind.adds_quantity() {
        QuantitySign::Plus
    } else if kind.removes_quantity() {
        QuantitySign::Minus
    } else {
        QuantitySign::None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub search: String,
    pub kind: Option<PortfolioTransactionType>,
    pub asset_key: Option<String>,
}

impl Filters {
    pub fn is_active(&self) -> bool {
        !self.search.trim().is_empty() || self.kind.is_some() || self.asset_key.is_some()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Rows matching the filters, sorted by `column`.
pub fn rows(
    ledger: &[PortfolioTransaction],
    projection: &dyn Fn(u64) -> Option<Projection>,
    subtitle: &dyn Fn(&PortfolioAsset) -> String,
    filters: &Filters,
    column: TransactionColumn,
    ascending: bool,
) -> Vec<TransactionRow> {
    let needle = filters.search.trim().to_lowercase();
    let mut rows: Vec<TransactionRow> = ledger
        .iter()
        .filter(|tx| filters.kind.is_none_or(|kind| tx.kind == kind))
        .filter(|tx| {
            filters
                .asset_key
                .as_ref()
                .is_none_or(|key| &tx.asset.key == key)
        })
        .filter(|tx| {
            needle.is_empty()
                || [
                    tx.asset.display_ticker(),
                    tx.asset.symbol.clone(),
                    tx.asset.name.clone(),
                    subtitle(&tx.asset),
                    tx.notes.clone(),
                    source_label(tx),
                    type_name(tx.kind).to_owned(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        })
        .map(|tx| {
            let projected = projection(tx.id);
            TransactionRow {
                transaction: tx.clone(),
                price: projected.and_then(|p| p.price),
                fee: projected.map_or(0.0, |p| p.fee),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        let order = compare(a, b, column);
        if ascending { order } else { order.reverse() }
    });
    rows
}

fn source_label(tx: &PortfolioTransaction) -> String {
    TransactionRow {
        transaction: tx.clone(),
        price: None,
        fee: 0.0,
    }
    .source_name()
    .to_owned()
}

fn compare(a: &TransactionRow, b: &TransactionRow, column: TransactionColumn) -> Ordering {
    let (x, y) = (&a.transaction, &b.transaction);
    match column {
        TransactionColumn::Date => x.timestamp.cmp(&y.timestamp),
        TransactionColumn::Asset => a.ticker().to_lowercase().cmp(&b.ticker().to_lowercase()),
        TransactionColumn::Type => a.type_name().cmp(b.type_name()),
        TransactionColumn::Quantity => x.quantity.total_cmp(&y.quantity),
        // An unpriced transaction sorts as zero, as in the Swift table.
        TransactionColumn::Price => a.price.unwrap_or(0.0).total_cmp(&b.price.unwrap_or(0.0)),
        TransactionColumn::Total => a
            .total()
            .unwrap_or(0.0)
            .total_cmp(&b.total().unwrap_or(0.0)),
        TransactionColumn::Fee => a.fee.total_cmp(&b.fee),
        TransactionColumn::Notes => x.notes.to_lowercase().cmp(&y.notes.to_lowercase()),
    }
}

/// The assets that appear in the ledger, once each, A to Z: `(key, ticker)`.
pub fn asset_choices(ledger: &[PortfolioTransaction]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut choices: Vec<(String, String)> = ledger
        .iter()
        .filter(|tx| seen.insert(tx.asset.key.clone()))
        .map(|tx| (tx.asset.key.clone(), tx.asset.display_ticker()))
        .collect();
    choices.sort_by_key(|(_, ticker)| ticker.to_lowercase());
    choices
}

/// "Oct 9, 2026 14:03" in the viewer's time zone.
pub fn date_time_text(ms: i64, utc_offset_secs: i64) -> String {
    format!(
        "{} {}",
        date_text(ms + utc_offset_secs * 1_000),
        clock_time(ms, utc_offset_secs)
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransactionAction {
    Add,
    Edit(u64),
    Duplicate(u64),
    Delete(u64),
}

pub struct TransactionsInput<'a> {
    pub ledger: &'a [PortfolioTransaction],
    pub projection: &'a dyn Fn(u64) -> Option<Projection>,
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
    pub formatter: PortfolioValueFormatter,
    pub utc_offset_secs: i64,
}

pub struct TransactionsState {
    pub filters: Filters,
    column: TransactionColumn,
    ascending: bool,
    selected: Option<u64>,
}

impl Default for TransactionsState {
    fn default() -> Self {
        Self {
            filters: Filters::default(),
            column: TransactionColumn::Date,
            ascending: false,
            selected: None,
        }
    }
}

impl TransactionsState {
    pub fn column(&self) -> TransactionColumn {
        self.column
    }

    pub fn is_ascending(&self) -> bool {
        self.ascending
    }

    pub fn click(&mut self, column: TransactionColumn) {
        if !column.is_sortable() {
            return;
        }
        if self.column == column {
            self.ascending = !self.ascending;
        } else {
            self.column = column;
            self.ascending = column.default_ascending();
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
) -> Vec<TransactionAction> {
    let mut actions = Vec::new();
    toolbar(ui, state, input, &mut actions);
    let shown = rows(
        input.ledger,
        input.projection,
        input.subtitle,
        &state.filters,
        state.column,
        state.ascending,
    );
    if shown.is_empty() {
        if state.filters.is_active() {
            table::empty_state(
                ui,
                "No Matching Transactions",
                "Nothing matches the current search and filters.",
            );
            ui.vertical_centered(|ui| {
                if ui.button("Clear Filters").clicked() {
                    state.filters.clear();
                }
            });
        } else {
            table::empty_state(
                ui,
                "No Transactions",
                "Add a transaction or import a CSV to get started.",
            );
        }
    } else {
        table_view(ui, state, input, &shown, &mut actions);
    }
    footer(
        ui,
        shown.len(),
        input.ledger.len(),
        ("transaction", "transactions"),
        &format!("Prices and fees in {}", input.formatter.currency.code()),
    );
    actions
}

fn toolbar(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
    actions: &mut Vec<TransactionAction>,
) {
    ui.horizontal(|ui| {
        ui.add_space(table::INSET);
        search_field(
            ui,
            &mut state.filters.search,
            "Search asset, notes, or source",
        );

        let kind_label = state.filters.kind.map_or("All Types", type_name);
        egui::ComboBox::from_id_salt("portfolio-type-filter")
            .selected_text(kind_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.filters.kind, None, "All Types");
                ui.separator();
                for kind in ALL_TYPES {
                    ui.selectable_value(&mut state.filters.kind, Some(kind), type_name(kind));
                }
            });

        let choices = asset_choices(input.ledger);
        let asset_label = state
            .filters
            .asset_key
            .as_ref()
            .and_then(|key| choices.iter().find(|(candidate, _)| candidate == key))
            .map_or("All Assets", |(_, ticker)| ticker.as_str());
        egui::ComboBox::from_id_salt("portfolio-asset-filter")
            .selected_text(asset_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.filters.asset_key, None, "All Assets");
                ui.separator();
                for (key, ticker) in &choices {
                    ui.selectable_value(&mut state.filters.asset_key, Some(key.clone()), ticker);
                }
            });
        if state.filters.is_active() && ui.link("Clear").clicked() {
            state.filters.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(table::INSET);
            if ui.button("Add Transaction").clicked() {
                actions.push(TransactionAction::Add);
            }
        });
    });
    ui.add_space(10.0);
}

fn table_view(
    ui: &mut egui::Ui,
    state: &mut TransactionsState,
    input: &TransactionsInput,
    shown: &[TransactionRow],
    actions: &mut Vec<TransactionAction>,
) {
    let f = input.formatter;
    let code = f.currency.code();
    let mut clicked_header = None;
    let height = table::table_height(ui);
    table::table_frame(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("portfolio-transactions")
            .striped(true)
            .sense(table::row_sense())
            .min_scrolled_height(height)
            .max_scroll_height(height)
            .column(Column::initial(150.0).at_least(120.0).clip(true))
            .column(Column::initial(80.0).at_least(70.0))
            .column(Column::initial(105.0).at_least(85.0))
            .column(Column::initial(90.0).at_least(75.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(75.0).at_least(60.0))
            .column(Column::remainder().at_least(90.0).clip(true))
            .header(HEADER_HEIGHT, |mut header| {
                for column in TransactionColumn::ALL {
                    header.col(|ui| {
                        let sorted = (state.column == column).then_some(state.ascending);
                        if header_cell(
                            ui,
                            &column.title(code),
                            sorted,
                            column.is_sortable(),
                            column.is_numeric(),
                        ) {
                            clicked_header = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, shown.len(), |mut row| {
                    let item = &shown[row.index()];
                    let tx = &item.transaction;
                    row.set_selected(state.selected == Some(tx.id));
                    row.col(|ui| {
                        ui.label(date_time_text(tx.timestamp, input.utc_offset_secs));
                    });
                    row.col(|ui| {
                        ui.horizontal_centered(|ui| {
                            table::asset_icon(ui, &tx.asset, 20.0);
                            ui.label(RichText::new(item.ticker()).strong());
                        });
                    });
                    row.col(|ui| {
                        pill(ui, item.type_name(), type_tint(tx.kind))
                            .on_hover_text(format!("Source: {}", item.source_name()));
                    });
                    row.col(|ui| {
                        let color = (!f.privacy).then(|| match quantity_sign(tx.kind) {
                            QuantitySign::Plus => theme::UP,
                            QuantitySign::Minus => theme::DOWN,
                            QuantitySign::None => ui.visuals().text_color(),
                        });
                        numeric_cell(ui, &f.quantity(tx.quantity, quantity_sign(tx.kind)), color);
                    });
                    row.col(|ui| {
                        numeric_cell(ui, &item.price.map_or("—".into(), |v| f.price(v)), None)
                    });
                    row.col(|ui| {
                        numeric_cell(ui, &item.total().map_or("—".into(), |v| f.money(v)), None)
                    });
                    row.col(|ui| {
                        let text = if item.fee == 0.0 {
                            "—".to_owned()
                        } else {
                            f.money(item.fee)
                        };
                        numeric_cell(ui, &text, Some(ui.visuals().weak_text_color()));
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button("⋯", |ui| {
                                if ui.button("Edit").clicked() {
                                    actions.push(TransactionAction::Edit(tx.id));
                                    ui.close();
                                }
                                if ui.button("Duplicate").clicked() {
                                    actions.push(TransactionAction::Duplicate(tx.id));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button(RichText::new("Delete").color(theme::DOWN))
                                    .clicked()
                                {
                                    actions.push(TransactionAction::Delete(tx.id));
                                    ui.close();
                                }
                            });
                            ui.add(egui::Label::new(RichText::new(&tx.notes).weak()).truncate())
                                .on_hover_text(&tx.notes);
                        });
                    });
                    let response = row.response();
                    if response.clicked() {
                        state.selected = Some(tx.id);
                    }
                    if response.double_clicked() {
                        actions.push(TransactionAction::Edit(tx.id));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Edit").clicked() {
                            actions.push(TransactionAction::Edit(tx.id));
                            ui.close();
                        }
                        if ui.button("Duplicate").clicked() {
                            actions.push(TransactionAction::Duplicate(tx.id));
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(RichText::new("Delete").color(theme::DOWN))
                            .clicked()
                        {
                            actions.push(TransactionAction::Delete(tx.id));
                            ui.close();
                        }
                    });
                });
            });
    });
    if let Some(column) = clicked_header {
        state.click(column);
    }
    // The Delete key removes the selected transaction.
    if let Some(id) = state.selected
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|input| input.key_pressed(egui::Key::Delete))
    {
        actions.push(TransactionAction::Delete(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource};
    use std::collections::HashMap;

    fn asset(ticker: &str, name: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: format!("binance:{ticker}USDT"),
            symbol: format!("{ticker}/USDT"),
            name: name.to_owned(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn tx(
        id: u64,
        ticker: &str,
        name: &str,
        kind: PortfolioTransactionType,
        qty: f64,
        at: i64,
        notes: &str,
    ) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: asset(ticker, name),
            kind,
            quantity: qty,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: at,
            notes: notes.to_owned(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ledger() -> Vec<PortfolioTransaction> {
        use PortfolioTransactionType::*;
        vec![
            tx(1, "BTC", "Bitcoin", Buy, 2.0, 100, "first buy"),
            tx(2, "ETH", "Ethereum", Buy, 10.0, 300, ""),
            tx(3, "BTC", "Bitcoin", Sell, 1.0, 200, "took profit"),
            tx(4, "ETH", "Ethereum", Reward, 0.5, 400, "staking"),
        ]
    }

    fn projection(id: u64) -> Option<Projection> {
        // Transaction 2 has no price (unpriced); 3 carries a fee.
        match id {
            2 => Some(Projection {
                price: None,
                fee: 0.0,
            }),
            3 => Some(Projection {
                price: Some(30.0),
                fee: 1.5,
            }),
            _ => Some(Projection {
                price: Some(10.0),
                fee: 0.0,
            }),
        }
    }

    fn subtitle(asset: &PortfolioAsset) -> String {
        asset.name.clone()
    }

    fn ids(rows: &[TransactionRow]) -> Vec<u64> {
        rows.iter().map(|r| r.transaction.id).collect()
    }

    fn query(filters: &Filters, column: TransactionColumn, ascending: bool) -> Vec<TransactionRow> {
        rows(
            &ledger(),
            &projection,
            &subtitle,
            filters,
            column,
            ascending,
        )
    }

    #[test]
    fn the_default_view_is_newest_first() {
        let found = query(&Filters::default(), TransactionColumn::Date, false);
        assert_eq!(ids(&found), [4, 2, 3, 1]);
        let oldest = query(&Filters::default(), TransactionColumn::Date, true);
        assert_eq!(ids(&oldest), [1, 3, 2, 4]);
    }

    #[test]
    fn prices_totals_and_fees_are_the_projection_not_the_ledger() {
        let found = query(&Filters::default(), TransactionColumn::Date, true);
        let sell = found.iter().find(|r| r.transaction.id == 3).unwrap();
        assert_eq!((sell.price, sell.fee), (Some(30.0), 1.5));
        assert_eq!(sell.total(), Some(30.0), "price × quantity");
        let unpriced = found.iter().find(|r| r.transaction.id == 2).unwrap();
        assert_eq!((unpriced.price, unpriced.total()), (None, None));
    }

    #[test]
    fn unpriced_rows_sort_as_zero() {
        let by_price = query(&Filters::default(), TransactionColumn::Price, true);
        assert_eq!(by_price[0].transaction.id, 2);
        let by_total = query(&Filters::default(), TransactionColumn::Total, false);
        assert_eq!(
            by_total[0].transaction.id, 3,
            "30 × 1 beats 10 × 2 and 10 × 0.5"
        );
    }

    #[test]
    fn type_asset_and_note_filters_combine() {
        use PortfolioTransactionType::*;
        let buys = Filters {
            kind: Some(Buy),
            ..Default::default()
        };
        assert_eq!(ids(&query(&buys, TransactionColumn::Date, true)), [1, 2]);
        let btc = Filters {
            asset_key: Some("binance:BTCUSDT".into()),
            ..Default::default()
        };
        assert_eq!(ids(&query(&btc, TransactionColumn::Date, true)), [1, 3]);
        let btc_buys = Filters {
            kind: Some(Buy),
            ..btc.clone()
        };
        assert_eq!(ids(&query(&btc_buys, TransactionColumn::Date, true)), [1]);
        assert!(btc.is_active());
        let mut cleared = btc;
        cleared.clear();
        assert!(!cleared.is_active());
    }

    #[test]
    fn search_reaches_notes_names_sources_and_types() {
        for (needle, expected) in [
            ("profit", vec![3]),
            ("ethereum", vec![2, 4]),
            ("MANUAL", vec![1, 2, 3, 4]),
            ("reward", vec![4]),
            ("  sell ", vec![3]),
            ("nope", vec![]),
        ] {
            let filters = Filters {
                search: needle.into(),
                ..Default::default()
            };
            let mut found = ids(&query(&filters, TransactionColumn::Date, true));
            found.sort_unstable();
            assert_eq!(found, expected, "{needle:?}");
        }
        assert!(
            !Filters {
                search: "   ".into(),
                ..Default::default()
            }
            .is_active()
        );
    }

    #[test]
    fn text_columns_sort_a_to_z_ignoring_case() {
        let by_asset = query(&Filters::default(), TransactionColumn::Asset, true);
        assert_eq!(by_asset[0].ticker(), "BTC");
        let by_notes = query(&Filters::default(), TransactionColumn::Notes, true);
        assert_eq!(ids(&by_notes)[0], 2, "the empty note sorts first");
        let by_type = query(&Filters::default(), TransactionColumn::Type, true);
        assert_eq!(by_type[0].type_name(), "Buy");
    }

    #[test]
    fn clicking_headers_flips_or_switches_and_the_fee_column_does_not_sort() {
        let mut state = TransactionsState::default();
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Date, false)
        );
        state.click(TransactionColumn::Date);
        assert!(state.is_ascending());
        state.click(TransactionColumn::Asset);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Asset, true)
        );
        state.click(TransactionColumn::Total);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (TransactionColumn::Total, false)
        );
        state.click(TransactionColumn::Fee);
        assert_eq!(
            state.column(),
            TransactionColumn::Total,
            "fees are not a sort key"
        );
    }

    #[test]
    fn asset_choices_list_each_asset_once_a_to_z() {
        let mut more = ledger();
        more.push(tx(
            5,
            "ada",
            "Cardano",
            PortfolioTransactionType::Buy,
            1.0,
            500,
            "",
        ));
        let choices = asset_choices(&more);
        let tickers: Vec<&str> = choices.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(tickers, ["ADA", "BTC", "ETH"]);
    }

    #[test]
    fn quantities_carry_inflow_and_outflow_signs() {
        use PortfolioTransactionType::*;
        assert_eq!(quantity_sign(Buy), QuantitySign::Plus);
        assert_eq!(quantity_sign(Reward), QuantitySign::Plus);
        assert_eq!(quantity_sign(Sell), QuantitySign::Minus);
        assert_eq!(quantity_sign(Fee), QuantitySign::Minus);
        assert_eq!(quantity_sign(Adjustment), QuantitySign::None);
        assert_eq!(type_name(StakingReward), "Staking Reward");
        assert_eq!(ALL_TYPES.len(), 11);
    }

    #[test]
    fn dates_read_in_the_viewers_zone() {
        // 2024-01-15 23:30 UTC is already the 16th in UTC+2.
        let ms = 1_705_361_400_000;
        assert_eq!(date_time_text(ms, 0), "Jan 15, 2024 23:30");
        assert_eq!(date_time_text(ms, 2 * 3_600), "Jan 16, 2024 01:30");
        assert_eq!(TransactionColumn::Price.title("USD"), "Price (USD)");
        assert_eq!(TransactionColumn::Fee.title("USD"), "Fee");
    }
}
