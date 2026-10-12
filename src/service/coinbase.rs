use super::catalog::Catalog;
use super::{MarketSource, SearchResult, bucket_start_ms, rank_results};
use crate::model::Kline;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const BASE_URL: &str = "https://api.exchange.coinbase.com";
/// Candles one `start`/`end` request spans; Coinbase rejects spans over 300.
const PAGE_BUCKETS: i64 = 290;
/// Cap on backwards pages so a slow fold cannot loop forever.
const MAX_PAGES: usize = 60;

pub struct Coinbase;

pub const COINBASE: Coinbase = Coinbase;

/// Coinbase ids are `BASE-QUOTE`. Accept `BASE/QUOTE` and a bare `BASE` (USD).
pub(crate) fn product_id(symbol: &str) -> String {
    let id = symbol.trim().to_uppercase().replace('/', "-");
    if id.contains('-') {
        id
    } else {
        format!("{id}-USD")
    }
}

struct Granularity {
    source_secs: i64,
    target_secs: i64,
    per_target: usize,
}

impl Granularity {
    fn for_interval(interval: &str) -> Option<Granularity> {
        match interval {
            "1h" => Some(Granularity {
                source_secs: 3_600,
                target_secs: 3_600,
                per_target: 1,
            }),
            "1d" => Some(Granularity {
                source_secs: 86_400,
                target_secs: 86_400,
                per_target: 1,
            }),
            "1w" => Some(Granularity {
                source_secs: 86_400,
                target_secs: 604_800,
                per_target: 7,
            }),
            "1M" => Some(Granularity {
                source_secs: 86_400,
                target_secs: 2_592_000,
                per_target: 31,
            }),
            "3M" => Some(Granularity {
                source_secs: 86_400,
                target_secs: 7_776_000,
                per_target: 92,
            }),
            "1Y" => Some(Granularity {
                source_secs: 86_400,
                target_secs: 31_536_000,
                per_target: 366,
            }),
            _ => None,
        }
    }

    fn needs_aggregation(&self) -> bool {
        self.source_secs != self.target_secs
    }
}

impl MarketSource for Coinbase {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String> {
        let plan = Granularity::for_interval(interval).ok_or_else(|| {
            format!(
                "Coinbase has no {} candles",
                crate::model::Timeframe::ALL
                    .iter()
                    .find(|tf| tf.binance_interval() == interval)
                    .map_or(interval.to_owned(), |tf| tf.label().to_owned())
            )
        })?;
        let id = product_id(symbol);
        let needed = limit as usize * plan.per_target
            + if plan.needs_aggregation() {
                plan.per_target
            } else {
                0
            };
        let source = self.source_candles(&id, plan.source_secs, needed)?;
        let shown = if plan.needs_aggregation() {
            fold(&source, plan.target_secs)
        } else {
            source
        };
        let start = shown.len().saturating_sub(limit as usize);
        Ok(shown[start..].to_vec())
    }

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let q = query.trim().to_uppercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let products = CATALOG.get(download_products)?;
        let results: Vec<SearchResult> = products
            .iter()
            .filter(|product| product.id.replace('-', "").contains(&q))
            .map(|product| SearchResult {
                symbol: format!("{}/{}", product.base, product.quote),
                full_symbol: product.id.clone(),
                source: crate::model::DataSource::Coinbase,
                price: None,
            })
            .collect();
        Ok(rank_results(results, query).into_iter().take(20).collect())
    }
}

/// One tradable product from `/products`: all the search needs of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Product {
    id: String,
    base: String,
    quote: String,
}

/// Every product that is online, downloaded at most once a day.
static CATALOG: Catalog<Product> = Catalog::new("catalog_coinbase");

/// Forgets the list held in memory (after the disk cache was cleared).
pub(super) fn clear_catalog() {
    CATALOG.clear_memory();
}

fn download_products() -> Result<Vec<Product>, String> {
    let body = super::http::get_text(
        "https://api.exchange.coinbase.com/products",
        &[],
        super::http::Retry::Transient,
    )
    .map_err(|error| format!("Network error: {error}"))?;
    parse_products(&body)
}

fn parse_products(body: &str) -> Result<Vec<Product>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    Ok(value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|product| product.get("status").and_then(Value::as_str) == Some("online"))
        .filter(|product| product.get("trading_disabled").and_then(Value::as_bool) != Some(true))
        .filter_map(|product| {
            Some(Product {
                id: product.get("id")?.as_str()?.to_owned(),
                base: product.get("base_currency")?.as_str()?.to_owned(),
                quote: product.get("quote_currency")?.as_str()?.to_owned(),
            })
        })
        .collect())
}

impl Coinbase {
    fn source_candles(
        &self,
        id: &str,
        granularity: i64,
        needed: usize,
    ) -> Result<Vec<Kline>, String> {
        let mut candles = self.fetch_page(id, granularity, None, None)?;
        let mut pages = 0;
        while candles.len() < needed && pages < MAX_PAGES {
            let Some(oldest) = candles.first().map(|c| c.open_time) else {
                break;
            };
            std::thread::sleep(Duration::from_millis(120));
            let span = granularity * PAGE_BUCKETS * 1_000;
            let older = self.fetch_page(id, granularity, Some(oldest - span), Some(oldest))?;
            let older: Vec<Kline> = older.into_iter().filter(|c| c.open_time < oldest).collect();
            if older.is_empty() {
                break;
            }
            candles = older.into_iter().chain(candles).collect();
            pages += 1;
        }
        Ok(candles)
    }

    fn fetch_page(
        &self,
        id: &str,
        granularity: i64,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<Vec<Kline>, String> {
        let mut url = format!("{BASE_URL}/products/{id}/candles?granularity={granularity}");
        if let (Some(start), Some(end)) = (start, end) {
            url.push_str(&format!(
                "&start={}&end={}",
                crate::format::iso8601(start),
                crate::format::iso8601(end)
            ));
        }
        let body = super::http::get_text(&url, &[], super::http::Retry::Transient)
            .map_err(|error| format!("Network error: {error}"))?;
        parse_page(&body, granularity)
    }
}

/// `[time s, low, high, open, close, volume]`, newest first.
fn parse_page(body: &str, granularity: i64) -> Result<Vec<Kline>, String> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("Data error: {error}"))?;
    let rows = value
        .as_array()
        .ok_or_else(|| "Data error: expected a candle array".to_owned())?;
    let mut candles = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(fields) = row.as_array() else {
            continue;
        };
        if fields.len() < 6 {
            continue;
        }
        let Some(seconds) = fields[0].as_f64() else {
            continue;
        };
        let Some(low) = fields[1].as_f64() else {
            continue;
        };
        let Some(high) = fields[2].as_f64() else {
            continue;
        };
        let Some(open) = fields[3].as_f64() else {
            continue;
        };
        let Some(close) = fields[4].as_f64() else {
            continue;
        };
        let volume = fields[5].as_f64().unwrap_or(0.0);
        let open_time = (seconds * 1_000.0) as i64;
        candles.push(Kline {
            open_time,
            close_time: open_time + granularity * 1_000 - 1,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    candles.sort_by_key(|candle| candle.open_time);
    Ok(candles)
}

/// Fold ascending daily candles into weekly/monthly/quarterly/yearly ones on the
/// same calendar boundaries Binance uses.
fn fold(daily: &[Kline], target_secs: i64) -> Vec<Kline> {
    let mut out: Vec<Kline> = Vec::new();
    for candle in daily {
        let bucket = bucket_start_ms(candle.open_time, target_secs);
        match out.last_mut() {
            Some(last) if last.open_time == bucket => {
                last.high = last.high.max(candle.high);
                last.low = last.low.min(candle.low);
                last.close = candle.close;
                last.volume += candle.volume;
                last.close_time = candle.close_time;
            }
            _ => out.push(Kline {
                open_time: bucket,
                close_time: bucket + target_secs * 1_000 - 1,
                open: candle.open,
                high: candle.high,
                low: candle.low,
                close: candle.close,
                volume: candle.volume,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalog_keeps_only_online_enabled_products() {
        let body = r#"[
            {"id":"BTC-USD","base_currency":"BTC","quote_currency":"USD","status":"online"},
            {"id":"OLD-USD","base_currency":"OLD","quote_currency":"USD","status":"delisted"},
            {"id":"HALT-USD","base_currency":"HALT","quote_currency":"USD","status":"online","trading_disabled":true},
            {"id":"BAD","status":"online"}
        ]"#;
        let products = parse_products(body).expect("products");
        assert_eq!(products.len(), 1);
        assert_eq!(products[0].id, "BTC-USD");
        assert_eq!(
            (products[0].base.as_str(), products[0].quote.as_str()),
            ("BTC", "USD")
        );
        assert!(parse_products("{").is_err());
    }

    use super::*;

    #[test]
    fn product_id_maps_bare_and_slashed_symbols() {
        assert_eq!(product_id("BTC"), "BTC-USD");
        assert_eq!(product_id("ETH/USD"), "ETH-USD");
        assert_eq!(product_id("BTC-USD"), "BTC-USD");
    }

    #[test]
    fn folds_daily_into_weekly_on_monday_boundaries() {
        // 2026-10-05 is a Monday; 2026-10-06 is Tuesday.
        let monday = crate::format::days_from_civil(2026, 10, 5) * 86_400_000;
        let tuesday = crate::format::days_from_civil(2026, 10, 6) * 86_400_000;
        let daily = vec![
            Kline {
                open_time: monday,
                close_time: monday + 86_399_999,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 1.0,
            },
            Kline {
                open_time: tuesday,
                close_time: tuesday + 86_399_999,
                open: 10.5,
                high: 12.0,
                low: 10.0,
                close: 11.5,
                volume: 2.0,
            },
        ];
        let weekly = fold(&daily, 604_800);
        assert_eq!(weekly.len(), 1);
        assert_eq!(weekly[0].open_time, monday);
        assert_eq!(weekly[0].high, 12.0);
        assert_eq!(weekly[0].low, 9.0);
        assert_eq!(weekly[0].close, 11.5);
        assert_eq!(weekly[0].volume, 3.0);
    }

    #[test]
    fn parses_coinbase_page() {
        let body = r#"[[1700000000,9.0,12.0,10.0,11.0,42.5],[1700003600,10.0,11.0,11.0,10.5,1.0]]"#;
        let candles = parse_page(body, 3600).expect("valid");
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open_time, 1_700_000_000_000);
        assert_eq!(candles[0].high, 12.0);
        assert_eq!(candles[0].low, 9.0);
        assert_eq!(candles[0].open, 10.0);
        assert_eq!(candles[0].close, 11.0);
    }
}
