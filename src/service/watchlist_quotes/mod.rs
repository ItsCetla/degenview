//! Watchlist prices, kept current for the markets the visible sidebars show.
//!
//! Sidebars say which markets they need and whether they are on screen
//! (`set_demand`); the coordinator merges those needs by market, polls each source
//! once per round in a background thread, and feeds one shared Coinbase socket. Quotes
//! land in a `WatchlistQuoteBook` the UI reads; a repaint callback fires when any
//! quote changes. Nothing here touches the database or the egui types.

mod book;
mod coinbase_ws;
mod engine;
mod provider;

pub use book::WatchlistQuoteBook;
#[allow(unused_imports)]
pub use engine::Config;
#[allow(unused_imports)]
pub use provider::{QuoteFailure, QuoteProvider, QuoteRequest};

use crate::model::InstrumentId;
use crate::model::watchlist::now_ms;
use coinbase_ws::{CoinbaseSocket, CoinbaseTick};
use engine::Engine;
use provider::LiveProvider;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How often the loop drains the Coinbase channel: a busy feed becomes a few updates
/// a second per product.
const TICK: Duration = Duration::from_millis(250);

pub struct WatchlistQuoteCoordinator {
    engine: Arc<Mutex<Engine>>,
    shutdown: Arc<AtomicBool>,
}

impl WatchlistQuoteCoordinator {
    /// Starts the background loop. `repaint` is called (from that thread) whenever a
    /// quote changed.
    pub fn new(book: WatchlistQuoteBook, repaint: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::with_provider(book, repaint, Arc::new(LiveProvider), Config::default())
    }

    pub fn with_provider(
        book: WatchlistQuoteBook,
        repaint: Arc<dyn Fn() + Send + Sync>,
        provider: Arc<dyn QuoteProvider>,
        config: Config,
    ) -> Self {
        let engine = Arc::new(Mutex::new(Engine::new(book.clone(), config)));
        let shutdown = Arc::new(AtomicBool::new(false));
        {
            let (engine, book, shutdown) =
                (Arc::clone(&engine), book.clone(), Arc::clone(&shutdown));
            std::thread::spawn(move || run(&engine, &book, provider.as_ref(), &repaint, &shutdown));
        }
        Self { engine, shutdown }
    }

    /// What `owner` (a window's sidebar) needs now. A hidden sidebar keeps its place
    /// but asks for nothing until it is visible again.
    pub fn set_demand(&self, owner: u64, instruments: Vec<InstrumentId>, visible: bool) {
        self.engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .update(owner, instruments, visible, now_ms());
    }

    /// Drops a window's demand when it closes.
    #[allow(dead_code)] // called once windows release their sidebar
    pub fn release(&self, owner: u64) {
        self.engine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .release(owner, now_ms());
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

impl Drop for WatchlistQuoteCoordinator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    engine: &Mutex<Engine>,
    book: &WatchlistQuoteBook,
    provider: &dyn QuoteProvider,
    repaint: &Arc<dyn Fn() + Send + Sync>,
    shutdown: &AtomicBool,
) {
    let (sender, receiver): (_, Receiver<CoinbaseTick>) = mpsc::channel();
    let mut socket: Option<CoinbaseSocket> = None;
    let mut last_round = i64::MIN / 2;
    let mut seen_revision = book.revision();

    while !shutdown.load(Ordering::Acquire) {
        std::thread::sleep(TICK);
        let now = now_ms();
        let (products, interval, active) = {
            let guard = engine.lock().unwrap_or_else(PoisonError::into_inner);
            (
                guard.coinbase_products(),
                guard.poll_interval_ms(),
                guard.has_subscriptions(),
            )
        };

        // One socket for every Coinbase product any visible sidebar shows.
        if socket.as_ref().map(CoinbaseSocket::products) != Some(products.as_slice())
            && !(socket.is_none() && products.is_empty())
        {
            socket = None;
            if !products.is_empty() {
                socket = Some(CoinbaseSocket::connect(products, sender.clone()));
            }
        }

        let ticks: Vec<CoinbaseTick> = receiver.try_iter().collect();
        if !ticks.is_empty() {
            engine
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .receive_ticks(ticks, now);
        }

        if active && now - last_round >= interval {
            last_round = now;
            engine::poll(engine, provider, &now_ms);
        }

        let revision = book.revision();
        if revision != seen_revision {
            seen_revision = revision;
            repaint();
        }
    }
}
