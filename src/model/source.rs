use serde::{Deserialize, Serialize};

/// A market data provider. The symbol alone never identifies a market: `BTCUSDT`
/// on Binance and `BTC-USD` on Coinbase are different instruments.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Binance,
    Coinbase,
    CoinGecko,
    DexScreener,
    Alpaca,
    Polymarket,
    Kalshi,
    CoinMarketCap,
}

impl DataSource {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Binance => "Binance",
            Self::Coinbase => "Coinbase",
            Self::CoinGecko => "CoinGecko",
            Self::DexScreener => "DEXScreener",
            Self::Alpaca => "Alpaca (IEX)",
            Self::Polymarket => "Polymarket",
            Self::Kalshi => "Kalshi",
            Self::CoinMarketCap => "CoinMarketCap",
        }
    }

    /// Upper-case provider prefix in the import/export text format.
    pub fn watchlist_slug(self) -> &'static str {
        match self {
            Self::Binance => "BINANCE",
            Self::Coinbase => "COINBASE",
            Self::CoinGecko => "COINGECKO",
            Self::DexScreener => "DEXSCREENER",
            Self::Alpaca => "ALPACA",
            Self::Polymarket => "POLYMARKET",
            Self::Kalshi => "KALSHI",
            Self::CoinMarketCap => "CMC",
        }
    }

    /// Whether this source quotes probabilities in 0…1, charted as lines.
    pub fn is_prediction_market(self) -> bool {
        matches!(self, Self::Polymarket | Self::Kalshi)
    }

    /// Whether this source reports per-candle turnover for the volume bars to draw. CoinGecko's
    /// OHLC endpoint has no volume column, and the line charts report none at all.
    pub fn provides_volume(self) -> bool {
        matches!(
            self,
            Self::Binance | Self::Coinbase | Self::DexScreener | Self::Alpaca
        )
    }

    /// Whether this source's data is drawn as a line rather than candles.
    pub fn is_line_chart(self) -> bool {
        self.is_prediction_market() || matches!(self, Self::CoinMarketCap)
    }

    /// A human-readable pair label: `BTCUSDT` → `BTC/USDT`, `BTC-USD` → `BTC/USD`.
    pub fn display_symbol(self, symbol: &str) -> String {
        let raw = symbol.trim();
        let symbol = raw.to_uppercase();
        match self {
            Self::Binance => match split_binance_pair(&symbol) {
                Some((base, quote)) => format!("{base}/{quote}"),
                None => symbol,
            },
            Self::Coinbase => symbol.replace('-', "/"),
            Self::CoinGecko | Self::Alpaca => symbol,
            Self::CoinMarketCap => crate::service::cmc::title(raw).to_owned(),
            _ => raw.to_owned(),
        }
    }
}

/// Quote assets Binance pairs end in, longest first so BTCFDUSD is BTC/FDUSD instead of
/// BTCFD/USD. Matches Swift's MarketSymbol quote list.
const BINANCE_QUOTES: [&str; 16] = [
    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY", "BRL", "JPY",
    "USD", "BTC", "ETH", "BNB",
];

/// Splits an upper-case Binance pair like `SOLBNB` into `("SOL", "BNB")`, or `None` when no
/// known quote ends it (a bare asset such as `SOL`).
pub fn split_binance_pair(symbol: &str) -> Option<(&str, &'static str)> {
    BINANCE_QUOTES.iter().find_map(|quote| {
        symbol
            .strip_suffix(quote)
            .filter(|base| !base.is_empty())
            .map(|base| (base, *quote))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_pairs_use_swift_display_labels_without_changing_their_ids() {
        assert_eq!(DataSource::Binance.display_symbol("BTCUSDT"), "BTC/USDT");
        assert_eq!(DataSource::Binance.display_symbol("BTCFDUSD"), "BTC/FDUSD");
        assert_eq!(DataSource::Binance.display_symbol("ETHBTC"), "ETH/BTC");
        assert_eq!(DataSource::Binance.display_symbol("BTCEUR"), "BTC/EUR");
        assert_eq!(DataSource::Binance.display_symbol("BTC"), "BTC");
        assert_eq!(DataSource::Coinbase.display_symbol("btc-usd"), "BTC/USD");
    }

    #[test]
    fn only_sources_with_per_candle_turnover_provide_volume() {
        assert!(DataSource::Binance.provides_volume());
        assert!(DataSource::Alpaca.provides_volume());
        assert!(!DataSource::CoinGecko.provides_volume());
        assert!(!DataSource::Polymarket.provides_volume());
        assert!(!DataSource::CoinMarketCap.provides_volume());
    }
}
