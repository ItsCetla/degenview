//! Pine alert-subscription runtime host: loads applied subscriptions and runs each active one's
//! source over a symbol's bars, returning the alerts that fired. Webhook/notification dispatch is
//! the caller's (the app), which already owns the webhook + notification services.
#![allow(dead_code)] // transient until the app threads it

use crate::model::Kline;
use crate::model::pine_alert::PineAlertSubscription;
use crate::pine::program::SymbolInfo;
use crate::pine::state::AlertEvent;
use crate::pine_host::run_script_alerts;
use crate::store::Store;

pub struct PineSubscriptionHost {
    subscriptions: Vec<PineAlertSubscription>,
}

impl PineSubscriptionHost {
    pub fn load(store: &Store) -> Self {
        Self {
            subscriptions: store.pine_subscriptions(),
        }
    }

    /// Re-reads the subscriptions after the store changed (created, paused, deleted …).
    pub fn reload(&mut self, store: &Store) {
        self.subscriptions = store.pine_subscriptions();
    }

    pub fn subscriptions(&self) -> &[PineAlertSubscription] {
        &self.subscriptions
    }

    /// Runs every active subscription that watches `symbol_key` over `bars`, returning
    /// `(subscription, alert)` pairs for the alerts that fired. A subscription without a stored
    /// source (saved before sources were persisted) is skipped.
    pub fn run_active(
        &self,
        symbol_key: &str,
        bars: &[Kline],
    ) -> Vec<(PineAlertSubscription, AlertEvent)> {
        let symbol = symbol_info(symbol_key);
        let mut fired = Vec::new();
        for subscription in &self.subscriptions {
            if !subscription.is_active()
                || subscription.symbol_key != symbol_key
                || subscription.source.is_empty()
            {
                continue;
            }
            for alert in run_script_alerts(&subscription.source, bars, &symbol) {
                fired.push((subscription.clone(), alert));
            }
        }
        fired
    }
}

/// Builds the `SymbolInfo` a subscription's `"<source>:<ticker>"` key implies. Ticker id is the
/// full key; the base currency is left `None` (the runtime treats it as a non-pair).
pub fn symbol_info(symbol_key: &str) -> SymbolInfo {
    let ticker = symbol_key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .unwrap_or(symbol_key);
    SymbolInfo {
        ticker: ticker.to_owned(),
        ticker_id: symbol_key.to_owned(),
        currency: "USD".to_owned(),
        kind: "crypto".to_owned(),
        base_currency: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::pine_alert::PineAlertState;

    fn bars() -> Vec<Kline> {
        (0..5)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1_000.0,
            })
            .collect()
    }

    fn subscription(id: u64, source: &str) -> PineAlertSubscription {
        PineAlertSubscription {
            id,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "S".into(),
            source: source.into(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "h".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: Vec::new(),
        }
    }

    #[test]
    fn active_alert_fires_per_watching_subscription() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[
                subscription(
                    1,
                    "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)",
                ),
                subscription(2, ""), // no source: skipped
            ])
            .expect("write");
        let host = PineSubscriptionHost::load(&store);
        let fired = host.run_active("binance:BTCUSDT", &bars());
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0.id, 1);
        assert_eq!(fired[0].1.message, "hi");
    }

    #[test]
    fn a_paused_or_other_symbol_subscription_does_not_fire() {
        let store = Store::open_in_memory().expect("open");
        let mut paused = subscription(
            1,
            "//@version=6\nindicator(\"S\")\nalert(\"x\", alert.freq_all)",
        );
        paused.state = PineAlertState::Paused;
        store.replace_pine_subscriptions(&[paused]).expect("write");
        let host = PineSubscriptionHost::load(&store);
        assert!(host.run_active("binance:BTCUSDT", &bars()).is_empty());
        assert!(host.run_active("binance:ETHUSDT", &bars()).is_empty());
    }

    #[test]
    fn reload_picks_up_a_subscription_added_after_load() {
        let store = Store::open_in_memory().expect("open");
        let mut host = PineSubscriptionHost::load(&store);
        assert!(host.subscriptions().is_empty());
        store
            .replace_pine_subscriptions(&[subscription(1, "plot(close)")])
            .expect("write");
        host.reload(&store);
        assert_eq!(host.subscriptions().len(), 1);
    }
}
