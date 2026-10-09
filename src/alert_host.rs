//! In-app alert runtime host: loads the persisted snapshot, drains the GUI -> runtime command
//! queue, and exposes the evaluated snapshot. Quote evaluation + webhook/banner delivery land in
//! a later increment. Ported from `AlertRuntimePersistence` + the `AlertRuntimeHost` ownership
//! model (in-app owner; no separate login-item agent yet).
#![allow(dead_code)] // transient until app.rs threads it

use crate::model::alert::{
    AlertCurrency, AlertNotificationSettings, AlertPersistenceSnapshot, AlertRuntimeHealth,
    AlertTriggerEvent, MarketQuote,
};
use crate::model::alert_engine::AlertEngine;
use crate::model::alert_feed::UnseenAlerts;
use crate::model::alert_message::render_for_event;
use crate::model::watchlist_quote::maximum_age_secs;
use crate::model::webhook::WebhookDeliverySource;
use crate::model::webhook_policy::price_event_is_sendable;
use crate::service::keychain_webhook_secrets::KeychainWebhookSecretStore;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::service::webhook_delivery::{self, Mode, UreqTransport};
use crate::store::Store;

pub struct AlertHost {
    engine: AlertEngine,
    unseen: UnseenAlerts,
    dirty: bool,
    transport: UreqTransport,
    secrets: KeychainWebhookSecretStore,
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
            transport: UreqTransport::default(),
            secrets: KeychainWebhookSecretStore::default(),
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

    /// Delivers each trigger to the alert's webhook endpoints: claim → deliver → finish. One
    /// trigger reaches one endpoint exactly once (the claim dedupes). The sendable gate and the
    /// disabled-endpoint skip mirror `PriceAlertWebhookDispatcher.swift`. Synchronous — a slow
    /// endpoint blocks up to the transport's 3 s timeout (a worker thread is a later increment).
    pub fn dispatch_webhooks(&self, events: &[AlertTriggerEvent], store: &Store, now_ms: i64) {
        if events.is_empty() {
            return;
        }
        let endpoints = store.webhook_endpoints();
        if endpoints.is_empty() {
            return;
        }
        let delivery_enabled = self.engine.snapshot().settings.delivery_enabled;
        for event in events {
            if !price_event_is_sendable(event, delivery_enabled, now_ms) {
                continue;
            }
            let Some(alert) = self
                .engine
                .snapshot()
                .alerts
                .iter()
                .find(|alert| alert.id == event.alert_id)
            else {
                continue;
            };
            for endpoint_id in &alert.webhook_endpoint_ids {
                let Some(endpoint) = endpoints
                    .iter()
                    .find(|endpoint| endpoint.id == *endpoint_id)
                else {
                    continue;
                };
                if !endpoint.is_enabled {
                    continue;
                }
                let Some(delivery_id) = store.claim_webhook_delivery(
                    endpoint.id,
                    WebhookDeliverySource::Price,
                    event.id,
                    now_ms,
                ) else {
                    continue;
                };
                let message = render_for_event(alert.webhook_message.as_deref(), event, now_ms);
                let result = webhook_delivery::deliver(
                    &self.transport,
                    &self.secrets,
                    &message,
                    endpoint,
                    Mode::Production,
                );
                store.finish_webhook_delivery(delivery_id, &result);
            }
        }
    }

    /// Builds a `MarketQuote` per active alert asset from the shared quote book and runs them
    /// through the engine. Quotes are assumed USD for now (FX conversion is a later increment),
    /// so alerts in other currencies are skipped. Per-source staleness comes from the quote layer.
    pub fn evaluate_from_book(
        &mut self,
        book: &WatchlistQuoteBook,
        now_ms: i64,
    ) -> Vec<AlertTriggerEvent> {
        let mut quotes = Vec::new();
        for asset in self.engine.active_assets() {
            let source = asset.source();
            let key = asset.key();
            let Some(quote) = book.get_by_key(&key) else {
                continue;
            };
            let (Some(price), Some(source_timestamp_ms)) = (quote.last, quote.timestamp_ms) else {
                continue;
            };
            quotes.push(MarketQuote {
                asset,
                price,
                currency: AlertCurrency::Usd,
                source_timestamp_ms,
                received_at_ms: now_ms,
                maximum_age_secs: maximum_age_secs(source) as f64,
                fingerprint: source_timestamp_ms.to_string(),
                candle: None,
            });
        }
        self.evaluate(&quotes)
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
