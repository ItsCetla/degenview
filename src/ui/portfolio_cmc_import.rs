//! First step of a CoinMarketCap import: match each exported ticker to a market the app can
//! price, resolve foreign-currency fees, then hand the converted transactions to the preview
//! sheet. A pure view over `CmcImportState`; the host does the slow work (searching exchanges,
//! fetching historical FX rates, the "Match…" asset picker) and feeds results back through the
//! state's methods.

// Transient: the portfolio host opens this once the importers are wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency, PortfolioTransaction};
use crate::model::portfolio_csv::{CmcPreview, CmcRow, CsvPreview, Mint};
use crate::ui::portfolio_import_preview::{plural, warning_title};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};

pub const SIZE: Vec2 = Vec2::new(700.0, 640.0);

#[derive(Default)]
pub struct CmcImportState {
    /// Ticker → the market it maps to.
    pub mappings: HashMap<String, PortfolioAsset>,
    pub skipped_symbols: HashSet<String>,
    pub auto_mapped: HashSet<String>,
    /// Tickers the host is searching for right now.
    pub resolving: HashSet<String>,
    pub did_auto_map: bool,
    /// Row id → USD per unit of the row's fee currency, on the row's date.
    pub fee_fx_rates: HashMap<String, f64>,
    pub fee_fx_errors: HashMap<String, String>,
    pub is_resolving_fee_fx: bool,
    pub fee_fx_progress: f32,
    pub skipped_row_ids: HashSet<String>,
}

impl CmcImportState {
    // MARK: Counts

    pub fn mapped_rows(&self, preview: &CmcPreview) -> usize {
        preview
            .rows
            .iter()
            .filter(|row| {
                self.mappings.contains_key(&row.token) && !self.skipped_row_ids.contains(&row.id)
            })
            .count()
    }

    pub fn skipped_rows(&self, preview: &CmcPreview) -> usize {
        preview.rows.len() - self.mapped_rows(preview)
    }

    /// Rows that import and carry a fee in a currency other than USD.
    pub fn foreign_fee_rows<'a>(&self, preview: &'a CmcPreview) -> Vec<&'a CmcRow> {
        preview
            .rows
            .iter()
            .filter(|row| {
                self.mappings.contains_key(&row.token)
                    && !self.skipped_row_ids.contains(&row.id)
                    && row.fee != 0.0
                    && row.fee_currency.unwrap_or(PortfolioCurrency::Usd) != PortfolioCurrency::Usd
            })
            .collect()
    }

    pub fn can_continue(&self, preview: &CmcPreview) -> bool {
        preview.is_valid()
            && self.mapped_rows(preview) > 0
            && self.did_auto_map
            && !self.is_resolving_fee_fx
            && self
                .foreign_fee_rows(preview)
                .iter()
                .all(|row| self.fee_fx_rates.contains_key(&row.id))
    }

    // MARK: Mapping

    /// Reuses an asset the ledger already holds under this symbol, when there is exactly one.
    pub fn apply_existing_mappings(
        &mut self,
        preview: &CmcPreview,
        ledger: &[PortfolioTransaction],
    ) {
        for symbol in preview.symbols() {
            let mut assets: Vec<&PortfolioAsset> = Vec::new();
            for transaction in ledger {
                let asset = &transaction.asset;
                if asset.symbol.eq_ignore_ascii_case(&symbol)
                    && !assets.iter().any(|known| known.key == asset.key)
                {
                    assets.push(asset);
                }
            }
            if let [asset] = assets.as_slice() {
                self.mappings.insert(symbol, (*asset).clone());
            }
        }
    }

    /// Tickers still to search for: not mapped, not skipped.
    pub fn symbols_to_auto_map(&self, preview: &CmcPreview) -> Vec<String> {
        preview
            .symbols()
            .into_iter()
            .filter(|symbol| {
                !self.mappings.contains_key(symbol) && !self.skipped_symbols.contains(symbol)
            })
            .collect()
    }

    /// The host found a market for `symbol`. Ignored when the user mapped or skipped it meanwhile.
    pub fn auto_mapped(&mut self, symbol: &str, asset: PortfolioAsset) {
        if !self.mappings.contains_key(symbol) && !self.skipped_symbols.contains(symbol) {
            self.mappings.insert(symbol.to_owned(), asset);
            self.auto_mapped.insert(symbol.to_owned());
        }
    }

    /// The user picked a market in the "Match…" sheet.
    pub fn set_mapping(&mut self, symbol: &str, asset: PortfolioAsset) {
        self.mappings.insert(symbol.to_owned(), asset);
        self.skipped_symbols.remove(symbol);
        self.auto_mapped.remove(symbol);
    }

    pub fn skip(&mut self, symbol: &str) {
        self.skipped_symbols.insert(symbol.to_owned());
    }

    pub fn reset(&mut self, symbol: &str) {
        self.mappings.remove(symbol);
        self.skipped_symbols.remove(symbol);
        self.auto_mapped.remove(symbol);
    }

    // MARK: Fee rates

    /// Rows whose fee needs a historical rate.
    pub fn fee_rows_to_resolve<'a>(&self, preview: &'a CmcPreview) -> Vec<&'a CmcRow> {
        preview
            .rows
            .iter()
            .filter(|row| {
                row.fee != 0.0
                    && row.fee_currency.unwrap_or(PortfolioCurrency::Usd) != PortfolioCurrency::Usd
            })
            .collect()
    }

    pub fn fee_rate_resolved(&mut self, row_id: &str, result: Result<f64, String>) {
        match result {
            Ok(rate) => {
                self.fee_fx_rates.insert(row_id.to_owned(), rate);
                self.fee_fx_errors.remove(row_id);
            }
            Err(message) => {
                self.fee_fx_errors.insert(row_id.to_owned(), message);
            }
        }
    }

    // MARK: Result

    /// The converted transactions for the preview sheet.
    pub fn result(&self, preview: &CmcPreview, portfolio_id: u64, mint: &mut Mint) -> CsvPreview {
        preview.transactions(
            portfolio_id,
            &self.mappings,
            &self.fee_fx_rates,
            &self.skipped_row_ids,
            mint,
        )
    }
}

pub struct CmcImportInput<'a> {
    pub preview: &'a CmcPreview,
    pub portfolio_currency: PortfolioCurrency,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CmcImportAction {
    /// Open the asset picker for this ticker; answer with `CmcImportState::set_mapping`.
    Match(String),
    /// Convert and open the preview sheet (`CmcImportState::result`).
    Continue,
    Cancel,
}

// MARK: Pure text

pub fn subtitle(preview: &CmcPreview) -> String {
    format!(
        "{} · {}",
        plural(preview.rows.len(), "transaction", "transactions"),
        plural(preview.symbols().len(), "asset", "assets")
    )
}

pub fn csv_error_title(count: usize) -> String {
    format!(
        "{count} CSV {}",
        if count == 1 {
            "error requires attention"
        } else {
            "errors require attention"
        }
    )
}

pub fn search_note(currency: PortfolioCurrency) -> String {
    format!(
        "Finding {} markets — Binance, then Coinbase, CoinGecko and DEXScreener…",
        currency.code()
    )
}

pub fn restore_title(count: usize) -> String {
    format!(
        "Restore {count} individually skipped {}",
        if count == 1 {
            "transaction"
        } else {
            "transactions"
        }
    )
}

pub fn fee_row_title(row: &CmcRow) -> String {
    format!(
        "Line {} · {} · {} {}",
        row.line,
        row.token,
        row.fee,
        row.fee_currency.map_or("", |currency| currency.code())
    )
}

pub fn progress_text(progress: f32) -> String {
    format!("{}%", (progress * 100.0).round() as i64)
}

/// What a ticker row says about its mapping.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenStatus {
    Resolving,
    Mapped {
        ticker: String,
        detail: String,
        auto: bool,
    },
    Skipped,
    NotMatched,
}

pub fn token_status(state: &CmcImportState, symbol: &str) -> TokenStatus {
    if state.resolving.contains(symbol) {
        return TokenStatus::Resolving;
    }
    if let Some(asset) = state.mappings.get(symbol) {
        let name = if asset.name.is_empty() {
            &asset.symbol
        } else {
            &asset.name
        };
        return TokenStatus::Mapped {
            ticker: asset.display_ticker(),
            detail: format!("{name} · {}", asset.source.display_name()),
            auto: state.auto_mapped.contains(symbol),
        };
    }
    if state.skipped_symbols.contains(symbol) {
        TokenStatus::Skipped
    } else {
        TokenStatus::NotMatched
    }
}

// MARK: View

pub fn show(
    ctx: &egui::Context,
    state: &mut CmcImportState,
    input: &CmcImportInput,
) -> Vec<CmcImportAction> {
    let mut actions = Vec::new();
    let preview = input.preview;
    egui::Window::new("Import from CoinMarketCap")
        .id(Id::new("portfolio-cmc-import"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(SIZE)
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Import from CoinMarketCap").heading());
            ui.label(RichText::new(subtitle(preview)).weak());
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .max_height(SIZE.y - 150.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    body(ui, state, input, &mut actions);
                });
            ui.separator();
            footer(ui, state, preview, &mut actions);
        });
    if actions.is_empty() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        actions.push(CmcImportAction::Cancel);
    }
    actions
}

fn notice(ui: &mut egui::Ui, tint: Color32, title: &str, lines: &[String]) {
    egui::Frame::new()
        .fill(tint.gamma_multiply(0.12))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(tint));
            for line in lines {
                ui.label(RichText::new(format!("• {line}")).small());
            }
        });
    ui.add_space(8.0);
}

fn body(
    ui: &mut egui::Ui,
    state: &mut CmcImportState,
    input: &CmcImportInput,
    actions: &mut Vec<CmcImportAction>,
) {
    let preview = input.preview;
    if !preview.errors.is_empty() {
        notice(
            ui,
            theme::DOWN,
            &format!("✕ {}", csv_error_title(preview.errors.len())),
            &preview.errors,
        );
    }
    tokens_section(ui, state, input, actions);
    if !state.foreign_fee_rows(preview).is_empty() {
        ui.add_space(14.0);
        fee_section(ui, state, preview);
    }
    if !state.skipped_row_ids.is_empty() {
        ui.add_space(8.0);
        if ui
            .link(RichText::new(restore_title(state.skipped_row_ids.len())).small())
            .clicked()
        {
            state.skipped_row_ids.clear();
        }
    }
    if !preview.warnings.is_empty() {
        ui.add_space(10.0);
        notice(
            ui,
            theme::REPLAY,
            &format!("⚠ {}", warning_title(preview.warnings.len())),
            &preview.warnings,
        );
    }
}

fn tokens_section(
    ui: &mut egui::Ui,
    state: &mut CmcImportState,
    input: &CmcImportInput,
    actions: &mut Vec<CmcImportAction>,
) {
    ui.label(RichText::new("Match tokens to markets").strong().size(15.0));
    ui.label(
        RichText::new(
            "Only transactions for matched tickers are imported. Skip a ticker to leave all of \
             its transactions out.",
        )
        .small()
        .weak(),
    );
    if !state.did_auto_map {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                RichText::new(search_note(input.portfolio_currency))
                    .small()
                    .weak(),
            );
        });
    }
    ui.add_space(6.0);
    for symbol in input.preview.symbols() {
        let count = input
            .preview
            .rows
            .iter()
            .filter(|row| row.token == symbol)
            .count();
        token_row(ui, state, &symbol, count, actions);
        ui.add_space(4.0);
    }
}

fn token_row(
    ui: &mut egui::Ui,
    state: &mut CmcImportState,
    symbol: &str,
    count: usize,
    actions: &mut Vec<CmcImportAction>,
) {
    let status = token_status(state, symbol);
    egui::Frame::new()
        .fill(Color32::from_white_alpha(9))
        .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.5)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                monogram(ui, symbol);
                ui.vertical(|ui| {
                    ui.set_width(120.0);
                    ui.label(RichText::new(symbol).strong());
                    ui.label(
                        RichText::new(plural(count, "transaction", "transactions"))
                            .small()
                            .weak(),
                    );
                });
                ui.label(RichText::new("→").weak());
                ui.vertical(|ui| {
                    ui.set_width(240.0);
                    status_view(ui, &status);
                });
                if status != TokenStatus::Resolving {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if state.mappings.contains_key(symbol) {
                            "Change…"
                        } else {
                            "Match…"
                        };
                        if ui.small_button(label).clicked() {
                            state.skipped_symbols.remove(symbol);
                            state.auto_mapped.remove(symbol);
                            actions.push(CmcImportAction::Match(symbol.to_owned()));
                        }
                        let unmapped = !state.mappings.contains_key(symbol);
                        let skipped = state.skipped_symbols.contains(symbol);
                        if unmapped && !skipped && ui.small_button("Skip").clicked() {
                            state.skip(symbol);
                        }
                        if (!unmapped || skipped) && ui.small_button("Reset").clicked() {
                            state.reset(symbol);
                        }
                    });
                }
            });
        });
}

fn monogram(ui: &mut egui::Ui, symbol: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 16.0, theme::ACCENT.gamma_multiply(0.2));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        symbol
            .chars()
            .next()
            .map_or(String::new(), |c| c.to_string()),
        egui::FontId::proportional(14.0),
        theme::ACCENT,
    );
}

fn small_pill(ui: &mut egui::Ui, text: &str, tint: Color32) {
    egui::Frame::new()
        .fill(tint.gamma_multiply(0.14))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).strong().color(tint));
        });
}

fn status_view(ui: &mut egui::Ui, status: &TokenStatus) {
    match status {
        TokenStatus::Resolving => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Searching exchanges…").weak());
            });
        }
        TokenStatus::Mapped {
            ticker,
            detail,
            auto,
        } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ticker).strong());
                if *auto {
                    small_pill(ui, "Auto-matched", Color32::from_rgb(10, 132, 255));
                }
            });
            ui.add(egui::Label::new(RichText::new(detail).small().weak()).truncate());
        }
        TokenStatus::Skipped => {
            small_pill(ui, "Skipped", Color32::from_gray(150));
            ui.label(RichText::new("Won't be imported").small().weak());
        }
        TokenStatus::NotMatched => {
            small_pill(ui, "Not matched", theme::REPLAY);
            ui.label(RichText::new("Skipped unless you match it").small().weak());
        }
    }
}

fn fee_section(ui: &mut egui::Ui, state: &mut CmcImportState, preview: &CmcPreview) {
    let rows = state.foreign_fee_rows(preview);
    let failed: Vec<(String, String, String)> = rows
        .iter()
        .filter_map(|row| {
            state
                .fee_fx_errors
                .get(&row.id)
                .map(|error| (row.id.clone(), fee_row_title(row), error.clone()))
        })
        .collect();
    egui::Frame::new()
        .fill(Color32::from_white_alpha(9))
        .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.5)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("⇄ Fee exchange rates").strong().size(15.0));
            if state.is_resolving_fee_fx {
                ui.add(
                    egui::ProgressBar::new(state.fee_fx_progress)
                        .text(progress_text(state.fee_fx_progress)),
                );
                ui.label(
                    RichText::new("Fetching historical exchange rates…")
                        .small()
                        .weak(),
                );
            } else if failed.is_empty() {
                ui.label(
                    RichText::new("✓ Historical exchange rates loaded")
                        .small()
                        .color(theme::UP),
                );
            }
            for (id, title, error) in failed {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(title);
                        ui.label(RichText::new(error).small().color(theme::DOWN));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Skip Item").clicked() {
                            state.skipped_row_ids.insert(id);
                        }
                    });
                });
            }
        });
}

fn footer(
    ui: &mut egui::Ui,
    state: &CmcImportState,
    preview: &CmcPreview,
    actions: &mut Vec<CmcImportAction>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("✓ {} to import", state.mapped_rows(preview))).color(theme::UP),
        );
        let skipped = state.skipped_rows(preview);
        if skipped > 0 {
            ui.label(RichText::new(format!("⊖ {skipped} skipped")).weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = egui::Button::new(
                RichText::new("Continue to Preview")
                    .strong()
                    .color(Color32::WHITE),
            )
            .fill(theme::ACCENT);
            if ui
                .add_enabled(state.can_continue(preview), button)
                .clicked()
            {
                actions.push(CmcImportAction::Continue);
            }
            if ui.button("Cancel").clicked() {
                actions.push(CmcImportAction::Cancel);
            }
        });
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio_csv::preview_coinmarketcap;

    const CMC: &str = "Date (UTC),Token,Type,Price,Amount,Total value,Fee,Fee currency,Notes\n\
        2024-03-05 14:30:00,BTC,Buy,60000,0.5,30000,--,,\n\
        2024-03-06 10:00:00,ETH,Buy,3000,2,6000,1.5,EUR,\n\
        2024-03-07 10:00:00,ETH,Sell,3000,1,3000,0,,\n\
        2024-03-08 10:00:00,DOGE,Buy,0.1,100,10,0,,";

    fn asset(symbol: &str, key: &str) -> PortfolioAsset {
        PortfolioAsset {
            key: key.into(),
            symbol: symbol.into(),
            name: format!("{symbol} coin"),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn ready() -> (CmcPreview, CmcImportState) {
        let preview = preview_coinmarketcap(CMC);
        assert!(preview.is_valid(), "{:?}", preview.errors);
        let state = CmcImportState {
            did_auto_map: true,
            ..CmcImportState::default()
        };
        (preview, state)
    }

    #[test]
    fn nothing_continues_until_something_is_mapped() {
        let (preview, mut state) = ready();
        assert!(!state.can_continue(&preview));
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert_eq!(state.mapped_rows(&preview), 1);
        assert_eq!(state.skipped_rows(&preview), 3);
        assert!(state.can_continue(&preview));
        state.did_auto_map = false;
        assert!(!state.can_continue(&preview), "still searching");
        state.did_auto_map = true;
        let mut invalid = preview.clone();
        invalid.errors.push("bad".into());
        assert!(!state.can_continue(&invalid));
    }

    #[test]
    fn a_foreign_fee_holds_the_button_until_its_rate_arrives() {
        let (preview, mut state) = ready();
        state.set_mapping("ETH", asset("ETH", "Binance:ETHUSDT"));
        let rows = state.foreign_fee_rows(&preview);
        assert_eq!(rows.len(), 1);
        let id = rows[0].id.clone();
        assert!(!state.can_continue(&preview));
        state.fee_rate_resolved(&id, Err("offline".into()));
        assert!(!state.can_continue(&preview));
        assert_eq!(state.fee_fx_errors[&id], "offline");
        state.is_resolving_fee_fx = true;
        state.fee_rate_resolved(&id, Ok(1.08));
        assert!(!state.can_continue(&preview), "still resolving others");
        assert!(state.fee_fx_errors.is_empty(), "a rate clears the error");
        state.is_resolving_fee_fx = false;
        assert!(state.can_continue(&preview));
        // Skipping the item instead of fetching its rate also unblocks.
        state.fee_fx_rates.clear();
        state.skipped_row_ids.insert(id);
        assert!(state.can_continue(&preview));
        assert_eq!(
            state.fee_rows_to_resolve(&preview).len(),
            1,
            "all foreign-fee rows get a lookup"
        );
        assert_eq!(
            fee_row_title(state.fee_rows_to_resolve(&preview)[0]),
            "Line 3 · ETH · 1.5 EUR"
        );
    }

    #[test]
    fn mapping_actions_keep_the_sets_consistent() {
        let (_, mut state) = ready();
        state.auto_mapped("DOGE", asset("DOGE", "Binance:DOGEUSDT"));
        assert!(state.auto_mapped.contains("DOGE"));
        state.skip("BTC");
        state.auto_mapped("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert!(
            !state.mappings.contains_key("BTC"),
            "the user's skip wins over a late search result"
        );
        state.set_mapping("DOGE", asset("DOGE", "CoinGecko:dogecoin"));
        assert!(
            !state.auto_mapped.contains("DOGE"),
            "a manual pick is no longer auto"
        );
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        assert!(
            !state.skipped_symbols.contains("BTC"),
            "matching clears a skip"
        );
        state.reset("BTC");
        assert!(!state.mappings.contains_key("BTC") && state.skipped_symbols.is_empty());
        state.auto_mapped("DOGE", asset("DOGE", "x"));
        assert_eq!(
            state.mappings["DOGE"].key, "CoinGecko:dogecoin",
            "an existing mapping is not replaced"
        );
    }

    #[test]
    fn tickers_the_ledger_already_holds_map_themselves() {
        let (preview, mut state) = ready();
        let held =
            |key: &str, symbol: &str| {
                let mut transaction = crate::model::portfolio_csv::preview(
                &format!(
                    "{}\nMain,{key},{symbol},,Binance,Buy,1,1,USD,0,USD,2024-01-01T00:00:00Z,,x",
                    crate::model::portfolio_csv::HEADER
                ),
                &[crate::model::portfolio::Portfolio::new(1, "Main", 0)],
                &mut Mint { now_ms: 0, next_id: &mut || 1 },
            )
            .transactions;
                transaction.remove(0)
            };
        let ledger = vec![
            held("Binance:BTCUSDT", "btc"),
            held("Binance:BTCUSDT", "BTC"),
            held("Binance:ETHUSDT", "ETH"),
            held("Coinbase:ETH-USD", "ETH"),
        ];
        state.apply_existing_mappings(&preview, &ledger);
        assert_eq!(
            state.mappings["BTC"].key, "Binance:BTCUSDT",
            "one asset under the symbol"
        );
        assert!(
            !state.mappings.contains_key("ETH"),
            "two candidates: the user decides"
        );
        assert_eq!(state.symbols_to_auto_map(&preview), ["DOGE", "ETH"]);
    }

    #[test]
    fn rows_convert_through_the_preview_with_the_chosen_mappings() {
        let (preview, mut state) = ready();
        state.set_mapping("BTC", asset("BTC", "Binance:BTCUSDT"));
        let mut next = 10;
        let mut ids = || {
            next += 1;
            next
        };
        let result = state.result(
            &preview,
            4,
            &mut Mint {
                now_ms: 7,
                next_id: &mut ids,
            },
        );
        assert!(result.is_valid());
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.transactions[0].portfolio_id, 4);
        assert_eq!(result.transactions[0].id, 11);
    }

    #[test]
    fn rows_read_in_words() {
        let (preview, mut state) = ready();
        assert_eq!(subtitle(&preview), "4 transactions · 3 assets");
        assert_eq!(csv_error_title(1), "1 CSV error requires attention");
        assert_eq!(csv_error_title(2), "2 CSV errors require attention");
        assert_eq!(
            restore_title(1),
            "Restore 1 individually skipped transaction"
        );
        assert_eq!(progress_text(0.456), "46%");
        assert_eq!(
            search_note(PortfolioCurrency::Eur),
            "Finding EUR markets — Binance, then Coinbase, CoinGecko and DEXScreener…"
        );
        assert_eq!(token_status(&state, "BTC"), TokenStatus::NotMatched);
        state.skip("BTC");
        assert_eq!(token_status(&state, "BTC"), TokenStatus::Skipped);
        state.resolving.insert("ETH".into());
        assert_eq!(token_status(&state, "ETH"), TokenStatus::Resolving);
        state.auto_mapped("DOGE", asset("DOGE", "Binance:DOGEUSDT"));
        assert_eq!(
            token_status(&state, "DOGE"),
            TokenStatus::Mapped {
                ticker: "DOGE".into(),
                detail: "DOGE coin · Binance".into(),
                auto: true
            }
        );
    }
}
