//! Which price alerts a chart draws as lines, and where.
//!
//! An alert watches one market for one level. A chart of that market shows each live alert as a
//! horizontal line at its level, in the chart's own price units. Pure: it reads the alerts and
//! the FX rates the app holds and says nothing about drawing.

use super::InstrumentId;
use super::alert::{AlertCurrency, AlertId, AlertState, PriceAlert};
use std::collections::HashMap;

/// One alert on one chart.
#[derive(Clone, Debug, PartialEq)]
pub struct AlertLine {
    pub id: AlertId,
    /// The level in the chart's prices (USD).
    pub price: f64,
    pub paused: bool,
    /// Whether the alert fires when the price rises to the level.
    pub above: bool,
}

/// The lines for the chart of `market`: its active and paused alerts that have a level. Levels
/// are converted to USD with `rates` (USD → currency); an alert in a currency with no rate is
/// left off rather than drawn at a guessed level.
pub fn lines_for(
    alerts: &[PriceAlert],
    market: &InstrumentId,
    rates: &HashMap<AlertCurrency, f64>,
) -> Vec<AlertLine> {
    alerts
        .iter()
        .filter(|alert| alert.asset.instrument == *market)
        .filter(|alert| matches!(alert.state, AlertState::Active | AlertState::Paused))
        .filter_map(|alert| {
            let level = alert.condition.target().filter(|level| level.is_finite())?;
            let rate = if alert.currency == AlertCurrency::Usd {
                1.0
            } else {
                *rates.get(&alert.currency).filter(|rate| **rate > 0.0)?
            };
            Some(AlertLine {
                id: alert.id,
                price: level / rate,
                paused: alert.state == AlertState::Paused,
                above: alert.condition.crosses_above(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::alert::{AlertAsset, AlertCondition};

    fn market(symbol: &str) -> InstrumentId {
        InstrumentId::new(DataSource::Binance, symbol)
    }

    fn alert(id: u64, symbol: &str, condition: AlertCondition) -> PriceAlert {
        let mut alert = PriceAlert::new(
            AlertAsset::new(market(symbol), symbol.to_owned(), symbol.to_owned()),
            condition,
            0,
        );
        alert.id = id;
        alert
    }

    #[test]
    fn only_this_markets_live_alerts_become_lines() {
        let mut paused = alert(2, "BTCUSDT", AlertCondition::CrossesBelow { target: 90.0 });
        paused.state = AlertState::Paused;
        let mut done = alert(3, "BTCUSDT", AlertCondition::CrossesAbove { target: 120.0 });
        done.state = AlertState::Triggered;
        let alerts = [
            alert(1, "BTCUSDT", AlertCondition::CrossesAbove { target: 110.0 }),
            paused,
            done,
            alert(4, "ETHUSDT", AlertCondition::CrossesAbove { target: 5.0 }),
        ];
        let lines = lines_for(&alerts, &market("BTCUSDT"), &HashMap::new());
        assert_eq!(
            lines,
            [
                AlertLine {
                    id: 1,
                    price: 110.0,
                    paused: false,
                    above: true
                },
                AlertLine {
                    id: 2,
                    price: 90.0,
                    paused: true,
                    above: false
                },
            ]
        );
    }

    #[test]
    fn another_currency_is_converted_or_left_off() {
        let mut euro = alert(1, "BTCUSDT", AlertCondition::CrossesAbove { target: 90.0 });
        euro.currency = AlertCurrency::Eur;
        let alerts = [euro];
        assert!(lines_for(&alerts, &market("BTCUSDT"), &HashMap::new()).is_empty());
        let rates = HashMap::from([(AlertCurrency::Eur, 0.9)]);
        let lines = lines_for(&alerts, &market("BTCUSDT"), &rates);
        assert!((lines[0].price - 100.0).abs() < 1e-9);
        let bad = HashMap::from([(AlertCurrency::Eur, 0.0)]);
        assert!(lines_for(&alerts, &market("BTCUSDT"), &bad).is_empty());
    }

    #[test]
    fn a_level_that_is_not_a_number_is_not_drawn() {
        let alerts = [alert(
            1,
            "BTCUSDT",
            AlertCondition::CrossesAbove { target: f64::NAN },
        )];
        assert!(lines_for(&alerts, &market("BTCUSDT"), &HashMap::new()).is_empty());
    }
}
