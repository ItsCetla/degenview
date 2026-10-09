//! Where watchlist prices come from: one batched call per source. The live provider
//! wraps each exchange's endpoint; tests substitute a recorder. Parsers are pure so the
//! response shapes are covered without a network.

use crate::model::DataSource;
use crate::model::watchlist_quote::{SourceQuote, VolumeKind};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// One market to price. `symbol` is the id the provider's quote call takes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuoteRequest {
    pub symbol: String,
    /// The network of a DEX pair; other sources leave it `None`.
    pub chain: Option<String>,
}

/// Why a batch of quotes did not come back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuoteFailure {
    RateLimited,
    CredentialsMissing,
    /// The provider refused the request itself, as it does for an unknown symbol.
    /// Retrying a smaller batch can find which symbol it was.
    Rejected(String),
    /// Network trouble or a server error; the whole source is treated as down for a moment.
    Failed(String),
}

impl QuoteFailure {
    pub fn message(&self) -> String {
        match self {
            Self::RateLimited => "Rate limited by the provider".to_owned(),
            Self::CredentialsMissing => {
                "Add your Alpaca API key in Settings to see stock quotes".to_owned()
            }
            Self::Rejected(message) | Self::Failed(message) => message.clone(),
        }
    }
}

/// Quotes keyed by `QuoteRequest::symbol`; a symbol the provider did not answer is omitted.
pub trait QuoteProvider: Send + Sync {
    fn quotes(
        &self,
        source: DataSource,
        requests: &[QuoteRequest],
    ) -> Result<HashMap<String, SourceQuote>, QuoteFailure>;

    /// The network of a DEX pair saved without one, from its address.
    fn resolve_chain(&self, address: &str) -> Option<String>;
}

/// The most symbols one request carries, per source.
pub fn chunk_size(source: DataSource) -> usize {
    match source {
        DataSource::Binance => 100,
        DataSource::Alpaca => 200,
        DataSource::CoinGecko => 250,
        DataSource::Coinbase => 20,
        DataSource::DexScreener => 30,
        _ => 100,
    }
}

// MARK: Live provider

pub struct LiveProvider;

const COINBASE_URL: &str = "https://api.exchange.coinbase.com";
const DEX_URL: &str = "https://api.dexscreener.com/latest/dex";

impl QuoteProvider for LiveProvider {
    fn quotes(
        &self,
        source: DataSource,
        requests: &[QuoteRequest],
    ) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
        if requests.is_empty() {
            return Ok(HashMap::new());
        }
        match source {
            DataSource::Binance => binance(requests),
            DataSource::Coinbase => coinbase(requests),
            DataSource::CoinGecko => coingecko(requests),
            DataSource::DexScreener => dexscreener(requests),
            DataSource::Alpaca => alpaca(requests),
            DataSource::Polymarket | DataSource::Kalshi => Ok(prediction(source, requests)),
            DataSource::CoinMarketCap => Ok(HashMap::new()),
        }
    }

    fn resolve_chain(&self, address: &str) -> Option<String> {
        let body = get(&format!("{DEX_URL}/search?q={address}"), &[]).ok()?;
        parse_chain(&body, address)
    }
}

/// A GET that maps HTTP failures onto `QuoteFailure`.
fn get(url: &str, headers: &[(&str, &str)]) -> Result<String, QuoteFailure> {
    let mut request = ureq::get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(12)))
        .build();
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    match request.call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .map_err(|error| QuoteFailure::Failed(format!("Network error: {error}"))),
        Err(ureq::Error::StatusCode(429)) => Err(QuoteFailure::RateLimited),
        Err(ureq::Error::StatusCode(code)) if (400..500).contains(&code) => Err(
            QuoteFailure::Rejected(format!("The provider answered HTTP {code}")),
        ),
        Err(error) => Err(QuoteFailure::Failed(format!("Network error: {error}"))),
    }
}

fn binance(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let list = requests
        .iter()
        .map(|request| format!("\"{}\"", request.symbol.to_uppercase()))
        .collect::<Vec<_>>()
        .join(",");
    let url = format!("https://api.binance.com/api/v3/ticker/24hr?symbols=[{list}]&type=MINI");
    let body = get(&url, &[])?;
    parse_binance(&body, requests)
}

fn coinbase(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    // No multi-product price call; `/stats` is one small request per product, paced under
    // the exchange's public limit. A product that fails is left out, not failing the rest.
    let mut quotes = HashMap::new();
    for (index, request) in requests.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(Duration::from_millis(125));
        }
        let id = crate::service::coinbase::product_id(&request.symbol);
        match get(&format!("{COINBASE_URL}/products/{id}/stats"), &[]) {
            Ok(body) => {
                if let Some(quote) = parse_coinbase_stats(&body, &id) {
                    quotes.insert(request.symbol.clone(), quote);
                }
            }
            Err(QuoteFailure::RateLimited) => return Err(QuoteFailure::RateLimited),
            Err(_) => {}
        }
    }
    Ok(quotes)
}

fn coingecko(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let ids = requests
        .iter()
        .map(|request| request.symbol.to_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    // One `/coins/markets` call prices every coin id, costing the limiter a single slot.
    super::super::rate_limit::coingecko().wait_for_slot();
    let url = format!(
        "https://api.coingecko.com/api/v3/coins/markets?vs_currency=usd&ids={ids}\
         &sparkline=false&price_change_percentage=24h&per_page={}",
        requests.len().min(250)
    );
    parse_coingecko_markets(&get(&url, &[])?, requests)
}

fn dexscreener(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    // One request per chain for up to 30 pools each. Pairs without a known chain are left out.
    let mut by_chain: HashMap<&str, Vec<&QuoteRequest>> = HashMap::new();
    for request in requests {
        if let Some(chain) = request.chain.as_deref().filter(|chain| *chain != "unknown") {
            by_chain.entry(chain).or_default().push(request);
        }
    }
    let mut quotes = HashMap::new();
    for (chain, pairs) in by_chain {
        for page in pairs.chunks(chunk_size(DataSource::DexScreener)) {
            let addresses = page
                .iter()
                .map(|request| request.symbol.as_str())
                .collect::<Vec<_>>()
                .join(",");
            if let Ok(body) = get(&format!("{DEX_URL}/pairs/{chain}/{addresses}"), &[]) {
                quotes.extend(parse_dex_pairs(&body, page));
            }
        }
    }
    Ok(quotes)
}

fn alpaca(requests: &[QuoteRequest]) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let Some((key, secret)) = crate::service::keychain_api_keys::alpaca_credentials() else {
        return Err(QuoteFailure::CredentialsMissing);
    };
    let symbols = requests
        .iter()
        .map(|request| request.symbol.to_uppercase())
        .collect::<Vec<_>>()
        .join(",");
    let url = format!("https://data.alpaca.markets/v2/stocks/snapshots?symbols={symbols}&feed=iex");
    let body = get(
        &url,
        &[("APCA-API-KEY-ID", &key), ("APCA-API-SECRET-KEY", &secret)],
    )?;
    parse_alpaca_snapshots(&body, requests)
}

/// Last price of the past day's series, and its first price as the comparison.
fn prediction(source: DataSource, requests: &[QuoteRequest]) -> HashMap<String, SourceQuote> {
    let mut quotes = HashMap::new();
    for request in requests {
        let Ok(series) = crate::service::fetch_klines(source, &request.symbol, "1h", 30) else {
            continue;
        };
        if let (Some(first), Some(last)) = (series.first(), series.last()) {
            quotes.insert(
                request.symbol.clone(),
                SourceQuote {
                    price: last.close,
                    previous_day_price: Some(first.close),
                    ..SourceQuote::default()
                },
            );
        }
    }
    quotes
}

// MARK: Parsers

fn json(body: &str) -> Result<Value, QuoteFailure> {
    serde_json::from_str(body).map_err(|error| QuoteFailure::Failed(format!("Data error: {error}")))
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

const DOLLAR_QUOTES: [&str; 8] = [
    "USDT", "USDC", "BUSD", "FDUSD", "TUSD", "USDP", "DAI", "USD",
];

/// `/ticker/24hr?type=MINI`: `openPrice` is the rolling 24h open, a truer day-change
/// reference than a candle picked from the last 25 hours.
pub fn parse_binance(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let tickers: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Binance response".to_owned()))?
        .iter()
        .filter_map(|ticker| Some((ticker.get("symbol")?.as_str()?.to_uppercase(), ticker)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let symbol = request.symbol.to_uppercase();
        let Some(ticker) = tickers.get(&symbol) else {
            continue;
        };
        let Some(price) = number(ticker.get("lastPrice")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = SourceQuote {
            price,
            previous_day_price: number(ticker.get("openPrice")),
            timestamp_ms: ticker.get("closeTime").and_then(Value::as_i64),
            ..SourceQuote::default()
        };
        // Turnover is only a dollar figure on a dollar-quoted pair; otherwise report units traded.
        let dollar_quoted = DOLLAR_QUOTES.iter().any(|quote| symbol.ends_with(quote));
        if let (true, Some(turnover)) = (dollar_quoted, number(ticker.get("quoteVolume"))) {
            quote.volume_24h = Some(turnover);
            quote.volume_kind = Some(VolumeKind::QuoteCurrency);
        } else if let Some(units) = number(ticker.get("volume")) {
            quote.volume_24h = Some(units);
            quote.volume_kind = Some(VolumeKind::Base);
        }
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

/// 24h volume as a dollar turnover where the product is dollar-quoted, otherwise in base
/// units. Coinbase reports base volume only, so turnover is that volume priced at the last trade.
pub fn coinbase_volume(product_id: &str, base_volume: f64, price: f64) -> (f64, VolumeKind) {
    let quote = product_id.rsplit('-').next().unwrap_or("").to_uppercase();
    if ["USD", "USDC", "USDT"].contains(&quote.as_str()) {
        (base_volume * price, VolumeKind::QuoteCurrency)
    } else {
        (base_volume, VolumeKind::Base)
    }
}

/// `/products/{id}/stats`: `{"open","high","low","last","volume"}` as strings.
pub fn parse_coinbase_stats(body: &str, product_id: &str) -> Option<SourceQuote> {
    let value: Value = serde_json::from_str(body).ok()?;
    let price = number(value.get("last")).filter(|price| *price > 0.0)?;
    let mut quote = SourceQuote {
        price,
        previous_day_price: number(value.get("open")),
        timestamp_ms: Some(crate::model::watchlist::now_ms()),
        ..SourceQuote::default()
    };
    if let Some(base) = number(value.get("volume")) {
        let (volume, kind) = coinbase_volume(product_id, base, price);
        quote.volume_24h = Some(volume);
        quote.volume_kind = Some(kind);
    }
    Some(quote)
}

/// Sources that report a 24h percent change rather than a price:
/// `previous = price / (1 + pct / 100)`.
pub fn from_change_percent(price: f64, change_percent_24h: Option<f64>) -> SourceQuote {
    SourceQuote {
        price,
        previous_day_price: change_percent_24h
            .filter(|percent| *percent > -100.0)
            .map(|percent| price / (1.0 + percent / 100.0)),
        ..SourceQuote::default()
    }
}

pub fn parse_coingecko_markets(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let coins: HashMap<String, &Value> = value
        .as_array()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected CoinGecko response".to_owned()))?
        .iter()
        .filter_map(|coin| Some((coin.get("id")?.as_str()?.to_lowercase(), coin)))
        .collect();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(coin) = coins.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(coin.get("current_price")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(coin.get("price_change_percentage_24h")));
        quote.volume_24h = number(coin.get("total_volume"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = coin
            .get("last_updated")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        quotes.insert(request.symbol.clone(), quote);
    }
    Ok(quotes)
}

/// `{"pairs":[{"pairAddress","priceUsd","priceChange":{"h24"},"volume":{"h24"}}]}`.
pub fn parse_dex_pairs(body: &str, requests: &[&QuoteRequest]) -> HashMap<String, SourceQuote> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return HashMap::new();
    };
    let pairs: HashMap<String, &Value> = value
        .get("pairs")
        .and_then(Value::as_array)
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| Some((pair.get("pairAddress")?.as_str()?.to_lowercase(), pair)))
                .collect()
        })
        .unwrap_or_default();
    let now = crate::model::watchlist::now_ms();
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(pair) = pairs.get(&request.symbol.to_lowercase()) else {
            continue;
        };
        let Some(price) = number(pair.get("priceUsd")).filter(|price| *price > 0.0) else {
            continue;
        };
        let mut quote = from_change_percent(price, number(pair.pointer("/priceChange/h24")));
        quote.volume_24h = number(pair.pointer("/volume/h24"));
        quote.volume_kind = quote.volume_24h.map(|_| VolumeKind::QuoteCurrency);
        quote.timestamp_ms = Some(now);
        quotes.insert(request.symbol.clone(), quote);
    }
    quotes
}

/// The network of the pair whose address is `address`, from a DEXScreener search.
pub fn parse_chain(body: &str, address: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("pairs")?
        .as_array()?
        .iter()
        .find(|pair| {
            pair.get("pairAddress")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(address))
        })?
        .get("chainId")?
        .as_str()
        .map(str::to_owned)
}

/// `/v2/stocks/snapshots`: the latest trade, and the previous session's close as the
/// day-change reference. The daily bar stands in while a symbol has no trade yet.
pub fn parse_alpaca_snapshots(
    body: &str,
    requests: &[QuoteRequest],
) -> Result<HashMap<String, SourceQuote>, QuoteFailure> {
    let value = json(body)?;
    let snapshots = value
        .as_object()
        .ok_or_else(|| QuoteFailure::Failed("Unexpected Alpaca response".to_owned()))?;
    let mut quotes = HashMap::new();
    for request in requests {
        let Some(snapshot) = snapshots.get(&request.symbol.to_uppercase()) else {
            continue;
        };
        let Some(price) = number(snapshot.pointer("/latestTrade/p"))
            .or_else(|| number(snapshot.pointer("/dailyBar/c")))
            .filter(|price| *price > 0.0)
        else {
            continue;
        };
        let volume = number(snapshot.pointer("/dailyBar/v"));
        quotes.insert(
            request.symbol.clone(),
            SourceQuote {
                price,
                previous_day_price: number(snapshot.pointer("/prevDailyBar/c")),
                volume_24h: volume,
                volume_kind: volume.map(|_| VolumeKind::Shares),
                timestamp_ms: snapshot
                    .pointer("/latestTrade/t")
                    .and_then(Value::as_str)
                    .and_then(parse_iso_ms),
            },
        );
    }
    Ok(quotes)
}

/// ISO-8601 UTC instant (`2024-01-15T15:00:00Z`, optional fractional seconds) in epoch ms.
pub fn parse_iso_ms(text: &str) -> Option<i64> {
    let text = text.trim().strip_suffix('Z').unwrap_or(text.trim());
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let (year, month, day) = (
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
        date_parts.next()?.parse::<i64>().ok()?,
    );
    let time = time.split(['+', '-']).next()?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock_parts = clock.split(':');
    let hour = clock_parts.next()?.parse::<i64>().ok()?;
    let minute = clock_parts.next()?.parse::<i64>().ok()?;
    let second = clock_parts
        .next()
        .map_or(Some(0), |s| s.parse::<i64>().ok())?;
    let millis = fraction
        .chars()
        .chain("000".chars())
        .take(3)
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    let days = crate::format::days_from_civil(year, month, day);
    Some(((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000) + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(symbol: &str) -> QuoteRequest {
        QuoteRequest {
            symbol: symbol.to_owned(),
            chain: None,
        }
    }

    #[test]
    fn binance_mini_ticker_with_dollar_and_base_volume() {
        let body = r#"[
            {"symbol":"BTCUSDT","lastPrice":"67000.5","openPrice":"66000","volume":"100","quoteVolume":"6700000","closeTime":1700000000000},
            {"symbol":"ETHBTC","lastPrice":"0.05","openPrice":"0.049","volume":"2000","quoteVolume":"100","closeTime":1700000000001}
        ]"#;
        let quotes = parse_binance(
            body,
            &[request("BTCUSDT"), request("ETHBTC"), request("GONEUSDT")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2, "an unanswered symbol is omitted");
        let btc = &quotes["BTCUSDT"];
        assert_eq!(
            (btc.price, btc.previous_day_price),
            (67_000.5, Some(66_000.0))
        );
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(6_700_000.0), Some(VolumeKind::QuoteCurrency))
        );
        assert_eq!(btc.timestamp_ms, Some(1_700_000_000_000));
        let eth = &quotes["ETHBTC"];
        assert_eq!(
            (eth.volume_24h, eth.volume_kind),
            (Some(2_000.0), Some(VolumeKind::Base)),
            "no dollar figure on a non-dollar pair"
        );
    }

    #[test]
    fn binance_rejects_non_arrays_and_zero_prices() {
        assert!(matches!(
            parse_binance(r#"{"code":-1121}"#, &[request("X")]),
            Err(QuoteFailure::Failed(_))
        ));
        let zero = r#"[{"symbol":"AUSDT","lastPrice":"0","openPrice":"1"}]"#;
        assert!(parse_binance(zero, &[request("AUSDT")]).unwrap().is_empty());
    }

    #[test]
    fn coinbase_stats_price_volume_in_dollars_only_for_dollar_pairs() {
        let body = r#"{"open":"100","high":"120","low":"90","last":"110","volume":"50"}"#;
        let usd = parse_coinbase_stats(body, "BTC-USD").unwrap();
        assert_eq!((usd.price, usd.previous_day_price), (110.0, Some(100.0)));
        assert_eq!(
            (usd.volume_24h, usd.volume_kind),
            (Some(5_500.0), Some(VolumeKind::QuoteCurrency))
        );
        let btc = parse_coinbase_stats(body, "ETH-BTC").unwrap();
        assert_eq!(
            (btc.volume_24h, btc.volume_kind),
            (Some(50.0), Some(VolumeKind::Base))
        );
        assert!(parse_coinbase_stats(r#"{"last":"0"}"#, "BTC-USD").is_none());
        assert!(parse_coinbase_stats("nope", "BTC-USD").is_none());
    }

    #[test]
    fn coingecko_derives_the_previous_price_from_the_percent_change() {
        let body = r#"[
            {"id":"bitcoin","current_price":110,"price_change_percentage_24h":10,"total_volume":5000,"last_updated":"2024-01-15T15:00:00.123Z"},
            {"id":"dead","current_price":null}
        ]"#;
        let quotes = parse_coingecko_markets(
            body,
            &[request("Bitcoin"), request("dead"), request("absent")],
        )
        .unwrap();
        assert_eq!(quotes.len(), 1);
        let btc = &quotes["Bitcoin"];
        assert!((btc.previous_day_price.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(btc.volume_kind, Some(VolumeKind::QuoteCurrency));
        assert_eq!(btc.timestamp_ms, Some(1_705_330_800_123));
    }

    #[test]
    fn percent_change_of_minus_100_or_worse_has_no_reference() {
        assert_eq!(
            from_change_percent(5.0, Some(-100.0)).previous_day_price,
            None
        );
        assert_eq!(
            from_change_percent(5.0, Some(-150.0)).previous_day_price,
            None
        );
        assert_eq!(from_change_percent(5.0, None).previous_day_price, None);
        assert_eq!(
            from_change_percent(50.0, Some(-50.0)).previous_day_price,
            Some(100.0)
        );
    }

    #[test]
    fn dex_pairs_match_by_address_ignoring_case() {
        let body = r#"{"pairs":[{"pairAddress":"0xAbC","priceUsd":"0.5","priceChange":{"h24":-50},"volume":{"h24":1234.5}}]}"#;
        let wanted = QuoteRequest {
            symbol: "0xabc".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let other = QuoteRequest {
            symbol: "0xdef".to_owned(),
            chain: Some("solana".to_owned()),
        };
        let quotes = parse_dex_pairs(body, &[&wanted, &other]);
        assert_eq!(quotes.len(), 1);
        let quote = &quotes["0xabc"];
        assert_eq!(quote.price, 0.5);
        assert!((quote.previous_day_price.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(quote.volume_24h, Some(1234.5));
        assert!(parse_dex_pairs("garbage", &[&wanted]).is_empty());
        assert!(parse_dex_pairs(r#"{"pairs":null}"#, &[&wanted]).is_empty());
    }

    #[test]
    fn chain_comes_from_the_matching_pair() {
        let body = r#"{"pairs":[
            {"pairAddress":"0x111","chainId":"ethereum"},
            {"pairAddress":"0xAAA","chainId":"base"}]}"#;
        assert_eq!(parse_chain(body, "0xaaa").as_deref(), Some("base"));
        assert_eq!(parse_chain(body, "0x999"), None);
        assert_eq!(parse_chain("{}", "0x1"), None);
    }

    #[test]
    fn alpaca_prefers_the_latest_trade_and_falls_back_to_the_daily_bar() {
        let body = r#"{
            "AAPL":{"latestTrade":{"p":190.5,"t":"2024-01-15T15:00:00Z"},"dailyBar":{"c":190,"v":1000},"prevDailyBar":{"c":188}},
            "TSLA":{"dailyBar":{"c":200,"v":5}},
            "ZERO":{"latestTrade":{"p":0}}
        }"#;
        let quotes = parse_alpaca_snapshots(
            body,
            &[
                request("aapl"),
                request("TSLA"),
                request("ZERO"),
                request("MISSING"),
            ],
        )
        .unwrap();
        assert_eq!(quotes.len(), 2);
        let aapl = &quotes["aapl"];
        assert_eq!((aapl.price, aapl.previous_day_price), (190.5, Some(188.0)));
        assert_eq!(aapl.volume_kind, Some(VolumeKind::Shares));
        assert_eq!(aapl.timestamp_ms, Some(1_705_330_800_000));
        assert_eq!(quotes["TSLA"].price, 200.0);
        assert_eq!(quotes["TSLA"].previous_day_price, None);
    }

    #[test]
    fn iso_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2024-01-15T15:00:00Z"),
            Some(1_705_330_800_000)
        );
        assert_eq!(
            parse_iso_ms("2024-01-15T15:00:00.5Z"),
            Some(1_705_330_800_500)
        );
        assert_eq!(
            parse_iso_ms("2024-01-15T15:00:00.123456Z"),
            Some(1_705_330_800_123)
        );
        assert_eq!(parse_iso_ms("yesterday"), None);
    }

    #[test]
    fn failures_describe_themselves() {
        assert_eq!(
            QuoteFailure::RateLimited.message(),
            "Rate limited by the provider"
        );
        assert!(
            QuoteFailure::CredentialsMissing
                .message()
                .contains("Alpaca")
        );
        assert_eq!(QuoteFailure::Failed("x".into()).message(), "x");
    }

    #[test]
    fn chunk_sizes_respect_each_providers_limit() {
        assert_eq!(chunk_size(DataSource::Coinbase), 20);
        assert_eq!(chunk_size(DataSource::DexScreener), 30);
        assert_eq!(chunk_size(DataSource::CoinGecko), 250);
    }
}
