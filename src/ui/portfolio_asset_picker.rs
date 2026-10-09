//! A search sheet that picks one market for a portfolio: the first step of "Add Transaction"
//! (which asset is it for?) and of "Match…" in the CoinMarketCap import. Searches the crypto
//! sources or the stock source as you type (debounced, off the UI thread) and hands back the
//! chosen `SearchResult`; the host turns it into a `PortfolioAsset`.

// Transient: the portfolio host opens this once it is wired.
#![allow(dead_code)]

use crate::model::DataSource;
use crate::service::SearchResult;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Vec2};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

pub const SIZE: Vec2 = Vec2::new(520.0, 560.0);
pub const DEBOUNCE: Duration = Duration::from_millis(300);
pub const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
];
pub const STOCK_SOURCES: [DataSource; 1] = [DataSource::Alpaca];

/// A provider's ticker search: results, or why there are none.
pub type SearchFn =
    Arc<dyn Fn(DataSource, &str) -> Result<Vec<SearchResult>, String> + Send + Sync>;

type Batch = Vec<(DataSource, Result<Vec<SearchResult>, String>)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerTab {
    Crypto,
    Stocks,
}

impl PickerTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Crypto => "Crypto",
            Self::Stocks => "Stocks",
        }
    }

    pub fn sources(self) -> &'static [DataSource] {
        match self {
            Self::Crypto => &CRYPTO_SOURCES,
            Self::Stocks => &STOCK_SOURCES,
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Crypto => "Search coins — BTC, ETH, PEPE…",
            Self::Stocks => "Search stocks and ETFs — AAPL, SPY…",
        }
    }
}

pub struct AssetPickerState {
    pub title: String,
    pub subtitle: String,
    pub action_label: String,
    pub tab: PickerTab,
    pub query: String,
    pub selected: Option<SearchResult>,
    results: Vec<(DataSource, Result<Vec<SearchResult>, String>)>,
    searched_query: String,
    searched_tab: Option<PickerTab>,
    inbox: Option<Receiver<Batch>>,
    debounce: Option<Instant>,
    search: SearchFn,
    focus: bool,
}

#[derive(Clone, Debug)]
pub enum AssetPickerOutcome {
    Choose(Box<SearchResult>),
    Cancel,
}

impl AssetPickerState {
    pub fn new(title: &str, subtitle: &str, action_label: &str) -> Self {
        Self::with_search(
            title,
            subtitle,
            action_label,
            Arc::new(crate::service::search),
        )
    }

    pub fn with_search(title: &str, subtitle: &str, action_label: &str, search: SearchFn) -> Self {
        Self {
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            action_label: action_label.to_owned(),
            tab: PickerTab::Crypto,
            query: String::new(),
            selected: None,
            results: Vec::new(),
            searched_query: String::new(),
            searched_tab: None,
            inbox: None,
            debounce: None,
            search,
            focus: true,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.inbox.is_some()
    }

    pub fn results(&self) -> &[(DataSource, Result<Vec<SearchResult>, String>)] {
        &self.results
    }

    /// Call after the query or tab changed: a search starts once typing pauses.
    pub fn touch(&mut self) {
        self.debounce = Some(Instant::now());
    }

    pub fn select_tab(&mut self, tab: PickerTab) {
        if self.tab != tab {
            self.tab = tab;
            self.selected = None;
            self.results.clear();
            self.searched_tab = None;
            self.touch();
        }
    }

    /// Whether a search should start now: the query or tab changed and typing has paused.
    pub fn is_due(&self, now: Instant) -> bool {
        self.inbox.is_none()
            && self
                .debounce
                .is_some_and(|since| now.duration_since(since) >= DEBOUNCE)
            && (self.query.trim() != self.searched_query || self.searched_tab != Some(self.tab))
    }

    /// Starts the search for the current query on a worker thread.
    pub fn start(&mut self, ctx: &egui::Context) {
        self.debounce = None;
        let query = self.query.trim().to_owned();
        self.searched_query = query.clone();
        self.searched_tab = Some(self.tab);
        if query.is_empty() {
            self.results.clear();
            return;
        }
        let (sender, receiver) = channel();
        self.inbox = Some(receiver);
        let search = Arc::clone(&self.search);
        let sources = self.tab.sources();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let batch: Batch = std::thread::scope(|scope| {
                let handles: Vec<_> = sources
                    .iter()
                    .map(|source| {
                        let (search, query) = (&search, &query);
                        scope.spawn(move || (*source, search(*source, query)))
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect()
            });
            let _ = sender.send(batch);
            ctx.request_repaint();
        });
    }

    /// Takes a finished search, if any. Results arrive in source priority order.
    pub fn receive(&mut self) {
        let Some(inbox) = &self.inbox else {
            return;
        };
        match inbox.try_recv() {
            Ok(mut batch) => {
                let order = |source: DataSource| {
                    self.tab
                        .sources()
                        .iter()
                        .position(|candidate| *candidate == source)
                        .unwrap_or(usize::MAX)
                };
                batch.sort_by_key(|(source, _)| order(*source));
                let query = self.searched_query.clone();
                self.results = batch
                    .into_iter()
                    .map(|(source, result)| {
                        (
                            source,
                            result.map(|hits| crate::service::rank_results(hits, &query)),
                        )
                    })
                    .collect();
                self.inbox = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.inbox = None,
        }
    }
}

// MARK: Pure text

pub fn hit_summary(hit: &SearchResult) -> String {
    match hit.price {
        Some(price) => format!("{}  ·  {}", hit.symbol, crate::format::price(price)),
        None => hit.symbol.clone(),
    }
}

/// What a source's section says when it has no rows.
pub fn empty_note(result: &Result<Vec<SearchResult>, String>) -> Option<String> {
    match result {
        Err(message) => Some(message.clone()),
        Ok(hits) if hits.is_empty() => Some("No matches".to_owned()),
        Ok(_) => None,
    }
}

// MARK: View

pub fn show(ctx: &egui::Context, state: &mut AssetPickerState) -> Option<AssetPickerOutcome> {
    state.receive();
    if state.is_due(Instant::now()) {
        state.start(ctx);
    }
    if state.debounce.is_some() || state.is_loading() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    let mut outcome = None;
    egui::Window::new(state.title.clone())
        .id(Id::new("portfolio-asset-picker"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(SIZE)
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(&state.title).heading());
            ui.label(RichText::new(&state.subtitle).weak());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for tab in [PickerTab::Crypto, PickerTab::Stocks] {
                    if ui.selectable_label(state.tab == tab, tab.title()).clicked() {
                        state.select_tab(tab);
                    }
                }
                if state.is_loading() {
                    ui.spinner();
                }
            });
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text(state.tab.placeholder())
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut state.focus) {
                response.request_focus();
            }
            if response.changed() {
                state.touch();
            }
            ui.add_space(6.0);
            let list_height = (ui.available_height() - 56.0).max(100.0);
            egui::ScrollArea::vertical()
                .max_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| results(ui, state, &mut outcome));
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    outcome = Some(AssetPickerOutcome::Cancel);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = egui::Button::new(
                        RichText::new(&state.action_label)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(theme::ACCENT);
                    if ui.add_enabled(state.selected.is_some(), button).clicked()
                        && let Some(selected) = state.selected.clone()
                    {
                        outcome = Some(AssetPickerOutcome::Choose(Box::new(selected)));
                    }
                });
            });
        });
    if outcome.is_none() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(AssetPickerOutcome::Cancel);
    }
    outcome
}

fn results(
    ui: &mut egui::Ui,
    state: &mut AssetPickerState,
    outcome: &mut Option<AssetPickerOutcome>,
) {
    if state.query.trim().is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Search for an asset").strong());
            ui.label(RichText::new("Pick the market this should be valued with.").weak());
        });
        return;
    }
    let snapshot: Vec<(DataSource, Result<Vec<SearchResult>, String>)> = state.results.clone();
    for (source, result) in &snapshot {
        ui.add_space(4.0);
        ui.label(RichText::new(source.display_name()).small().strong().weak());
        if let Some(note) = empty_note(result) {
            ui.label(RichText::new(note).small().weak());
            continue;
        }
        if let Ok(hits) = result {
            for hit in hits.iter().take(12) {
                let selected = state.selected.as_ref().is_some_and(|chosen| {
                    chosen.source == hit.source && chosen.full_symbol == hit.full_symbol
                });
                let response = ui.selectable_label(selected, hit_summary(hit));
                if response.clicked() {
                    state.selected = Some(hit.clone());
                }
                if response.double_clicked() {
                    *outcome = Some(AssetPickerOutcome::Choose(Box::new(hit.clone())));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn hit(source: DataSource, symbol: &str, full: &str) -> SearchResult {
        SearchResult {
            symbol: symbol.into(),
            full_symbol: full.into(),
            source,
            price: None,
        }
    }

    #[allow(clippy::type_complexity)]
    fn fake() -> (SearchFn, Arc<Mutex<Vec<(DataSource, String)>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&calls);
        let search: SearchFn = Arc::new(move |source, query| {
            recorded.lock().unwrap().push((source, query.to_owned()));
            match source {
                DataSource::Binance => Ok(vec![
                    hit(source, "BTC/USDT", "BTCUSDT"),
                    hit(source, "WBTC/USDT", "WBTCUSDT"),
                ]),
                DataSource::Coinbase => Ok(vec![]),
                DataSource::CoinGecko => Err("rate limited".to_owned()),
                _ => Ok(vec![hit(source, "BTC/WETH", "0xabc")]),
            }
        });
        (search, calls)
    }

    fn wait(state: &mut AssetPickerState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.is_loading() && Instant::now() < deadline {
            state.receive();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_search_waits_for_typing_to_pause_and_never_repeats_itself() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        let ctx = egui::Context::default();
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(1)),
            "nothing typed yet"
        );
        state.query = "btc".into();
        state.touch();
        let typed = Instant::now();
        assert!(!state.is_due(typed), "still inside the debounce");
        assert!(state.is_due(typed + DEBOUNCE + Duration::from_millis(10)));
        state.start(&ctx);
        assert!(state.is_loading());
        wait(&mut state);
        assert_eq!(
            calls.lock().unwrap().len(),
            4,
            "one request per crypto source"
        );
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(5)),
            "the same query is not searched twice"
        );
        state.query = "btc ".into();
        state.touch();
        assert!(
            !state.is_due(Instant::now() + Duration::from_secs(5)),
            "whitespace is not a new query"
        );
        state.query = "eth".into();
        state.touch();
        assert!(state.is_due(Instant::now() + Duration::from_secs(5)));
    }

    #[test]
    fn results_keep_source_priority_and_are_ranked_within_a_source() {
        let (search, _) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "BTC".into();
        state.start(&egui::Context::default());
        wait(&mut state);
        let sources: Vec<DataSource> = state.results().iter().map(|(source, _)| *source).collect();
        assert_eq!(sources, CRYPTO_SOURCES);
        let binance = state.results()[0].1.as_ref().unwrap();
        assert_eq!(
            binance[0].full_symbol, "BTCUSDT",
            "the exact base ranks first"
        );
        assert_eq!(
            empty_note(&state.results()[1].1).as_deref(),
            Some("No matches")
        );
        assert_eq!(
            empty_note(&state.results()[2].1).as_deref(),
            Some("rate limited")
        );
        assert_eq!(empty_note(&state.results()[0].1), None);
    }

    #[test]
    fn switching_tabs_clears_the_choice_and_searches_the_stock_source() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "aapl".into();
        state.selected = Some(hit(DataSource::Binance, "BTC/USDT", "BTCUSDT"));
        state.select_tab(PickerTab::Stocks);
        assert!(state.selected.is_none() && state.results().is_empty());
        assert!(state.is_due(Instant::now() + Duration::from_secs(1)));
        state.start(&egui::Context::default());
        wait(&mut state);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(DataSource::Alpaca, "aapl".to_owned())]
        );
        state.select_tab(PickerTab::Stocks);
        assert!(
            state.results().len() == 1,
            "re-selecting the same tab changes nothing"
        );
        assert_eq!(PickerTab::Stocks.sources(), STOCK_SOURCES);
    }

    #[test]
    fn an_empty_query_clears_results_without_a_request() {
        let (search, calls) = fake();
        let mut state = AssetPickerState::with_search("Add", "Pick", "Use Asset", search);
        state.query = "  ".into();
        state.touch();
        assert!(state.is_due(Instant::now() + Duration::from_secs(1)));
        state.start(&egui::Context::default());
        assert!(!state.is_loading());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            hit_summary(&hit(DataSource::Binance, "BTC/USDT", "x")),
            "BTC/USDT"
        );
        let mut priced = hit(DataSource::Binance, "BTC/USDT", "x");
        priced.price = Some(67_432.19);
        assert!(hit_summary(&priced).starts_with("BTC/USDT  ·  "));
    }
}
