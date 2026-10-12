use crate::model::DataSource;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const STATIC_CDN: &str =
    "https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color";
const STOCK_CDN: &str = "https://raw.githubusercontent.com/nvstly/icons/main/ticker_icons";

struct Entry {
    url: Option<String>,
    resolving: bool,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(source: DataSource, symbol: &str) -> String {
    format!("{}:{}", source.watchlist_slug(), symbol.to_lowercase())
}

/// The cached icon URL for a chart, or `None` if unresolved (the card draws a
/// monogram meanwhile).
pub fn get_url(source: DataSource, symbol: &str) -> Option<String> {
    cache()
        .lock()
        .ok()?
        .get(&key(source, symbol))
        .and_then(|entry| entry.url.clone())
}

/// Ensure an icon is resolving in the background; the URL appears via `get_url`.
pub fn request(source: DataSource, symbol: &str) {
    let cache_key = key(source, symbol);
    {
        let mut guard = cache().lock().expect("icon cache poisoned");
        match guard.get_mut(&cache_key) {
            Some(entry) => {
                if entry.resolving || entry.url.is_some() {
                    return;
                }
                entry.resolving = true;
            }
            None => {
                guard.insert(
                    cache_key.clone(),
                    Entry {
                        url: None,
                        resolving: true,
                    },
                );
            }
        }
    }

    let base = base_symbol(source, symbol);
    std::thread::spawn(move || {
        let url = resolve(source, &base);
        if let Ok(mut guard) = cache().lock()
            && let Some(entry) = guard.get_mut(&cache_key)
        {
            entry.url = url;
            entry.resolving = false;
        }
    });
}

/// The symbol reduced to its base asset, used by the symbol-keyed lookups.
fn base_symbol(source: DataSource, symbol: &str) -> String {
    match source {
        DataSource::Binance => {
            let display = source.display_symbol(symbol);
            display.split('/').next().unwrap_or(&display).to_lowercase()
        }
        DataSource::Coinbase => symbol.split('-').next().unwrap_or(symbol).to_lowercase(),
        _ => symbol.to_lowercase(),
    }
}

fn resolve(source: DataSource, base: &str) -> Option<String> {
    if source.is_prediction_market() {
        return None;
    }
    if source == DataSource::Alpaca {
        return static_url(&format!("{STOCK_CDN}/{}.png", base.to_uppercase()));
    }
    // The static CDN is a plain file fetch with no rate limit; CoinGecko `/search` queues
    // behind a multi-second limiter shared with OHLC, so it only runs for coins the CDN lacks.
    if let Some(url) = static_url(&format!("{STATIC_CDN}/{base}.png")) {
        return Some(url);
    }
    coingecko_search_icon(base)
}

/// CoinGecko `/search` reaches every listed coin. Prefers an exact symbol match
/// and, among those, the best-ranked coin.
fn coingecko_search_icon(base: &str) -> Option<String> {
    super::rate_limit::coingecko().wait_for_slot();
    let url = format!("https://api.coingecko.com/api/v3/search?query={base}");
    let body = super::http::get_text(&url, &[], super::http::Retry::Never).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    let coins = value.get("coins")?.as_array()?;

    let exact: Vec<&serde_json::Value> = coins
        .iter()
        .filter(|coin| {
            coin.get("symbol")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(base))
        })
        .collect();
    let candidates: Vec<&serde_json::Value> = if exact.is_empty() {
        coins.iter().collect()
    } else {
        exact
    };
    let best = candidates.into_iter().min_by_key(|coin| {
        coin.get("market_cap_rank")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(i64::MAX)
    })?;
    best.get("large")
        .or_else(|| best.get("thumb"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// A plain file in a public repo — no API, no rate limit, 404 on miss. The 404 is
/// checked here so an unknown symbol never caches a broken image reference.
fn static_url(url: &str) -> Option<String> {
    (super::http::head_status(url).ok()? == 200).then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_base_drops_any_known_quote() {
        for (symbol, base) in [
            ("BTCUSDT", "btc"),
            ("XRPBTC", "xrp"),
            ("SOLBNB", "sol"),
            ("ETHFDUSD", "eth"),
        ] {
            assert_eq!(base_symbol(DataSource::Binance, symbol), base);
        }
    }
}
