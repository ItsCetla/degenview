use super::{
    MarketSource, SearchResult, downsample, effective_span_days, line_chart_point_count,
    replace_last_price,
};
use crate::model::Kline;
use serde_json::Value;

const GAMMA_BASE: &str = "https://gamma-api.polymarket.com";
const CLOB_BASE: &str = "https://clob.polymarket.com";

pub struct Polymarket;

pub const POLYMARKET: Polymarket = Polymarket;

/// Gamma serializes `outcomes`/`outcomePrices`/`clobTokenIds` as JSON strings that
/// themselves contain arrays; they need a second decode pass.
fn decode_string_array(raw: &Value) -> Vec<String> {
    let text = raw.as_str().unwrap_or("");
    serde_json::from_str::<Vec<String>>(text).unwrap_or_default()
}

/// A price in 0…1, either as a number or a string.
fn probability(raw: &Value) -> Option<f64> {
    raw.as_f64()
        .or_else(|| raw.as_str().and_then(|text| text.parse().ok()))
        .filter(|value| (0.0..=1.0).contains(value))
}

/// (interval token, fidelity) for the CLOB `/prices-history` endpoint.
fn pm_window(interval: &str) -> (String, i64) {
    let span = effective_span_days(interval);
    let needed = line_chart_point_count(interval) * 2;

    let token = if span <= 1 {
        "1h"
    } else if span <= 2 {
        "1d"
    } else if span <= 8 {
        "1w"
    } else if span <= 31 {
        "1m"
    } else {
        "max"
    };
    let interval_minutes: i64 = match token {
        "max" | "1m" => 43_200,
        "1w" => 10_080,
        "1d" => 1_440,
        _ => 60,
    };
    let fidelity = (interval_minutes / needed as i64).max(1);
    (token.to_owned(), fidelity)
}

impl MarketSource for Polymarket {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let (window, fidelity) = pm_window(interval);
        let url = format!(
            "{CLOB_BASE}/prices-history?market={symbol}&interval={window}&fidelity={fidelity}"
        );
        let body = super::http::get_text(&url, &[], super::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

        let mut points: Vec<(i64, f64)> = value
            .get("history")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|point| {
                let t = point.get("t").and_then(Value::as_f64)?;
                let p = point.get("p").and_then(Value::as_f64)?;
                Some(((t * 1_000.0) as i64, p))
            })
            .collect();
        points.sort_by_key(|(t, _)| *t);

        let candles: Vec<Kline> = points
            .into_iter()
            .map(|(t, p)| Kline {
                open_time: t,
                close_time: t,
                open: p,
                high: p,
                low: p,
                close: p,
                volume: 0.0,
            })
            .collect();
        if candles.is_empty() {
            return Err("No price history for this market in this range".to_owned());
        }

        let thinned = downsample(&candles, limit as usize);
        Ok(match fetch_current_ask(symbol) {
            Some(price) => replace_last_price(thinned, price),
            None => thinned,
        })
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url =
            format!("{GAMMA_BASE}/public-search?q={q}&limit_per_type=10&events_status=active");
        let body = super::http::get_text(&url, &[], super::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;

        let mut results = Vec::new();
        for event in value
            .get("events")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            for market in event
                .get("markets")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                if market.get("closed").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let Some(token_id) = decode_string_array(&market["clobTokenIds"])
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                let label = market
                    .get("groupItemTitle")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| market.get("question").and_then(Value::as_str))
                    .unwrap_or_default();
                if label.is_empty() {
                    continue;
                }
                let price = probability(&market["bestAsk"]).or_else(|| {
                    decode_string_array(&market["outcomePrices"])
                        .into_iter()
                        .next()
                        .and_then(|text| text.parse().ok())
                        .filter(|value| (0.0..=1.0).contains(value))
                });
                results.push(SearchResult {
                    symbol: label.to_owned(),
                    full_symbol: token_id,
                    source: crate::model::DataSource::Polymarket,
                    price,
                });
            }
        }
        Ok(results)
    }
}

/// The executable YES ask, matching the market page's headline.
fn fetch_current_ask(token_id: &str) -> Option<f64> {
    let url = format!("{CLOB_BASE}/price?token_id={token_id}&side=SELL");
    let body = super::http::get_text(&url, &[], super::http::Retry::Transient).ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    probability(&value["price"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_matches_span() {
        assert_eq!(pm_window("1h").0, "1d");
        assert_eq!(pm_window("1d").0, "max");
        assert_eq!(pm_window("1w").0, "max");
        assert_eq!(pm_window("1M").0, "max");
    }

    #[test]
    fn decodes_string_arrays() {
        let value: Value = serde_json::from_str(r#""[\"Yes\",\"No\"]""#).expect("string");
        assert_eq!(decode_string_array(&value), vec!["Yes", "No"]);
    }
}
