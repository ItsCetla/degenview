//! Price-alert persistence: the runtime snapshot, the GUI -> runtime command queue, and the
//! sidebar bell's "seen" timestamp. Ported from `AlertRuntimePersistence.swift` +
//! `UnseenAlertsStore.swift`.
#![allow(dead_code)] // transient until the alert runtime host + dispatcher wire these

use crate::model::alert::{
    AlertNotificationSettings, AlertPersistenceSnapshot, AlertRuntimeCommand, AlertRuntimeHealth,
    PriceAlert,
};
use crate::model::alert_feed::LAST_SEEN_KEY;
use crate::store::{Document, Store};
use rusqlite::params;
use serde::Serialize;

const KEY_REVISION: &str = "alert.revision";
const KEY_SETTINGS: &str = "alert.settings";
const KEY_PROCESSED_COMMAND_IDS: &str = "alert.processedCommandIDs";
const KEY_HEALTH: &str = "alert.health";

impl Document for PriceAlert {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    /// `None` until a runtime has saved once, so the engine can start from defaults. A bad
    /// alert or event row is skipped rather than failing the whole snapshot.
    pub fn load_alert_snapshot(&self) -> Option<AlertPersistenceSnapshot> {
        let revision = self.setting::<u64>(KEY_REVISION)?;
        Some(AlertPersistenceSnapshot {
            revision,
            alerts: self.documents("price_alert"),
            history: self.payloads("SELECT payload FROM alert_event ORDER BY rowid"),
            settings: self
                .setting::<AlertNotificationSettings>(KEY_SETTINGS)
                .unwrap_or_default(),
            processed_command_ids: self
                .setting::<Vec<u64>>(KEY_PROCESSED_COMMAND_IDS)
                .unwrap_or_default(),
            health: self
                .setting::<AlertRuntimeHealth>(KEY_HEALTH)
                .unwrap_or_default(),
        })
    }

    /// Writes the whole snapshot in one pass, bumping the revision so readers never see a
    /// half-saved state (see note in the module docstring about the lock owner).
    pub fn save_alert_snapshot(&self, snapshot: &AlertPersistenceSnapshot) -> rusqlite::Result<()> {
        self.replace_documents("price_alert", &snapshot.alerts)?;
        self.connection.execute("DELETE FROM alert_event", [])?;
        let mut event = self.connection.prepare(
            "INSERT INTO alert_event (alert_id, timestamp, payload) VALUES (?1, ?2, ?3)",
        )?;
        for value in &snapshot.history {
            event.execute(params![
                value.alert_id.to_string(),
                value.timestamp_ms,
                json(value)?,
            ])?;
        }
        self.set_setting(KEY_REVISION, &snapshot.revision)?;
        self.set_setting(KEY_SETTINGS, &snapshot.settings)?;
        self.set_setting(KEY_PROCESSED_COMMAND_IDS, &snapshot.processed_command_ids)?;
        self.set_setting(KEY_HEALTH, &snapshot.health)?;
        Ok(())
    }

    /// Enqueues a change for whoever owns the runtime. Idempotent by command id.
    pub fn enqueue_alert_command(&self, command: &AlertRuntimeCommand) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO alert_command (id, created_at, payload) VALUES (?1, ?2, ?3)",
            params![
                command.id.to_string(),
                command.created_at_ms,
                json(command)?,
            ],
        )?;
        Ok(())
    }

    /// Oldest first. A row that no longer decodes is left queued (it can never be acknowledged)
    /// and simply omitted here.
    pub fn pending_alert_commands(&self) -> Vec<AlertRuntimeCommand> {
        self.payloads("SELECT payload FROM alert_command ORDER BY created_at, rowid")
    }

    pub fn acknowledge_alert_command(&self, id: u64) {
        let _ = self.connection.execute(
            "DELETE FROM alert_command WHERE id = ?1",
            params![id.to_string()],
        );
    }

    /// The sidebar bell's last-seen timestamp, epoch ms. See `alert_feed::UnseenAlerts`.
    pub fn last_seen_at(&self) -> Option<i64> {
        self.setting(LAST_SEEN_KEY)
    }

    pub fn set_last_seen_at(&self, ms: i64) -> rusqlite::Result<()> {
        self.set_setting(LAST_SEEN_KEY, &ms)
    }
}

fn json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::{
        AlertAsset, AlertCondition, AlertCurrency, AlertEventOrigin, AlertRuntimeCommandPayload,
        AlertTriggerEvent,
    };
    use crate::model::{DataSource, InstrumentId};

    fn asset() -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTCUSDT",
            "Bitcoin",
        )
    }

    fn alert(id: u64) -> PriceAlert {
        let mut alert = PriceAlert::new(
            asset(),
            AlertCondition::CrossesAbove { target: 50_000.0 },
            1_000,
        );
        alert.id = id;
        alert
    }

    #[test]
    fn snapshot_is_none_until_saved_then_round_trips() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.load_alert_snapshot().is_none());

        let mut snapshot = AlertPersistenceSnapshot::default();
        snapshot.revision = 3;
        snapshot.alerts = vec![alert(7)];
        snapshot.history = vec![AlertTriggerEvent {
            id: 11,
            alert_id: 7,
            asset: snapshot.alerts[0].asset.clone(),
            observed_value: 50_001.0,
            target: 50_000.0,
            currency: AlertCurrency::Usd,
            timestamp_ms: 2_000,
            quote_fingerprint: "fp".into(),
            origin: AlertEventOrigin::Live,
            delivery: Default::default(),
            candle: None,
        }];
        snapshot.settings.macos_notifications_enabled = false;
        snapshot.processed_command_ids = vec![1, 2];
        store.save_alert_snapshot(&snapshot).expect("save");

        let loaded = store.load_alert_snapshot().expect("loaded");
        assert_eq!(loaded.revision, 3);
        assert_eq!(loaded.alerts.len(), 1);
        assert_eq!(loaded.history.len(), 1);
        assert!(!loaded.settings.macos_notifications_enabled);
        assert_eq!(loaded.processed_command_ids, vec![1, 2]);
    }

    #[test]
    fn commands_enqueue_once_and_acknowledge_deletes() {
        let store = Store::open_in_memory().expect("open");
        let command = AlertRuntimeCommand::new(AlertRuntimeCommandPayload::Delete(9), 1_000);
        store.enqueue_alert_command(&command).expect("enqueue");
        store.enqueue_alert_command(&command).expect("idempotent");
        assert_eq!(store.pending_alert_commands().len(), 1);
        store.acknowledge_alert_command(command.id);
        assert!(store.pending_alert_commands().is_empty());
    }

    #[test]
    fn last_seen_round_trips() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.last_seen_at(), None);
        store.set_last_seen_at(5_000).expect("write");
        assert_eq!(store.last_seen_at(), Some(5_000));
    }
}
