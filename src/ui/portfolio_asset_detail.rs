//! One asset's position and its transactions, shown as a sheet over the Holdings tab. A pure
//! view: the host resolves the holding and the transactions from the ledger on every frame (so
//! editing or deleting a transaction refreshes the figures in place) and applies the actions.
//! Every amount is in the reporting currency — the same one the Holdings table shows — and says so.

// Transient: the portfolio host opens this once it is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioTransaction};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::kit;
use crate::ui::portfolio_overview::{StatCard, grid};
use crate::ui::portfolio_table::{HEADER_HEIGHT, ROW_HEIGHT, header_cell, numeric_cell, pill};
use crate::ui::portfolio_transactions::{date_time_text, quantity_sign, type_name, type_tint};
use crate::ui::sheet;
use crate::ui::theme;
use eframe::egui::{self, Id, RichText, Vec2};
use egui_extras::{Column, TableBuilder};

pub const SIZE: Vec2 = Vec2::new(880.0, 660.0);

pub struct AssetDetailInput<'a> {
    /// None when the transactions net to zero.
    pub holding: Option<&'a PortfolioHolding>,
    /// The asset, from the holding or else from its first transaction.
    pub asset: Option<&'a PortfolioAsset>,
    /// "Bitcoin" or the stored label.
    pub subtitle: &'a str,
    /// This asset's transactions, newest first (`asset_transactions`).
    pub transactions: &'a [&'a PortfolioTransaction],
    /// A transaction's price in the reporting currency; the ledger's own may be in another.
    pub price_of: &'a dyn Fn(&PortfolioTransaction) -> Option<f64>,
    pub formatter: PortfolioValueFormatter,
    pub utc_offset_secs: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetDetailAction {
    Edit(u64),
    Duplicate(u64),
    /// The host confirms before deleting.
    Delete(u64),
    Close,
}

/// The ledger's transactions in one asset, newest first.
pub fn asset_transactions<'a>(
    ledger: &'a [PortfolioTransaction],
    asset_key: &str,
) -> Vec<&'a PortfolioTransaction> {
    let mut rows: Vec<&PortfolioTransaction> = ledger
        .iter()
        .filter(|transaction| transaction.asset.key == asset_key)
        .collect();
    rows.sort_by_key(|transaction| std::cmp::Reverse(transaction.timestamp));
    rows
}

// MARK: Pure text

pub fn values_note(code: &str) -> String {
    format!("Values in {code}")
}

pub const NO_POSITION: &str = "No open position — the transactions below net to zero.";

/// The eight position cards.
pub fn stat_cards<'a>(
    holding: &'a PortfolioHolding,
    f: PortfolioValueFormatter,
) -> Vec<StatCard<'a>> {
    let unavailable = "Unavailable".to_owned();
    vec![
        StatCard::new(
            "Holdings",
            if f.privacy {
                f.quantity(holding.quantity, Default::default())
            } else {
                format!(
                    "{} {}",
                    f.quantity(holding.quantity, Default::default()),
                    holding.asset.display_ticker()
                )
            },
        ),
        StatCard::new(
            "Current Value",
            holding
                .current_value
                .map_or(unavailable.clone(), |v| f.money(v)),
        )
        .placeholder(holding.current_value.is_none()),
        StatCard::new("Average Cost", f.price(holding.average_cost)),
        StatCard::new(
            "Current Price",
            holding.current_price.map_or(unavailable, |v| f.price(v)),
        )
        .placeholder(holding.current_price.is_none()),
        StatCard::new(
            "Unrealized P&L",
            holding
                .unrealized_pnl
                .map_or("—".to_owned(), |v| f.signed_money(v)),
        )
        .tone(f.tone(holding.unrealized_pnl))
        .placeholder(holding.unrealized_pnl.is_none()),
        StatCard::new("Realized P&L", f.signed_money(holding.realized_pnl))
            .tone(f.tone(Some(holding.realized_pnl))),
        StatCard::new(
            "Total P&L",
            holding
                .total_pnl()
                .map_or("—".to_owned(), |v| f.signed_money(v)),
        )
        .tone(f.tone(holding.total_pnl()))
        .caption(holding.pnl_percent().map(|p| f.signed_percent(p, 2)))
        .placeholder(holding.total_pnl().is_none()),
        StatCard::new("Allocation", f.percent(holding.allocation, 2)),
    ]
}

// MARK: View

pub fn show(ctx: &egui::Context, input: &AssetDetailInput) -> Vec<AssetDetailAction> {
    let mut actions = Vec::new();
    sheet::show(ctx, Id::new("portfolio-asset-detail"), SIZE.x, |ui| {
        ui.set_height(SIZE.y);
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            header(ui, input);
            ui.add_space(18.0);
            match input.holding {
                Some(holding) => {
                    ui.label(
                        RichText::new(values_note(input.formatter.currency.code()))
                            .small()
                            .weak(),
                    );
                    ui.add_space(4.0);
                    grid(ui, &stat_cards(holding, input.formatter));
                }
                None => {
                    egui::Frame::new()
                        .fill(theme::ink(10))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(format!("▭ {NO_POSITION}")).weak());
                        });
                }
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Transactions").strong().size(15.0));
                ui.label(RichText::new(input.transactions.len().to_string()).weak());
            });
        });
        ui.add_space(8.0);
        let footer_height = 48.0;
        let height = (ui.available_height() - footer_height).max(100.0);
        sheet::inset(ui, |ui| {
            ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
                transactions(ui, input, &mut actions);
            })
        });
        ui.separator();
        sheet::inset(ui, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Done").strong().color(egui::Color32::WHITE),
                            )
                            .fill(theme::SYSTEM_ACCENT)
                            .min_size(Vec2::new(76.0, 30.0)),
                        )
                        .clicked()
                    {
                        actions.push(AssetDetailAction::Close);
                    }
                });
            });
            ui.add_space(10.0);
        });
    });
    if actions.is_empty() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(AssetDetailAction::Close);
    }
    actions
}

fn header(ui: &mut egui::Ui, input: &AssetDetailInput) {
    let Some(asset) = input.asset else {
        return;
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
        kit::ticker_icon(ui, rect, &asset.display_ticker(), asset.source);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                asset.display_ticker(),
                kit::size::TITLE2 + 2.0,
                kit::BOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                format!("{} · {}", input.subtitle, asset.source.display_name()),
                kit::size::SUBHEADLINE + 1.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    });
}

fn transactions(ui: &mut egui::Ui, input: &AssetDetailInput, actions: &mut Vec<AssetDetailAction>) {
    if input.transactions.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No Transactions").strong().size(15.0));
        });
        return;
    }
    let f = input.formatter;
    let code = f.currency.code();
    let height = ui.available_height();
    TableBuilder::new(ui)
        .id_salt("portfolio-asset-detail-table")
        .striped(true)
        .max_scroll_height(height)
        .column(Column::initial(150.0).at_least(110.0).clip(true))
        .column(Column::initial(100.0).at_least(90.0))
        .column(Column::initial(110.0).at_least(80.0))
        .column(Column::initial(110.0).at_least(80.0))
        .column(Column::initial(120.0).at_least(80.0))
        .column(Column::remainder().at_least(110.0))
        .header(HEADER_HEIGHT, |mut header| {
            let titles = [
                "Date".to_owned(),
                "Type".to_owned(),
                "Quantity".to_owned(),
                format!("Price ({code})"),
                format!("Total ({code})"),
                "Notes".to_owned(),
            ];
            for (index, title) in titles.iter().enumerate() {
                header.col(|ui| {
                    header_cell(ui, title, None, false, (2..5).contains(&index));
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, input.transactions.len(), |mut row| {
                let transaction = input.transactions[row.index()];
                let price = (input.price_of)(transaction);
                row.col(|ui| {
                    ui.label(date_time_text(transaction.timestamp, input.utc_offset_secs));
                });
                row.col(|ui| {
                    pill(ui, type_name(transaction.kind), type_tint(transaction.kind));
                });
                row.col(|ui| {
                    let color = if f.privacy {
                        None
                    } else if transaction.kind.adds_quantity() {
                        Some(theme::UP)
                    } else if transaction.kind.removes_quantity() {
                        Some(theme::DOWN)
                    } else {
                        None
                    };
                    numeric_cell(
                        ui,
                        &f.quantity(transaction.quantity, quantity_sign(transaction.kind)),
                        color,
                    );
                });
                row.col(|ui| {
                    numeric_cell(ui, &price.map_or("—".to_owned(), |p| f.price(p)), None);
                });
                row.col(|ui| {
                    numeric_cell(
                        ui,
                        &price.map_or("—".to_owned(), |p| f.money(p * transaction.quantity)),
                        None,
                    );
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&transaction.notes).weak()).truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.menu_button("⋯", |ui| {
                                if ui.button("Edit").clicked() {
                                    actions.push(AssetDetailAction::Edit(transaction.id));
                                    ui.close();
                                }
                                if ui.button("Duplicate").clicked() {
                                    actions.push(AssetDetailAction::Duplicate(transaction.id));
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button(RichText::new("Delete").color(theme::DOWN))
                                    .clicked()
                                {
                                    actions.push(AssetDetailAction::Delete(transaction.id));
                                    ui.close();
                                }
                            });
                        });
                    });
                });
                if row.response().double_clicked() {
                    actions.push(AssetDetailAction::Edit(transaction.id));
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioCurrency, PortfolioSource, PortfolioTransactionType};
    use std::collections::HashMap;

    fn asset(key: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: key.into(),
            symbol: "BTC/USDT".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn transaction(id: u64, key: &str, timestamp: i64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id: 1,
            asset: asset(key),
            kind: PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(10.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn holding() -> PortfolioHolding {
        PortfolioHolding {
            asset: asset("Binance:BTCUSDT"),
            quantity: 0.5,
            cost_basis: 100.0,
            average_cost: 200.0,
            realized_pnl: -3.0,
            current_price: Some(260.0),
            previous_day_price: None,
            current_value: Some(130.0),
            unrealized_pnl: Some(30.0),
            allocation: 0.25,
            is_price_stale: false,
        }
    }

    #[test]
    fn an_assets_transactions_list_newest_first() {
        let ledger = vec![
            transaction(1, "Binance:BTCUSDT", 100),
            transaction(2, "Binance:ETHUSDT", 500),
            transaction(3, "Binance:BTCUSDT", 300),
        ];
        let ids: Vec<u64> = asset_transactions(&ledger, "Binance:BTCUSDT")
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, [3, 1]);
        assert!(asset_transactions(&ledger, "none").is_empty());
    }

    #[test]
    fn the_position_cards_read_like_the_holdings_table() {
        let f = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let holding = holding();
        let cards = stat_cards(&holding, f);
        let titles: Vec<&str> = cards.iter().map(|c| c.title).collect();
        assert_eq!(
            titles,
            [
                "Holdings",
                "Current Value",
                "Average Cost",
                "Current Price",
                "Unrealized P&L",
                "Realized P&L",
                "Total P&L",
                "Allocation"
            ]
        );
        assert_eq!(cards[0].value, "0.5 BTC");
        assert_eq!(cards[1].value, "$130.00");
        assert_eq!(cards[2].value, "$200.00");
        assert_eq!(cards[4].value, f.signed_money(30.0));
        assert_eq!(
            cards[6].value,
            f.signed_money(27.0),
            "30 unrealized − 3 realized"
        );
        assert!(cards[6].caption.is_some());
        assert_eq!(cards[7].value, "25.00%");
        assert!(!cards[1].placeholder);
    }

    #[test]
    fn missing_prices_read_unavailable_and_privacy_hides_the_ticker_suffix() {
        let f = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let mut unpriced = holding();
        unpriced.current_price = None;
        unpriced.current_value = None;
        unpriced.unrealized_pnl = None;
        let cards = stat_cards(&unpriced, f);
        assert_eq!(cards[1].value, "Unavailable");
        assert!(cards[1].placeholder && cards[3].placeholder);
        assert_eq!(cards[4].value, "—");
        assert_eq!(cards[6].value, "—");
        let shown = holding();
        let masked = stat_cards(
            &shown,
            PortfolioValueFormatter::new(PortfolioCurrency::Usd, true),
        );
        assert!(!masked[0].value.contains("BTC"), "{}", masked[0].value);
        assert_eq!(values_note("EUR"), "Values in EUR");
        assert!(NO_POSITION.starts_with("No open position"));
    }
}
