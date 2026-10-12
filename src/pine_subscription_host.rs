//! Pine alert-subscription runtime host: loads applied subscriptions and feeds each active one's
//! persistent [`ExecutionController`] the chart's latest candles, returning the alerts raised by
//! realtime ticks and closes. Webhook/notification dispatch is the caller's (the app), which
//! already owns the webhook + notification services.
#![allow(dead_code)] // transient until the app threads it

use crate::model::Kline;
use crate::model::pine_alert::{PineAlertState, PineAlertSubscription};
use crate::model::pine_instance::PineInstance;
use crate::pine::execution::ExecutionController;
use crate::pine::program::SymbolInfo;
use crate::pine::state::AlertEvent;
use crate::pine::token::Limits;
use crate::pine_host::KlineSecurityDataProvider;
use crate::script_manager::registry::LibraryRegistry;
use crate::store::Store;
use std::collections::HashMap;

/// What a running controller was built from. Any difference means the controller is stale and
/// must be rebuilt (without notifying for the history it replays).
#[derive(PartialEq)]
struct RunKey {
    chart_id: u64,
    instance_id: Option<u64>,
    symbol_key: String,
    timeframe: String,
    source_hash: String,
    inputs: HashMap<String, crate::model::pine_instance::StoredInput>,
}

struct Running {
    key: RunKey,
    /// `None` when the source failed to compile; retried only when the key changes.
    controller: Option<ExecutionController>,
}

pub struct PineSubscriptionHost {
    subscriptions: Vec<PineAlertSubscription>,
    running: HashMap<u64, Running>,
}

impl PineSubscriptionHost {
    pub fn load(store: &Store) -> Self {
        Self {
            subscriptions: store.pine_subscriptions(),
            running: HashMap::new(),
        }
    }

    /// Re-reads the subscriptions after the store changed (created, paused, deleted …).
    pub fn reload(&mut self, store: &Store) {
        self.subscriptions = store.pine_subscriptions();
    }

    pub fn subscriptions(&self) -> &[PineAlertSubscription] {
        &self.subscriptions
    }

    /// Pins active subscriptions to the applied instance that created them. A removed instance
    /// or changed source becomes silent until explicitly re-armed. Current instance inputs are
    /// persisted so the independent host never falls back to input defaults.
    pub fn reconcile_chart(&mut self, chart_id: u64, instances: &[PineInstance], store: &Store) {
        let mut changed = false;
        for subscription in self
            .subscriptions
            .iter_mut()
            .filter(|subscription| subscription.chart_id == chart_id && subscription.is_active())
        {
            let Some(instance_id) = subscription.instance_id else {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            };
            let Some(instance) = instances.iter().find(|instance| instance.id == instance_id)
            else {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            };
            let current_hash = crate::script_manager::store::hash_source(&instance.source);
            if subscription.source_hash != current_hash {
                subscription.state = PineAlertState::ScriptChanged;
                changed = true;
                continue;
            }
            if subscription.inputs != instance.inputs {
                subscription.inputs = instance.inputs.clone();
                changed = true;
            }
            if subscription.script_id != instance.script_id {
                subscription.script_id = instance.script_id.clone();
                changed = true;
            }
        }
        if changed {
            let _ = store.replace_pine_subscriptions(&self.subscriptions);
        }
    }

    /// Feeds `bars` to every active subscription that watches `symbol_key`, returning
    /// `(subscription, alert)` pairs for the alerts raised by the new realtime tick or close.
    /// Each subscription owns one persistent controller, so a forming bar is re-evaluated as a
    /// tick (`freq_all`, `freq_once_per_bar`) and `freq_once_per_bar_close` waits for the real
    /// close. The first snapshot, and any rebuild after the market, source or inputs changed,
    /// replays history silently. A subscription without a stored source is skipped.
    pub fn run_active(
        &mut self,
        chart_id: u64,
        symbol_key: &str,
        timeframe: &str,
        bars: &[Kline],
        instances: &[PineInstance],
        now_ms: i64,
    ) -> Vec<(PineAlertSubscription, AlertEvent)> {
        let symbol = symbol_info(symbol_key);
        let mut fired = Vec::new();
        let live: Vec<u64> = self
            .subscriptions
            .iter()
            .filter(|subscription| subscription.is_active())
            .map(|subscription| subscription.id)
            .collect();
        self.running.retain(|id, _| live.contains(id));
        for subscription in &self.subscriptions {
            if !subscription.is_active()
                || subscription.chart_id != chart_id
                || subscription.symbol_key != symbol_key
                || subscription.timeframe != timeframe
                || subscription.source.is_empty()
                || !subscription.instance_id.is_some_and(|instance_id| {
                    instances.iter().any(|instance| {
                        instance.id == instance_id
                            && crate::script_manager::store::hash_source(&instance.source)
                                == subscription.source_hash
                    })
                })
            {
                continue;
            }
            let key = RunKey {
                chart_id,
                instance_id: subscription.instance_id,
                symbol_key: symbol_key.to_owned(),
                timeframe: timeframe.to_owned(),
                source_hash: subscription.source_hash.clone(),
                inputs: subscription.inputs.clone(),
            };
            if self
                .running
                .get(&subscription.id)
                .is_none_or(|running| running.key != key)
            {
                let controller = ExecutionController::new(
                    &subscription.source,
                    subscription.input_values(),
                    symbol.clone(),
                    None,
                    Limits::default(),
                    &LibraryRegistry,
                    Some(Box::new(KlineSecurityDataProvider)),
                    None,
                )
                .ok();
                self.running
                    .insert(subscription.id, Running { key, controller });
            }
            let Some(controller) = self
                .running
                .get_mut(&subscription.id)
                .and_then(|running| running.controller.as_mut())
            else {
                continue;
            };
            let update = controller.update(bars, None, now_ms);
            if update.rebuilt {
                continue;
            }
            fired.extend(
                update
                    .alerts
                    .into_iter()
                    .map(|alert| (subscription.clone(), alert)),
            );
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
            instance_id: Some(10),
            script_id: None,
            script_name: "S".into(),
            source: source.into(),
            inputs: Default::default(),
            symbol_key: "binance:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: crate::script_manager::store::hash_source(source),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: Vec::new(),
        }
    }

    fn instance(source: &str) -> PineInstance {
        PineInstance::new(10, Some("saved-script".into()), "S", source)
    }

    const LIVE_NOW: i64 = 17_000_000;

    #[test]
    fn active_alert_fires_on_realtime_updates_not_the_initial_snapshot() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"hi\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source), subscription(2, "")])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let applied = [instance(source)];
        let mut data = bars();
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW)
                .is_empty()
        );

        data[4].close += 1.0;
        let fired = host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + 1);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].0.id, 1);
        assert_eq!(fired[0].1.message, "hi");

        assert!(
            host.run_active(2, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW)
                .is_empty()
        );
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1d", &data, &applied, LIVE_NOW)
                .is_empty()
        );
    }

    #[test]
    fn close_frequency_waits_for_the_real_bar_close() {
        let source =
            "//@version=6\nindicator(\"S\")\nalert(\"done\", alert.freq_once_per_bar_close)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let applied = [instance(source)];
        let mut data = bars();
        host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW);
        data[4].close += 1.0;
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, LIVE_NOW + 1)
                .is_empty()
        );
        let fired = host.run_active(1, "binance:BTCUSDT", "1h", &data, &applied, 18_000_000);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].1.message, "done");
    }

    #[test]
    fn a_paused_or_other_symbol_subscription_does_not_fire() {
        let source = "//@version=6\nindicator(\"S\")\nalert(\"x\", alert.freq_all)";
        let store = Store::open_in_memory().expect("open");
        let mut paused = subscription(1, source);
        paused.state = PineAlertState::Paused;
        store.replace_pine_subscriptions(&[paused]).expect("write");
        let mut host = PineSubscriptionHost::load(&store);
        let data = bars();
        assert!(
            host.run_active(1, "binance:BTCUSDT", "1h", &data, &[], LIVE_NOW)
                .is_empty()
        );
        assert!(
            host.run_active(1, "binance:ETHUSDT", "1h", &data, &[], LIVE_NOW)
                .is_empty()
        );
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

    #[test]
    fn reconciliation_persists_inputs_and_silences_changed_or_removed_instances() {
        use crate::model::pine_instance::StoredInput;

        let source = "//@version=6\nindicator(\"S\")\nlen = input.int(2)\nplot(len)";
        let store = Store::open_in_memory().expect("open");
        store
            .replace_pine_subscriptions(&[subscription(1, source)])
            .expect("write");
        let mut applied = instance(source);
        applied.inputs.insert("len".into(), StoredInput::Int(9));
        let mut host = PineSubscriptionHost::load(&store);
        host.reconcile_chart(1, &[applied.clone()], &store);
        assert_eq!(
            store.pine_subscriptions()[0].inputs.get("len"),
            Some(&StoredInput::Int(9))
        );

        applied.source.push_str("\nplot(open)");
        host.reconcile_chart(1, &[applied], &store);
        assert_eq!(
            store.pine_subscriptions()[0].state,
            PineAlertState::ScriptChanged
        );
    }
}
