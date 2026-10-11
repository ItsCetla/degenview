use super::interval_seconds;
use crate::model::Kline;
use serde_json::Value;

const BASE_URL: &str = "https://api.geckoterminal.com/api/v2";
const MAX_CANDLES: i64 = 1_000;

/// Buckets GeckoTerminal serves natively, in seconds.
const BUCKETS: [(&str, i64, i64); 7] = [
    ("minute", 1, 60),
    ("minute", 5, 300),
    ("minute", 15, 900),
    ("hour", 1, 3_600),
    ("hour", 4, 14_400),
    ("hour", 12, 43_200),
    ("day", 1, 86_400),
];

struct Window {
    timeframe: &'static str,
    aggregate: i64,
    fetch_limit: usize,
    needs_aggregation: bool,
}

impl Window {
    fn for_interval(interval: &str, limit: usize) -> Window {
        let target = interval_seconds(interval).unwrap_or(3_600);
        let span = target * limit as i64;
        let bucket = BUCKETS
            .iter()
            .rev()
            .find(|(_, _, seconds)| *seconds <= target)
            .unwrap_or(&BUCKETS[0]);
        let wanted = (span + bucket.2 - 1) / bucket.2;
        Window {
            timeframe: bucket.0,
            aggregate: bucket.1,
            fetch_limit: wanted.clamp(1, MAX_CANDLES) as usize,
            needs_aggregation: bucket.2 < target,
        }
    }
}

/// OHLCV for an on-chain DEX pool. `pair_address` is a contract address; the
/// network is resolved through GeckoTerminal's pool search.
pub fn fetch_klines(pair_address: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
    if let Some(cached) = super::cache::get(
        "geckoterminal",
        pair_address,
        interval,
        std::time::Duration::from_secs(120),
    ) {
        let start = cached.len().saturating_sub(limit as usize);
        return Ok(cached[start..].to_vec());
    }
    let window = Window::for_interval(interval, limit as usize);
    let network = network_for(pair_address)?;
    let raw = fetch_ohlcv(&network, pair_address, &window)?;
    let mut ascending = raw;
    ascending.sort_by_key(|candle| candle.open_time);
    let candles = if window.needs_aggregation {
        aggregate_into(&ascending, limit as usize)
    } else {
        ascending
    };
    super::cache::set("geckoterminal", pair_address, interval, candles.clone());
    let start = candles.len().saturating_sub(limit as usize);
    Ok(candles[start..].to_vec())
}

fn fetch_ohlcv(network: &str, pool: &str, window: &Window) -> Result<Vec<Kline>, String> {
    let url = format!(
        "{BASE_URL}/networks/{network}/pools/{pool}/ohlcv/{}?aggregate={}&limit={}&currency=usd",
        window.timeframe, window.aggregate, window.fetch_limit
    );
    super::rate_limit::geckoterminal().wait_for_slot();
    let body = super::http::get_text(&url, &[], super::http::Retry::Never)
        .map_err(|error| format!("Network error: {error}"))?;
    parse_ohlcv(&body)
}

/// `[timestamp_seconds, open, high, low, close, volumeUSD]`, newest first.
fn parse_ohlcv(body: &str) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .pointer("/data/attributes/ohlcv_list")
        .and_then(Value::as_array)
        .ok_or_else(|| "No chart data for this pool".to_owned())?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let (Some(seconds), Some(open), Some(high), Some(low), Some(close)) = (
            fields[0].as_f64(),
            fields[1].as_f64(),
            fields[2].as_f64(),
            fields[3].as_f64(),
            fields[4].as_f64(),
        ) else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    if candles.is_empty() {
        return Err("No chart data for this pool".to_owned());
    }
    Ok(candles)
}

/// Which chain a pool address sits on. The per-pool endpoints are namespaced by
/// network, and DEXScreener's chain names don't match GeckoTerminal's ids.
fn network_for(pair_address: &str) -> Result<String, String> {
    let key = pair_address.to_lowercase();
    let url = format!("{BASE_URL}/search/pools?query={pair_address}");
    super::rate_limit::geckoterminal().wait_for_slot();
    let body = super::http::get_text(&url, &[], super::http::Retry::Never)
        .map_err(|error| format!("Network error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
    let suffix = format!("_{key}");
    for pool in value
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let Some(id) = pool.get("id").and_then(Value::as_str) else {
            continue;
        };
        let address = pool
            .pointer("/attributes/address")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let matches = address.to_lowercase() == key || id.to_lowercase().ends_with(&suffix);
        if !matches {
            continue;
        }
        if let Some(position) = id.to_lowercase().rfind(&suffix) {
            let network = &id[..position];
            if !network.is_empty() {
                return Ok(network.to_owned());
            }
        }
    }
    Err("Pool not indexed by GeckoTerminal".to_owned())
}

/// Fold ascending candles into `count` equal runs, keeping the extremes. Used where
/// the source has no bucket this coarse (GeckoTerminal tops out at daily).
fn aggregate_into(candles: &[Kline], count: usize) -> Vec<Kline> {
    if count == 0 || candles.len() <= count {
        return candles.to_vec();
    }
    let mut merged = Vec::with_capacity(count);
    for bucket in 0..count {
        let start = candles.len() * bucket / count;
        let end = candles.len() * (bucket + 1) / count;
        if start >= end {
            continue;
        }
        let run = &candles[start..end];
        let first = &run[0];
        let last = &run[run.len() - 1];
        merged.push(Kline {
            open_time: first.open_time,
            close_time: last.close_time,
            open: first.open,
            high: run.iter().map(|c| c.high).fold(f64::MIN, f64::max),
            low: run.iter().map(|c| c.low).fold(f64::MAX, f64::min),
            close: last.close,
            volume: run.iter().map(|c| c.volume).sum(),
        });
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_bucket_and_caps_fetch_limit() {
        let hourly = Window::for_interval("1h", 500);
        assert_eq!(hourly.timeframe, "hour");
        assert_eq!(hourly.aggregate, 1);
        assert_eq!(hourly.fetch_limit, 500);
        assert!(!hourly.needs_aggregation);

        let weekly = Window::for_interval("1w", 500);
        assert_eq!(weekly.timeframe, "day");
        assert!(weekly.needs_aggregation);
        assert_eq!(weekly.fetch_limit, MAX_CANDLES as usize);
    }

    #[test]
    fn aggregates_into_equal_runs() {
        let candles: Vec<Kline> = (0..6)
            .map(|i| Kline {
                open_time: i,
                close_time: i,
                open: 1.0,
                high: (i + 1) as f64,
                low: 0.5,
                close: 2.0,
                volume: 1.0,
            })
            .collect();
        let merged = aggregate_into(&candles, 3);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].high, 2.0);
        assert_eq!(merged[0].low, 0.5);
        assert_eq!(merged[0].volume, 2.0);
    }
}
