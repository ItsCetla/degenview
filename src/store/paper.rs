//! Paper-trading storage. `PaperTradingEngine` persists whole snapshots, so each save replaces
//! the paper tables in one transaction. Ported from `AppDatabase+PaperTrading.swift`.
#![allow(dead_code)] // transient until the paper trading host wires these

use crate::model::paper::{PaperAccount, PaperTradingSnapshot};
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;

const KEY_SELECTED_ACCOUNT: &str = "paper.selectedAccountID";
const KEY_QUOTES: &str = "paper.quotes";

impl Document for PaperAccount {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// A row that no longer decodes fails the read: dropping a fill or order would silently
    /// change the account's cash and positions.
    pub fn paper_trading(&self) -> rusqlite::Result<PaperTradingSnapshot> {
        Ok(PaperTradingSnapshot {
            accounts: self.documents("paper_account"),
            orders: self.payloads_strict("SELECT payload FROM paper_order ORDER BY rowid")?,
            fills: self.payloads_strict("SELECT payload FROM paper_fill ORDER BY rowid")?,
            positions: self.payloads_strict("SELECT payload FROM paper_position ORDER BY rowid")?,
            order_events: self
                .payloads_strict("SELECT payload FROM paper_order_event ORDER BY rowid")?,
            closed_trades: self
                .payloads_strict("SELECT payload FROM paper_closed_trade ORDER BY rowid")?,
            journal: self.payloads_strict("SELECT payload FROM paper_journal ORDER BY rowid")?,
            selected_account_id: self.setting(KEY_SELECTED_ACCOUNT),
            quotes: self.setting(KEY_QUOTES).unwrap_or_default(),
        })
    }

    pub fn save_paper_trading(&self, snapshot: &PaperTradingSnapshot) -> rusqlite::Result<()> {
        self.replace_documents("paper_account", &snapshot.accounts)?;
        self.replace_paper_rows("paper_order", &snapshot.orders, |order| order.account_id)?;
        self.replace_paper_rows("paper_fill", &snapshot.fills, |fill| fill.account_id)?;
        self.replace_paper_rows("paper_position", &snapshot.positions, |position| {
            position.account_id
        })?;
        self.replace_paper_rows("paper_order_event", &snapshot.order_events, |event| {
            event.account_id
        })?;
        self.replace_paper_rows("paper_closed_trade", &snapshot.closed_trades, |trade| {
            trade.account_id
        })?;
        self.replace_paper_rows("paper_journal", &snapshot.journal, |entry| entry.account_id)?;

        match snapshot.selected_account_id {
            Some(id) => self.set_setting(KEY_SELECTED_ACCOUNT, &id)?,
            None => self.remove_setting(KEY_SELECTED_ACCOUNT)?,
        }
        if snapshot.quotes.is_empty() {
            self.remove_setting(KEY_QUOTES)?;
        } else {
            self.set_setting(KEY_QUOTES, &snapshot.quotes)?;
        }
        Ok(())
    }

    /// Replaces one account-scoped collection, kept in engine order via the rowid.
    fn replace_paper_rows<T: Serialize>(
        &self,
        table: &str,
        items: &[T],
        account_id: impl Fn(&T) -> u64,
    ) -> rusqlite::Result<()> {
        self.connection
            .execute(&format!("DELETE FROM {table}"), [])?;
        let mut statement = self.connection.prepare(&format!(
            "INSERT INTO {table} (account_id, payload) VALUES (?1, ?2)"
        ))?;
        for item in items {
            statement.execute(params![account_id(item).to_string(), json(item)?])?;
        }
        Ok(())
    }
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::paper::PaperQuote;
    use std::collections::HashMap;

    #[test]
    fn paper_snapshot_round_trips() {
        let store = Store::open_in_memory().expect("open");
        let snapshot = PaperTradingSnapshot {
            accounts: vec![PaperAccount::new(3, "Main", 1_000)],
            selected_account_id: Some(3),
            quotes: HashMap::from([(
                "binance:BTCUSDT".to_owned(),
                PaperQuote {
                    instrument_key: "binance:BTCUSDT".to_owned(),
                    bid: Some(49_999.0),
                    ask: Some(50_001.0),
                    last: Some(50_000.0),
                    timestamp: 2_000,
                    is_market_open: true,
                },
            )]),
            ..Default::default()
        };
        store.save_paper_trading(&snapshot).expect("save");

        let loaded = store.paper_trading().expect("load");
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.accounts[0].id, 3);
        assert_eq!(loaded.selected_account_id, Some(3));
        assert_eq!(loaded.quotes.len(), 1);
        assert_eq!(loaded.quotes["binance:BTCUSDT"].last, Some(50_000.0));
    }

    #[test]
    fn an_empty_snapshot_is_clean() {
        let store = Store::open_in_memory().expect("open");
        store
            .save_paper_trading(&PaperTradingSnapshot::default())
            .expect("save empty");
        let loaded = store.paper_trading().expect("load");
        assert!(loaded.accounts.is_empty());
        assert_eq!(loaded.selected_account_id, None);
        assert!(loaded.quotes.is_empty());
    }
}
