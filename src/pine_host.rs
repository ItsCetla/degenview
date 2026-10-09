//! Minimal Pine runtime host: compiles a script and runs it over a series of bars, returning the
//! chart `VisualOutput` the renderer consumes. Input resolution and library imports are wired
//! incrementally; the session itself owns the per-bar state machine.
#![allow(dead_code)] // transient until the chart wires this

use crate::model::{DataSource, Kline};
use crate::pine::compiler::{self, NoLibraries};
use crate::pine::program::{BarEvent, BarPhase, SymbolInfo, VisualOutput};
use crate::pine::runtime_support::{SecurityDataProvider, SecurityKey};
use crate::pine::session::Session;
use crate::pine::state::AlertEvent;
use crate::pine::token::Limits;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fetches `request.security` series through the REST kline service, so a script can read a symbol
/// the chart does not carry. `candles` is blocking: one synchronous request per distinct key, which
/// the session caches for the rest of the run.
pub struct KlineSecurityDataProvider;

/// ~30 s TTL so `run_script_alerts` (once per new bar) does not re-fetch a foreign symbol each bar.
const SECURITY_CACHE_TTL_MS: i64 = 30_000;
type SecurityCache = HashMap<SecurityKey, (Vec<Kline>, i64)>;
static SECURITY_CACHE: LazyLock<Mutex<SecurityCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl SecurityDataProvider for KlineSecurityDataProvider {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        let now = now_millis();
        {
            let cache = SECURITY_CACHE.lock().unwrap();
            if let Some((candles, fetched_at)) = cache.get(key)
                && now - fetched_at < SECURITY_CACHE_TTL_MS
            {
                return Some(candles.clone());
            }
        }
        let (source, symbol) = resolve_symbol(&key.symbol)?;
        let interval = resolve_interval(key.interval)?;
        let candles = crate::service::fetch_klines(source, &symbol, interval, 500).ok()?;
        SECURITY_CACHE
            .lock()
            .unwrap()
            .insert(key.clone(), (candles.clone(), now));
        Some(candles)
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as i64)
}

/// `SOURCE:SYMBOL` resolves to that source; a bare symbol defaults to Binance, the chart's default
/// source.
fn resolve_symbol(symbol: &str) -> Option<(DataSource, String)> {
    if let Some((prefix, rest)) = symbol.split_once(':') {
        return Some((source_for_slug(prefix)?, rest.to_owned()));
    }
    Some((DataSource::Binance, symbol.to_owned()))
}

fn source_for_slug(slug: &str) -> Option<DataSource> {
    match slug.to_ascii_uppercase().as_str() {
        "BINANCE" => Some(DataSource::Binance),
        "COINBASE" => Some(DataSource::Coinbase),
        "COINGECKO" => Some(DataSource::CoinGecko),
        "DEXSCREENER" => Some(DataSource::DexScreener),
        "ALPACA" => Some(DataSource::Alpaca),
        "POLYMARKET" => Some(DataSource::Polymarket),
        "KALSHI" => Some(DataSource::Kalshi),
        "CMC" => Some(DataSource::CoinMarketCap),
        _ => None,
    }
}

/// The kline-service interval token for a bar length in seconds. None for a length the service does
/// not serve directly.
fn resolve_interval(seconds: f64) -> Option<&'static str> {
    const TOKENS: &[(f64, &str)] = &[
        (60.0, "1m"),
        (300.0, "5m"),
        (900.0, "15m"),
        (1_800.0, "30m"),
        (3_600.0, "1h"),
        (14_400.0, "4h"),
        (86_400.0, "1d"),
        (604_800.0, "1w"),
        (2_592_000.0, "1M"),
        (7_776_000.0, "3M"),
        (31_536_000.0, "1Y"),
    ];
    TOKENS
        .iter()
        .find(|(length, _)| (*length - seconds).abs() < 0.5)
        .map(|(_, token)| *token)
}

/// Compiles `source` and runs it once over every bar (historical phase), returning the chart
/// output. A script with syntax/type errors yields an empty output plus the program's diagnostics.
pub fn run_script(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> VisualOutput {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    if !program.is_valid() || program.statements.is_empty() {
        return VisualOutput {
            overlay: false,
            bar_count: 0,
            ..Default::default()
        };
    }
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol.clone(),
        Some(Box::new(KlineSecurityDataProvider)),
        None,
    );
    session.infer_mintick(bars);
    for (index, bar) in bars.iter().enumerate() {
        let is_last = index + 1 == bars.len();
        if session
            .execute(
                BarEvent {
                    candle: bar.clone(),
                    phase: BarPhase::Historical,
                },
                is_last,
                None,
            )
            .is_err()
        {
            break;
        }
    }
    session.output()
}

/// Compiles + runs `source` over the bars, replaying the last bar as a realtime close so Pine
/// `alert()`/`alertcondition()` calls fire, and returns the alerts that fired on that bar.
pub fn run_script_alerts(source: &str, bars: &[Kline], symbol: &SymbolInfo) -> Vec<AlertEvent> {
    let program = compiler::compile(source, Limits::default(), &NoLibraries);
    if !program.is_valid() || program.statements.is_empty() {
        return Vec::new();
    }
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol.clone(),
        Some(Box::new(KlineSecurityDataProvider)),
        None,
    );
    session.infer_mintick(bars);
    for (index, bar) in bars.iter().enumerate() {
        let is_last = index + 1 == bars.len();
        let phase = if is_last {
            BarPhase::RealtimeClose { is_new: true }
        } else {
            BarPhase::Historical
        };
        if session
            .execute(
                BarEvent {
                    candle: bar.clone(),
                    phase,
                },
                is_last,
                None,
            )
            .is_err()
        {
            return Vec::new();
        }
    }
    session.emitted_alerts.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Vec<Kline> {
        (0..5)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 102.0 + i as f64,
                volume: 1_000.0,
            })
            .collect()
    }

    #[test]
    fn a_plot_script_produces_output() {
        let source = "//@version=6\nindicator(\"T\")\nplot(close)";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.bar_count >= 5);
        assert_eq!(output.plots.len(), 1);
    }

    #[test]
    fn an_invalid_script_produces_empty_output() {
        let source = "this is not pine";
        let output = run_script(source, &bars(), &SymbolInfo::default());
        assert!(output.plots.is_empty());
    }

    #[test]
    fn an_alert_script_fires_on_the_realtime_bar() {
        let source = "//@version=6\nindicator(\"T\")\nalert(\"hi\", alert.freq_all)";
        let alerts = run_script_alerts(source, &bars(), &SymbolInfo::default());
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].message, "hi");
        assert!(alerts[0].is_realtime);
    }

    #[test]
    fn mintick_is_inferred_from_low_priced_bars() {
        // Four-decimal prices infer a 0.0001 tick, not the 0.01 default.
        let low_bars: Vec<Kline> = (0..3)
            .map(|i| Kline {
                open_time: i as i64 * 3_600_000,
                close_time: (i as i64 + 1) * 3_600_000,
                open: 0.0001 + i as f64 * 0.0001,
                high: 0.0002 + i as f64 * 0.0001,
                low: 0.0001,
                close: 0.0001 + i as f64 * 0.0001,
                volume: 1_000.0,
            })
            .collect();
        let source = "//@version=6\nindicator(\"T\")\nplot(syminfo.mintick)";
        let output = run_script(source, &low_bars, &SymbolInfo::default());
        let last = output.plots[0].values.last().copied().flatten();
        assert_eq!(last, Some(0.0001));
    }

    #[test]
    fn security_symbols_resolve_to_a_source() {
        assert_eq!(
            resolve_symbol("ETHUSDT"),
            Some((DataSource::Binance, "ETHUSDT".to_owned()))
        );
        assert_eq!(
            resolve_symbol("BINANCE:ETHUSDT"),
            Some((DataSource::Binance, "ETHUSDT".to_owned()))
        );
        assert_eq!(
            resolve_symbol("COINBASE:BTC-USD"),
            Some((DataSource::Coinbase, "BTC-USD".to_owned()))
        );
        assert_eq!(resolve_symbol("NOPE:ETHUSDT"), None);
    }

    #[test]
    fn security_intervals_map_to_kline_tokens() {
        assert_eq!(resolve_interval(60.0), Some("1m"));
        assert_eq!(resolve_interval(3_600.0), Some("1h"));
        assert_eq!(resolve_interval(86_400.0), Some("1d"));
        assert_eq!(resolve_interval(2_592_000.0), Some("1M"));
        assert_eq!(resolve_interval(120.0), None); // 2m is not served directly
    }
}
