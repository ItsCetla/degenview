//! The market/symbol/timeframe identifiers the session's `market()` does not already handle,
//! ported from the tail of `PineRuntimeSession+Market.swift`. Pure: everything it needs to resolve
//! a name arrives in the [`MarketContext`], so the session folds [`read`] into its identifier
//! resolver. The `strategy.*` reads live in `strategy_builtins::strategy_value`.

use super::program::SymbolInfo;
use super::runtime_value::RuntimeValue;
use super::state::RuntimeState;
use super::ta;
use super::time;
use crate::model::Kline;

const MILLIS_PER_DAY: i64 = 86_400_000;

/// What [`read`] needs from the session to resolve a market identifier.
pub struct MarketContext<'a> {
    pub state: &'a RuntimeState,
    pub bar: &'a Kline,
    pub symbol: &'a SymbolInfo,
    pub bar_seconds: f64,
    pub visible_range: Option<(i64, i64)>,
    pub first_bar_time: Option<i64>,
    pub last_bar_time: Option<i64>,
}

/// The value of a market/symbol/timeframe identifier, or None when `name` is not one of them.
pub fn read(name: &str, context: &MarketContext) -> Option<RuntimeValue> {
    match name {
        "time_tradingday" => Some(RuntimeValue::Int(
            context.bar.open_time.div_euclid(MILLIS_PER_DAY) * MILLIS_PER_DAY,
        )),
        "timenow" => Some(RuntimeValue::Int(now_millis())),
        "ta.tr" => Some(RuntimeValue::Float(ta::true_range(
            context.bar,
            previous_close(context.state),
        ))),
        "syminfo.prefix" => Some(
            exchange_prefix(context.symbol)
                .map(RuntimeValue::String)
                .unwrap_or(RuntimeValue::Na),
        ),
        // Crypto exchanges run on UTC; for anything else the exchange's zone is not known.
        "syminfo.timezone" => Some(if context.symbol.kind == "crypto" {
            RuntimeValue::String("Etc/UTC".to_owned())
        } else {
            RuntimeValue::Na
        }),
        // The chart is always plain candles.
        "chart.is_standard" => Some(RuntimeValue::Bool(true)),
        "chart.is_heikinashi"
        | "chart.is_renko"
        | "chart.is_kagi"
        | "chart.is_linebreak"
        | "chart.is_pnf"
        | "chart.is_range" => Some(RuntimeValue::Bool(false)),
        "chart.left_visible_bar_time" => Some(
            context
                .visible_range
                .map(|range| range.0)
                .or(context.first_bar_time)
                .map(RuntimeValue::Int)
                .unwrap_or(RuntimeValue::Na),
        ),
        "chart.right_visible_bar_time" => Some(
            context
                .visible_range
                .map(|range| range.1)
                .or(context.last_bar_time)
                .map(RuntimeValue::Int)
                .unwrap_or(RuntimeValue::Na),
        ),
        // The chart theme is not modelled, so Pine reads both as `na`.
        "chart.fg_color" | "chart.bg_color" => Some(RuntimeValue::Na),
        _ if name.starts_with("timeframe.") => time::timeframe(name, context.bar_seconds),
        // Declared-but-unimplemented variables (`syminfo.session`, `weekofyear`, …) read as `na`.
        _ if super::symbol_catalog::is_unimplemented_variable(name) => Some(RuntimeValue::Na),
        _ => None,
    }
}

fn previous_close(state: &RuntimeState) -> Option<f64> {
    state
        .histories
        .get("close")?
        .last()
        .and_then(RuntimeValue::number)
}

/// The exchange part of `<exchange>:<ticker>`, upper-cased as TradingView shows it.
fn exchange_prefix(symbol: &SymbolInfo) -> Option<String> {
    let (prefix, _) = symbol.ticker_id.split_once(':')?;
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_uppercase())
    }
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        state: RuntimeState,
        bar: Kline,
        symbol: SymbolInfo,
        bar_seconds: f64,
        visible_range: Option<(i64, i64)>,
        first_bar_time: Option<i64>,
        last_bar_time: Option<i64>,
    }

    impl Fixture {
        fn new(open_time: i64, bar_seconds: f64) -> Self {
            Self {
                state: RuntimeState::default(),
                bar: Kline {
                    open_time,
                    close_time: open_time + 60_000,
                    open: 10.0,
                    high: 15.0,
                    low: 8.0,
                    close: 12.0,
                    volume: 100.0,
                },
                symbol: SymbolInfo {
                    ticker: "BTCUSDT".to_owned(),
                    ticker_id: "BINANCE:BTCUSDT".to_owned(),
                    ..SymbolInfo::default()
                },
                bar_seconds,
                visible_range: None,
                first_bar_time: Some(1_000),
                last_bar_time: Some(2_000),
            }
        }

        fn read(&self, name: &str) -> Option<RuntimeValue> {
            read(
                name,
                &MarketContext {
                    state: &self.state,
                    bar: &self.bar,
                    symbol: &self.symbol,
                    bar_seconds: self.bar_seconds,
                    visible_range: self.visible_range,
                    first_bar_time: self.first_bar_time,
                    last_bar_time: self.last_bar_time,
                },
            )
        }
    }

    #[test]
    fn trading_day_floors_to_utc_midnight() {
        // 2024-05-01 13:45:00 UTC = 1714567500000 ms; the day starts 1714521600000.
        let fixture = Fixture::new(1_714_567_500_000, 60.0);
        assert_eq!(
            fixture.read("time_tradingday"),
            Some(RuntimeValue::Int(1_714_521_600_000))
        );
    }

    #[test]
    fn true_range_uses_the_previous_close() {
        let mut fixture = Fixture::new(1_000, 60.0);
        fixture
            .state
            .histories
            .insert("close".to_owned(), vec![RuntimeValue::Float(9.0)]);
        // max(high, prev_close) - min(low, prev_close) = 15 - 8 = 7.
        assert_eq!(fixture.read("ta.tr"), Some(RuntimeValue::Float(7.0)));
    }

    #[test]
    fn symbol_and_chart_facts() {
        let fixture = Fixture::new(1_000, 60.0);
        assert_eq!(
            fixture.read("syminfo.prefix"),
            Some(RuntimeValue::String("BINANCE".to_owned()))
        );
        assert_eq!(
            fixture.read("syminfo.timezone"),
            Some(RuntimeValue::String("Etc/UTC".to_owned()))
        );
        assert_eq!(
            fixture.read("chart.is_standard"),
            Some(RuntimeValue::Bool(true))
        );
        assert_eq!(
            fixture.read("chart.is_renko"),
            Some(RuntimeValue::Bool(false))
        );
        assert_eq!(fixture.read("syminfo.nope"), None);
    }

    #[test]
    fn visible_bar_times_fall_back_to_the_run_bounds() {
        let mut fixture = Fixture::new(1_000, 60.0);
        assert_eq!(
            fixture.read("chart.left_visible_bar_time"),
            Some(RuntimeValue::Int(1_000))
        );
        assert_eq!(
            fixture.read("chart.right_visible_bar_time"),
            Some(RuntimeValue::Int(2_000))
        );
        fixture.visible_range = Some((500, 5_000));
        assert_eq!(
            fixture.read("chart.left_visible_bar_time"),
            Some(RuntimeValue::Int(500))
        );
        assert_eq!(
            fixture.read("chart.right_visible_bar_time"),
            Some(RuntimeValue::Int(5_000))
        );
    }

    #[test]
    fn unmodelled_chart_and_unimplemented_names_read_na() {
        let fixture = Fixture::new(1_000, 60.0);
        assert_eq!(fixture.read("chart.fg_color"), Some(RuntimeValue::Na));
        assert_eq!(fixture.read("chart.bg_color"), Some(RuntimeValue::Na));
        assert_eq!(fixture.read("syminfo.session"), Some(RuntimeValue::Na));
        assert_eq!(fixture.read("weekofyear"), Some(RuntimeValue::Na));
    }
}
