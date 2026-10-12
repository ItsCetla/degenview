# AGENTS.md — DegenView Rust

Guidance for AI coding agents working in this repo. Adapted from the Swift app's
`AGENTS.md` to the Rust rewrite.

## Project

macOS market dashboard (candlestick charts, watchlists, portfolio/paper trading, price and
Pine-script alerts, a full Pine Script engine). Written in Rust with `eframe`/`egui` for the
UI and a small `objc2` AppKit boundary for native windows, tabs, menus, Keychain, and
accessibility. Market data comes from Binance/Coinbase/CoinGecko/DEXScreener/GeckoTerminal,
Alpaca, Polymarket, Kalshi, and CoinMarketCap over REST + WebSocket.

See [Architecture](docs/architecture.md) for the project structure and data flow.

## Build

```bash
cargo test
cargo run
```

Build a Finder-launchable macOS app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

Release: push a tag like `v0.1.0` and the `Release` workflow (`.github/workflows`) builds
macOS (arm64 + Intel), Windows, and Linux binaries and publishes a GitHub release with a
commit summary since the previous tag.

The binary is named `degenview` (`Cargo.toml`), GPL-3.0-only. Non-macOS platforms build the
coordinator/workspace model but get the AppKit bridge behind `#[cfg(target_os = "macos")]`.

## Code conventions

- **Module split, not MVVM**: `model/` — pure domain values, enums, and pure engines;
  `service/` — provider/network boundaries; `ui/` — egui renderers; `store/` — SQLite
  persistence; the top-level `*_host.rs` files — stateful bridges between an engine, SQLite,
  and the pure views; `workspace.rs` — one chart workspace; `app.rs` — the hidden coordinator.
- **Immediate-mode UI**: egui draws every frame; there are no view models or observed state.
  Candles are hand-drawn via `egui::Painter` — do not pull in a chart library.
- **No async runtime**: blocking work runs on background threads (`service::work_queue`) and
  reports back over `std::sync::mpsc` channels. Shared state is `Arc<Mutex<…>>`. Keep the store
  mutex held only for short database operations.
- **One `ChartWorkspace` per tab** — never treat it as app-global state. `DegenViewApp` owns the
  `Vec<TabSlot>`; a slot wraps a `ChartWorkspace`, a `PortfolioTab`, or a `ScriptManager`.
- **Persistence**: user data lives in SQLite (`degenview.sqlite`, bundled `rusqlite`) through
  `store::Store`. Document tables are `id/position/payload` JSON rows; `setting` and
  `window_group` hold keyed values. The schema is one idempotent `Store::create_schema`
  (`IF NOT EXISTS`); there are no versioned migrations and no readers for older data. A row this
  version cannot decode must not be silently replaced by an empty collection —
  `replace_documents` refuses to rewrite a collection containing an undecodable row. The one-way
  Swift import is `store::import::run_once`, gated per table on the Rust counterpart being empty.
- **Pine is a feature folder**: everything for the Pine engine lives under `src/pine/`
  (language, runtime, broker, model). `pine/` and the Pine domain in `model/` are pure — keep
  them free of egui types; the alert and subscription hosts compile the same pure modules.
- **Caching**: `service::cache` stores candles keyed by `(source, symbol, interval)` with a
  per-source TTL (memory only). `service::rate_limit` spaces CoinGecko and GeckoTerminal requests.
  Anything *regenerable* that is slow to fetch — logo bytes (`service::image_loader`), resolved
  logo URLs and coin names (`service::icon`), exchange symbol lists (`service::catalog`) — is
  persisted in the OS cache folder (`store::cache_dir`, never the data folder) through
  `service::disk_cache` / `service::lookup_cache`, with a TTL, stale-while-revalidate, and
  stale-on-error. Unlike the SQLite document tables, an undecodable cache file is discarded and
  rebuilt, not preserved. A new cache must be emptied by `service::cache_control::clear`
  (Settings ▸ Storage), and a write that starts before a clear must go through
  `DiskCache::write(.., generation)` so it cannot resurrect cleared data. Tests use
  `disk_cache::testing::TempCache`; the global cache is disabled under `cfg!(test)`.
- **WebSocket**: Binance kline streams update the forming candle (`service::live`); the
  Coinbase watchlist ticker socket (`service::watchlist_quotes::coinbase_ws`) feeds quotes.
  Coinbase has no candle channel.

## Key patterns

### Adding a new data source
1. Add a case to `model::source::DataSource` (display name, watchlist slug, `is_prediction_market`,
   `provides_volume`, line-vs-candle flag).
2. Create the provider service and implement `service::MarketSource` (`fetch_klines`, and
   `search_tickers` when it searches).
3. Register it in `service::for_source` (and `supports_trending`/`trending` only if it can list
   busy markets — Kalshi cannot).
4. Add a kline parser if the API shape differs from the existing ones.
5. Add it to the Add Chart sheet's source tab (`ui::add_chart_sheet`); prediction markets use
   `is_prediction_market()` rather than matching `.polymarket`.
6. Add a `SourceLogo` asset under `assets/source_logos/` and a `ui::source_logo::source` match
   arm; record the logo's origin in `docs/source-logos.md`.

Prediction markets are the exception to step 5: they render as lines (not candles), appear under
the Predictions tab, and refresh over REST (their sockets need signed keys or no candle channel).
Kalshi ids are `"SERIES/MARKET"`; its series index is cached in memory and ranked locally.

The Add Chart sheet groups a provider's hits by event (`SearchResult::event`, set by the
provider) and lists each event's choices as a tree. Ticking choices of one event builds a single
pick whose `EventInfo::series` lists them; adding it opens one chart that draws every choice as a
line (`ChartCard::set_series`, colored by position, labelled at the line's end) under the event's
title. The first ticked choice's market is the chart's symbol and supplies its candles. A chart
persists its choices and title (`PersistedChart::pm_series`/`title`); watchlist rows do not yet.

### Adding a new timeframe
1. Add a case to `model::timeframe::Timeframe` and set its `binance_interval` (and any
   display/date-format helpers).
2. If a provider cannot serve the interval natively, fold a finer one into calendar buckets
   (boundaries in `service::bucket_start_ms`) rather than picking a nearby size. `3M`/`1Y` are
   quarterly/yearly candles on every source: Binance and Alpaca fold monthly bars, Coinbase folds
   daily, CoinGecko/GeckoTerminal build from their own series. Binance has no live stream for
   them — the five-second REST refresh covers it.
3. Prediction markets keep their own `effective_span_days`/`line_chart_point_count` windows
   (`service/mod.rs`).

### Chart rendering
- `ui::chart_card::ChartCard` owns the draw loop via `egui::Painter`.
- Chart state (`log_scale`, y-zoom, appearance, indicators, drawings) lives in `workspace::Chart`;
  the view reads it each frame — pass it, don't observe it.
- Y-axis: the price transform handles both linear and log scale.
- Draw grid first, then candles, then overlays/indicators, then axis labels.
- Price formatting auto-adjusts decimal places by magnitude (`format.rs`).

### Coin icons
- `service::icon` resolves one icon per `"<source>:<ticker>"` key with an in-memory
  positive/negative cache (static CDN with a HEAD check first — unthrottled, so most icons appear
  at once — then CoinGecko `/search` for coins the CDN lacks).
- All CoinGecko traffic (OHLC and icons) queues behind the CoinGecko rate limiter.
- `nil`/miss is not a failure state: the header falls back to a letter monogram so the slot is
  occupied either way and card headers stay aligned.
- Icon lookups key off the market, never the numeric chart id — an id survives a ticker switch
  and would pin the old coin's artwork to a renamed card.

### Alert events
- Price alerts are evaluated in-app by `alert_host::AlertHost` (the in-app owner); there is no
  separate login-item agent yet. It loads the persisted snapshot, drains the `alert_command`
  queue, and exposes the evaluated snapshot.
- The unseen bell count is the `alerts.lastSeenAt` setting, never a flag on the events
  (`replaceSnapshot` rewrites them). A trigger banner renders over every workspace
  (`ui::alert_banner`).

### Webhooks
- `docs/architecture.md` has the flow. Endpoints are global (`store::webhook`, the
  `webhook_endpoint` document table). URL and header values are templates stored in plain text;
  show them only through `webhook_policy::redact`. The secret is the one value in Keychain
  (`service::keychain_webhook_secrets`), placed with `{{secret}}`: percent-encoded in the URL, raw
  in headers, never in the host or body. Never log or store a resolved URL/header.
- `service::webhook_delivery` is the only webhook transport; it claims a `webhook_delivery` row
  before sending, so one trigger reaches one endpoint once. No retries, no redirects. Delivery is
  currently osascript-backed, not a native HTTP client (known gap).
- New fields on persisted alert/webhook models must decode old rows (serde `#[serde(default)]`
  and missing-field tolerance): a row that fails to decode is dropped, and the snapshot rewrite
  then replaces the table without it.

### Pine editor assistance
- Pure logic, no egui: `script_manager::edit_ops` (pairing, indentation, `⌘/`, `⌥↑↓` move,
  `⇧⌥↑↓` duplicate, delimiter matching). Each function takes the text + selection and returns one
  `Edit`, or `None` to let the text field handle the input normally. New assistance goes there,
  not into `editor.rs`. Indices are `char` (egui cursor) offsets, not UTF-16.
- The lexical scan (`script_manager::syntax`) is the one per-text classification; strings come
  from the real lexer, `//` comments from a scan that respects string tokens, and names are
  matched whole-token, never substring. Never use indent/dedent for indentation decisions — an
  unclosed `(` mid-typing erases them.
- Indent unit is four spaces. Pine has no braces, so `{}` is deliberately not paired.
- Completion never compiles, runs, lexes again, or touches the network/database. Builtins come
  from the symbol catalog; signatures from `script_manager::signature` (no documentation text).
  A new builtin needs its name in the completion catalogue and its signature in the signature
  table, or both tests fail.

### Script Manager preview
- `script_manager::ScriptManager` owns the window; `script_manager::workspace::PreviewState`
  drives one standalone chart (market, timeframe, debounced apply, stale output). It never
  attaches a Pine alert coordinator, so a preview cannot raise alerts. Markets are crypto and
  stock only.
- `script_manager::inputs` is the one `input.*` → controls renderer, shared with the chart
  settings sheet; it takes a precompiled `InputSchema` — never compile inside a view.

### Example scripts
- `script_manager::demo` seeds example scripts once (`demo::SEEDED_KEY`, skipped under tests and
  when the user already has scripts) and on demand from the Script Manager's empty state, so a
  demo the user deletes stays deleted. File name is the script name (no "Demo" prefix).
- Demos are original work only (the repo is GPL-3.0; never copy community scripts) and must
  compile with no diagnostics, warnings included.

### WebSocket updates
- `service::live::BinanceLiveFeed` owns the combined kline stream for one workspace's Binance
  charts and reconnects with backoff; it drops on workspace drop.
- `service::watchlist_quotes::coinbase_ws::CoinbaseSocket` is one shared `ticker` socket for
  every product a visible watchlist shows. Coinbase has no candle channel; its ticker carries the
  last trade plus 24h open/volume.
- A websocket update for the current or later bar wins over an older REST snapshot
  (`workspace::merge_fetched_candles`).

### Watchlists
- `store::watchlist::WatchlistStore` is the one source of truth: `watchlist` document table, one
  row per list with its flat ordered entries (instrument or section). Mutations validate, persist
  in one transaction, then publish. A list that fails to read must not be overwritten by an empty
  one. Favorites live on as the list with `isFavorites`; old Swift `favorite` rows fold in once
  via import.
- Identity is `model::InstrumentId` (source + the provider's own id; `chain` is metadata, not
  identity). Never key a watchlist by ticker text. CoinMarketCap cards are not instruments.
- Flags are global per market (`setting` key `watchlist.flags`), not per list. Sorting/filtering
  never change the stored order; `model::watchlist_layout` sorts inside each section.
- Quotes are transient: `WatchlistQuoteBook` (one cell per market) is fed by
  `WatchlistQuoteCoordinator`, which merges every sidebar's demand by `InstrumentId`, makes one
  batched request per source, shares one Coinbase socket, and backs off per source. It never
  writes to the database and never uses the alert quote path.
- Selecting a row focuses/opens the market in the focused chart via `ChartWorkspace`; the
  highlighted row mirrors the focused chart's market.

### Tabs and windows
- Each tab is a real `NSWindow` in a tab group, created through `native_bridge`. `DegenViewApp`
  stays hidden; every visible workspace is a peer window.
- **AppKit owns all tab dragging.** Reorder, drag-out-to-detach, drag-window-onto-tab-bar to
  merge, the `+` button, and the Window-menu items come from `tabbingIdentifier` +
  `tabbingMode = .preferred`. Do not reimplement any of it. `native_bridge::reconcile_tabs`
  reads the arrangement back; `NSWindow.tabGroup` is the authority on layout.
- **`newWindowForTab:` is load-bearing beyond the `+` button**: AppKit refuses to draw the tab
  bar for a single-tab window unless something in the responder chain answers that selector.
  Keep that delegate installed or a one-tab window silently loses its bar (and the `+`).
- A new tab (`+`, ⌘T) and a first launch start with **no chart**: `ChartWorkspace::empty` shows the
  empty page (`ui::empty_state`), as Swift's `ChartTab()` does. Only opening a market from a
  watchlist seeds a chart. The page's database/Keychain reads are cached per visit
  (`EmptyPageCache`), never per frame.
- Restore is ours, not AppKit's: windows are rebuilt from the persisted `tab` document table on
  launch; `PersistedTab` is `#[serde(untagged)]` so existing chart-tab rows stay byte-compatible
  and special tabs (Portfolio, Pine) carry a required `kind`.

### Anything app-wide must be scoped per tab
Check for this shape when adding state:
- A per-workspace live feed and per-workspace refresh must gate on the workspace's own window
  visibility/occlusion; hidden tabs suspend so API load does not scale with tab count.
- Shared stores (`watchlist`, portfolio/paper/alert/webhook/Pine hosts) are `Arc<Mutex>` and
  app-wide by design; only their per-window *presentation* state belongs in the tab.

### State management
- Saved layouts: `store::saved_view` owns the library; `ChartWorkspace` is the tab's active
  layout. **Dirty is a fingerprint, not a flag**: a `LayoutSnapshot` (timeframe, chart configs,
  column membership — not zoom, drawings, replay, or market data) is compared against the
  baseline taken at open/save. Autosave is per layout, debounced 2 s (`AutosaveState`), never
  applies to Unnamed, and is flushed before a switch or quit.
- The tab name is always the active layout's name or "Unnamed"; there is no tab-only rename.
- The session (`PersistedSession { next_id, tabs }`) writes to SQLite on a 10-second debounce and
  on save/quit.

## File naming

- Snake-case module names; one concern per file.
- Services: `*service.rs` under `service/` (or the provider's own name: `binance.rs`, `alpaca.rs`).
- Renderers: `*_view.rs` or a role suffix under `ui/` (`*_sheet.rs`, `*_editor.rs`).
- Stores: `*_store.rs` under `store/`, plus the `Store` impl in `store/mod.rs`.

## Testing

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Unit tests cover market-data parsing and folding, indicators, chart layout/sizing, drawing
geometry, replay, portfolio accounting, paper execution, alert evaluation, webhook policy, and
the store. `src/pine/corpus_tests.rs` runs the cached community-script corpus
(`tools/pine-corpus/`) and skips without it; reproduce a failure with an original reduction
rather than copying corpus code. See `tools/pine-corpus/README.md`.

Tests never touch Keychain: `keychain_webhook_secrets::is_disabled` is true under `cfg!(test)`,
so the Alpaca/CMC/webhook stores behave as "nothing saved". When launching the app itself, set
`DEGENVIEW_NO_KEYCHAIN=1` in the environment — otherwise a rebuilt, re-signed binary can block
on a Keychain access prompt. Set `DEGENVIEW_CACHE_DIR=<scratch dir>` to keep a dev run's cache
(logos, lookups) out of your real one. Provider credentials come from `DEGENVIEW_ALPACA_KEY_ID` /
`DEGENVIEW_ALPACA_SECRET_KEY` / `DEGENVIEW_CMC_API_KEY` (or Keychain), so live-endpoint checks
are gated on those env vars.

Verify window/tab, chart, drawing, alert, replay, webhook, indicator, script editor, and
watchlist behaviour by hand. Nothing beyond the Script Manager has had a manual GUI pass yet.
