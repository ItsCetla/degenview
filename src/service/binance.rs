use super::{MarketSource, SearchResult, rank_results};
use crate::model::Kline;
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
        let mut response = ureq::get(&url)
            .call()
            .map_err(|error| format!("Network error: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("Network error: {error}"))?;
        parse_klines(&body)
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let needle = query.trim().to_uppercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut response = ureq::get("https://api.binance.com/api/v3/exchangeInfo")
            .call()
            .map_err(|error| format!("Network error: {error}"))?;
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("Network error: {error}"))?;
        let value: Value =
            serde_json::from_str(&body).map_err(|error| format!("Data error: {error}"))?;
        let mut results = Vec::new();
        for symbol in value
            .get("symbols")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if symbol.get("status").and_then(Value::as_str) != Some("TRADING") {
                continue;
            }
            let (Some(symbol), Some(base), Some(quote)) = (
                symbol.get("symbol").and_then(Value::as_str),
                symbol.get("baseAsset").and_then(Value::as_str),
                symbol.get("quoteAsset").and_then(Value::as_str),
            ) else {
                continue;
            };
            if !symbol.contains(&needle) {
                continue;
            }
            results.push(SearchResult {
                symbol: format!("{base}/{quote}"),
                full_symbol: symbol.to_owned(),
                source: crate::model::DataSource::Binance,
                price: None,
            });
        }
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
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
}
