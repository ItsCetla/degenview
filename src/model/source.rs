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
        let raw = symbol.trim();
        let symbol = raw.to_uppercase();
        match self {
            Self::Binance => {
                // Match Swift's MarketSymbol quote list, longest first so BTCFDUSD is
                // BTC/FDUSD instead of BTCFD/USD.
                const QUOTES: [&str; 16] = [
                    "FDUSD", "USDT", "USDC", "BUSD", "TUSD", "USDP", "DAI", "EUR", "GBP", "TRY",
                    "BRL", "JPY", "USD", "BTC", "ETH", "BNB",
                ];
                QUOTES
                    .iter()
                    .find_map(|quote| {
                        symbol
                            .strip_suffix(quote)
                            .filter(|base| !base.is_empty())
                            .map(|base| format!("{base}/{quote}"))
                    })
                    .unwrap_or(symbol)
            }
            Self::Coinbase => symbol.replace('-', "/"),
            Self::CoinGecko | Self::Alpaca => symbol,
            Self::CoinMarketCap => crate::service::cmc::title(raw).to_owned(),
            _ => raw.to_owned(),
        }
    }
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
}
