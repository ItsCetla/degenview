//! Coin and company artwork, resolved the way the Swift app's `IconResolver` does: a chain of
//! sources, widest coverage last, with a positive and a negative cache.
//!
//! 1. the cached answer for `(source, ticker)`, positive or negative (a miss is remembered for
//!    a day, so a card that has none does not re-walk the chain every frame);
//! 2. a snapshot of the top coins by market cap, indexed by symbol *and* by coin id;
//! 3. a source-specific lookup (CoinGecko by coin id, DEXScreener by pair address);
//! 4. CoinGecko `/search`, which reaches every coin it lists;
//! 5. a static icon set, for long-established symbols the APIs could not place.
//!
//! Equities (Alpaca) are kept out of every coin-oriented step: an absent logo must become a
//! monogram, never a plausible but wrong crypto match. Prediction markets carry their own
//! artwork elsewhere. Callers draw a monogram until `get_url` has an answer.

use crate::model::DataSource;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const STATIC_CDN: &str =
    "https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color";
const STOCK_CDN: &str = "https://raw.githubusercontent.com/nvstly/icons/main/ticker_icons";
const COINGECKO: &str = "https://api.coingecko.com/api/v3";
const DEXSCREENER: &str = "https://api.dexscreener.com/latest/dex";

/// How long an exhausted lookup is remembered before the chain runs again.
const NEGATIVE_TTL: Duration = Duration::from_secs(86_400);
/// Coins pulled into the market snapshot.
const MAX_COINS: usize = 250;
/// Minimum spacing between snapshot attempts: a failed refresh must not re-fire for every card.
const REFRESH_RETRY: Duration = Duration::from_secs(300);
/// Longest string still plausibly a ticker; anything longer is a contract address.
const MAX_SYMBOL_LENGTH: usize = 12;

#[derive(Default)]
struct Entry {
    url: Option<String>,
    resolving: bool,
    /// When the chain last came up empty.
    missed_at: Option<Instant>,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(source: DataSource, ticker: &str) -> String {
    format!("{}:{}", source.watchlist_slug(), ticker.to_lowercase())
}

/// The cached icon URL for a market, or `None` if unresolved (callers draw a monogram).
pub fn get_url(source: DataSource, ticker: &str) -> Option<String> {
    cache()
        .lock()
        .ok()?
        .get(&key(source, ticker))
        .and_then(|entry| entry.url.clone())
}

/// Ensure an icon is resolving in the background; the URL appears via `get_url`. The base
/// asset is read off the ticker (`SEIUSDT` → `sei`).
pub fn request(source: DataSource, ticker: &str) {
    request_with_base(source, ticker, &base_symbol(source, ticker));
}

/// Like `request`, for callers that know the market's base asset already (a portfolio asset's
/// ticker, while its `ticker` is the provider's own id: a CoinGecko coin id, a DEX pair address).
pub fn request_with_base(source: DataSource, ticker: &str, base: &str) {
    let cache_key = key(source, ticker);
    {
        let mut guard = cache().lock().expect("icon cache poisoned");
        let entry = guard.entry(cache_key.clone()).or_default();
        if entry.resolving || entry.url.is_some() {
            return;
        }
        if entry
            .missed_at
            .is_some_and(|missed| missed.elapsed() < NEGATIVE_TTL)
        {
            return;
        }
        entry.resolving = true;
    }

    let ticker = ticker.to_owned();
    let base = base.to_lowercase();
    std::thread::spawn(move || {
        let url = resolve(source, &ticker, &base);
        if let Ok(mut guard) = cache().lock()
            && let Some(entry) = guard.get_mut(&cache_key)
        {
            entry.missed_at = url.is_none().then(Instant::now);
            entry.url = url;
            entry.resolving = false;
        }
    });
}

/// The symbol reduced to its base asset, used by the symbol-keyed steps of the chain.
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

// MARK: Chain

fn resolve(source: DataSource, ticker: &str, base: &str) -> Option<String> {
    // Prediction markets have no coin behind them, and a CoinMarketCap card is an index.
    if source.is_prediction_market() || source == DataSource::CoinMarketCap {
        return None;
    }
    if source == DataSource::Alpaca {
        return stock_icon(ticker);
    }
    let mut symbol = base.to_owned();

    // 1. Market snapshot: Binance/Coinbase base symbols and top-ranked coin ids.
    let snapshot = snapshot();
    let by_source = match source {
        DataSource::CoinGecko => snapshot
            .by_id
            .get(&ticker.to_lowercase())
            .or_else(|| snapshot.by_symbol.get(&symbol)),
        _ => snapshot.by_symbol.get(&symbol),
    };
    if let Some(url) = by_source {
        return Some(url.clone());
    }

    // 2. Source-specific lookups.
    match source {
        DataSource::CoinGecko => {
            if let Some(url) = coingecko_icon_by_id(ticker) {
                return Some(url);
            }
        }
        DataSource::DexScreener => {
            let pair = dex_pair_icon(ticker);
            if let Some(url) = pair.as_ref().and_then(|pair| pair.image.clone()) {
                return Some(url);
            }
            // The pair handed back a real ticker, so the symbol-keyed steps below have
            // something usable instead of a contract address.
            if let Some(resolved) = pair.and_then(|pair| pair.base_symbol)
                && !resolved.is_empty()
            {
                symbol = resolved.to_lowercase();
                if let Some(url) = snapshot.by_symbol.get(&symbol) {
                    return Some(url.clone());
                }
            }
        }
        _ => {}
    }

    if !is_plausible_symbol(&symbol) {
        return None;
    }
    // 3 and 4 are shared by every market of the same base asset.
    resolve_symbol(&symbol)
}

/// Symbol-shaped enough to be worth spending a search on.
fn is_plausible_symbol(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol.chars().count() <= MAX_SYMBOL_LENGTH
        && symbol.chars().all(char::is_alphanumeric)
}

// MARK: Symbol lookups (search, static set)

enum Lookup {
    Pending,
    Done(Option<String>, Instant),
}

fn symbol_lookups() -> &'static Mutex<HashMap<String, Lookup>> {
    static LOOKUPS: OnceLock<Mutex<HashMap<String, Lookup>>> = OnceLock::new();
    LOOKUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// CoinGecko search, then the static set, once per base symbol: a hundred markets of one coin
/// share a single search, the others wait for it.
fn resolve_symbol(symbol: &str) -> Option<String> {
    loop {
        {
            let mut lookups = symbol_lookups().lock().ok()?;
            match lookups.get(symbol) {
                Some(Lookup::Done(url, at)) if url.is_some() || at.elapsed() < NEGATIVE_TTL => {
                    return url.clone();
                }
                Some(Lookup::Pending) => {}
                _ => {
                    lookups.insert(symbol.to_owned(), Lookup::Pending);
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let url = coingecko_search_icon(symbol).or_else(|| static_icon(symbol));
    if let Ok(mut lookups) = symbol_lookups().lock() {
        lookups.insert(symbol.to_owned(), Lookup::Done(url.clone(), Instant::now()));
    }
    url
}

/// CoinGecko `/search` reaches every listed coin. Prefers an exact symbol match and, among
/// those, the best-ranked coin, so "btc" resolves to bitcoin rather than a same-ticker copycat.
fn coingecko_search_icon(base: &str) -> Option<String> {
    super::rate_limit::coingecko().wait_for_slot();
    let url = format!("{COINGECKO}/search?query={base}");
    let body = super::http::get_text(&url, &[], super::http::Retry::Never).ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    best_search_icon(&value, base)
}

fn best_search_icon(value: &Value, base: &str) -> Option<String> {
    let coins = value.get("coins")?.as_array()?;
    let exact: Vec<&Value> = coins
        .iter()
        .filter(|coin| {
            coin.get("symbol")
                .and_then(Value::as_str)
                .is_some_and(|symbol| symbol.eq_ignore_ascii_case(base))
        })
        .collect();
    let candidates: Vec<&Value> = if exact.is_empty() {
        coins.iter().collect()
    } else {
        exact
    };
    let best = candidates.into_iter().min_by_key(|coin| {
        coin.get("market_cap_rank")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX)
    })?;
    best.get("large")
        .or_else(|| best.get("thumb"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// A plain file in a public repo: no API, no rate limit, 404 on miss. The 404 is checked here
/// so an unknown symbol never caches a broken image reference.
fn static_icon(symbol: &str) -> Option<String> {
    verified(format!("{STATIC_CDN}/{}.png", symbol.to_lowercase()))
}

/// Company artwork for Alpaca equities, keyed by uppercase exchange ticker.
fn stock_icon(ticker: &str) -> Option<String> {
    let ticker = ticker.trim().to_uppercase();
    if !is_plausible_symbol(&ticker) {
        return None;
    }
    verified(format!("{STOCK_CDN}/{ticker}.png"))
}

fn verified(url: String) -> Option<String> {
    (super::http::head_status(&url).ok()? == 200).then_some(url)
}

// MARK: Market snapshot

#[derive(Clone, Default)]
struct Snapshot {
    by_symbol: HashMap<String, String>,
    by_id: HashMap<String, String>,
}

struct SnapshotState {
    data: Snapshot,
    loaded: bool,
    last_attempt: Option<Instant>,
}

fn snapshot_state() -> &'static Mutex<SnapshotState> {
    static STATE: OnceLock<Mutex<SnapshotState>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(SnapshotState {
            data: Snapshot::default(),
            loaded: false,
            last_attempt: None,
        })
    })
}

/// Serialises refreshes: the first caller fetches, the rest wait on the lock and find the
/// snapshot loaded.
fn snapshot_refresh() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// The top coins by market cap, indexed by symbol and by id. Fetched once per launch; a
/// failure is retried no sooner than `REFRESH_RETRY` later.
fn snapshot() -> Snapshot {
    if let Ok(state) = snapshot_state().lock()
        && state.loaded
    {
        return state.data.clone();
    }
    let _refreshing = snapshot_refresh().lock();
    {
        let Ok(mut state) = snapshot_state().lock() else {
            return Snapshot::default();
        };
        if state.loaded {
            return state.data.clone();
        }
        if state
            .last_attempt
            .is_some_and(|at| at.elapsed() < REFRESH_RETRY)
        {
            return state.data.clone();
        }
        state.last_attempt = Some(Instant::now());
    }
    let fetched = fetch_snapshot();
    let Ok(mut state) = snapshot_state().lock() else {
        return Snapshot::default();
    };
    if let Some(data) = fetched {
        state.data = data;
        state.loaded = true;
    }
    state.data.clone()
}

fn fetch_snapshot() -> Option<Snapshot> {
    super::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "{COINGECKO}/coins/markets?vs_currency=usd&order=market_cap_desc&per_page={MAX_COINS}&sparkline=false"
    );
    let body = super::http::get_text(&url, &[], super::http::Retry::Transient).ok()?;
    parse_snapshot(&body)
}

/// Highest-cap coin wins per symbol (so "btc" is bitcoin, not batcat): the response is already
/// in market-cap order, so the first coin of a symbol keeps it.
fn parse_snapshot(body: &str) -> Option<Snapshot> {
    let coins: Vec<Value> = serde_json::from_str(body).ok()?;
    let mut snapshot = Snapshot::default();
    for coin in &coins {
        let (Some(id), Some(symbol), Some(image)) = (
            coin.get("id").and_then(Value::as_str),
            coin.get("symbol").and_then(Value::as_str),
            coin.get("image").and_then(Value::as_str),
        ) else {
            continue;
        };
        snapshot
            .by_symbol
            .entry(symbol.to_lowercase())
            .or_insert_with(|| image.to_owned());
        snapshot.by_id.insert(id.to_lowercase(), image.to_owned());
    }
    (!snapshot.by_id.is_empty()).then_some(snapshot)
}

// MARK: Source-specific

/// A coin's artwork by CoinGecko id, for ids outside the snapshot.
fn coingecko_icon_by_id(id: &str) -> Option<String> {
    super::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "{COINGECKO}/coins/markets?vs_currency=usd&ids={}&per_page=1&sparkline=false",
        id.to_lowercase()
    );
    let body = super::http::get_text(&url, &[], super::http::Retry::Never).ok()?;
    let coins: Vec<Value> = serde_json::from_str(&body).ok()?;
    coins
        .first()?
        .get("image")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

struct DexPairIcon {
    image: Option<String>,
    base_symbol: Option<String>,
}

/// DEXScreener's own artwork for a pair, and the real ticker behind its address.
fn dex_pair_icon(address: &str) -> Option<DexPairIcon> {
    let query = address.trim();
    if query.is_empty() {
        return None;
    }
    let url = format!("{DEXSCREENER}/search?q={query}");
    let body = super::http::get_text(&url, &[], super::http::Retry::Transient).ok()?;
    parse_dex_pair(&body)
}

fn parse_dex_pair(body: &str) -> Option<DexPairIcon> {
    let value: Value = serde_json::from_str(body).ok()?;
    let pairs = value.get("pairs")?.as_array()?;
    let first = pairs.first()?;
    Some(DexPairIcon {
        image: pairs
            .iter()
            .find_map(|pair| pair.pointer("/info/imageUrl").and_then(Value::as_str))
            .map(str::to_owned),
        base_symbol: first
            .pointer("/baseToken/symbol")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
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
            ("SEIUSDT", "sei"),
        ] {
            assert_eq!(base_symbol(DataSource::Binance, symbol), base);
        }
        assert_eq!(base_symbol(DataSource::Coinbase, "SEI-USD"), "sei");
    }

    #[test]
    fn the_snapshot_is_indexed_by_symbol_and_id_and_the_biggest_coin_owns_a_symbol() {
        let body = r#"[
            {"id":"bitcoin","symbol":"btc","image":"https://x/bitcoin.png"},
            {"id":"sei-network","symbol":"sei","image":"https://x/sei.png"},
            {"id":"batcat","symbol":"btc","image":"https://x/batcat.png"},
            {"id":"broken"}
        ]"#;
        let snapshot = parse_snapshot(body).expect("snapshot");
        assert_eq!(snapshot.by_symbol["btc"], "https://x/bitcoin.png");
        assert_eq!(snapshot.by_symbol["sei"], "https://x/sei.png");
        assert_eq!(snapshot.by_id["sei-network"], "https://x/sei.png");
        assert_eq!(snapshot.by_id["batcat"], "https://x/batcat.png");
        assert!(!snapshot.by_id.contains_key("broken"));
        assert!(parse_snapshot("[]").is_none());
        assert!(parse_snapshot("not json").is_none());
    }

    #[test]
    fn search_prefers_an_exact_symbol_and_the_best_rank() {
        let value: Value = serde_json::from_str(
            r#"{"coins":[
                {"symbol":"SEIX","large":"https://x/seix.png","market_cap_rank":3},
                {"symbol":"SEI","large":"https://x/copy.png","market_cap_rank":900},
                {"symbol":"SEI","large":"https://x/sei.png","market_cap_rank":120},
                {"symbol":"SEI","thumb":"https://x/none.png"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            best_search_icon(&value, "sei").as_deref(),
            Some("https://x/sei.png")
        );
        // With no exact symbol the best-ranked coin of any symbol answers.
        let loose: Value = serde_json::from_str(
            r#"{"coins":[{"symbol":"AAA","large":"https://x/a.png","market_cap_rank":50}]}"#,
        )
        .unwrap();
        assert_eq!(
            best_search_icon(&loose, "zzz").as_deref(),
            Some("https://x/a.png")
        );
        assert!(best_search_icon(&serde_json::json!({"coins": []}), "sei").is_none());
    }

    #[test]
    fn a_dex_pair_gives_its_artwork_and_the_real_ticker() {
        let body = r#"{"pairs":[
            {"baseToken":{"symbol":"MCADE"},"info":null},
            {"baseToken":{"symbol":"MCADE"},"info":{"imageUrl":"https://x/mcade.png"}}
        ]}"#;
        let pair = parse_dex_pair(body).expect("pair");
        assert_eq!(pair.image.as_deref(), Some("https://x/mcade.png"));
        assert_eq!(pair.base_symbol.as_deref(), Some("MCADE"));
        assert!(parse_dex_pair(r#"{"pairs":[]}"#).is_none());
        assert!(parse_dex_pair(r#"{"pairs":null}"#).is_none());
    }

    #[test]
    fn only_ticker_shaped_strings_are_searched() {
        assert!(is_plausible_symbol("sei"));
        assert!(is_plausible_symbol("mcade"));
        assert!(!is_plausible_symbol(""));
        assert!(!is_plausible_symbol("5zpyutju9ee6jfymdgok"));
        assert!(!is_plausible_symbol("so-l"));
    }
}
