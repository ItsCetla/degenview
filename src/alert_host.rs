//! In-app alert runtime host: loads the persisted snapshot, drains the GUI -> runtime command
//! queue, and exposes the evaluated snapshot. Quote evaluation + webhook/banner delivery land in
//! a later increment. Ported from `AlertRuntimePersistence` + the `AlertRuntimeHost` ownership
//! model (in-app owner; no separate login-item agent yet).
#![allow(dead_code)] // transient until app.rs threads it

use crate::model::alert::{
    AlertNotificationSettings, AlertPersistenceSnapshot, AlertRuntimeHealth, AlertTriggerEvent,
    MarketQuote,
};
use crate::model::alert_engine::AlertEngine;
use crate::model::alert_feed::UnseenAlerts;
use crate::store::Store;

pub struct AlertHost {
    engine: AlertEngine,
    unseen: UnseenAlerts,
    dirty: bool,
}

impl AlertHost {
    /// Loads the snapshot (or starts from defaults when nothing was ever saved) and seeds the
    /// unseen-count from the stored last-seen timestamp.
    pub fn load(store: &Store, now_ms: i64) -> Self {
        let engine = AlertEngine::new(store.load_alert_snapshot());
        let (unseen, to_save) = UnseenAlerts::new(store.last_seen_at(), now_ms);
        if let Some(ms) = to_save {
            let _ = store.set_last_seen_at(ms);
        }
        Self {
            engine,
            unseen,
            dirty: false,
        }
    }

    pub fn snapshot(&self) -> &AlertPersistenceSnapshot {
        self.engine.snapshot()
    }

    pub fn settings(&self) -> &AlertNotificationSettings {
        &self.engine.snapshot().settings
    }

    pub fn health(&self) -> &AlertRuntimeHealth {
        &self.engine.snapshot().health
    }

    pub fn unseen(&self) -> &UnseenAlerts {
        &self.unseen
    }

    pub fn unseen_mut(&mut self) -> &mut UnseenAlerts {
        &mut self.unseen
    }

    /// Applies every queued command exactly once (idempotent by command id) and marks the
    /// snapshot dirty.
    pub fn drain_commands(&mut self, store: &Store, now_ms: i64) {
        for command in store.pending_alert_commands() {
            self.engine.apply(&command, now_ms);
            store.acknowledge_alert_command(command.id);
            self.dirty = true;
        }
    }

    /// Judges fresh quotes against the active alerts and returns the ones that triggered. The
    /// caller builds each `MarketQuote` (from the quote feed, with FX conversion already applied
    /// to the alert's currency). Alerts in a currency the quote does not carry are skipped by the
    /// engine. Marks the snapshot dirty when anything fires.
    pub fn evaluate(&mut self, quotes: &[MarketQuote]) -> Vec<AlertTriggerEvent> {
        let mut events = Vec::new();
        for quote in quotes {
            events.extend(self.engine.process(quote, None, Some(quote.currency)));
        }
        if !events.is_empty() {
            self.dirty = true;
        }
        events
    }

    pub fn save_if_dirty(&mut self, store: &Store) {
        if self.dirty {
            let _ = store.save_alert_snapshot(self.engine.snapshot());
            self.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{AlertRuntimeCommand, AlertRuntimeCommandPayload};
    use crate::model::watchlist::new_id;

    #[test]
    fn a_fresh_host_starts_empty_and_seeds_last_seen() {
        let store = Store::open_in_memory().expect("open");
        let now_ms = 1_000;
        let mut host = AlertHost::load(&store, now_ms);
        assert!(host.snapshot().alerts.is_empty());
        assert_eq!(store.last_seen_at(), Some(now_ms));
        host.save_if_dirty(&store);
        assert_eq!(store.load_alert_snapshot().map(|s| s.revision), None);
    }

    #[test]
    fn commands_drain_once_and_persist() {
        let store = Store::open_in_memory().expect("open");
        let command = AlertRuntimeCommand {
            id: new_id(),
            created_at_ms: 1_000,
            expected_revision: None,
            payload: AlertRuntimeCommandPayload::ClearHistory,
        };
        store.enqueue_alert_command(&command).expect("enqueue");

        let mut host = AlertHost::load(&store, 1_000);
        host.drain_commands(&store, 1_000);
        assert!(store.pending_alert_commands().is_empty());
        host.save_if_dirty(&store);
        // A revision is now written, so a reload finds a snapshot.
        assert!(store.load_alert_snapshot().is_some());
    }
}
