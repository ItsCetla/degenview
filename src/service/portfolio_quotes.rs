//! Portfolio valuation over the app-wide watchlist quote stream.
//!
//! A portfolio is another consumer of `WatchlistQuoteCoordinator`: it contributes its markets to
//! the same merged demand set as every watchlist and reads the same transient quote book. Currency
//! projection can touch the network, so it runs off the UI thread and publishes an immutable
//! snapshot when every historical/current conversion is ready.

use crate::model::InstrumentId;
use crate::model::portfolio::{
    PortfolioAsset, PortfolioCurrency, PortfolioLedgerSnapshot, PortfolioQuote, PortfolioSnapshot,
    PortfolioTransaction,
};
use crate::model::portfolio_accounting::unique_assets;
use crate::model::watchlist_quote::Freshness;
use crate::service::fx_rates::{Conversion, FxRateService};
use crate::service::watchlist_quotes::{WatchlistQuoteBook, WatchlistQuoteCoordinator};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

pub trait PortfolioFxRates: Send + Sync {
    fn conversion(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        on: Option<i64>,
    ) -> Result<Conversion, String>;

    fn conversions(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        dates: &[i64],
    ) -> HashMap<i64, Result<Conversion, String>>;
}

impl PortfolioFxRates for FxRateService {
    fn conversion(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        on: Option<i64>,
    ) -> Result<Conversion, String> {
        FxRateService::conversion(self, from, to, on).map_err(|error| error.message())
    }

    fn conversions(
        &self,
        from: PortfolioCurrency,
        to: PortfolioCurrency,
        dates: &[i64],
    ) -> HashMap<i64, Result<Conversion, String>> {
        FxRateService::conversions(self, from, to, dates)
            .into_iter()
            .map(|(date, result)| (date, result.map_err(|error| error.message())))
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioProjection {
    pub transactions: Vec<PortfolioTransaction>,
    pub quotes: HashMap<String, PortfolioQuote>,
    pub history: Vec<PortfolioSnapshot>,
    pub uses_cached_exchange_rates: bool,
    pub unpriced_asset_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioQuoteState {
    pub projection: Option<PortfolioProjection>,
    pub is_updating: bool,
    pub error: Option<String>,
}

#[derive(Clone, PartialEq)]
struct ProjectionBasis {
    snapshot: PortfolioLedgerSnapshot,
    portfolio_ids: HashSet<u64>,
    reporting_currency: PortfolioCurrency,
}

struct FeedState {
    basis: Option<ProjectionBasis>,
    book_revision: u64,
    generation: u64,
    published: PortfolioQuoteState,
    pending: Option<(ProjectionBasis, u64, i64)>,
    worker_running: bool,
}

type CurrencyPair = (PortfolioCurrency, PortfolioCurrency);
type HistoricalConversions = HashMap<CurrencyPair, HashMap<i64, Result<Conversion, String>>>;

impl Default for FeedState {
    fn default() -> Self {
        Self {
            basis: None,
            book_revision: u64::MAX,
            generation: 0,
            published: PortfolioQuoteState::default(),
            pending: None,
            worker_running: false,
        }
    }
}

struct Inner {
    book: WatchlistQuoteBook,
    coordinator: WatchlistQuoteCoordinator,
    fx: Arc<dyn PortfolioFxRates>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    state: Mutex<FeedState>,
}

/// Cheap shared handle. Each visible portfolio tab supplies one demand owner id; expensive
/// projections are generation-checked so a late result can never replace a newer selection.
#[derive(Clone)]
pub struct PortfolioQuoteFeed {
    inner: Arc<Inner>,
}

impl PortfolioQuoteFeed {
    pub fn new(
        book: WatchlistQuoteBook,
        coordinator: WatchlistQuoteCoordinator,
        fx: Arc<dyn PortfolioFxRates>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                book,
                coordinator,
                fx,
                repaint,
                state: Mutex::new(FeedState::default()),
            }),
        }
    }

    /// The FX service the feed converts with, for callers that need rates of their own.
    #[allow(dead_code)] // the portfolio host wires this when it needs its own rates
    pub fn fx_rates(&self) -> Arc<dyn PortfolioFxRates> {
        Arc::clone(&self.inner.fx)
    }

    pub fn update(
        &self,
        owner: u64,
        snapshot: &PortfolioLedgerSnapshot,
        portfolio_ids: &HashSet<u64>,
        reporting_currency: PortfolioCurrency,
        visible: bool,
        now_ms: i64,
    ) {
        let assets = eligible_assets(snapshot, portfolio_ids);
        let instruments = assets.iter().map(instrument_for).collect();
        self.inner
            .coordinator
            .set_demand(owner, instruments, visible);
        if !visible {
            return;
        }

        let basis = ProjectionBasis {
            snapshot: snapshot.clone(),
            portfolio_ids: portfolio_ids.clone(),
            reporting_currency,
        };
        let book_revision = self.inner.book.revision();
        let start_worker = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.basis.as_ref() == Some(&basis) && state.book_revision == book_revision {
                return;
            }
            if state.basis.as_ref() != Some(&basis) {
                state.published.projection = None;
            }
            state.basis = Some(basis.clone());
            state.book_revision = book_revision;
            state.generation = state.generation.wrapping_add(1);
            state.published.is_updating = true;
            state.published.error = None;
            let generation = state.generation;
            state.pending = Some((basis, generation, now_ms));
            if state.worker_running {
                false
            } else {
                state.worker_running = true;
                true
            }
        };
        if start_worker {
            let inner = Arc::clone(&self.inner);
            crate::service::work_queue::submit(move || run_projections(&inner));
        }
    }

    pub fn state(&self) -> PortfolioQuoteState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .published
            .clone()
    }

    pub fn release(&self, owner: u64) {
        self.inner.coordinator.release(owner);
    }
}

fn run_projections(inner: &Inner) {
    loop {
        let request = {
            let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            match state.pending.take() {
                Some(request) => request,
                None => {
                    state.worker_running = false;
                    return;
                }
            }
        };
        let (basis, generation, now_ms) = request;
        let result = build_projection(
            &basis.snapshot,
            &basis.portfolio_ids,
            basis.reporting_currency,
            &inner.book,
            inner.fx.as_ref(),
            now_ms,
        );
        let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.generation == generation {
            match result {
                Ok(projection) => {
                    state.published.projection = Some(projection);
                    state.published.error = None;
                }
                Err(error) => state.published.error = Some(error),
            }
            state.published.is_updating = false;
            drop(state);
            (inner.repaint)();
        }
    }
}

pub fn instrument_for(asset: &PortfolioAsset) -> InstrumentId {
    let market_symbol = asset
        .key
        .split_once(':')
        .map(|(_, symbol)| symbol)
        .filter(|symbol| !symbol.is_empty())
        .unwrap_or(&asset.symbol);
    InstrumentId::new(asset.source, market_symbol)
}

fn eligible_assets(
    snapshot: &PortfolioLedgerSnapshot,
    portfolio_ids: &HashSet<u64>,
) -> Vec<PortfolioAsset> {
    let transactions: Vec<PortfolioTransaction> = snapshot
        .transactions
        .iter()
        .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
        .cloned()
        .collect();
    unique_assets(&transactions)
}

fn build_projection(
    snapshot: &PortfolioLedgerSnapshot,
    portfolio_ids: &HashSet<u64>,
    reporting_currency: PortfolioCurrency,
    book: &WatchlistQuoteBook,
    fx: &dyn PortfolioFxRates,
    now_ms: i64,
) -> Result<PortfolioProjection, String> {
    let portfolios: HashMap<u64, PortfolioCurrency> = snapshot
        .portfolios
        .iter()
        .filter(|portfolio| portfolio_ids.contains(&portfolio.id))
        .map(|portfolio| (portfolio.id, portfolio.base_currency))
        .collect();
    let selected_transactions: Vec<PortfolioTransaction> = snapshot
        .transactions
        .iter()
        .filter(|transaction| portfolio_ids.contains(&transaction.portfolio_id))
        .cloned()
        .collect();
    let selected_history: Vec<PortfolioSnapshot> = snapshot
        .historical_snapshots
        .iter()
        .filter(|point| portfolio_ids.contains(&point.portfolio_id))
        .cloned()
        .collect();

    let mut dates_by_pair: HashMap<CurrencyPair, Vec<i64>> = HashMap::new();
    for transaction in &selected_transactions {
        if transaction.price.is_some() && transaction.price_currency != reporting_currency {
            dates_by_pair
                .entry((transaction.price_currency, reporting_currency))
                .or_default()
                .push(transaction.timestamp);
        }
        if transaction.fee != 0.0 && transaction.fee_currency != reporting_currency {
            dates_by_pair
                .entry((transaction.fee_currency, reporting_currency))
                .or_default()
                .push(transaction.timestamp);
        }
    }
    for point in &selected_history {
        if let Some(base) = portfolios.get(&point.portfolio_id)
            && *base != reporting_currency
        {
            dates_by_pair
                .entry((*base, reporting_currency))
                .or_default()
                .push(point.timestamp);
        }
    }

    let mut historical = HashMap::new();
    for ((from, to), dates) in dates_by_pair {
        historical.insert((from, to), fx.conversions(from, to, &dates));
    }
    let mut used_cache = false;
    let mut transactions = Vec::with_capacity(selected_transactions.len());
    for mut transaction in selected_transactions {
        if transaction.price.is_some() && transaction.price_currency != reporting_currency {
            let conversion = historical_conversion(
                &historical,
                transaction.price_currency,
                reporting_currency,
                transaction.timestamp,
            )?;
            transaction.price = transaction.price.map(|price| price * conversion.rate);
            used_cache |= conversion.is_cached;
        }
        if transaction.fee != 0.0 && transaction.fee_currency != reporting_currency {
            let conversion = historical_conversion(
                &historical,
                transaction.fee_currency,
                reporting_currency,
                transaction.timestamp,
            )?;
            transaction.fee *= conversion.rate;
            used_cache |= conversion.is_cached;
        }
        transaction.price_currency = reporting_currency;
        transaction.fee_currency = reporting_currency;
        transactions.push(transaction);
    }

    let assets = unique_assets(&transactions);
    let mut current_rates: HashMap<PortfolioCurrency, Conversion> = HashMap::new();
    let mut quotes = HashMap::new();
    for asset in &assets {
        let Some(source) = book.get(&instrument_for(asset)) else {
            continue;
        };
        if matches!(
            source.freshness,
            Freshness::Unavailable | Freshness::Unsupported
        ) {
            continue;
        }
        let Some(price) = source
            .last
            .filter(|price| price.is_finite() && *price > 0.0)
        else {
            continue;
        };
        let conversion = if asset.quote_currency == reporting_currency {
            Conversion {
                rate: 1.0,
                observation_ms: now_ms,
                is_cached: false,
            }
        } else if let Some(conversion) = current_rates.get(&asset.quote_currency) {
            *conversion
        } else {
            let conversion = fx.conversion(asset.quote_currency, reporting_currency, None)?;
            current_rates.insert(asset.quote_currency, conversion);
            conversion
        };
        used_cache |= conversion.is_cached;
        let timestamp = if matches!(source.freshness, Freshness::Live | Freshness::MarketClosed) {
            now_ms
        } else {
            source.timestamp_ms.unwrap_or(0)
        };
        quotes.insert(
            asset.key.clone(),
            PortfolioQuote {
                price: price * conversion.rate,
                previous_day_price: source
                    .change
                    .map(|change| (price - change) * conversion.rate),
                timestamp,
            },
        );
    }

    let unpriced_asset_count = assets
        .iter()
        .filter(|asset| !quotes.contains_key(&asset.key))
        .count();
    let mut history = Vec::with_capacity(selected_history.len());
    for mut point in selected_history {
        let base = portfolios
            .get(&point.portfolio_id)
            .copied()
            .unwrap_or(reporting_currency);
        if base != reporting_currency {
            let conversion =
                historical_conversion(&historical, base, reporting_currency, point.timestamp)?;
            point.value *= conversion.rate;
            point.net_contributions *= conversion.rate;
            point.realized_pnl *= conversion.rate;
            point.unrealized_pnl *= conversion.rate;
            used_cache |= conversion.is_cached;
        }
        history.push(point);
    }
    history.sort_by_key(|point| point.timestamp);
    if portfolio_ids.len() > 1 {
        history = aggregate_history(&history);
    }

    Ok(PortfolioProjection {
        transactions,
        quotes,
        history,
        uses_cached_exchange_rates: used_cache,
        unpriced_asset_count,
    })
}

fn historical_conversion(
    conversions: &HistoricalConversions,
    from: PortfolioCurrency,
    to: PortfolioCurrency,
    timestamp: i64,
) -> Result<Conversion, String> {
    conversions
        .get(&(from, to))
        .and_then(|by_date| by_date.get(&timestamp))
        .cloned()
        .unwrap_or_else(|| {
            Err(format!(
                "Exchange rate from {} to {} is unavailable.",
                from.display_name(),
                to.display_name()
            ))
        })
}

fn aggregate_history(points: &[PortfolioSnapshot]) -> Vec<PortfolioSnapshot> {
    let mut by_timestamp: BTreeMap<i64, Vec<&PortfolioSnapshot>> = BTreeMap::new();
    for point in points {
        by_timestamp.entry(point.timestamp).or_default().push(point);
    }
    by_timestamp
        .into_iter()
        .map(|(timestamp, points)| PortfolioSnapshot {
            portfolio_id: 0,
            timestamp,
            value: points.iter().map(|point| point.value).sum(),
            net_contributions: points.iter().map(|point| point.net_contributions).sum(),
            realized_pnl: points.iter().map(|point| point.realized_pnl).sum(),
            unrealized_pnl: points.iter().map(|point| point.unrealized_pnl).sum(),
            is_complete: points.iter().all(|point| point.is_complete),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{Portfolio, PortfolioSource, PortfolioTransactionType};
    use crate::model::watchlist_quote::WatchlistQuote;

    struct FakeFx;

    impl PortfolioFxRates for FakeFx {
        fn conversion(
            &self,
            from: PortfolioCurrency,
            to: PortfolioCurrency,
            on: Option<i64>,
        ) -> Result<Conversion, String> {
            Ok(Conversion {
                rate: rate(from, to),
                observation_ms: on.unwrap_or(9_000),
                is_cached: false,
            })
        }

        fn conversions(
            &self,
            from: PortfolioCurrency,
            to: PortfolioCurrency,
            dates: &[i64],
        ) -> HashMap<i64, Result<Conversion, String>> {
            dates
                .iter()
                .map(|date| {
                    (
                        *date,
                        Ok(Conversion {
                            rate: rate(from, to),
                            observation_ms: *date,
                            is_cached: false,
                        }),
                    )
                })
                .collect()
        }
    }

    fn rate(from: PortfolioCurrency, to: PortfolioCurrency) -> f64 {
        match (from, to) {
            (PortfolioCurrency::Usd, PortfolioCurrency::Eur) => 0.9,
            _ if from == to => 1.0,
            _ => panic!("unexpected conversion"),
        }
    }

    fn asset() -> PortfolioAsset {
        PortfolioAsset {
            key: "binance:BTCUSDT".into(),
            symbol: "BTC/USDT".into(),
            name: "Bitcoin".into(),
            source: DataSource::Binance,
            quote_currency: PortfolioCurrency::Usd,
            metadata: HashMap::new(),
        }
    }

    fn snapshot() -> PortfolioLedgerSnapshot {
        PortfolioLedgerSnapshot {
            portfolios: vec![Portfolio {
                base_currency: PortfolioCurrency::Usd,
                ..Portfolio::new(1, "Main", 1_000)
            }],
            transactions: vec![PortfolioTransaction {
                id: 2,
                portfolio_id: 1,
                asset: asset(),
                kind: PortfolioTransactionType::Buy,
                quantity: 2.0,
                price: Some(10_000.0),
                price_currency: PortfolioCurrency::Usd,
                fee: 10.0,
                fee_currency: PortfolioCurrency::Usd,
                timestamp: 2_000,
                notes: String::new(),
                source: PortfolioSource::Manual,
                external_transaction_id: None,
                created_at: 2_000,
                updated_at: 2_000,
            }],
            ..PortfolioLedgerSnapshot::default()
        }
    }

    #[test]
    fn asset_keys_supply_the_provider_symbol() {
        let instrument = instrument_for(&asset());
        assert_eq!(instrument.source, DataSource::Binance);
        assert_eq!(instrument.symbol, "BTCUSDT");
    }

    #[test]
    fn projection_converts_costs_and_shared_live_quotes() {
        let book = WatchlistQuoteBook::default();
        let instrument = instrument_for(&asset());
        book.apply(HashMap::from([(
            instrument.key(),
            WatchlistQuote {
                last: Some(20_000.0),
                change: Some(2_000.0),
                timestamp_ms: Some(8_000),
                freshness: Freshness::Live,
                ..WatchlistQuote::default()
            },
        )]));

        let projection = build_projection(
            &snapshot(),
            &HashSet::from([1]),
            PortfolioCurrency::Eur,
            &book,
            &FakeFx,
            10_000,
        )
        .expect("projection");
        assert_eq!(projection.transactions[0].price, Some(9_000.0));
        assert_eq!(projection.transactions[0].fee, 9.0);
        assert_eq!(projection.quotes["binance:BTCUSDT"].price, 18_000.0);
        assert_eq!(
            projection.quotes["binance:BTCUSDT"].previous_day_price,
            Some(16_200.0)
        );
        assert_eq!(projection.unpriced_asset_count, 0);
    }

    #[test]
    fn absent_quotes_remain_unpriced_instead_of_becoming_zero() {
        let projection = build_projection(
            &snapshot(),
            &HashSet::from([1]),
            PortfolioCurrency::Usd,
            &WatchlistQuoteBook::default(),
            &FakeFx,
            10_000,
        )
        .expect("projection");
        assert!(projection.quotes.is_empty());
        assert_eq!(projection.unpriced_asset_count, 1);
    }
}
