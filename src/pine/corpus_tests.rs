//! Runs cached community TradingView scripts through the engine and diffs the outcome against
//! `src/pine/corpus/expectations.json`. A port of the Swift `PineCorpusTests`.
//!
//! The sources are third-party and never committed: `tools/pine-corpus/fetch.py` caches them in
//! `.pine-corpus/` (gitignored), and the test skips when that cache is missing. Point at another
//! location with `PINE_CORPUS_DIR=/path`; run just a few scripts with `PINE_CORPUS_ONLY=slug,slug`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::Kline;
use crate::pine::compiler::{self, LibraryResolver};
use crate::pine::program::SymbolInfo;
use crate::pine::runtime_support::{SecurityDataProvider, SecurityKey};
use crate::pine::session::{RunError, Session};
use crate::pine::token::{Diagnostic, DiagnosticSeverity, Limits};

#[derive(Deserialize)]
struct ManifestEntry {
    slug: String,
    name: String,
    kind: String,
    status: String,
}

#[derive(Clone, Deserialize, PartialEq)]
struct Expectation {
    #[serde(default = "default_ok")]
    status: String,
    #[serde(default)]
    codes: Vec<String>,
}

fn default_ok() -> String {
    "ok".to_owned()
}

/// Resolves `import` from the libraries `fetch.py --imports` cached beside the scripts.
struct CachedLibraries {
    directory: PathBuf,
}

impl LibraryResolver for CachedLibraries {
    fn source_for_library(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.directory.join(format!("{path}.pine"))).ok()
    }
}

/// A deterministic random walk per symbol, like a data feed with more history than the chart shows.
struct SyntheticSecurityData {
    range_start_ms: i64,
    range_end_ms: i64,
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl SecurityDataProvider for SyntheticSecurityData {
    fn candles(&self, key: &SecurityKey) -> Option<Vec<Kline>> {
        const HISTORY: i64 = 500;
        let interval_ms = (key.interval * 1000.0) as i64;
        if interval_ms <= 0 {
            return None;
        }
        let mut state = fnv(key.symbol.as_bytes());
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut open_time = self.range_start_ms - HISTORY * interval_ms;
        let mut close = 100.0;
        let mut bars = Vec::new();
        while open_time <= self.range_end_ms {
            let open = close;
            close = (open * (1.0 + (next() - 0.5) * 0.04)).max(1.0);
            bars.push(Kline {
                open_time,
                close_time: open_time + interval_ms,
                open,
                high: open.max(close) * (1.0 + next() * 0.01),
                low: open.min(close) * (1.0 - next() * 0.01),
                close,
                volume: 100.0 + next() * 900.0,
            });
            open_time += interval_ms;
        }
        Some(bars)
    }
}

/// A deterministic random walk (fixed-seed LCG), so a failure reproduces on every machine.
fn synthetic_bars(count: usize) -> Vec<Kline> {
    const SPACING_MS: i64 = 60_000;
    const START_MS: i64 = 1_700_000_000_000;
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut close = 100.0;
    (0..count)
        .map(|index| {
            let open = close;
            close = (open * (1.0 + (next() - 0.5) * 0.04)).max(1.0);
            Kline {
                open_time: START_MS + index as i64 * SPACING_MS,
                close_time: START_MS + (index as i64 + 1) * SPACING_MS,
                open,
                high: open.max(close) * (1.0 + next() * 0.01),
                low: open.min(close) * (1.0 - next() * 0.01),
                close,
                volume: 100.0 + next() * 900.0,
            }
        })
        .collect()
}

fn codes(diagnostics: &[&Diagnostic]) -> Vec<String> {
    let mut codes: Vec<String> = diagnostics.iter().map(|d| d.code.clone()).collect();
    codes.sort();
    codes.dedup();
    codes
}

/// Compiles `source` and, when it is valid, runs it over `bars` with default inputs.
fn run(source: &str, bars: &[Kline], libraries: &dyn LibraryResolver) -> (String, Vec<String>) {
    let program = compiler::compile(source, Limits::default(), libraries);
    let compile_errors: Vec<&Diagnostic> = program
        .diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    if !compile_errors.is_empty() {
        return ("compile-error".to_owned(), codes(&compile_errors));
    }
    if program.statements.is_empty() {
        return ("ok".to_owned(), Vec::new());
    }
    let symbol = SymbolInfo {
        ticker: "BTCUSDT".to_owned(),
        ticker_id: "BINANCE:BTCUSDT".to_owned(),
        currency: "USD".to_owned(),
        kind: "crypto".to_owned(),
        base_currency: None,
    };
    let security = SyntheticSecurityData {
        range_start_ms: bars[0].open_time,
        range_end_ms: bars.last().map_or(bars[0].open_time, |bar| bar.open_time),
    };
    let mut session = Session::new(
        program,
        HashMap::new(),
        Limits::default(),
        None,
        symbol,
        Some(Box::new(security)),
        None,
    );
    match session.evaluate(bars) {
        Ok(_) => ("ok".to_owned(), Vec::new()),
        Err(RunError::Script(diagnostic)) => ("runtime-error".to_owned(), vec![diagnostic.code]),
        Err(RunError::Rejection(_)) => ("ok".to_owned(), Vec::new()),
    }
}

#[test]
fn corpus_matches_expectations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus_dir = root.join("src/pine/corpus");
    let cache_dir = std::env::var_os("PINE_CORPUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".pine-corpus"));
    let expectations: HashMap<String, Expectation> = serde_json::from_str(
        &std::fs::read_to_string(corpus_dir.join("expectations.json")).unwrap(),
    )
    .unwrap();
    let only: Option<HashSet<String>> = std::env::var("PINE_CORPUS_ONLY")
        .ok()
        .map(|value| value.split(',').map(str::to_owned).collect());
    let bars = synthetic_bars(1_000);
    let libraries = CachedLibraries {
        directory: cache_dir.join("imports"),
    };

    // Committed original scripts always run, so the harness diffs non-trivial output without the
    // network. `scripts/<kind>/<slug>.pine`; the slug is the file stem.
    let scripts_dir = corpus_dir.join("scripts");
    for kind in ["indicator", "strategy", "library"] {
        let Ok(entries) = std::fs::read_dir(scripts_dir.join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("pine") {
                continue;
            }
            let Some(slug) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if only.as_ref().is_some_and(|set| !set.contains(&slug)) {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (status, codes) = run(&source, &bars, &libraries);
            let expected = expectations.get(&slug).cloned().unwrap_or(Expectation {
                status: "ok".to_owned(),
                codes: Vec::new(),
            });
            assert_eq!(status, expected.status, "{slug} status differs");
            assert_eq!(codes, expected.codes, "{slug} codes differ");
        }
    }

    // Community scripts, fetched by `tools/pine-corpus/fetch.py` into the gitignored cache.
    if !cache_dir.is_dir() {
        eprintln!(
            "skipping community corpus: no cache at {} (run tools/pine-corpus/fetch.py)",
            cache_dir.display()
        );
        return;
    }
    let manifest: Vec<ManifestEntry> =
        serde_json::from_str(&std::fs::read_to_string(corpus_dir.join("manifest.json")).unwrap())
            .unwrap();
    let fetched: Vec<&ManifestEntry> = manifest
        .iter()
        .filter(|entry| {
            entry.status == "fetched" && only.as_ref().is_none_or(|set| set.contains(&entry.slug))
        })
        .collect();
    for entry in &fetched {
        let path = cache_dir
            .join(&entry.kind)
            .join(format!("{}.pine", entry.slug));
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (status, codes) = run(&source, &bars, &libraries);
        let expected = expectations
            .get(&entry.slug)
            .cloned()
            .unwrap_or(Expectation {
                status: "ok".to_owned(),
                codes: Vec::new(),
            });
        assert_eq!(
            status, expected.status,
            "{} ({}) status differs from expectations.json",
            entry.slug, entry.name
        );
        assert_eq!(
            codes, expected.codes,
            "{} ({}) diagnostic codes differ from expectations.json",
            entry.slug, entry.name
        );
    }
    eprintln!("Pine corpus: {} scripts checked", fetched.len());
}
