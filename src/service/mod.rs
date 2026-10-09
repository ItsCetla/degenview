mod alpaca;
mod binance;
mod cache;
pub mod cmc;
mod coinbase;
mod coingecko;
mod dexscreener;
mod geckoterminal;
pub mod icon;
mod kalshi;
pub mod live;
mod polymarket;
mod rate_limit;
pub mod watchlist_quotes;
pub mod webhook_delivery;

use crate::model::{DataSource, Kline};

/// A search hit on one provider: a display label, the provider's own id, and the
/// source it came from.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub symbol: String,
    pub full_symbol: String,
    pub source: DataSource,
    pub price: Option<f64>,
}

/// A provider that can serve OHLC candles and, optionally, ticker search.
pub trait MarketSource {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String>;

    fn search_tickers(&self, query: &str) -> Result<Vec<SearchResult>, String> {
        let _ = query;
        Ok(Vec::new())
    }
}

pub fn for_source(source: DataSource) -> Option<&'static dyn MarketSource> {
    match source {
        DataSource::Binance => Some(&binance::BINANCE),
        DataSource::Coinbase => Some(&coinbase::COINBASE),
        DataSource::CoinGecko => Some(&coingecko::COINGECKO),
        DataSource::DexScreener => Some(&dexscreener::DEXSCREENER),
        DataSource::Alpaca => Some(&alpaca::ALPACA),
        DataSource::Polymarket => Some(&polymarket::POLYMARKET),
        DataSource::Kalshi => Some(&kalshi::KALSHI),
        DataSource::CoinMarketCap => Some(&cmc::COIN_MARKET_CAP),
    }
}

/// Fetch candles from the provider behind `source`, oldest first.
pub fn fetch_klines(
    source: DataSource,
    symbol: &str,
    interval: &str,
    limit: u32,
) -> Result<Vec<Kline>, String> {
    for_source(source)
        .ok_or_else(|| {
            format!(
                "{} data arrives in a later Stage 4 slice",
                source.display_name()
            )
        })?
        .fetch_klines(symbol, interval, limit)
}

/// Search one provider. Crypto sources only; the others arrive with their slices.
pub fn search(source: DataSource, query: &str) -> Result<Vec<SearchResult>, String> {
    for_source(source)
        .ok_or_else(|| format!("{} search arrives in a later slice", source.display_name()))?
        .search_tickers(query)
}

/// Stable, provider-local ordering that favors the asset a query most likely
/// refers to. Equal-ranked results retain the source's original order.
pub(crate) fn rank_results(results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
    let needle = query.trim().to_uppercase();
    if needle.is_empty() {
        return results;
    }
    let mut indexed: Vec<(usize, SearchResult)> = results.into_iter().enumerate().collect();
    indexed.sort_by_key(|(offset, result)| (rank(result, &needle), *offset));
    indexed.into_iter().map(|(_, result)| result).collect()
}

fn rank(result: &SearchResult, needle: &str) -> u8 {
    let display = result.symbol.to_uppercase();
    let full = result.full_symbol.to_uppercase();
    let base = display.split('/').next().unwrap_or(&display).to_owned();
    if display == needle || full == needle {
        0
    } else if base == needle {
        1
    } else if display.starts_with(needle) || full.starts_with(needle) || base.starts_with(needle) {
        3
    } else if display.contains(needle) || full.contains(needle) {
        4
    } else {
        5
    }
}

/// Binance-style interval token → seconds.
pub(crate) fn interval_seconds(token: &str) -> Option<i64> {
    match token {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "30m" => Some(1_800),
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        "1w" => Some(604_800),
        "1M" => Some(2_592_000),
        "3M" => Some(7_776_000),
        "1Y" => Some(31_536_000),
        _ => None,
    }
}

/// Open time of the calendar bucket that contains `epoch_ms`. Sub-week intervals
/// floor onto the epoch; weeks start on Monday and months/quarters/years on their
/// calendar start, matching Binance's candle boundaries.
pub(crate) fn bucket_start_ms(epoch_ms: i64, interval_secs: i64) -> i64 {
    if interval_secs < 604_800 {
        return epoch_ms.div_euclid(interval_secs * 1_000) * interval_secs * 1_000;
    }
    let days = epoch_ms.div_euclid(86_400_000);
    if interval_secs < 2_419_200 {
        // Week, starting Monday.
        let monday_days = (days - 4).div_euclid(7) * 7 + 4;
        return monday_days * 86_400_000;
    }
    let (year, month, _) = crate::format::civil_from_days(days);
    let (target_year, target_month) = if interval_secs < 7_000_000 {
        (year, month)
    } else if interval_secs < 30_000_000 {
        (year, (month - 1) / 3 * 3 + 1)
    } else {
        (year, 1)
    };
    crate::format::days_from_civil(target_year, target_month, 1) * 86_400_000
}

/// Span a prediction-market line chart covers at an interval, in days.
pub(crate) fn effective_span_days(interval: &str) -> i64 {
    match interval {
        "1h" => 2,
        "1d" => 60,
        "1w" => 182,
        "1M" => 360,
        "3M" => 90,
        "1Y" => 364,
        _ => 60,
    }
}

/// Points a prediction-market line chart draws at an interval.
pub(crate) fn line_chart_point_count(interval: &str) -> usize {
    match interval {
        "1h" => 48,
        "1d" => 60,
        "1w" => 26,
        "1M" => 12,
        "3M" => 90,
        "1Y" => 52,
        _ => 60,
    }
}

/// Thin to at most `count` points by uniform stride, always keeping the newest one.
pub(crate) fn downsample(candles: &[Kline], count: usize) -> Vec<Kline> {
    if count == 0 || candles.len() <= count {
        return candles.to_vec();
    }
    if count == 1 {
        return candles.last().cloned().into_iter().collect();
    }
    let stride = (candles.len() - 1) as f64 / (count - 1) as f64;
    (0..count)
        .map(|i| {
            let index = (i as f64 * stride).round() as usize;
            candles[index.min(candles.len() - 1)].clone()
        })
        .collect()
}

/// Overlay the executable YES ask on the final sample so the chart's endpoint,
/// current-price line, and headline match the provider's market page.
pub(crate) fn replace_last_price(mut candles: Vec<Kline>, price: f64) -> Vec<Kline> {
    if price.is_finite()
        && (0.0..=1.0).contains(&price)
        && let Some(last) = candles.last_mut()
    {
        last.close = price;
        last.high = price;
        last.low = price;
    }
    candles
}
