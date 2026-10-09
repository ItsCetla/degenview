//! Pine alert-subscription persistence. Each row is one applied `PineAlertSubscription` document,
//! keyed by its id, stored under `pine_alert_subscription` like the other ordered document tables.
#![allow(dead_code)] // transient until the Pine runtime host wires these

use crate::model::pine_alert::PineAlertSubscription;
use crate::store::{Document, Store};

impl Document for PineAlertSubscription {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    pub fn pine_subscriptions(&self) -> Vec<PineAlertSubscription> {
        self.documents("pine_alert_subscription")
    }

    pub fn replace_pine_subscriptions(
        &self,
        subscriptions: &[PineAlertSubscription],
    ) -> rusqlite::Result<()> {
        self.replace_documents("pine_alert_subscription", subscriptions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::pine_alert::PineAlertState;

    fn subscription(id: u64) -> PineAlertSubscription {
        PineAlertSubscription {
            id,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "My Script".into(),
            source: String::new(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "abc".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 1_000,
            webhook_endpoint_ids: Vec::new(),
        }
    }

    #[test]
    fn subscriptions_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(7), subscription(9)])
            .expect("write");
        let loaded = store.pine_subscriptions();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            (loaded[0].id, loaded[0].script_name.as_str()),
            (7, "My Script")
        );
        assert_eq!(loaded[1].id, 9);
    }
}
