mod binance;
mod coinbase;
mod coingecko;

use crate::model::{DataSource, Kline};

/// A provider that can serve OHLC candles.
pub trait MarketSource {
    fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Kline>, String>;
}

pub fn for_source(source: DataSource) -> Option<&'static dyn MarketSource> {
    match source {
        DataSource::Binance => Some(&binance::BINANCE),
        DataSource::Coinbase => Some(&coinbase::COINBASE),
        DataSource::CoinGecko => Some(&coingecko::COINGECKO),
        _ => None,
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
