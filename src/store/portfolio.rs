//! Portfolio ledger storage. `PortfolioLedger` persists whole snapshots, so each save replaces
//! the ledger tables in one transaction. Ported from `AppDatabase+Portfolio.swift`.
#![allow(dead_code)] // transient until the portfolio ledger host wires these

use crate::model::portfolio::{Portfolio, PortfolioLedgerSnapshot};
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;

const KEY_SELECTED: &str = "portfolio.selectedPortfolioID";
const KEY_INVALIDATED_AFTER: &str = "portfolio.invalidatedAfter";

impl Document for Portfolio {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// Unlike the document list, a transaction or snapshot that no longer decodes fails the
    /// read: dropping one would silently change every balance after it.
    pub fn portfolio_ledger(&self) -> rusqlite::Result<PortfolioLedgerSnapshot> {
        Ok(PortfolioLedgerSnapshot {
            portfolios: self.documents_strict("portfolio")?,
            transactions: self
                .payloads_strict("SELECT payload FROM portfolio_transaction ORDER BY position")?,
            historical_snapshots: self
                .payloads_strict("SELECT payload FROM portfolio_snapshot ORDER BY rowid")?,
            selected_portfolio_id: self.setting_strict(KEY_SELECTED)?,
            invalidated_after: self
                .setting_strict(KEY_INVALIDATED_AFTER)?
                .unwrap_or_default(),
        })
    }

    pub fn save_portfolio_ledger(
        &self,
        snapshot: &PortfolioLedgerSnapshot,
    ) -> rusqlite::Result<()> {
        self.in_transaction(|| {
            self.replace_documents("portfolio", &snapshot.portfolios)?;

            self.connection
                .execute("DELETE FROM portfolio_transaction", [])?;
            let mut transaction = self.connection.prepare(
                "INSERT INTO portfolio_transaction (id, portfolio_id, timestamp, position, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (position, value) in snapshot.transactions.iter().enumerate() {
                transaction.execute(params![
                    value.id.to_string(),
                    value.portfolio_id.to_string(),
                    value.timestamp,
                    position as i64,
                    json(value)?,
                ])?;
            }

            self.connection
                .execute("DELETE FROM portfolio_snapshot", [])?;
            let mut history = self.connection.prepare(
                "INSERT INTO portfolio_snapshot (portfolio_id, timestamp, payload) VALUES (?1, ?2, ?3)",
            )?;
            for value in &snapshot.historical_snapshots {
                history.execute(params![
                    value.portfolio_id.to_string(),
                    value.timestamp,
                    json(value)?,
                ])?;
            }

            match snapshot.selected_portfolio_id {
                Some(id) => self.set_setting(KEY_SELECTED, &id)?,
                None => self.remove_setting(KEY_SELECTED)?,
            }
            if snapshot.invalidated_after.is_empty() {
                self.remove_setting(KEY_INVALIDATED_AFTER)?;
            } else {
                self.set_setting(KEY_INVALIDATED_AFTER, &snapshot.invalidated_after)?;
            }
            Ok(())
        })
    }
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{
        PortfolioAsset, PortfolioCurrency, PortfolioSnapshot, PortfolioTransaction,
    };
    use std::collections::HashMap;

    fn portfolio(id: u64) -> Portfolio {
        Portfolio::new(id, "Main", 1_000)
    }

    fn transaction(id: u64, portfolio_id: u64) -> PortfolioTransaction {
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
            kind: crate::model::portfolio::PortfolioTransactionType::Buy,
            quantity: 1.0,
            price: Some(50_000.0),
            price_currency: PortfolioCurrency::Usd,
            fee: 0.0,
            fee_currency: PortfolioCurrency::Usd,
            timestamp: 2_000,
            notes: String::new(),
            source: crate::model::portfolio::PortfolioSource::Manual,
            external_transaction_id: None,
            created_at: 2_000,
            updated_at: 2_000,
        }
    }

    #[test]
    fn ledger_round_trips() {
        let store = Store::open_in_memory().expect("open");
        let snapshot = PortfolioLedgerSnapshot {
            portfolios: vec![portfolio(3)],
            transactions: vec![transaction(9, 3)],
            historical_snapshots: vec![PortfolioSnapshot {
                portfolio_id: 3,
                timestamp: 2_500,
                value: 50_000.0,
                net_contributions: 50_000.0,
                realized_pnl: 0.0,
                unrealized_pnl: 0.0,
                is_complete: true,
            }],
            selected_portfolio_id: Some(3),
            invalidated_after: HashMap::from([(3, 2_500)]),
        };
        store.save_portfolio_ledger(&snapshot).expect("save");

        let loaded = store.portfolio_ledger().expect("load");
        assert_eq!(loaded.portfolios.len(), 1);
        assert_eq!(loaded.transactions.len(), 1);
        assert_eq!(loaded.historical_snapshots.len(), 1);
        assert_eq!(loaded.selected_portfolio_id, Some(3));
        assert_eq!(loaded.invalidated_after.get(&3), Some(&2_500));
    }

    #[test]
    fn an_empty_ledger_is_clean() {
        let store = Store::open_in_memory().expect("open");
        store
            .save_portfolio_ledger(&PortfolioLedgerSnapshot::default())
            .expect("save empty");
        let loaded = store.portfolio_ledger().expect("load");
        assert!(loaded.portfolios.is_empty());
        assert_eq!(loaded.selected_portfolio_id, None);
        assert!(loaded.invalidated_after.is_empty());
    }

    #[test]
    fn unreadable_ledger_setting_fails_the_load() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO setting (key, value) VALUES ('portfolio.invalidatedAfter', 'bad')",
                [],
            )
            .expect("seed");
        assert!(store.portfolio_ledger().is_err());
    }
}
