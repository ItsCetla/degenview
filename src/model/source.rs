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

    /// One-letter mark shown in the card header until asset icons are designed.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Binance => "B",
            Self::Coinbase => "C",
            Self::CoinGecko => "G",
            Self::DexScreener => "D",
            Self::Alpaca => "A",
            Self::Polymarket => "P",
            Self::Kalshi => "K",
            Self::CoinMarketCap => "M",
        }
    }

    /// Whether this source quotes probabilities in 0…1, charted as lines.
    pub fn is_prediction_market(self) -> bool {
        matches!(self, Self::Polymarket | Self::Kalshi)
    }

    /// Whether this source's data is drawn as a line rather than candles.
    pub fn is_line_chart(self) -> bool {
        self.is_prediction_market() || matches!(self, Self::CoinMarketCap)
    }

    /// A human-readable pair label: `BTCUSDT` → `BTC/USDT`, `BTC-USD` → `BTC/USD`.
    pub fn display_symbol(self, symbol: &str) -> String {
        match self {
            Self::Binance => match symbol.strip_suffix("USDT") {
                Some(base) => format!("{base}/USDT"),
                None => symbol.to_owned(),
            },
            Self::Coinbase => symbol.replace('-', "/"),
            Self::CoinGecko => symbol.to_uppercase(),
            _ => symbol.to_owned(),
        }
    }
}
