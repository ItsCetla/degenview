//! Webhook endpoints and the history of delivery attempts. Ported from
//! `AppDatabase+Webhooks.swift`. The endpoint list is an ordered document table; each
//! delivery attempt is one row keyed so one trigger reaches one endpoint exactly once.

use crate::model::watchlist::new_id;
use crate::model::webhook::{
    EndpointId, WebhookDeliveryRecord, WebhookDeliveryResult, WebhookDeliverySource,
    WebhookDeliveryState, WebhookEndpoint,
};
use crate::store::{Document, Store};
use rusqlite::{Row, params};
use std::collections::HashMap;

/// Delivery rows kept; older ones are pruned when a new attempt is claimed.
const WEBHOOK_DELIVERY_HISTORY_LIMIT: i64 = 5000;

impl Document for WebhookEndpoint {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// Endpoints in position order. Unlike `documents`, one undecodable row fails the whole
    /// read (returns empty): an empty or partial list must not be mistaken for "no endpoints"
    /// by a caller that then writes it back.
    pub fn webhook_endpoints(&self) -> Vec<WebhookEndpoint> {
        let Ok(mut statement) = self
            .connection
            .prepare("SELECT payload FROM webhook_endpoint ORDER BY position")
        else {
            return Vec::new();
        };
        let payloads = statement
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<Result<Vec<String>, _>>());
        let Ok(payloads) = payloads else {
            return Vec::new();
        };
        payloads
            .iter()
            .map(|payload| serde_json::from_str::<WebhookEndpoint>(payload))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default()
    }

    pub fn replace_webhook_endpoints(&self, endpoints: &[WebhookEndpoint]) -> rusqlite::Result<()> {
        self.replace_documents("webhook_endpoint", endpoints)
    }

    /// Claims the right to send `event_id` to `endpoint_id`. Returns the new row id, or `None`
    /// when that trigger already reached the endpoint (or is being sent): whoever gets `None`
    /// must not send. The row is written before the request, so an owner that dies mid-request
    /// leaves a `pending` row behind and the next owner does not send it again.
    pub fn claim_webhook_delivery(
        &self,
        endpoint_id: EndpointId,
        source: WebhookDeliverySource,
        event_id: u64,
        now_ms: i64,
    ) -> Option<u64> {
        let id = new_id();
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO webhook_delivery
                (id, endpoint_id, source, event_id, timestamp, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.to_string(),
                endpoint_id.to_string(),
                source.as_str(),
                event_id.to_string(),
                now_ms,
                WebhookDeliveryState::Pending.as_str(),
            ],
        );
        match inserted {
            Ok(changes) if changes > 0 => {
                let _ = self.connection.execute(
                    "DELETE FROM webhook_delivery WHERE id NOT IN (
                        SELECT id FROM webhook_delivery ORDER BY timestamp DESC, rowid DESC LIMIT ?1)",
                    params![WEBHOOK_DELIVERY_HISTORY_LIMIT],
                );
                Some(id)
            }
            _ => None,
        }
    }

    /// Records how a claimed attempt ended.
    pub fn finish_webhook_delivery(&self, id: u64, result: &WebhookDeliveryResult) {
        let state = if result.succeeded() {
            WebhookDeliveryState::Delivered
        } else {
            WebhookDeliveryState::Failed
        };
        let _ = self.connection.execute(
            "UPDATE webhook_delivery
             SET state = ?1, status_code = ?2, duration = ?3, error = ?4, timestamp = ?5
             WHERE id = ?6",
            params![
                state.as_str(),
                result.status_code.map(i64::from),
                result.duration_secs,
                result.error.map(|error| error.as_str()),
                result.timestamp_ms,
                id.to_string(),
            ],
        );
    }

    /// Attempts for the given triggers, oldest first within each.
    pub fn webhook_deliveries(
        &self,
        event_ids: &[u64],
    ) -> HashMap<u64, Vec<WebhookDeliveryRecord>> {
        let mut grouped: HashMap<u64, Vec<WebhookDeliveryRecord>> = HashMap::new();
        // SQLite caps bound variables, so large lists go in chunks.
        for chunk in event_ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut statement = match self.connection.prepare(&format!(
                "SELECT id, endpoint_id, source, event_id, timestamp, state, status_code, duration, error
                 FROM webhook_delivery WHERE event_id IN ({marks}) ORDER BY timestamp, rowid"
            )) {
                Ok(statement) => statement,
                Err(_) => continue,
            };
            let args: Vec<String> = chunk.iter().map(u64::to_string).collect();
            let rows = statement.query_map(
                rusqlite::params_from_iter(args.iter()),
                webhook_delivery_record,
            );
            let Ok(rows) = rows else { continue };
            for record in rows.flatten() {
                if let Some(event_id) = record.event_id {
                    grouped.entry(event_id).or_default().push(record);
                }
            }
        }
        grouped
    }
}

fn webhook_delivery_record(row: &Row<'_>) -> rusqlite::Result<WebhookDeliveryRecord> {
    let event_id = row
        .get::<_, Option<String>>("event_id")?
        .and_then(|value| value.parse().ok());
    let error = row
        .get::<_, Option<String>>("error")?
        .and_then(|value| crate::model::webhook::WebhookDeliveryError::from_str(&value));
    Ok(WebhookDeliveryRecord {
        id: row.get::<_, String>("id")?.parse().unwrap_or(0),
        endpoint_id: row.get::<_, String>("endpoint_id")?.parse().unwrap_or(0),
        source: crate::model::webhook::WebhookDeliverySource::from_str(
            &row.get::<_, String>("source")?,
        )
        .unwrap_or(WebhookDeliverySource::Price),
        event_id,
        timestamp_ms: row.get("timestamp")?,
        state: WebhookDeliveryState::from_str(&row.get::<_, String>("state")?)
            .unwrap_or(WebhookDeliveryState::Pending),
        status_code: row
            .get::<_, Option<i64>>("status_code")?
            .map(|value| value as u16),
        duration_secs: row.get("duration")?,
        error,
    })
}

// `with_id` is a test convenience so ids are stable across a round-trip.
#[cfg(test)]
impl WebhookEndpoint {
    fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::webhook::WebhookDeliveryError;

    fn endpoint(id: u64, name: &str) -> WebhookEndpoint {
        WebhookEndpoint::new(name, "https://example.com/hook", 1_000).with_id(id)
    }

    #[test]
    fn endpoints_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_webhook_endpoints(&[endpoint(7, "A"), endpoint(9, "B")])
            .expect("write");
        let loaded = store.webhook_endpoints();
        assert_eq!(loaded.len(), 2);
        assert_eq!((loaded[0].id, loaded[0].name.as_str()), (7, "A"));
        assert_eq!((loaded[1].id, loaded[1].name.as_str()), (9, "B"));
    }

    #[test]
    fn an_undecodable_endpoint_row_fails_the_whole_read() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO webhook_endpoint (id, position, payload) VALUES ('x', 0, 'not json')",
                [],
            )
            .expect("insert bad row");
        assert!(store.webhook_endpoints().is_empty());
    }

    #[test]
    fn a_delivery_is_claimed_once_per_trigger_and_endpoint() {
        let store = Store::open_in_memory().expect("open");
        let first = store
            .claim_webhook_delivery(1, WebhookDeliverySource::Price, 42, 1_000)
            .expect("first claim");
        assert_eq!(
            store.claim_webhook_delivery(1, WebhookDeliverySource::Price, 42, 2_000),
            None,
            "the same trigger must not reach the endpoint twice"
        );
        // A different endpoint or event still claims.
        assert!(
            store
                .claim_webhook_delivery(2, WebhookDeliverySource::Price, 42, 2_000)
                .is_some()
        );
        assert!(
            store
                .claim_webhook_delivery(1, WebhookDeliverySource::Price, 43, 2_000)
                .is_some()
        );

        let result = WebhookDeliveryResult {
            endpoint_id: 1,
            timestamp_ms: 2_500,
            duration_secs: 0.143,
            status_code: Some(200),
            error: None,
        };
        store.finish_webhook_delivery(first, &result);
        let by_event = store.webhook_deliveries(&[42, 43]);
        let delivered = by_event
            .get(&42)
            .and_then(|rows| rows.iter().find(|row| row.endpoint_id == 1))
            .expect("delivered row");
        assert_eq!(delivered.state, WebhookDeliveryState::Delivered);
        assert_eq!(delivered.status_code, Some(200));
        assert_eq!(delivered.duration_secs, Some(0.143));
    }

    #[test]
    fn a_failed_attempt_records_the_error() {
        let store = Store::open_in_memory().expect("open");
        let id = store
            .claim_webhook_delivery(1, WebhookDeliverySource::Pine, 5, 1_000)
            .expect("claim");
        store.finish_webhook_delivery(
            id,
            &WebhookDeliveryResult {
                endpoint_id: 1,
                timestamp_ms: 2_000,
                duration_secs: 3.0,
                status_code: None,
                error: Some(WebhookDeliveryError::Timeout),
            },
        );
        let rows = &store.webhook_deliveries(&[5])[&5];
        assert_eq!(rows[0].state, WebhookDeliveryState::Failed);
        assert_eq!(rows[0].error, Some(WebhookDeliveryError::Timeout));
    }
}
