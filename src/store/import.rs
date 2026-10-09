//! One-way import of the Swift app's SQLite data into the Rust store.
//!
//! Both apps persist user data as JSON payloads in document tables
//! (`id/position/payload`), but their serde/Codable models differ, so each table
//! needs a small mapping rather than a raw copy. `tab` and `watchlist` map (the
//! legacy `favorite` rows fold into the Favorites list). `saved_view` is deferred
//! (v3): the Rust app has no `SavedView` model yet, and a raw copy would decode
//! wrong under the future model (Swift `Date` + field-name mismatch).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use super::Store;
use crate::model::watchlist::{
    Watchlist, WatchlistDisplaySettings, WatchlistEntry, WatchlistInstrument, WatchlistSection,
    new_id, now_ms,
};
use crate::model::{ChartColumn, DataSource, IndicatorConfig, InstrumentId, SavedView, Timeframe};
use crate::workspace::{PersistedChart, PersistedWorkspace};

/// What one `run_once` did, for a one-line launch log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Swift `tab` rows that mapped into a Rust workspace and were written.
    pub imported: usize,
    /// Swift `tab` rows skipped: portfolio tabs, cards with no importable market,
    /// or rows that failed to decode.
    pub skipped: usize,
    /// Swift `watchlist` rows that mapped into a Rust watchlist.
    pub watchlists: usize,
    /// Swift `watchlist` rows skipped (failed to decode).
    pub watchlist_skipped: usize,
    /// 1 when the legacy `favorite` rows were folded into a Favorites list.
    pub favorites: usize,
    /// Swift `saved_view` rows that mapped into a Rust saved layout.
    pub saved_views: usize,
    /// Swift `saved_view` rows skipped (failed to decode).
    pub saved_view_skipped: usize,
}

/// Import the Swift app's data into `target`, once. Safe to call every launch: each
/// table is gated on its Rust counterpart being empty, and a missing Swift database
/// simply skips everything.
pub fn run_once(target: &Store, source: Option<&Path>) -> ImportSummary {
    let source = source.map(Path::to_path_buf).or_else(swift_db_path);
    let Some(source) = source else {
        return ImportSummary::default();
    };
    let mut summary = ImportSummary::default();
    // Continue the app's single u64 id space so imported ids never collide with ones
    // the workspace mints later.
    let mut next_id = target.setting::<u64>("session.next_id").unwrap_or(2);

    // Tabs.
    if target.documents::<serde_json::Value>("tab").is_empty()
        && let Some(payloads) = read_swift_documents(&source, "tab")
    {
        let mut workspaces = Vec::new();
        for payload in payloads {
            let Ok(tab) = serde_json::from_str::<SwiftChartTab>(&payload) else {
                summary.skipped += 1;
                continue;
            };
            match map_tab(tab, &mut next_id) {
                Some(workspace) => {
                    workspaces.push(workspace);
                    summary.imported += 1;
                }
                None => summary.skipped += 1,
            }
        }
        if !workspaces.is_empty() {
            let _ = target.replace_documents("tab", &workspaces);
        }
    }

    // Saved layouts.
    if target
        .documents::<serde_json::Value>("saved_view")
        .is_empty()
        && let Some(payloads) = read_swift_documents(&source, "saved_view")
    {
        let mut views = Vec::new();
        for payload in payloads {
            match serde_json::from_str::<SwiftSavedView>(&payload)
                .map(|view| map_saved_view(view, &mut next_id))
            {
                Ok(view) => {
                    views.push(view);
                    summary.saved_views += 1;
                }
                Err(_) => summary.saved_view_skipped += 1,
            }
        }
        if !views.is_empty() {
            let _ = target.replace_documents("saved_view", &views);
        }
    }

    // Watchlists. The legacy `favorite` rows fold into a Favorites list only when no
    // imported watchlist already carries `is_favorites` (the Swift app left the old
    // `favorite` rows in place after migrating them into `watchlist`).
    if target
        .documents::<serde_json::Value>("watchlist")
        .is_empty()
    {
        let mut lists: Vec<Watchlist> = Vec::new();
        if let Some(payloads) = read_swift_documents(&source, "watchlist") {
            for payload in payloads {
                match serde_json::from_str::<SwiftWatchlist>(&payload).map(map_watchlist) {
                    Ok(list) => {
                        lists.push(list);
                        summary.watchlists += 1;
                    }
                    Err(_) => summary.watchlist_skipped += 1,
                }
            }
        }
        if !lists.iter().any(|list| list.is_favorites)
            && let Some(payloads) = read_swift_documents(&source, "favorite")
        {
            let favorites: Vec<SwiftLegacyFavorite> = payloads
                .into_iter()
                .filter_map(|payload| serde_json::from_str(&payload).ok())
                .collect();
            if let Some(list) = map_favorites(favorites) {
                lists.push(list);
                summary.favorites += 1;
            }
        }
        if !lists.is_empty() {
            let _ = target.replace_documents("watchlist", &lists);
        }
    }

    if summary.imported > 0 || summary.saved_views > 0 {
        let _ = target.set_setting("session.next_id", &next_id);
    }
    summary
}

// MARK: Mapping

fn map_tab(tab: SwiftChartTab, next_id: &mut u64) -> Option<PersistedWorkspace> {
    if tab.kind.as_deref() == Some("portfolio") {
        return None;
    }

    let (charts, id_map) = map_charts(tab.ticker_configs, next_id);
    if charts.is_empty() {
        return None;
    }

    let columns = tab
        .chart_columns
        .unwrap_or_default()
        .into_iter()
        .filter_map(|column| {
            let chart_ids: Vec<u64> = column
                .chart_ids
                .iter()
                .filter_map(|uuid| id_map.get(uuid).copied())
                .collect();
            if chart_ids.is_empty() {
                None
            } else {
                Some(ChartColumn::new(mint(next_id), chart_ids))
            }
        })
        .collect();

    Some(PersistedWorkspace {
        id: mint(next_id),
        timeframe: map_timeframe(&tab.time_range),
        watchlist_visible: true,
        charts,
        columns,
        focused: None,
    })
}

/// Maps Swift `tickerConfigs` into Rust charts (minting ids), plus the chart-id remap for columns.
fn map_charts(
    configs: Vec<SwiftTickerConfig>,
    next_id: &mut u64,
) -> (Vec<PersistedChart>, HashMap<String, u64>) {
    let mut charts = Vec::new();
    let mut id_map: HashMap<String, u64> = HashMap::new();
    for config in configs {
        if config.portfolio_chart.is_some() || config.coin_market_cap_chart.is_some() {
            continue;
        }
        let Some(source) = map_source(&config.source) else {
            continue;
        };
        let id = mint(next_id);
        id_map.insert(config.chart_id, id);

        let mut indicators = IndicatorConfig::default();
        indicators.show_volume = config.show_volume.unwrap_or(false);
        indicators.show_rsi = config.show_rsi.unwrap_or(false);
        indicators.show_ema = config.show_ema.unwrap_or(false);
        indicators.ema_period = config.ema_period.unwrap_or(indicators.ema_period);
        indicators.show_bollinger = config.show_bollinger.unwrap_or(false);
        indicators.show_trend_flips = config.show_trend_flips.unwrap_or(false);

        charts.push(PersistedChart {
            id,
            symbol: config.symbol,
            source,
            log_scale: false,
            y_zoom: config.y_zoom.unwrap_or(1.0),
            indicators,
            drawings: Vec::new(),
            pine_source: None,
            candle_count: 0,
        });
    }
    (charts, id_map)
}

/// Maps a Swift `SavedView` into the Rust saved layout. `tickers` is redundant with each chart's
/// symbol, so only `tickerConfigs` are mapped.
fn map_saved_view(swift: SwiftSavedView, next_id: &mut u64) -> SavedView {
    let (charts, id_map) = map_charts(swift.ticker_configs, next_id);
    let columns = swift.chart_columns.map(|columns| {
        columns
            .into_iter()
            .filter_map(|column| {
                let chart_ids: Vec<u64> = column
                    .chart_ids
                    .iter()
                    .filter_map(|uuid| id_map.get(uuid).copied())
                    .collect();
                if chart_ids.is_empty() {
                    None
                } else {
                    Some(ChartColumn::new(mint(next_id), chart_ids))
                }
            })
            .collect()
    });
    SavedView {
        id: mint(next_id),
        name: swift.name,
        timeframe: map_timeframe(&swift.time_range),
        created_at_ms: swift
            .created_at
            .map(swift_date_to_ms)
            .unwrap_or_else(now_ms),
        charts,
        columns,
        candle_count: swift.candle_count,
        last_opened_at_ms: swift.last_opened_at.map(swift_date_to_ms),
        autosave: swift.autosave,
    }
}

fn map_source(raw: &str) -> Option<DataSource> {
    match raw {
        "Binance" => Some(DataSource::Binance),
        "Coinbase" => Some(DataSource::Coinbase),
        "CoinGecko" => Some(DataSource::CoinGecko),
        "DEXScreener" => Some(DataSource::DexScreener),
        "Alpaca (IEX)" => Some(DataSource::Alpaca),
        "Polymarket" => Some(DataSource::Polymarket),
        "Kalshi" => Some(DataSource::Kalshi),
        "CoinMarketCap" => Some(DataSource::CoinMarketCap),
        _ => None,
    }
}

fn map_timeframe(raw: &str) -> Timeframe {
    match raw {
        "1H" => Timeframe::Hour,
        "1D" => Timeframe::Day,
        "1W" => Timeframe::Week,
        "1M" => Timeframe::Month,
        "3M" => Timeframe::Quarter,
        "1Y" => Timeframe::Year,
        _ => Timeframe::Day,
    }
}

fn mint(next_id: &mut u64) -> u64 {
    let id = *next_id;
    *next_id += 1;
    id
}

// MARK: Watchlist and favorites mapping

/// Swift encodes `Date` as seconds since 2001-01-01; the Rust model keeps epoch ms.
fn swift_date_to_ms(seconds: f64) -> i64 {
    ((seconds + 978_307_200.0) * 1000.0).round() as i64
}

fn map_watchlist(swift: SwiftWatchlist) -> Watchlist {
    let entries: Vec<WatchlistEntry> = swift.entries.into_iter().filter_map(map_entry).collect();
    let created_at = swift
        .created_at
        .map(swift_date_to_ms)
        .unwrap_or_else(now_ms);
    let updated_at = swift.updated_at.map(swift_date_to_ms).unwrap_or(created_at);
    Watchlist {
        id: new_id(),
        name: swift.name,
        entries,
        display: WatchlistDisplaySettings::default(),
        is_favorites: swift.is_favorites,
        created_at_ms: created_at,
        updated_at_ms: updated_at,
    }
}

fn map_entry(swift: SwiftWatchlistEntry) -> Option<WatchlistEntry> {
    match swift.kind.as_str() {
        "instrument" => swift
            .instrument
            .and_then(map_instrument)
            .map(WatchlistEntry::Instrument),
        "section" => swift.section.map(|section| {
            WatchlistEntry::Section(WatchlistSection {
                id: new_id(),
                title: section.title,
                is_collapsed: section.is_collapsed,
            })
        }),
        _ => None,
    }
}

fn map_instrument(swift: SwiftWatchlistInstrument) -> Option<WatchlistInstrument> {
    let source = map_source(&swift.instrument.source)?;
    if source == DataSource::CoinMarketCap {
        return None;
    }
    Some(WatchlistInstrument {
        id: new_id(),
        instrument: InstrumentId::new(source, swift.instrument.symbol),
        name: swift.name,
        label: swift.label,
        display_name: swift.display_name,
    })
}

fn map_favorite(swift: SwiftLegacyFavorite) -> Option<WatchlistInstrument> {
    let source = map_source(&swift.config.source)?;
    if source == DataSource::CoinMarketCap {
        return None;
    }
    Some(WatchlistInstrument {
        id: new_id(),
        instrument: InstrumentId::new(source, swift.config.symbol),
        name: swift.name,
        label: swift.ticker,
        display_name: swift.config.display_name,
    })
}

/// The legacy `favorite` rows as one Favorites list, deduped by market, or None when empty.
fn map_favorites(favorites: Vec<SwiftLegacyFavorite>) -> Option<Watchlist> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for favorite in favorites {
        let Some(item) = map_favorite(favorite) else {
            continue;
        };
        if !seen.insert(item.instrument.key()) {
            continue;
        }
        entries.push(WatchlistEntry::Instrument(item));
    }
    if entries.is_empty() {
        return None;
    }
    let now = now_ms();
    Some(Watchlist {
        id: new_id(),
        name: "Favorites".to_owned(),
        entries,
        display: WatchlistDisplaySettings::default(),
        is_favorites: true,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// Mirror of the Swift `Watchlist` JSON. `id`, `display` and `pmSeries` are dropped:
/// the Rust model mints its own ids, and per-list columns/sort are re-set.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftWatchlist {
    name: String,
    #[serde(default)]
    entries: Vec<SwiftWatchlistEntry>,
    #[serde(default)]
    is_favorites: bool,
    #[serde(default)]
    created_at: Option<f64>,
    #[serde(default)]
    updated_at: Option<f64>,
}

#[derive(Deserialize)]
struct SwiftWatchlistEntry {
    kind: String,
    #[serde(default)]
    instrument: Option<SwiftWatchlistInstrument>,
    #[serde(default)]
    section: Option<SwiftWatchlistSection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftWatchlistInstrument {
    instrument: SwiftInstrumentId,
    name: String,
    label: String,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftWatchlistSection {
    title: String,
    #[serde(default)]
    is_collapsed: bool,
}

#[derive(Deserialize)]
struct SwiftInstrumentId {
    source: String,
    symbol: String,
}

/// Mirror of the Swift `LegacyFavorite` JSON (`WatchlistStore+FavoritesMigration`).
#[derive(Deserialize)]
struct SwiftLegacyFavorite {
    name: String,
    ticker: String,
    config: SwiftFavoriteConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftFavoriteConfig {
    symbol: String,
    source: String,
    #[serde(default)]
    display_name: Option<String>,
}

// MARK: Swift payloads

/// Mirror of the Swift `ChartTab` JSON, trimmed to the fields the import consumes.
/// Unknown fields (`name`, `savedViewID`, `candleCount`, `replaySession`) are ignored.
#[derive(Deserialize)]
struct SwiftChartTab {
    #[serde(default, rename = "tickerConfigs")]
    ticker_configs: Vec<SwiftTickerConfig>,
    #[serde(default, rename = "chartColumns")]
    chart_columns: Option<Vec<SwiftChartColumn>>,
    #[serde(default = "one_day", rename = "timeRange")]
    time_range: String,
    #[serde(default)]
    kind: Option<String>,
}

fn one_day() -> String {
    "1D".to_owned()
}

#[derive(Deserialize)]
struct SwiftTickerConfig {
    #[serde(default, rename = "chartID")]
    chart_id: String,
    symbol: String,
    source: String,
    #[serde(default, rename = "yZoom")]
    y_zoom: Option<f64>,
    #[serde(default, rename = "showVolume")]
    show_volume: Option<bool>,
    #[serde(default, rename = "showRSI")]
    show_rsi: Option<bool>,
    #[serde(default, rename = "showEMA")]
    show_ema: Option<bool>,
    #[serde(default, rename = "emaPeriod")]
    ema_period: Option<usize>,
    #[serde(default, rename = "showBollinger")]
    show_bollinger: Option<bool>,
    #[serde(default, rename = "showTrendFlips")]
    show_trend_flips: Option<bool>,
    #[serde(default, rename = "portfolioChart")]
    portfolio_chart: Option<serde_json::Value>,
    #[serde(default, rename = "coinMarketCapChart")]
    coin_market_cap_chart: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct SwiftChartColumn {
    #[serde(default, rename = "chartIDs")]
    chart_ids: Vec<String>,
}

/// Mirror of the Swift `SavedView` JSON. `id` and `tickers` are dropped: a fresh id is minted and
/// each chart carries its own symbol.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwiftSavedView {
    name: String,
    #[serde(default = "one_day")]
    time_range: String,
    #[serde(default)]
    created_at: Option<f64>,
    #[serde(default)]
    ticker_configs: Vec<SwiftTickerConfig>,
    #[serde(default)]
    chart_columns: Option<Vec<SwiftChartColumn>>,
    #[serde(default)]
    candle_count: i64,
    #[serde(default)]
    last_opened_at: Option<f64>,
    #[serde(default)]
    autosave: Option<bool>,
}

// MARK: Source read

/// The Swift app's on-disk store, opened read-only. None when it is missing or
/// cannot be opened (best-effort: a missing Swift DB simply skips the import).
fn read_swift_documents(path: &Path, table: &str) -> Option<Vec<String>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut statement = connection
        .prepare(&format!("SELECT payload FROM {table} ORDER BY position"))
        .ok()?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?;
    Some(rows.filter_map(Result::ok).collect())
}

fn swift_db_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/DegenView/degenview.sqlite"))
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn config(source: &str, symbol: &str, chart_id: &str) -> SwiftTickerConfig {
        SwiftTickerConfig {
            chart_id: chart_id.to_owned(),
            symbol: symbol.to_owned(),
            source: source.to_owned(),
            y_zoom: None,
            show_volume: None,
            show_rsi: None,
            show_ema: None,
            ema_period: None,
            show_bollinger: None,
            show_trend_flips: None,
            portfolio_chart: None,
            coin_market_cap_chart: None,
        }
    }

    #[test]
    fn source_and_timeframe_map_one_to_one() {
        assert_eq!(map_source("Binance"), Some(DataSource::Binance));
        assert_eq!(map_source("Alpaca (IEX)"), Some(DataSource::Alpaca));
        assert_eq!(map_source("CoinMarketCap"), Some(DataSource::CoinMarketCap));
        assert_eq!(map_source("Unknown"), None);
        assert_eq!(map_timeframe("3M"), Timeframe::Quarter);
        assert_eq!(map_timeframe("1Y"), Timeframe::Year);
        assert_eq!(map_timeframe("bogus"), Timeframe::Day);
    }

    #[test]
    fn full_tab_maps_indicators_and_columns() {
        let mut next_id = 2;
        let tab = SwiftChartTab {
            ticker_configs: vec![
                SwiftTickerConfig {
                    show_volume: Some(true),
                    show_rsi: Some(true),
                    show_ema: Some(true),
                    ema_period: Some(50),
                    ..config("Binance", "BTCUSDT", "aaaa")
                },
                config("Coinbase", "BTC-USD", "bbbb"),
            ],
            chart_columns: Some(vec![
                SwiftChartColumn {
                    chart_ids: vec!["aaaa".into()],
                },
                SwiftChartColumn {
                    chart_ids: vec!["bbbb".into()],
                },
            ]),
            time_range: "1D".into(),
            kind: Some("charts".into()),
        };

        let workspace = map_tab(tab, &mut next_id).expect("maps");

        assert_eq!(workspace.timeframe, Timeframe::Day);
        assert_eq!(workspace.charts.len(), 2);
        assert_eq!(workspace.charts[0].symbol, "BTCUSDT");
        assert_eq!(workspace.charts[0].source, DataSource::Binance);
        assert_eq!(workspace.charts[0].y_zoom, 1.0);
        assert!(workspace.charts[0].indicators.show_volume);
        assert!(workspace.charts[0].indicators.show_rsi);
        assert!(workspace.charts[0].indicators.show_ema);
        assert_eq!(workspace.charts[0].indicators.ema_period, 50);
        assert_eq!(workspace.charts[1].source, DataSource::Coinbase);
        assert!(!workspace.charts[1].indicators.show_ema);

        assert_eq!(workspace.columns.len(), 2);
        // Column membership is remapped to the newly minted chart ids.
        assert_eq!(workspace.columns[0].chart_ids, vec![workspace.charts[0].id]);
        assert_eq!(workspace.columns[1].chart_ids, vec![workspace.charts[1].id]);
        // Everything minted from the shared id space, strictly increasing.
        let mut ids = vec![workspace.id];
        ids.extend(workspace.charts.iter().map(|chart| chart.id));
        ids.extend(workspace.columns.iter().map(|column| column.id));
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len());
        assert_eq!(next_id, 2 + ids.len() as u64);
    }

    #[test]
    fn portfolio_tab_and_cards_are_skipped() {
        let mut next_id = 2;
        let portfolio_tab = SwiftChartTab {
            ticker_configs: vec![config("Binance", "BTCUSDT", "aaaa")],
            chart_columns: None,
            time_range: "1D".into(),
            kind: Some("portfolio".into()),
        };
        assert!(map_tab(portfolio_tab, &mut next_id).is_none());

        let cid = |source: &str| SwiftTickerConfig {
            portfolio_chart: Some(serde_json::json!({ "accountId": "x" })),
            coin_market_cap_chart: None,
            ..config(source, "X", "cid")
        };
        let tab = SwiftChartTab {
            ticker_configs: vec![cid("Binance")],
            chart_columns: None,
            time_range: "1D".into(),
            kind: Some("charts".into()),
        };
        assert!(map_tab(tab, &mut next_id).is_none());
    }

    fn write_swift_db(path: &Path, payloads: &[&str]) {
        let connection = Connection::open(path).expect("open swift db");
        connection
            .execute_batch(
                "CREATE TABLE tab (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );",
            )
            .expect("create tab");
        for (index, payload) in payloads.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO tab (id, position, payload) VALUES (?1, ?2, ?3)",
                    rusqlite::params![format!("id-{index}"), index as i64, payload],
                )
                .expect("insert");
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dv_import_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn run_once_imports_into_an_empty_store() {
        let payload = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "tickerConfigs": [
                { "chartID": "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA",
                  "symbol": "BTCUSDT", "source": "Binance" }
            ],
            "chartColumns": null,
            "timeRange": "1W",
            "kind": "charts"
        })
        .to_string();
        let dir = temp_dir("run_once");
        let swift = dir.join("swift.sqlite");
        write_swift_db(&swift, &[&payload]);

        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped, 0);

        let tabs: Vec<PersistedWorkspace> = store.documents("tab");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].timeframe, Timeframe::Week);
        assert_eq!(tabs[0].charts[0].symbol, "BTCUSDT");
        assert!(store.setting::<u64>("session.next_id").unwrap_or(0) > 2);
    }

    #[test]
    fn run_once_is_a_no_op_when_tabs_exist_or_source_is_missing() {
        let missing = std::env::temp_dir().join(format!("dv_nope_{}.sqlite", std::process::id()));
        let store = Store::open_in_memory().expect("store");
        store
            .replace_documents(
                "tab",
                &[crate::workspace::PersistedWorkspace {
                    id: 1,
                    timeframe: Timeframe::Day,
                    watchlist_visible: true,
                    charts: Vec::new(),
                    columns: Vec::new(),
                    focused: None,
                }],
            )
            .expect("seed");
        // Non-empty target: gated off regardless of a source (which is also absent).
        assert_eq!(run_once(&store, Some(&missing)), ImportSummary::default());

        // Empty target + missing source: no-op, no panic.
        let empty = Store::open_in_memory().expect("store");
        assert_eq!(run_once(&empty, Some(&missing)), ImportSummary::default());
    }

    #[test]
    fn undecodable_rows_are_skipped_not_fatal() {
        let dir = temp_dir("bad_row");
        let swift = dir.join("swift.sqlite");
        write_swift_db(&swift, &["not json"]);

        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        assert_eq!(
            summary,
            ImportSummary {
                imported: 0,
                skipped: 1,
                ..ImportSummary::default()
            }
        );
        let tabs: Vec<PersistedWorkspace> = store.documents("tab");
        assert!(tabs.is_empty());
    }

    fn write_watchlist_tables(path: &Path, watchlists: &[String], favorites: &[String]) {
        let connection = Connection::open(path).expect("open swift db");
        connection
            .execute_batch(
                "CREATE TABLE watchlist (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );
                CREATE TABLE favorite (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );",
            )
            .expect("create tables");
        for (index, payload) in watchlists.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO watchlist (id, position, payload) VALUES (?1, ?2, ?3)",
                    rusqlite::params![format!("w-{index}"), index as i64, payload],
                )
                .expect("insert watchlist");
        }
        for (index, payload) in favorites.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO favorite (id, position, payload) VALUES (?1, ?2, ?3)",
                    rusqlite::params![format!("f-{index}"), index as i64, payload],
                )
                .expect("insert favorite");
        }
    }

    #[test]
    fn watchlist_and_favorites_import() {
        let watchlist = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "name": "Crypto",
            "entries": [
                { "kind": "section",
                  "section": { "id": "22222222-2222-2222-2222-222222222222", "title": "Majors", "isCollapsed": false } },
                { "kind": "instrument",
                  "instrument": { "id": "33333333-3333-3333-3333-333333333333",
                                 "instrument": { "source": "Binance", "symbol": "BTCUSDT" },
                                 "name": "Bitcoin", "label": "BTC" } }
            ],
            "isFavorites": false,
            "createdAt": 757_382_400.0,
            "updatedAt": 757_382_400.0
        })
        .to_string();
        let favorite = serde_json::json!({
            "id": "44444444-4444-4444-4444-444444444444",
            "name": "Ethereum",
            "ticker": "ETH",
            "config": { "symbol": "ETHUSDT", "source": "Binance" }
        })
        .to_string();

        let dir = temp_dir("watchlist");
        let swift = dir.join("swift.sqlite");
        write_watchlist_tables(&swift, &[watchlist], &[favorite]);

        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        assert_eq!(summary.watchlists, 1);
        assert_eq!(summary.favorites, 1);

        let lists: Vec<Watchlist> = store.documents("watchlist");
        assert_eq!(lists.len(), 2);
        let crypto = lists.iter().find(|list| !list.is_favorites).unwrap();
        assert_eq!(crypto.name, "Crypto");
        assert_eq!(crypto.entries.len(), 2);
        assert!(matches!(&crypto.entries[0], WatchlistEntry::Section(s) if s.title == "Majors"));
        assert!(
            matches!(&crypto.entries[1], WatchlistEntry::Instrument(i) if i.instrument.key() == "BINANCE:BTCUSDT")
        );
        let favorites = lists.iter().find(|list| list.is_favorites).unwrap();
        assert_eq!(favorites.name, "Favorites");
        assert_eq!(favorites.entries.len(), 1);
        assert!(matches!(&favorites.entries[0], WatchlistEntry::Instrument(i) if i.label == "ETH"));
    }

    #[test]
    fn a_coinmarketcap_favorite_is_skipped_and_an_empty_favorite_table_makes_no_list() {
        let favorite = serde_json::json!({
            "id": "44444444-4444-4444-4444-444444444444",
            "name": "CMC index",
            "ticker": "CMC",
            "config": { "symbol": "BTC", "source": "CoinMarketCap" }
        })
        .to_string();
        let dir = temp_dir("cmc_favorite");
        let swift = dir.join("swift.sqlite");
        write_watchlist_tables(&swift, &[], &[favorite]);

        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        // The CMC favorite is not a watchlist instrument, so no Favorites list is made.
        assert_eq!(summary.favorites, 0);
        let lists: Vec<Watchlist> = store.documents("watchlist");
        assert!(lists.is_empty());
    }

    fn write_saved_view_table(path: &Path, payloads: &[String]) {
        let connection = Connection::open(path).expect("open swift db");
        connection
            .execute_batch(
                "CREATE TABLE saved_view (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );",
            )
            .expect("create saved_view");
        for (index, payload) in payloads.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO saved_view (id, position, payload) VALUES (?1, ?2, ?3)",
                    rusqlite::params![format!("v-{index}"), index as i64, payload],
                )
                .expect("insert saved_view");
        }
    }

    #[test]
    fn a_saved_view_imports_as_a_layout() {
        let view = serde_json::json!({
            "id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "name": "BTC + ETH",
            "tickers": ["BTCUSDT", "ETHUSDT"],
            "timeRange": "1H",
            "createdAt": 757_382_400.0,
            "tickerConfigs": [
                { "chartID": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "symbol": "BTCUSDT", "source": "Binance" },
                { "chartID": "cccccccc-cccc-cccc-cccc-cccccccccccc", "symbol": "ETHUSDT", "source": "Binance" }
            ],
            "chartColumns": [ { "chartIDs": ["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "cccccccc-cccc-cccc-cccc-cccccccccccc"] } ],
            "candleCount": 200,
            "lastOpenedAt": 757_382_400.0,
            "autosave": true
        })
        .to_string();
        let dir = temp_dir("saved_view");
        let swift = dir.join("swift.sqlite");
        write_saved_view_table(&swift, &[view]);

        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        assert_eq!(summary.saved_views, 1);

        let views = store.saved_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "BTC + ETH");
        assert_eq!(views[0].timeframe, Timeframe::Hour);
        assert_eq!(views[0].charts.len(), 2);
        assert_eq!(views[0].charts[0].symbol, "BTCUSDT");
        assert_eq!(views[0].candle_count, 200);
        assert_eq!(views[0].autosave, Some(true));
        let columns = views[0].columns.as_ref().unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].chart_ids.len(), 2);
    }

    #[test]
    fn an_undecodable_saved_view_row_is_skipped() {
        let dir = temp_dir("saved_view_bad");
        let swift = dir.join("swift.sqlite");
        write_saved_view_table(&swift, &["not json".into()]);
        let store = Store::open_in_memory().expect("store");
        let summary = run_once(&store, Some(&swift));
        assert_eq!(summary.saved_views, 0);
        assert_eq!(summary.saved_view_skipped, 1);
        assert!(store.saved_views().is_empty());
    }
}
