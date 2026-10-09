//! One-way import of the Swift app's SQLite data into the Rust store.
//!
//! Both apps persist user data as JSON payloads in document tables
//! (`id/position/payload`), but their serde/Codable models differ, so each table
//! needs a small mapping rather than a raw copy. This increment imports `tab` only —
//! the one table with a live Rust consumer. `watchlist`, `saved_view` and `favorite`
//! follow once their Rust stores land.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use super::Store;
use crate::model::{ChartColumn, DataSource, IndicatorConfig, Timeframe};
use crate::workspace::{PersistedChart, PersistedWorkspace};

/// What one `run_once` did, for a one-line launch log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Swift `tab` rows that mapped into a Rust workspace and were written.
    pub imported: usize,
    /// Swift `tab` rows skipped: portfolio tabs, cards with no importable market,
    /// or rows that failed to decode.
    pub skipped: usize,
}

/// Import the Swift app's tabs into `target`, once. Safe to call every launch: it
/// returns an empty summary when the target already has tabs or the Swift database
/// is missing.
pub fn run_once(target: &Store, source: Option<&Path>) -> ImportSummary {
    if !target.documents::<serde_json::Value>("tab").is_empty() {
        return ImportSummary::default();
    }
    let source = source.map(Path::to_path_buf).or_else(swift_db_path);
    let Some(source) = source else {
        return ImportSummary::default();
    };
    let Some(payloads) = read_swift_tabs(&source) else {
        return ImportSummary::default();
    };

    // Continue the app's single u64 id space so imported ids never collide with
    // ones the workspace mints later.
    let mut next_id = target.setting::<u64>("session.next_id").unwrap_or(2);
    let mut workspaces = Vec::new();
    let mut summary = ImportSummary::default();
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
    if workspaces.is_empty() {
        return summary;
    }
    let _ = target.replace_documents("tab", &workspaces);
    let _ = target.set_setting("session.next_id", &next_id);
    summary
}

// MARK: Mapping

fn map_tab(tab: SwiftChartTab, next_id: &mut u64) -> Option<PersistedWorkspace> {
    if tab.kind.as_deref() == Some("portfolio") {
        return None;
    }

    let mut charts = Vec::new();
    let mut id_map: HashMap<String, u64> = HashMap::new();
    for config in tab.ticker_configs {
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
        });
    }
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

// MARK: Source read

/// The Swift app's on-disk store, opened read-only. None when it is missing or
/// cannot be opened (best-effort: a missing Swift DB simply skips the import).
fn read_swift_tabs(path: &Path) -> Option<Vec<String>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let mut statement = connection
        .prepare("SELECT payload FROM tab ORDER BY position")
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
        // Non-empty target: gated off regardless of a source.
        assert_eq!(run_once(&store, None), ImportSummary::default());

        // Empty target + missing source: no-op, no panic.
        let empty = Store::open_in_memory().expect("store");
        let missing = std::env::temp_dir().join(format!("dv_nope_{}.sqlite", std::process::id()));
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
                skipped: 1
            }
        );
        let tabs: Vec<PersistedWorkspace> = store.documents("tab");
        assert!(tabs.is_empty());
    }
}
