//! Stateful bridge between the portfolio ledger, SQLite, and the pure portfolio views.
//!
//! Mutations are copy-on-write: validate against a candidate ledger, persist its complete
//! snapshot, and publish it only after SQLite succeeds. A failed initial load permanently blocks
//! writes for this host instance so corrupt data can never be replaced by an empty snapshot.
#![allow(dead_code)]

use crate::model::portfolio::{
    Portfolio, PortfolioCurrency, PortfolioError, PortfolioHoldingsSort, PortfolioLedgerSnapshot,
    PortfolioSnapshot, PortfolioTransaction,
};
use crate::model::portfolio_accounting::holdings;
use crate::model::portfolio_format::{PORTFOLIO_CURRENCIES, PortfolioValueFormatter};
use crate::model::portfolio_ledger::PortfolioLedger;
use crate::store::SharedStore;
use crate::ui::portfolio_holdings::{self, HoldingsAction, HoldingsInput, HoldingsState};
use crate::ui::portfolio_overview::{
    self, OverviewAction, OverviewInput, OverviewState, StatisticsInput,
};
use crate::ui::portfolio_sheets::{
    self, CreateOutcome, CreateState, ManageAction, ManageInput, ManageRow, ManageState,
};
use crate::ui::portfolio_transactions::{
    self, Projection, TransactionAction, TransactionsInput, TransactionsState,
};
use crate::ui::theme;
use eframe::egui::{self, Align2, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum PortfolioHostError {
    Load(String),
    Mutation(PortfolioError),
    Persistence(String),
    StoreUnavailable,
}

impl PortfolioHostError {
    pub fn message(&self) -> String {
        match self {
            Self::Load(message) => format!("Could not load portfolios: {message}"),
            Self::Mutation(error) => error.message(),
            Self::Persistence(message) => format!("Could not save portfolios: {message}"),
            Self::StoreUnavailable => "The portfolio database is unavailable.".to_owned(),
        }
    }
}

impl fmt::Display for PortfolioHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PortfolioHostError {}

pub struct PortfolioHost {
    store: SharedStore,
    ledger: PortfolioLedger,
    load_error: Option<PortfolioHostError>,
    last_error: Option<PortfolioHostError>,
}

impl PortfolioHost {
    pub fn load(store: SharedStore) -> Self {
        let loaded = store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .portfolio_ledger()
                    .map_err(|error| PortfolioHostError::Load(error.to_string()))
            });
        match loaded {
            Ok(snapshot) => Self {
                store,
                ledger: PortfolioLedger::new(snapshot),
                load_error: None,
                last_error: None,
            },
            Err(error) => Self {
                store,
                ledger: PortfolioLedger::new(PortfolioLedgerSnapshot::default()),
                load_error: Some(error.clone()),
                last_error: Some(error),
            },
        }
    }

    pub fn snapshot(&self) -> &PortfolioLedgerSnapshot {
        self.ledger.snapshot()
    }

    pub fn load_failed(&self) -> bool {
        self.load_error.is_some()
    }

    pub fn last_error(&self) -> Option<&PortfolioHostError> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn create_portfolio(
        &mut self,
        name: &str,
        currency: PortfolioCurrency,
    ) -> Result<u64, PortfolioHostError> {
        self.mutate(|ledger| ledger.create_portfolio(name, currency))
    }

    pub fn select(&mut self, id: Option<u64>) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.select(id))
    }

    pub fn update_portfolio(&mut self, portfolio: Portfolio) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.update_portfolio(portfolio))
    }

    pub fn duplicate_portfolio(&mut self, id: u64) -> Result<u64, PortfolioHostError> {
        self.mutate(|ledger| ledger.duplicate_portfolio(id))
    }

    pub fn reorder(
        &mut self,
        from: &[usize],
        destination: usize,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| {
            ledger.reorder(from, destination);
            Ok(())
        })
    }

    pub fn delete_portfolio(&mut self, id: u64) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.delete_portfolio(id))
    }

    pub fn add_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.add_transaction(transaction))
    }

    pub fn update_transaction(
        &mut self,
        transaction: PortfolioTransaction,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.update_transaction(transaction))
    }

    pub fn delete_transaction(&mut self, id: u64) -> Result<(), PortfolioHostError> {
        self.mutate(|ledger| ledger.delete_transaction(id))
    }

    pub fn import_transactions(
        &mut self,
        transactions: Vec<PortfolioTransaction>,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.import_transactions(transactions))
    }

    pub fn store_snapshots(
        &mut self,
        snapshots: Vec<PortfolioSnapshot>,
        portfolio_id: u64,
        from: i64,
    ) -> Result<(), PortfolioHostError> {
        self.mutate(move |ledger| ledger.store_snapshots(snapshots, portfolio_id, from))
    }

    fn mutate<T>(
        &mut self,
        mutation: impl FnOnce(&mut PortfolioLedger) -> Result<T, PortfolioError>,
    ) -> Result<T, PortfolioHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

        let mut candidate = PortfolioLedger::new(self.ledger.snapshot().clone());
        let value = match mutation(&mut candidate) {
            Ok(value) => value,
            Err(error) => return self.fail(PortfolioHostError::Mutation(error)),
        };
        let saved = self
            .store
            .lock()
            .map_err(|_| PortfolioHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .save_portfolio_ledger(candidate.snapshot())
                    .map_err(|error| PortfolioHostError::Persistence(error.to_string()))
            });
        if let Err(error) = saved {
            return self.fail(error);
        }

        self.ledger = candidate;
        self.last_error = None;
        Ok(value)
    }

    fn fail<T>(&mut self, error: PortfolioHostError) -> Result<T, PortfolioHostError> {
        self.last_error = Some(error.clone());
        Err(error)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PortfolioSection {
    #[default]
    Overview,
    Holdings,
    Transactions,
    Statistics,
}

impl PortfolioSection {
    const ALL: [Self; 4] = [
        Self::Overview,
        Self::Holdings,
        Self::Transactions,
        Self::Statistics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Holdings => "Holdings",
            Self::Transactions => "Transactions",
            Self::Statistics => "Statistics",
        }
    }
}

/// Per-window state for the dedicated Portfolio tab. The ledger itself stays app-wide in
/// `PortfolioHost`, so opening/focusing the tab never creates a second source of truth.
pub struct PortfolioTab {
    id: u64,
    section: PortfolioSection,
    reporting_currency: PortfolioCurrency,
    privacy: bool,
    overview: OverviewState,
    holdings: HoldingsState,
    transactions: TransactionsState,
    create: Option<CreateState>,
    manage: Option<ManageState>,
    pending_delete_transaction: Option<u64>,
    notice: Option<String>,
}

impl PortfolioTab {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            section: PortfolioSection::Overview,
            reporting_currency: PortfolioCurrency::Usd,
            privacy: false,
            overview: OverviewState::default(),
            holdings: HoldingsState::default(),
            transactions: TransactionsState::default(),
            create: None,
            manage: None,
            pending_delete_transaction: None,
            notice: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn title(&self) -> &'static str {
        "DegenView — Portfolio"
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, host: &mut PortfolioHost) {
        let ctx = ui.ctx().clone();
        let snapshot = host.snapshot().clone();
        let selected = snapshot.selected_portfolio_id;

        egui::Panel::top(egui::Id::new(("portfolio-header", self.id)))
            .exact_size(58.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .show(ui, |ui| self.show_header(ui, host, &snapshot));

        egui::Panel::top(egui::Id::new(("portfolio-sections", self.id)))
            .exact_size(48.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for section in PortfolioSection::ALL {
                        ui.selectable_value(&mut self.section, section, section.label());
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::same(8)),
            )
            .show(ui, |ui| {
                if snapshot.portfolios.is_empty() {
                    empty_state(
                        ui,
                        "No Portfolios",
                        "Create a portfolio to start tracking investments.",
                    );
                    ui.vertical_centered(|ui| {
                        if ui.button("Create Portfolio").clicked() {
                            self.create = Some(CreateState::default());
                        }
                    });
                    return;
                }
                if snapshot.transactions.is_empty() {
                    empty_state(
                        ui,
                        "Your portfolio is empty",
                        "Add a transaction or import a CSV to begin tracking holdings.",
                    );
                    return;
                }
                self.show_content(ui, host, &snapshot, selected);
            });

        self.show_sheets(&ctx, host);
        self.show_error(&ctx, host);
    }

    fn show_header(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
    ) {
        ui.horizontal_centered(|ui| {
            let selected_name = snapshot
                .selected_portfolio_id
                .and_then(|id| {
                    snapshot
                        .portfolios
                        .iter()
                        .find(|portfolio| portfolio.id == id)
                })
                .map_or("All Portfolios", |portfolio| portfolio.name.as_str());
            egui::ComboBox::from_id_salt(("portfolio-selector", self.id))
                .selected_text(format!("▣  {selected_name}"))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            snapshot.selected_portfolio_id.is_none(),
                            "All Portfolios",
                        )
                        .clicked()
                    {
                        let _ = host.select(None);
                    }
                    ui.separator();
                    for portfolio in snapshot
                        .portfolios
                        .iter()
                        .filter(|portfolio| !portfolio.is_archived)
                    {
                        if ui
                            .selectable_label(
                                snapshot.selected_portfolio_id == Some(portfolio.id),
                                &portfolio.name,
                            )
                            .clicked()
                        {
                            self.reporting_currency = portfolio.base_currency;
                            let _ = host.select(Some(portfolio.id));
                        }
                    }
                    ui.separator();
                    if ui.button("Create Portfolio…").clicked() {
                        self.create = Some(CreateState::default());
                    }
                    if ui.button("Manage Portfolios…").clicked() {
                        self.manage = Some(ManageState::default());
                    }
                });

            egui::ComboBox::from_id_salt(("reporting-currency", self.id))
                .selected_text(self.reporting_currency.code())
                .show_ui(ui, |ui| {
                    for currency in PORTFOLIO_CURRENCIES {
                        ui.selectable_value(
                            &mut self.reporting_currency,
                            currency,
                            format!("{}  {}", currency.glyph(), currency.code()),
                        );
                    }
                });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("＋ Add Transaction").clicked() {
                    self.notice = Some(
                        "The transaction editor is waiting for its Stage 7 UI contract.".into(),
                    );
                }
                if ui
                    .button(if self.privacy {
                        "Reveal"
                    } else {
                        "Hide values"
                    })
                    .clicked()
                {
                    self.privacy = !self.privacy;
                }
            });
        });
    }

    fn show_content(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut PortfolioHost,
        snapshot: &PortfolioLedgerSnapshot,
        selected: Option<u64>,
    ) {
        let portfolio_ids: HashSet<u64> = match selected {
            Some(id) => [id].into_iter().collect(),
            None => snapshot
                .portfolios
                .iter()
                .filter(|portfolio| !portfolio.is_archived)
                .map(|portfolio| portfolio.id)
                .collect(),
        };
        let now_ms = now_ms();
        let holding_values = holdings(
            &snapshot.transactions,
            &portfolio_ids,
            None,
            &HashMap::new(),
            now_ms,
        )
        .unwrap_or_default();
        let transactions: Vec<PortfolioTransaction> = snapshot
            .transactions
            .iter()
            .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
            .cloned()
            .collect();
        let history: Vec<PortfolioSnapshot> = selected.map_or_else(Vec::new, |id| {
            snapshot
                .historical_snapshots
                .iter()
                .filter(|point| point.portfolio_id == id)
                .cloned()
                .collect()
        });
        let selected_portfolio = selected.and_then(|id| {
            snapshot
                .portfolios
                .iter()
                .find(|portfolio| portfolio.id == id)
        });
        let formatter = PortfolioValueFormatter::new(self.reporting_currency, self.privacy);
        let subtitle = |asset: &crate::model::portfolio::PortfolioAsset| {
            if asset.name.is_empty() {
                asset.embedded_name().unwrap_or(&asset.symbol).to_owned()
            } else {
                asset.name.clone()
            }
        };
        let total_value: f64 = holding_values
            .iter()
            .filter_map(|holding| holding.current_value)
            .sum();

        match self.section {
            PortfolioSection::Overview => {
                let actions = portfolio_overview::show_overview(
                    ui,
                    &mut self.overview,
                    &OverviewInput {
                        portfolio_name: selected_portfolio.map(|portfolio| portfolio.name.as_str()),
                        holdings: &holding_values,
                        history: &history,
                        total_value,
                        formatter,
                        subtitle: &subtitle,
                        market_value_caption: "Live portfolio quotes are not connected yet",
                        is_updating: false,
                        is_loading_initial: false,
                        is_loading_history: false,
                        now_ms,
                        utc_offset_secs: 0,
                    },
                );
                for action in actions {
                    match action {
                        OverviewAction::Select(key) => {
                            self.transactions.filters.asset_key = Some(key);
                            self.section = PortfolioSection::Transactions;
                        }
                        OverviewAction::ShowHoldings => self.section = PortfolioSection::Holdings,
                    }
                }
            }
            PortfolioSection::Holdings => {
                let saved_sort = selected_portfolio
                    .map_or(PortfolioHoldingsSort::CurrentValue, |portfolio| {
                        portfolio.sort
                    });
                let actions = portfolio_holdings::show(
                    ui,
                    &mut self.holdings,
                    &HoldingsInput {
                        holdings: &holding_values,
                        saved_sort,
                        formatter,
                        subtitle: &subtitle,
                    },
                );
                for action in actions {
                    match action {
                        HoldingsAction::SetSort(sort) => {
                            if let Some(mut portfolio) = selected_portfolio.cloned() {
                                portfolio.sort = sort;
                                let _ = host.update_portfolio(portfolio);
                            }
                        }
                        HoldingsAction::Open(key) => {
                            self.transactions.filters.asset_key = Some(key);
                            self.section = PortfolioSection::Transactions;
                        }
                        HoldingsAction::Remap(_) => {
                            self.notice = Some("Asset remapping is not wired yet.".into())
                        }
                    }
                }
            }
            PortfolioSection::Transactions => {
                let projections: HashMap<u64, Projection> = transactions
                    .iter()
                    .map(|transaction| {
                        (
                            transaction.id,
                            Projection {
                                price: transaction.price,
                                fee: transaction.fee,
                            },
                        )
                    })
                    .collect();
                let projection = |id: u64| projections.get(&id).copied();
                let actions = portfolio_transactions::show(
                    ui,
                    &mut self.transactions,
                    &TransactionsInput {
                        ledger: &transactions,
                        projection: &projection,
                        subtitle: &subtitle,
                        formatter,
                        utc_offset_secs: 0,
                    },
                );
                for action in actions {
                    match action {
                        TransactionAction::Delete(id) => self.pending_delete_transaction = Some(id),
                        TransactionAction::Add
                        | TransactionAction::Edit(_)
                        | TransactionAction::Duplicate(_) => {
                            self.notice = Some(
                                "The transaction editor is waiting for its Stage 7 UI contract."
                                    .into(),
                            )
                        }
                    }
                }
            }
            PortfolioSection::Statistics => portfolio_overview::show_statistics(
                ui,
                &StatisticsInput {
                    holdings: &holding_values,
                    history: &history,
                    transactions: &transactions,
                    formatter,
                    now_ms,
                    utc_offset_secs: 0,
                },
            ),
        }
    }

    fn show_sheets(&mut self, ctx: &egui::Context, host: &mut PortfolioHost) {
        if let Some(state) = &mut self.create {
            let names: Vec<String> = host
                .snapshot()
                .portfolios
                .iter()
                .map(|portfolio| portfolio.name.clone())
                .collect();
            if let Some(outcome) = portfolio_sheets::show_create(ctx, state, &names) {
                match outcome {
                    CreateOutcome::Create { name, currency } => {
                        if host.create_portfolio(&name, currency).is_ok() {
                            self.reporting_currency = currency;
                            self.create = None;
                        }
                    }
                    CreateOutcome::Cancel => self.create = None,
                }
            }
        }

        if self.manage.is_some() {
            let snapshot = host.snapshot().clone();
            let rows: Vec<ManageRow<'_>> = snapshot
                .portfolios
                .iter()
                .map(|portfolio| ManageRow {
                    portfolio,
                    transactions: snapshot
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .count(),
                    assets: snapshot
                        .transactions
                        .iter()
                        .filter(|transaction| transaction.portfolio_id == portfolio.id)
                        .map(|transaction| transaction.asset.key.as_str())
                        .collect::<HashSet<_>>()
                        .len(),
                })
                .collect();
            let input = ManageInput {
                rows: &rows,
                current: snapshot.selected_portfolio_id,
            };
            let actions = portfolio_sheets::show_manage(
                ctx,
                self.manage.as_mut().expect("manage state exists"),
                &input,
            );
            for action in actions {
                match action {
                    ManageAction::Rename { id, name } => {
                        if let Some(mut portfolio) = snapshot
                            .portfolios
                            .iter()
                            .find(|portfolio| portfolio.id == id)
                            .cloned()
                        {
                            portfolio.name = name;
                            let _ = host.update_portfolio(portfolio);
                        }
                    }
                    ManageAction::Duplicate(id) => {
                        let _ = host.duplicate_portfolio(id);
                    }
                    ManageAction::Delete(id) => {
                        let _ = host.delete_portfolio(id);
                    }
                    ManageAction::Reorder { from, destination } => {
                        let _ = host.reorder(&from, destination);
                    }
                    ManageAction::New => self.create = Some(CreateState::default()),
                    ManageAction::Done => self.manage = None,
                }
            }
        }

        if let Some(id) = self.pending_delete_transaction {
            egui::Window::new("Delete transaction?")
                .id(egui::Id::new(("portfolio-delete-transaction", self.id)))
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(
                        "Holdings, cost basis, P&L, allocation, and later history will be recalculated.",
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.pending_delete_transaction = None;
                        }
                        if ui
                            .button(RichText::new("Delete").color(theme::DOWN))
                            .clicked()
                            && host.delete_transaction(id).is_ok()
                        {
                            self.pending_delete_transaction = None;
                        }
                    });
                });
        }
    }

    fn show_error(&mut self, ctx: &egui::Context, host: &mut PortfolioHost) {
        let message = host
            .last_error()
            .map(PortfolioHostError::message)
            .or_else(|| self.notice.clone());
        let Some(message) = message else { return };
        egui::Window::new("Portfolio")
            .id(egui::Id::new(("portfolio-error", self.id)))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(message);
                if ui.button("OK").clicked() {
                    host.clear_error();
                    self.notice = None;
                }
            });
    }
}

fn empty_state(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.add_space(120.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).strong().size(22.0));
        ui.label(RichText::new(detail).weak());
    });
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioSource, PortfolioTransactionType};
    use crate::store::Store;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
    }

    fn transaction(id: u64, portfolio_id: u64, quantity: f64) -> PortfolioTransaction {
        PortfolioTransaction {
            id,
            portfolio_id,
            asset: PortfolioAsset {
                key: "binance:BTCUSDT".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            kind: PortfolioTransactionType::Buy,
            quantity,
            price: Some(50_000.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 1_000,
            notes: String::new(),
            source: PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    #[test]
    fn mutation_persists_before_publishing() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let id = host
            .create_portfolio("Long-term", PortfolioCurrency::Eur)
            .expect("create");
        host.add_transaction(transaction(40, id, 1.0))
            .expect("add transaction");

        assert_eq!(
            host.snapshot().portfolios[0].base_currency,
            PortfolioCurrency::Eur
        );
        let persisted = store.lock().unwrap().portfolio_ledger().expect("reload");
        assert_eq!(persisted, *host.snapshot());
    }

    #[test]
    fn rejected_mutation_changes_neither_memory_nor_store() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let id = host
            .create_portfolio("Main", PortfolioCurrency::Usd)
            .expect("create");
        let baseline = host.snapshot().clone();

        let error = host
            .add_transaction(transaction(1, id, 0.0))
            .expect_err("zero quantity");
        assert!(matches!(
            error,
            PortfolioHostError::Mutation(PortfolioError::InvalidQuantity)
        ));
        assert_eq!(host.snapshot(), &baseline);
        assert_eq!(store.lock().unwrap().portfolio_ledger().unwrap(), baseline);
    }

    #[test]
    fn selection_and_delete_are_durable() {
        let store = store();
        let mut host = PortfolioHost::load(store.clone());
        let first = host
            .create_portfolio("First", PortfolioCurrency::Usd)
            .unwrap();
        let second = host
            .create_portfolio("Second", PortfolioCurrency::Usd)
            .unwrap();
        host.select(Some(first)).unwrap();
        host.delete_portfolio(first).unwrap();

        assert_eq!(host.snapshot().selected_portfolio_id, Some(second));
        assert_eq!(
            store
                .lock()
                .unwrap()
                .portfolio_ledger()
                .unwrap()
                .selected_portfolio_id,
            Some(second)
        );
    }
}
