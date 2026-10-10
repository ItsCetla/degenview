//! The selected portfolio's positions as a sortable table.
//!
//! The sort column follows the portfolio's saved `PortfolioHoldingsSort`; clicking a header
//! that the saved sort has a case for writes it back. Double-click (or Return) opens the
//! asset's transactions. Pure over `PortfolioHolding`s: the host prices them and persists.

// Transient: the Portfolio window hosts this once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioHoldingsSort};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::portfolio_table::{
    self as table, HEADER_HEIGHT, ROW_HEIGHT, footer, header_cell, numeric_cell, search_field,
};
use eframe::egui::{self, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldingsColumn {
    Asset,
    Price,
    DayChange,
    Quantity,
    AverageCost,
    Value,
    Allocation,
    ProfitLoss,
}

impl HoldingsColumn {
    pub const ALL: [Self; 8] = [
        Self::Asset,
        Self::Price,
        Self::DayChange,
        Self::Quantity,
        Self::AverageCost,
        Self::Value,
        Self::Allocation,
        Self::ProfitLoss,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Asset => "Asset",
            Self::Price => "Price",
            Self::DayChange => "24h %",
            Self::Quantity => "Quantity",
            Self::AverageCost => "Avg Cost",
            Self::Value => "Value",
            Self::Allocation => "Allocation",
            Self::ProfitLoss => "P&L",
        }
    }

    /// The saved sort a click on this column writes, for the columns the portfolio remembers.
    pub fn saved_sort(self) -> Option<PortfolioHoldingsSort> {
        match self {
            Self::Asset => Some(PortfolioHoldingsSort::Asset),
            Self::DayChange => Some(PortfolioHoldingsSort::DayChange),
            Self::Value => Some(PortfolioHoldingsSort::CurrentValue),
            Self::Allocation => Some(PortfolioHoldingsSort::Allocation),
            Self::ProfitLoss => Some(PortfolioHoldingsSort::ProfitLoss),
            Self::Price | Self::Quantity | Self::AverageCost => None,
        }
    }

    /// The column a saved sort shows. The table has one P&L column; it sorts by amount.
    pub fn for_saved(sort: PortfolioHoldingsSort) -> Self {
        match sort {
            PortfolioHoldingsSort::Asset => Self::Asset,
            PortfolioHoldingsSort::CurrentValue => Self::Value,
            PortfolioHoldingsSort::Allocation => Self::Allocation,
            PortfolioHoldingsSort::DayChange => Self::DayChange,
            PortfolioHoldingsSort::ProfitLoss | PortfolioHoldingsSort::ProfitLossPercent => {
                Self::ProfitLoss
            }
        }
    }

    /// A first click sorts names A to Z and numbers largest first.
    pub fn default_ascending(self) -> bool {
        self == Self::Asset
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoldingRow {
    pub holding: PortfolioHolding,
    pub subtitle: String,
}

impl HoldingRow {
    pub fn ticker(&self) -> String {
        self.holding.asset.display_ticker()
    }

    pub fn key(&self) -> &str {
        &self.holding.asset.key
    }
}

/// Rows matching the search (ticker, symbol, name, subtitle), sorted by `column`. An
/// unpriced value sorts as the lowest: last when largest-first.
pub fn rows(
    holdings: &[PortfolioHolding],
    subtitle: &dyn Fn(&PortfolioAsset) -> String,
    search: &str,
    column: HoldingsColumn,
    ascending: bool,
) -> Vec<HoldingRow> {
    let needle = search.trim().to_lowercase();
    let mut rows: Vec<HoldingRow> = holdings
        .iter()
        .map(|holding| HoldingRow {
            subtitle: subtitle(&holding.asset),
            holding: holding.clone(),
        })
        .filter(|row| {
            needle.is_empty()
                || [
                    row.ticker(),
                    row.holding.asset.symbol.clone(),
                    row.holding.asset.name.clone(),
                    row.subtitle.clone(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        })
        .collect();
    rows.sort_by(|a, b| {
        let order = compare(a, b, column);
        if ascending { order } else { order.reverse() }
    });
    rows
}

fn number(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

fn compare(a: &HoldingRow, b: &HoldingRow, column: HoldingsColumn) -> Ordering {
    let (x, y) = (&a.holding, &b.holding);
    match column {
        HoldingsColumn::Asset => a.ticker().to_lowercase().cmp(&b.ticker().to_lowercase()),
        HoldingsColumn::Price => number(x.current_price).total_cmp(&number(y.current_price)),
        HoldingsColumn::DayChange => {
            number(x.day_change_percent()).total_cmp(&number(y.day_change_percent()))
        }
        HoldingsColumn::Quantity => x.quantity.total_cmp(&y.quantity),
        HoldingsColumn::AverageCost => x.average_cost.total_cmp(&y.average_cost),
        HoldingsColumn::Value => number(x.current_value).total_cmp(&number(y.current_value)),
        HoldingsColumn::Allocation => x.allocation.total_cmp(&y.allocation),
        HoldingsColumn::ProfitLoss => number(x.total_pnl()).total_cmp(&number(y.total_pnl())),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HoldingsAction {
    /// Write the portfolio's saved sort.
    SetSort(PortfolioHoldingsSort),
    /// Open this asset's transactions (double-click, or the menu).
    Open(String),
    /// Remap the asset to another market.
    Remap(String),
}

pub struct HoldingsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    pub saved_sort: PortfolioHoldingsSort,
    pub formatter: PortfolioValueFormatter,
    /// "Bitcoin", or the stored label.
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
}

pub struct HoldingsState {
    pub search: String,
    column: HoldingsColumn,
    ascending: bool,
    selected: Option<String>,
    /// The saved sort the table last adopted, so switching portfolios re-applies it.
    adopted: Option<PortfolioHoldingsSort>,
}

impl Default for HoldingsState {
    fn default() -> Self {
        Self {
            search: String::new(),
            column: HoldingsColumn::Value,
            ascending: false,
            selected: None,
            adopted: None,
        }
    }
}

impl HoldingsState {
    pub fn column(&self) -> HoldingsColumn {
        self.column
    }

    pub fn is_ascending(&self) -> bool {
        self.ascending
    }

    /// Follows the portfolio's saved sort when it changes (another portfolio, a write).
    pub fn adopt(&mut self, saved: PortfolioHoldingsSort) {
        if self.adopted == Some(saved) {
            return;
        }
        self.adopted = Some(saved);
        self.column = HoldingsColumn::for_saved(saved);
        self.ascending = self.column.default_ascending();
    }

    /// A click on a header: flips the direction on the sorted column, otherwise sorts by it.
    /// Returns the saved sort to write when this column is one the portfolio remembers and
    /// differs from what is saved.
    pub fn click(
        &mut self,
        column: HoldingsColumn,
        saved: PortfolioHoldingsSort,
    ) -> Option<PortfolioHoldingsSort> {
        if self.column == column {
            self.ascending = !self.ascending;
        } else {
            self.column = column;
            self.ascending = column.default_ascending();
        }
        let sort = column.saved_sort()?;
        (HoldingsColumn::for_saved(saved) != column).then(|| {
            self.adopted = Some(sort);
            sort
        })
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
) -> Vec<HoldingsAction> {
    let mut actions = Vec::new();
    state.adopt(input.saved_sort);

    ui.horizontal(|ui| {
        ui.add_space(table::INSET);
        search_field(ui, &mut state.search, "Search asset or name");
    });
    ui.add_space(10.0);

    let shown = rows(
        input.holdings,
        input.subtitle,
        &state.search,
        state.column,
        state.ascending,
    );
    if shown.is_empty() {
        if input.holdings.is_empty() {
            table::empty_state(
                ui,
                "No Holdings",
                "Positions appear once a transaction leaves you holding an asset.",
            );
        } else {
            table::empty_state(ui, "No Results", "No holding matches this search.");
        }
    } else {
        table_view(ui, state, input, &shown, &mut actions);
    }
    footer(
        ui,
        shown.len(),
        input.holdings.len(),
        ("asset", "assets"),
        &format!(
            "Values in {} · Double-click a row for its transactions",
            input.formatter.currency.code()
        ),
    );
    actions
}

fn table_view(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
    shown: &[HoldingRow],
    actions: &mut Vec<HoldingsAction>,
) {
    let f = input.formatter;
    let mut clicked_header = None;
    let height = table::table_height(ui);
    table::table_frame(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("portfolio-holdings")
            .striped(true)
            .sense(table::row_sense())
            .min_scrolled_height(height)
            .max_scroll_height(height)
            .column(Column::initial(150.0).at_least(110.0).clip(true))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(70.0).at_least(55.0))
            .column(Column::initial(90.0).at_least(65.0))
            .column(Column::initial(100.0).at_least(80.0))
            .column(Column::initial(105.0).at_least(80.0))
            .column(Column::initial(70.0).at_least(55.0))
            .column(Column::remainder().at_least(110.0))
            .header(HEADER_HEIGHT, |mut header| {
                for column in HoldingsColumn::ALL {
                    header.col(|ui| {
                        let sorted = (state.column == column).then_some(state.ascending);
                        let right = column != HoldingsColumn::Asset;
                        if header_cell(ui, column.title(), sorted, true, right) {
                            clicked_header = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, shown.len(), |mut row| {
                    let item = &shown[row.index()];
                    let holding = &item.holding;
                    row.set_selected(state.selected.as_deref() == Some(item.key()));
                    row.col(|ui| {
                        ui.horizontal_centered(|ui| {
                            table::asset_icon(ui, &holding.asset, 24.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(RichText::new(item.ticker()).strong());
                                ui.label(RichText::new(&item.subtitle).small().weak());
                            });
                        });
                    });
                    row.col(|ui| {
                        numeric_cell(
                            ui,
                            &holding.current_price.map_or("—".into(), |v| f.price(v)),
                            None,
                        );
                    });
                    row.col(|ui| {
                        let day = holding.day_change_percent();
                        numeric_cell(
                            ui,
                            &day.map_or("—".into(), |v| f.signed_percent(v, 2)),
                            table::tone_option(f.tone(day))
                                .or(Some(ui.visuals().weak_text_color())),
                        );
                    });
                    row.col(|ui| {
                        numeric_cell(ui, &f.quantity(holding.quantity, Default::default()), None)
                    });
                    row.col(|ui| numeric_cell(ui, &f.price(holding.average_cost), None));
                    row.col(|ui| {
                        numeric_cell(
                            ui,
                            &holding.current_value.map_or("—".into(), |v| f.money(v)),
                            None,
                        );
                    });
                    row.col(|ui| numeric_cell(ui, &f.percent(holding.allocation, 1), None));
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let pnl = holding.total_pnl();
                            ui.menu_button("⋯", |ui| {
                                if ui.button("View Transactions").clicked() {
                                    actions.push(HoldingsAction::Open(item.key().to_owned()));
                                    ui.close();
                                }
                                if ui.button("Remap Asset…").clicked() {
                                    actions.push(HoldingsAction::Remap(item.key().to_owned()));
                                    ui.close();
                                }
                            });
                            numeric_cell(
                                ui,
                                &pnl.map_or("—".into(), |v| f.signed_money(v)),
                                table::tone_option(f.tone(pnl)),
                            );
                        });
                    });
                    let response = row.response();
                    if response.clicked() {
                        state.selected = Some(item.key().to_owned());
                    }
                    if response.double_clicked() {
                        actions.push(HoldingsAction::Open(item.key().to_owned()));
                    }
                    response.context_menu(|ui| {
                        if ui.button("View Transactions").clicked() {
                            actions.push(HoldingsAction::Open(item.key().to_owned()));
                            ui.close();
                        }
                        if ui.button("Remap Asset…").clicked() {
                            actions.push(HoldingsAction::Remap(item.key().to_owned()));
                            ui.close();
                        }
                    });
                });
            });
    });
    if let Some(column) = clicked_header
        && let Some(sort) = state.click(column, input.saved_sort)
    {
        actions.push(HoldingsAction::SetSort(sort));
    }
    let _ = Vec2::ZERO;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn holding(
        ticker: &str,
        name: &str,
        value: Option<f64>,
        cost: f64,
        qty: f64,
    ) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}USDT"),
                symbol: format!("{ticker}/USDT"),
                name: name.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: qty,
            cost_basis: cost,
            average_cost: if qty == 0.0 { 0.0 } else { cost / qty },
            realized_pnl: 0.0,
            current_price: value.map(|v| v / qty),
            previous_day_price: value.map(|v| v / qty / 1.1),
            current_value: value,
            unrealized_pnl: value.map(|v| v - cost),
            allocation: value.unwrap_or(0.0) / 1_000.0,
            is_price_stale: false,
        }
    }

    fn fixtures() -> Vec<PortfolioHolding> {
        vec![
            holding("BTC", "Bitcoin", Some(600.0), 500.0, 0.01),
            holding("ETH", "Ethereum", Some(300.0), 350.0, 0.1),
            holding("DOGE", "Dogecoin", None, 50.0, 400.0),
            holding("ADA", "Cardano", Some(100.0), 60.0, 200.0),
        ]
    }

    fn tickers(rows: &[HoldingRow]) -> Vec<String> {
        rows.iter().map(HoldingRow::ticker).collect()
    }

    fn subtitle(asset: &PortfolioAsset) -> String {
        asset.name.clone()
    }

    #[test]
    fn value_sorts_largest_first_with_unpriced_assets_last() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, false);
        assert_eq!(tickers(&found), ["BTC", "ETH", "ADA", "DOGE"]);
        let reversed = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, true);
        assert_eq!(tickers(&reversed), ["DOGE", "ADA", "ETH", "BTC"]);
    }

    #[test]
    fn assets_sort_a_to_z_ignoring_case() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Asset, true);
        assert_eq!(tickers(&found), ["ADA", "BTC", "DOGE", "ETH"]);
    }

    #[test]
    fn profit_and_loss_sorts_by_amount_with_unknown_last() {
        let found = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::ProfitLoss,
            false,
        );
        assert_eq!(tickers(&found), ["BTC", "ADA", "ETH", "DOGE"]);
    }

    #[test]
    fn quantity_and_allocation_sort_numerically() {
        let by_quantity = rows(&fixtures(), &subtitle, "", HoldingsColumn::Quantity, false);
        assert_eq!(tickers(&by_quantity), ["DOGE", "ADA", "ETH", "BTC"]);
        let by_allocation = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::Allocation,
            false,
        );
        assert_eq!(by_allocation[0].ticker(), "BTC");
    }

    #[test]
    fn search_matches_ticker_pair_name_and_subtitle() {
        let all = fixtures();
        for (needle, expected) in [
            ("btc", vec!["BTC"]),
            ("COIN", vec!["BTC", "DOGE"]),
            ("usdt", vec!["BTC", "ETH", "DOGE", "ADA"]),
            ("  card ", vec!["ADA"]),
            ("zzz", vec![]),
        ] {
            let mut found = tickers(&rows(&all, &subtitle, needle, HoldingsColumn::Value, false));
            found.sort();
            let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
            want.sort();
            assert_eq!(found, want, "{needle:?}");
        }
    }

    #[test]
    fn saved_sorts_map_onto_columns_and_back() {
        use PortfolioHoldingsSort::*;
        for (sort, column) in [
            (Asset, HoldingsColumn::Asset),
            (CurrentValue, HoldingsColumn::Value),
            (Allocation, HoldingsColumn::Allocation),
            (DayChange, HoldingsColumn::DayChange),
            (ProfitLoss, HoldingsColumn::ProfitLoss),
            (ProfitLossPercent, HoldingsColumn::ProfitLoss),
        ] {
            assert_eq!(HoldingsColumn::for_saved(sort), column);
        }
        assert_eq!(HoldingsColumn::Price.saved_sort(), None);
        assert_eq!(HoldingsColumn::Quantity.saved_sort(), None);
        assert_eq!(HoldingsColumn::AverageCost.saved_sort(), None);
        assert_eq!(HoldingsColumn::Value.saved_sort(), Some(CurrentValue));
    }

    #[test]
    fn a_header_click_flips_or_switches_and_writes_back_only_remembered_columns() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::CurrentValue);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Value, false)
        );

        // Clicking the sorted column flips direction; the saved sort already names it.
        assert_eq!(
            state.click(HoldingsColumn::Value, PortfolioHoldingsSort::CurrentValue),
            None
        );
        assert!(state.is_ascending());

        // A new remembered column is written back, names A to Z, numbers largest first.
        assert_eq!(
            state.click(HoldingsColumn::Asset, PortfolioHoldingsSort::CurrentValue),
            Some(PortfolioHoldingsSort::Asset)
        );
        assert!(state.is_ascending());
        assert_eq!(
            state.click(HoldingsColumn::Allocation, PortfolioHoldingsSort::Asset),
            Some(PortfolioHoldingsSort::Allocation)
        );
        assert!(!state.is_ascending());

        // A transient column sorts the view but writes nothing.
        assert_eq!(
            state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Allocation),
            None
        );
        assert_eq!(state.column(), HoldingsColumn::Price);
    }

    #[test]
    fn switching_portfolios_adopts_the_other_ones_saved_sort_once() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Asset, true)
        );
        state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Asset);
        // The same saved sort again must not undo the user's transient choice...
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(state.column(), HoldingsColumn::Price);
        // ...but a different one (another portfolio) is followed.
        state.adopt(PortfolioHoldingsSort::DayChange);
        assert_eq!(state.column(), HoldingsColumn::DayChange);
        assert!(!state.is_ascending());
    }
}
