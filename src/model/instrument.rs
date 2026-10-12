use crate::model::DataSource;
use serde::{Deserialize, Serialize};

/// A market on one provider. Equality and identity are source-qualified: a
/// display title is never identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct InstrumentId {
    pub source: DataSource,
    pub symbol: String,
}

impl InstrumentId {
    pub fn new(source: DataSource, symbol: impl Into<String>) -> Self {
        Self {
            source,
            symbol: symbol.into().trim().to_owned(),
        }
    }

    /// Stable identity, case-insensitive where the provider is. DEX addresses and
    /// prediction-market ids are case-sensitive. Consumed by watchlist persistence
    /// (Stage 6); exercised by tests today.
    #[allow(dead_code)]
    pub fn key(&self) -> String {
        format!(
            "{}:{}",
            self.source.watchlist_slug(),
            Self::normalized(&self.symbol, self.source)
        )
    }

    /// The id the provider's API takes. A bare Binance asset (`BTC`) is quoted in
    /// USDT, and Coinbase ids are `BASE-QUOTE`.
    pub fn api_symbol(&self) -> String {
        match self.source {
            DataSource::Binance => {
                let upper = self.symbol.to_uppercase().replace('/', "");
                // A one-letter "base" is a bare wrapped asset (WBTC, WETH), not a pair.
                if crate::model::source::split_binance_pair(&upper)
                    .is_some_and(|(base, _)| base.len() > 1)
                {
                    upper
                } else {
                    format!("{upper}USDT")
                }
            }
            DataSource::Coinbase => {
                let id = self.symbol.to_uppercase().replace('/', "-");
                if id.contains('-') {
                    id
                } else {
                    format!("{id}-USD")
                }
            }
            _ => self.symbol.clone(),
        }
    }

    /// Whether both name one market on one provider, comparing by the id the
    /// provider's API takes.
    pub fn is_same_market(&self, other: &InstrumentId) -> bool {
        self.source == other.source && self.api_symbol().eq_ignore_ascii_case(&other.api_symbol())
    }

    fn normalized(symbol: &str, source: DataSource) -> String {
        match source {
            DataSource::Binance | DataSource::Coinbase | DataSource::Alpaca => {
                symbol.to_uppercase()
            }
            DataSource::CoinGecko => symbol.to_lowercase(),
            _ => symbol.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_source_qualified_and_case_normalized() {
        let binance = InstrumentId::new(DataSource::Binance, "btcusdt");
        let binance_upper = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let coinbase = InstrumentId::new(DataSource::Coinbase, "BTCUSDT");
        assert_eq!(binance.key(), binance_upper.key());
        assert_ne!(binance.key(), coinbase.key());

        let coingecko = InstrumentId::new(DataSource::CoinGecko, "Bitcoin");
        assert_eq!(coingecko.key(), "COINGECKO:bitcoin");
    }

    #[test]
    fn api_symbol_quotes_bare_binance_and_coinbase() {
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "BTC").api_symbol(),
            "BTCUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "ETHUSDT").api_symbol(),
            "ETHUSDT"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "BTC").api_symbol(),
            "BTC-USD"
        );
        assert_eq!(
            InstrumentId::new(DataSource::Coinbase, "ETH/USD").api_symbol(),
            "ETH-USD"
        );
    }

    #[test]
    fn api_symbol_keeps_any_quoted_binance_pair() {
        for pair in ["SOLBNB", "XRPBTC", "ETHFDUSD", "sol/bnb"] {
            let api = InstrumentId::new(DataSource::Binance, pair).api_symbol();
            assert_eq!(api, pair.to_uppercase().replace('/', ""));
        }
        assert_eq!(
            InstrumentId::new(DataSource::Binance, "WBTC").api_symbol(),
            "WBTCUSDT"
        );
    }

    #[test]
    fn same_market_compares_by_api_symbol() {
        let bare = InstrumentId::new(DataSource::Binance, "BTC");
        let quoted = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        assert!(bare.is_same_market(&quoted));
        assert!(!bare.is_same_market(&InstrumentId::new(DataSource::Coinbase, "BTC")));
    }
}
