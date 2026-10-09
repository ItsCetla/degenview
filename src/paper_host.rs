//! Stateful bridge between the paper-trading execution engine, SQLite, and the pure panel views.
//!
//! The execution engine deliberately records rejected orders before returning their error. This
//! host therefore persists and publishes every candidate engine state, including a domain-error
//! result, while still refusing all writes after an initial load failure.
#![allow(dead_code)]

use crate::model::paper::{
    PaperAccountMetrics, PaperAccountSettings, PaperCurrency, PaperOrderChanges, PaperOrderRequest,
    PaperOrderSide, PaperOrderType, PaperPosition, PaperQuote, PaperTimeInForce, PaperTradingError,
    PaperTradingSnapshot,
};
use crate::model::paper_engine::PaperTradingEngine;
use crate::store::SharedStore;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum PaperHostError {
    Load(String),
    Engine(PaperTradingError),
    Persistence(String),
    StoreUnavailable,
}

impl PaperHostError {
    pub fn message(&self) -> String {
        match self {
            Self::Load(message) => format!("Could not load paper trading: {message}"),
            Self::Engine(error) => error.message(),
            Self::Persistence(message) => format!("Could not save paper trading: {message}"),
            Self::StoreUnavailable => "The paper-trading database is unavailable.".to_owned(),
        }
    }
}

impl fmt::Display for PaperHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PaperHostError {}

pub struct PaperHost {
    store: SharedStore,
    engine: PaperTradingEngine,
    load_error: Option<PaperHostError>,
    last_error: Option<PaperHostError>,
}

impl PaperHost {
    pub fn load(store: SharedStore) -> Self {
        let loaded = store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .paper_trading()
                    .map_err(|error| PaperHostError::Load(error.to_string()))
            });
        match loaded {
            Ok(mut snapshot) => {
                // A quote restored from an earlier process is stale by definition. Keeping it
                // would put the previous session's price into a new order ticket until the first
                // live update arrives.
                snapshot.quotes.clear();
                Self {
                    store,
                    engine: PaperTradingEngine::new(snapshot),
                    load_error: None,
                    last_error: None,
                }
            }
            Err(error) => Self {
                store,
                engine: PaperTradingEngine::new(PaperTradingSnapshot::default()),
                load_error: Some(error.clone()),
                last_error: Some(error),
            },
        }
    }

    pub fn snapshot(&self) -> &PaperTradingSnapshot {
        self.engine.snapshot()
    }

    pub fn load_failed(&self) -> bool {
        self.load_error.is_some()
    }

    pub fn last_error(&self) -> Option<&PaperHostError> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn metrics(&self, account_id: u64) -> PaperAccountMetrics {
        self.engine.metrics(account_id)
    }

    pub fn unrealized_pnl(&self, position: &PaperPosition) -> f64 {
        self.engine.unrealized_pnl(position)
    }

    pub fn create_account(
        &mut self,
        name: &str,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<u64, PaperHostError> {
        self.apply(|engine| engine.create_account(name, currency, initial_balance, settings))
    }

    pub fn select_account(&mut self, id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.select_account(id))
    }

    pub fn update_settings(
        &mut self,
        account_id: u64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.update_settings(account_id, settings))
    }

    pub fn reset_account(
        &mut self,
        id: u64,
        currency: PaperCurrency,
        initial_balance: f64,
        settings: PaperAccountSettings,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.reset_account(id, currency, initial_balance, settings))
    }

    pub fn delete_account(&mut self, id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.delete_account(id))
    }

    pub fn submit(&mut self, request: PaperOrderRequest) -> Result<u64, PaperHostError> {
        self.apply(move |engine| engine.submit(request))
    }

    pub fn cancel(&mut self, order_id: u64) -> Result<(), PaperHostError> {
        self.apply(|engine| engine.cancel(order_id))
    }

    pub fn modify(
        &mut self,
        order_id: u64,
        changes: PaperOrderChanges,
    ) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.modify(order_id, changes))
    }

    pub fn process_quote(&mut self, quote: PaperQuote) -> Result<(), PaperHostError> {
        self.apply(move |engine| engine.process(quote))
    }

    pub fn close_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
    ) -> Result<u64, PaperHostError> {
        self.market_against_position(account_id, instrument_key, 1.0)
    }

    pub fn reverse_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
    ) -> Result<u64, PaperHostError> {
        self.market_against_position(account_id, instrument_key, 2.0)
    }

    pub fn close_all_positions(&mut self, account_id: u64) -> Vec<Result<u64, PaperHostError>> {
        let keys: Vec<String> = self
            .snapshot()
            .positions
            .iter()
            .filter(|position| position.account_id == account_id)
            .map(|position| position.instrument.key.clone())
            .collect();
        keys.into_iter()
            .map(|key| self.close_position(account_id, &key))
            .collect()
    }

    pub fn cancel_all_orders(&mut self, account_id: u64) -> Vec<Result<(), PaperHostError>> {
        let ids: Vec<u64> = self
            .snapshot()
            .orders
            .iter()
            .filter(|order| order.account_id == account_id && order.status.is_working())
            .map(|order| order.id)
            .collect();
        ids.into_iter().map(|id| self.cancel(id)).collect()
    }

    fn market_against_position(
        &mut self,
        account_id: u64,
        instrument_key: &str,
        quantity_multiplier: f64,
    ) -> Result<u64, PaperHostError> {
        let position = self
            .snapshot()
            .positions
            .iter()
            .find(|position| {
                position.account_id == account_id && position.instrument.key == instrument_key
            })
            .cloned()
            .ok_or(PaperHostError::Engine(PaperTradingError::UnsupportedSymbol))?;
        let side = if position.signed_quantity > 0.0 {
            PaperOrderSide::Sell
        } else {
            PaperOrderSide::Buy
        };
        let quantity = position.quantity() * quantity_multiplier;
        self.submit(PaperOrderRequest {
            account_id,
            instrument: position.instrument,
            side,
            kind: PaperOrderType::Market,
            quantity,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        })
    }

    /// Runs an engine operation, persists whatever state it produced (including rejected-order
    /// audit rows), and publishes only after persistence succeeds.
    fn apply<T>(
        &mut self,
        operation: impl FnOnce(&mut PaperTradingEngine) -> Result<T, PaperTradingError>,
    ) -> Result<T, PaperHostError> {
        if let Some(error) = &self.load_error {
            self.last_error = Some(error.clone());
            return Err(error.clone());
        }

        let mut candidate = PaperTradingEngine::new(self.engine.snapshot().clone());
        let result = operation(&mut candidate);
        let saved = self
            .store
            .lock()
            .map_err(|_| PaperHostError::StoreUnavailable)
            .and_then(|store| {
                store
                    .save_paper_trading(candidate.snapshot())
                    .map_err(|error| PaperHostError::Persistence(error.to_string()))
            });
        if let Err(error) = saved {
            self.last_error = Some(error.clone());
            return Err(error);
        }

        self.engine = candidate;
        match result {
            Ok(value) => {
                self.last_error = None;
                Ok(value)
            }
            Err(error) => {
                let error = PaperHostError::Engine(error);
                self.last_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::paper::{PaperInstrument, PaperOrderStatus};
    use crate::store::Store;
    use std::sync::{Arc, Mutex};

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().expect("open store")))
    }

    fn account(host: &mut PaperHost) -> u64 {
        host.create_account(
            "Main",
            PaperCurrency::Usd,
            100_000.0,
            PaperAccountSettings::default(),
        )
        .expect("account")
    }

    fn market_request(account_id: u64) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance),
            side: PaperOrderSide::Buy,
            kind: PaperOrderType::Market,
            quantity: 1.0,
            limit_price: None,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    #[test]
    fn account_mutations_are_durable() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let first = account(&mut host);
        let second = host
            .create_account(
                "Second",
                PaperCurrency::Eur,
                50_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        host.select_account(first).unwrap();
        host.delete_account(first).unwrap();

        assert_eq!(host.snapshot().selected_account_id, Some(second));
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap(),
            *host.snapshot()
        );
    }

    #[test]
    fn a_rejected_order_is_saved_before_its_error_is_returned() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        let error = host.submit(market_request(id)).expect_err("no quote");
        assert!(matches!(
            error,
            PaperHostError::Engine(PaperTradingError::NoMarketData)
        ));
        assert_eq!(host.snapshot().orders.len(), 1);
        assert_eq!(host.snapshot().orders[0].status, PaperOrderStatus::Rejected);
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap().orders[0].status,
            PaperOrderStatus::Rejected
        );
    }

    #[test]
    fn quotes_and_fills_persist_together() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        let id = account(&mut host);
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(49_999.0),
            ask: Some(50_001.0),
            last: Some(50_000.0),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        host.submit(market_request(id)).unwrap();

        assert_eq!(host.snapshot().positions.len(), 1);
        assert_eq!(host.snapshot().fills.len(), 1);
        assert_eq!(
            store.lock().unwrap().paper_trading().unwrap(),
            *host.snapshot()
        );
    }

    #[test]
    fn restored_quotes_are_discarded_as_stale() {
        let store = store();
        let mut host = PaperHost::load(store.clone());
        host.process_quote(PaperQuote {
            instrument_key: "Binance:BTCUSDT".into(),
            bid: Some(49_999.0),
            ask: Some(50_001.0),
            last: Some(50_000.0),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        assert!(!host.snapshot().quotes.is_empty());

        let restored = PaperHost::load(store);
        assert!(restored.snapshot().quotes.is_empty());
    }

    fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64)
    }
}
