use super::catalog::Catalog;
use super::{MarketSource, SearchResult, rank_results};
use crate::model::Kline;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const BASE_URL: &str = "https://api.binance.com/api/v3/klines";

pub struct Binance;

pub const BINANCE: Binance = Binance;

impl MarketSource for Binance {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let url = format!(
            "{BASE_URL}?symbol={}&interval={interval}&limit={limit}",
            symbol.to_uppercase()
        );
        let body = super::http::get_text(&url, &[], super::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
        parse_klines(&body)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let needle = query.trim().to_uppercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let pairs = CATALOG.get(download_pairs)?;
        let results: Vec<SearchResult> = pairs
            .iter()
            .filter(|pair| pair.symbol.contains(&needle))
            .map(|pair| SearchResult {
                symbol: format!("{}/{}", pair.base, pair.quote),
                full_symbol: pair.symbol.clone(),
                source: crate::model::DataSource::Binance,
                price: None,
                event: None,
            })
            .collect();
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

/// One tradable market from `exchangeInfo`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pair {
    symbol: String,
    base: String,
    quote: String,
}

/// Every market that is trading, downloaded at most once a day.
static CATALOG: Catalog<Pair> = Catalog::new("catalog_binance");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_pairs() -> Result<Vec<Pair>, String> {
    let body = super::http::get_text(
        "https://api.binance.com/api/v3/exchangeInfo",
        &[],
        super::http::Retry::Transient,
    )
    .map_err(|error| format!("Network error: {error}"))?;
    parse_pairs(&body)
}

fn parse_pairs(body: &str) -> Result<Vec<Pair>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    Ok(value
        .get("symbols")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|symbol| symbol.get("status").and_then(Value::as_str) == Some("TRADING"))
        .filter_map(|symbol| {
            Some(Pair {
                symbol: symbol.get("symbol")?.as_str()?.to_owned(),
                base: symbol.get("baseAsset")?.as_str()?.to_owned(),
                quote: symbol.get("quoteAsset")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

fn parse_klines(body: &str) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .as_array()
        .ok_or_else(|| "Data error: expected a candle array".to_owned())?;
    rows.iter().map(parse_row).collect()
}

fn parse_row(row: &Value) -> Result<Kline, String> {
    let value = |index: usize| -> Result<f64, String> {
        row.get(index)
            .and_then(Value::as_str)
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| format!("Data error: invalid field {index}"))
    };
    let time = |index: usize| -> Result<i64, String> {
        row.get(index)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("Data error: invalid timestamp {index}"))
    };

    Ok(Kline {
        open_time: time(0)?,
        open: value(1)?,
        high: value(2)?,
        low: value(3)?,
        close: value(4)?,
        volume: value(5)?,
        close_time: time(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binance_rows() {
        let body = r#"[[1700000000000,"10.0","12.0","9.0","11.0","42.5",1700003599999]]"#;
        let candles = parse_klines(body).expect("valid response");
        assert_eq!(candles.len(), 1);
        assert_eq!(
            candles[0],
            Kline {
                open_time: 1_700_000_000_000,
                close_time: 1_700_003_599_999,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                volume: 42.5,
            }
        );
    }

    #[test]
    fn rejects_error_objects() {
        let error = parse_klines(r#"{"code":-1121,"msg":"Invalid symbol."}"#)
            .expect_err("object must fail");
        assert!(error.contains("expected a candle array"));
    }

    #[test]
    fn the_catalog_keeps_only_trading_markets_with_all_their_fields() {
        let body = r#"{"symbols":[
            {"symbol":"BTCUSDT","status":"TRADING","baseAsset":"BTC","quoteAsset":"USDT"},
            {"symbol":"LUNAUSDT","status":"BREAK","baseAsset":"LUNA","quoteAsset":"USDT"},
            {"symbol":"NOQUOTE","status":"TRADING","baseAsset":"X"}
        ]}"#;
        let pairs = parse_pairs(body).expect("pairs");
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            (
                pairs[0].symbol.as_str(),
                pairs[0].base.as_str(),
                pairs[0].quote.as_str()
            ),
            ("BTCUSDT", "BTC", "USDT")
        );
        assert!(parse_pairs("not json").is_err());
        assert!(parse_pairs("{}").expect("no symbols").is_empty());
    }
}
