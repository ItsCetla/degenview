use super::{MarketSource, SearchResult, downsample};
use crate::model::{DataSource, Kline};
use serde_json::Value;

const BASE: &str = "https://pro-api.coinmarketcap.com";

pub const ALTCOIN_HISTORICAL: &str = "cmc.altcoin_season_historical";
pub const ALTCOIN_LATEST: &str = "cmc.altcoin_season_latest";
pub const FEAR_GREED_HISTORICAL: &str = "cmc.fear_and_greed_historical";
pub const FEAR_GREED_LATEST: &str = "cmc.fear_and_greed_latest";

/// The chart types offered by the CoinMarketCap tab.
pub const CHART_TYPES: [(&str, &str); 4] = [
    (ALTCOIN_HISTORICAL, "Altcoin Season Index Historical"),
    (ALTCOIN_LATEST, "Altcoin Season Index Latest"),
    (FEAR_GREED_HISTORICAL, "Crypto Fear and Greed Historical"),
    (FEAR_GREED_LATEST, "Crypto Fear and Greed Latest"),
];

pub struct CoinMarketCap;

pub const COIN_MARKET_CAP: CoinMarketCap = CoinMarketCap;

fn api_key() -> Result<String, String> {
    let key = std::env::var("DEGENVIEW_CMC_API_KEY")
        .ok()
        .unwrap_or_default();
    if key.is_empty() {
        Err("CoinMarketCap API key required (set DEGENVIEW_CMC_API_KEY).".to_owned())
    } else {
        Ok(key)
    }
}

fn get(path: &str, key: &str) -> Result<Value, String> {
    let url = format!("{BASE}{path}");
    let mut response = ureq::get(&url)
        .header("X-CMC_PRO_API_KEY", key)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| format!("CoinMarketCap error: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Network error: {error}"))?;
    let value: Value =
        serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
    if value["status"]["error_code"].as_i64().unwrap_or(0) != 0 {
        return Err(value["status"]["error_message"]
            .as_str()
            .unwrap_or("CoinMarketCap API error")
            .to_owned());
    }
    Ok(value)
}

fn to_klines(points: Vec<(i64, f64)>, limit: u32) -> Result<Vec<Kline>, String> {
    let candles: Vec<Kline> = points
        .into_iter()
        .map(|(t, value)| Kline {
            open_time: t,
            close_time: t,
            open: value,
            high: value,
            low: value,
            close: value,
            volume: 0.0,
        })
        .collect();
    if candles.is_empty() {
        return Err("No CoinMarketCap data".to_owned());
    }
    Ok(downsample(&candles, limit as usize))
}

impl MarketSource for CoinMarketCap {
    fn fetch_klines(
        &self,
        symbol: &str,
        _interval: &str,
        limit: u32,
    ) -> Result<Vec<Kline>, String> {
        let key = api_key()?;
        match symbol {
            ALTCOIN_HISTORICAL => {
                let value = get("/v1/altcoin-season-index/historical?timeframe=7d", &key)?;
                let points = value
                    .pointer("/data/points")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|point| {
                        let t = parse_date(point.get("timestamp")?.as_str()?)?;
                        let v = point.get("altcoin_index")?.as_f64()?;
                        Some((t, v))
                    })
                    .collect();
                to_klines(points, limit)
            }
            FEAR_GREED_HISTORICAL => {
                let value = get("/v3/fear-and-greed/historical?start=1&limit=500", &key)?;
                let points = value
                    .get("data")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|point| {
                        let t = parse_date(point.get("timestamp")?.as_str()?)?;
                        let v = point.get("value")?.as_f64()?;
                        Some((t, v))
                    })
                    .collect();
                to_klines(points, limit)
            }
            ALTCOIN_LATEST | FEAR_GREED_LATEST => {
                Err("CoinMarketCap gauge charts arrive in a later slice".to_owned())
            }
            _ => Err("Unknown CoinMarketCap chart".to_owned()),
        }
    }

    fn search_tickers(&self, _query: &str) -> Result<Vec<SearchResult>, String> {
        Ok(CHART_TYPES
            .iter()
            .map(|(key, title)| SearchResult {
                symbol: (*title).to_owned(),
                full_symbol: (*key).to_owned(),
                source: DataSource::CoinMarketCap,
                price: None,
            })
            .collect())
    }
}

/// CMC timestamps arrive as epoch seconds, ISO-8601, or a bare date.
fn parse_date(text: &str) -> Option<i64> {
    if let Ok(seconds) = text.parse::<f64>() {
        return Some((seconds * 1_000.0) as i64);
    }
    let (date, time) = if let Some((date, time)) = text.split_once('T') {
        (date, time.trim_end_matches('Z'))
    } else if let Some((date, time)) = text.split_once(' ') {
        (date, time)
    } else {
        (text, "00:00:00")
    };
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts
        .next()
        .unwrap_or("0")
        .split('.')
        .next()?
        .parse()
        .ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cmc_dates() {
        assert_eq!(parse_date("2026-10-09"), Some(1_791_504_000_000));
        assert_eq!(parse_date("2026-10-09T12:00:00Z"), Some(1_791_547_200_000));
        assert_eq!(parse_date("1791504000"), Some(1_791_504_000_000));
    }
}
