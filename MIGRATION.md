# DegenView Rust migration

Last updated: 2026-10-09

This document is the durable handoff and progress ledger for the Swift-to-Rust rewrite. Update
it in the same commit as each accepted stage so another session can resume without reconstructing
decisions from chat history.

## Goal and constraints

- Rebuild DegenView as a Rust application while preserving the macOS-first interaction model.
- Match the existing dark dashboard, chart density, card hierarchy, spacing, color semantics,
  and native window behavior as closely as practical.
- Migrate in small, runnable vertical slices. Each slice is built and launched, reviewed by the
  user, adjusted if needed, and only then committed.
- Keep the Swift application as the behavioral and visual reference until final cutover.
- Do not call a feature migrated until its tests and acceptance gate pass.

## Architecture decisions

| Area | Decision | Reason |
| --- | --- | --- |
| UI and rendering | `eframe` + `egui` | Immediate-mode UI suits frequently updating dashboards and hand-drawn charts; the existing proof-of-concept already validates the renderer and Binance path. |
| macOS integration | Small `objc2` / `objc2-app-kit` boundary, introduced in Stage 2 | Keeps the application Rust-only while allowing real `NSWindow` tab groups, menus, titles, occlusion, and restoration behavior. |
| Async work | Background workers behind message channels initially; graduate to a shared async runtime when live feeds arrive | Keeps Stage 1 deterministic and prevents network work from blocking the UI without prematurely fixing the final concurrency model. |
| Data contracts | Provider-neutral domain models separated from services | Preserves the Swift architecture's source abstraction and makes parsers/cache behavior unit-testable. |
| Drawing | Custom `egui::Painter` code | Mirrors the current hand-drawn AppKit chart and avoids tying parity to a generic chart library. |
| Persistence | SQLite in a later isolated crate/module | Retains the current single-database model and enables compatibility/import tooling to be designed explicitly. |

`egui` does not itself provide the macOS application menu or AppKit tab lifecycle. Those are not
being approximated with fake UI: Stage 2 owns a native bridge and must pass the same detach, merge,
single-tab tab-bar, and title tests as the Swift app before it is accepted.

## Repository shape

```text
src/
├── app.rs              # Hidden coordinator, tab lifecycle, and session persistence
├── format.rs           # Price and timestamp presentation
├── model/              # Provider-neutral domain values
├── native_bridge.rs    # AppKit menus, tab grouping, and native actions
├── service/            # REST/WebSocket/provider boundaries
├── ui/                 # Theme and component renderers
└── workspace.rs        # Independent chart workspace owned by one native tab
tools/
└── bundle.sh           # Finder-launchable macOS .app bundle
```

The shape will grow by feature, not by copying the Swift directory tree mechanically. Large
features such as Pine, portfolios, and alerts should become their own modules or workspace crates
when their stages begin.

## Stage ledger

Status values: `planned`, `in progress`, `awaiting feedback`, `accepted`, `blocked`.

| Stage | Scope and acceptance gate | Status | Commit |
| --- | --- | --- | --- |
| 0. Foundation | Separate Git repo, build tooling, migration ledger, architecture decision, basic unit-test path. | accepted | `eeac906` |
| 1. First component: ChartCard | One real Binance chart card in a fidelity-oriented shell. REST refresh, timeframe selection, candles, grid/axes, last-price marker, crosshair/OHLC readout, pan, scroll zoom, and log toggle work. `cargo test`, `cargo clippy`, release build, bundle, and launch all pass. | accepted | `eeac906` |
| 2. Native tabs and windows | Real AppKit window tab groups through the macOS bridge: add, reorder, detach, merge, title sync, one-tab tab bar, multiple independent tab states, basic session restoration. | accepted | `b4f9fa6` |
| 3. Dashboard composition | Add/remove charts, columns/stack layouts, drag reorder, responsive cards, toolbar, tool rail, watchlist split, focus behavior, saved layout fingerprints. | accepted | `ed3cb18` |
| 4. Market data parity | Provider traits; Binance, Coinbase, CoinGecko, DEXScreener/GeckoTerminal, Alpaca, Polymarket, Kalshi, and CMC; cache; rate limits; live WebSockets; source-qualified identity; icons. | accepted | `22bd47b` |
| 5. Chart analysis and drawings | Volume, RSI, EMA, Bollinger, Supertrend, linear/log scale parity, cross-chart crosshair, price-axis drag, trend lines, Fibonacci, brush, ruler, undo/redo, replay. | accepted | `9167a44` |
| 6. Persistence and watchlists | SQLite schema, settings, tabs/layouts, watchlists/sections/flags, quote coordinator, imports, compatibility/import decision for existing Swift data. | accepted | `2f69d6c` |
| 7. Portfolio and paper trading | Ledger/accounting, history, FX, imports/exports, portfolio UI, paper execution engine, tables, chart markers, test fixtures. | planned | — |
| 8. Alerts and webhooks | Price alert engine, history/UI, background agent strategy, notifications, unseen state, webhook policy/secrets/delivery and redaction. | planned | — |
| 9. Pine | Language frontend, type checker, runtime, broker, editor assistance, completion, Script Manager, chart plots, alert subscriptions, corpus/regression strategy. | planned | — |
| 10. Parity hardening and cutover | Accessibility, keyboard commands, performance/energy, failure recovery, packaging/signing/update plan, full behavior checklist, data migration, release candidate. | planned | — |

## Current checkpoint

### Stage 1 — ChartCard

Implementation checklist:

- [x] Initialize a separate Rust repository.
- [x] Record architecture and stage gates.
- [x] Render the DegenView-inspired macOS dashboard shell.
- [x] Fetch and parse Binance candles away from the UI thread.
- [x] Draw candles, grid, axes, last-price marker, and hover readout.
- [x] Support timeframe changes, pan, scroll zoom, and linear/log scale.
- [x] Unit-test parsing and presentation helpers.
- [x] Pass formatting, tests, lint, release build, bundle, and launch checks.
- [x] Capture user feedback.
- [x] Apply requested adjustments and mark the stage accepted.
- [x] Commit the accepted stage before starting Stage 2.

Known deliberate gaps in this checkpoint:

- The right watchlist and left tool rail are visual context only.
- Only Binance REST history is wired; the five-second refresh replaces the latest snapshot.
- No WebSocket, saved state, native tab group, add-chart flow, indicators, drawings, or database.
- The iconography uses simple text/drawn marks until asset and SF Symbol integration is designed.

Stage 1 is accepted.

### Stage 2 — Native tabs and windows

Implementation checklist:

- [x] Keep a hidden eframe coordinator so every visible workspace is a real peer `NSWindow`.
- [x] Preserve winit's application delegate and attach `newWindowForTab:` to it at runtime.
- [x] Add native File ▸ New Tab (⌘T) and Window tab-management commands.
- [x] Group newly created workspace windows with AppKit and keep the tab bar visible at one tab.
- [x] Give each tab an independent workspace, market, timeframe, fetch worker, and chart state.
- [x] Synchronize native window/tab titles with the workspace symbol and timeframe.
- [x] Persist and restore tab identity, symbol, timeframe, log scale, and watchlist visibility.
- [x] Build, test, lint, bundle, launch, and visually smoke-test the stage.
- [x] User acceptance: add tabs with both ⌘T and `+`, reorder, detach, merge, and confirm state independence.
- [x] Apply requested adjustments and commit the accepted stage.

The current build is running for review. It restores the three tabs used by the smoke test (two
BTC/USDT 1D workspaces and one ETH/USDT 1H workspace); their distinct market/timeframe state is
intentional test data.

Stage 2 is accepted.

Known deliberate gaps in this checkpoint:

- Restoration currently rebuilds saved tabs into one native group; detached-window grouping,
  selected-tab identity, window frames, and user-reordered tab order are not persisted yet.
- The AppKit bridge is macOS-specific. Other platforms keep the coordinator/workspace model but
  need a later platform window shell as part of the cutover strategy.
- Closing the last workspace leaves the hidden application coordinator alive; ⌘T creates a new
  workspace, and a later stage will add the final empty-window experience.

Stage 2 accepted and committed; next stage is Stage 3 (dashboard composition).

### Stage 3 — Dashboard composition

Implementation checklist:

- [x] Port `ChartColumn` with `resolved()` row-major default and stale/duplicate/empty repair.
- [x] Port `ChartLayout` sizing math (equal card height, occupancy, minimum column width) with tests.
- [x] Add `LayoutSnapshot` fingerprint and dirty detection; zoom, focus, and watchlist edits excluded.
- [x] Refactor `ChartWorkspace` to own multiple charts in columns with a shared timeframe.
- [x] Add and remove charts via toolbar, tool rail, and the card close button.
- [x] Drag reorder within and between columns plus a trailing new-column drop zone.
- [x] Focus behavior: resolved focused chart, ring only with more than one card, click-to-focus.
- [x] Watchlist rows switch the focused chart (focus existing, switch, or add).
- [x] Responsive cards sized to fit the available height; column count tracks the layout.
- [x] Persist and restore per-tab charts, columns, focus, and timeframe.
- [x] Build, test, lint, bundle, and launch pass.
- [x] User acceptance: add/remove, reorder, new column, focus ring, watchlist switch, dirty state.
- [x] Apply requested adjustments and commit the accepted stage.

Known deliberate gaps in this checkpoint:

- Card drag uses the header as its handle (egui cannot cleanly separate card-drag from chart
  pan); the Swift app starts a card drag from anywhere on the card.
- "Add Chart" mints a chart with the next default symbol rather than opening a search sheet;
  full multi-source search is Stage 4.
- Named views and their save/load are Stage 6. "My Chart *" reports the layout fingerprint and
  clicking it marks the layout clean as a placeholder.
- The watchlist is still a static list (now interactive); real watchlist persistence is Stage 6.
- Column identity uses a monotonic counter rather than UUIDs; equivalence compares membership.

Resume by collecting Stage 3 feedback. If accepted, commit this checkpoint before starting Stage 4.

Stage 3 accepted and committed; next stage is Stage 4 (market data parity).

### Stage 4 — Market data parity

Stage 4 is sliced; the ledger row below tracks the whole stage. Slices 4a–4e are implemented; Stage 4 is complete pending acceptance.

**4a — Provider trait, identity, Binance/Coinbase/CoinGecko klines:**

- [x] `DataSource` enum (eight providers) with display name, glyph, slug, and symbol display.
- [x] `InstrumentId` source-qualified identity (`key`, `api_symbol`, `is_same_market`).
- [x] `MarketSource` trait and a dispatch that maps a source to its provider.
- [x] Binance refactored onto the trait; Coinbase and CoinGecko kline services added.
- [x] Coinbase folds daily candles into weekly/monthly/quarterly/yearly on Binance boundaries.
- [x] CoinGecko builds candles from `/market_chart` with interval-appropriate windows.
- [x] Per-chart `source`, set at add/switch time; source-selection UI arrives with the 4b search sheet.
- [x] Card header and window title show the source and a source-correct pair label.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4a:

- Only Binance/Coinbase/CoinGecko fetch; DEXScreener, Alpaca, Polymarket, Kalshi, and CMC
  are stubs that error until their slice (4b–4e).
- No cache or rate limiter yet: Coinbase's backwards paging for folded intervals re-fetches on
  every refresh and can be slow for monthly/yearly. Caching lands in 4c.
- No source-selection UI yet (the Swift app picks source in the Add Chart search sheet, which is
  4b); until then charts are Binance and the other two sources are exercised by unit tests.
- CoinGecko free-tier windows cap weekly/monthly/yearly depth at ~365 days of daily samples.

Resume by collecting 4a feedback. Accepted work is committed before the next slice.

**4b — Search sheet, DEXScreener/GeckoTerminal:**

- [x] `SearchResult` type and `search_tickers` on the `MarketSource` trait.
- [x] Search on Binance (exchangeInfo), Coinbase (products), CoinGecko (/search), and
  DEXScreener (/search), each with source-local relevance ranking.
- [x] DEXScreener charts via GeckoTerminal OHLCV, with network-id resolution from pool search.
- [x] Add Chart opens a search sheet (Crypto only): debounced query, results grouped by source,
  select a hit, Add commits a source-qualified chart or focuses an existing one.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4b:

- The search sheet shows only the Crypto tab; Stocks (4c), Prediction Markets (4d), and
  CoinMarketCap/Portfolio/Models tabs arrive with their slices.
- Search fans out sequentially in one worker thread rather than concurrently; a later slice can
  parallelize it.
- Recents and suggestion chips are absent (they need the watchlist/recents store from Stage 6).

Resume by collecting 4b feedback. Accepted work is committed before the next slice.

**4c — Alpaca, kline cache, rate limiter:**

- [x] Alpaca service: IEX stock bars (monthly fold for quarterly/yearly) and asset search,
  authenticated from `DEGENVIEW_ALPACA_KEY_ID` / `DEGENVIEW_ALPACA_SECRET_KEY`.
- [x] In-memory kline cache with per-source TTL, wired into CoinGecko and GeckoTerminal.
- [x] Rate limiter spacing CoinGecko (4 s) and GeckoTerminal (2.2 s) requests.
- [x] Add Chart sheet gains a Stocks tab (Alpaca), with an inline error when it isn't configured.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4c:

- Alpaca credentials come from environment variables rather than the Settings UI (cutover).
- The kline cache is in-memory only; CoinGecko's disk warm-start cache is deferred.
- The rate limiter spaces requests but does not widen on 429; backoff arrives with retry handling.

Resume by collecting 4c feedback. Accepted work is committed before the next slice.

**4d — Polymarket + Kalshi prediction markets:**

- [x] Polymarket search (Gamma) and price history (CLOB), downsampled with the YES ask overlay.
- [x] Kalshi series index (in-memory), events search, and candlesticks with carry-forward prices.
- [x] Prediction-market charts render as lines rather than candles.
- [x] Add Chart sheet gains a Predictions tab (Polymarket + Kalshi, grouped by source).
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4d:

- Multi-outcome events chart one selected market at a time; the Swift app carries every choice
  on a row so one selection charts them all.
- Probability is rendered with the currency price formatter rather than a percent label.
- The Kalshi series index is in-memory only (disk cache deferred); the provider picker that
  switches between Polymarket and Kalshi inside the tab is omitted (both search together).

Resume by collecting 4d feedback. Accepted work is committed before the next slice.

**4e-i — Icons:**

- [x] Coin icons resolved by base symbol: CoinGecko `/search` (exact-symbol, best rank), then the
  static-CDN fallback with a HEAD check; in-memory positive/negative cache.
- [x] Stock icons from the community CDN; prediction markets keep the monogram.
- [x] Card header shows the resolved icon (via egui image loaders) or the letter monogram.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4e-i:

- The market-snapshot batch (top-250 coins in one call) is omitted; every new symbol costs one
  CoinGecko search, which is rate-limited like the OHLC fetches.
- Icons render in the card header only; watchlist rows and search results still use text.
- Polymarket's market artwork lookup is deferred (Kalshi serves none, matching the original).

Resume by collecting 4e-i feedback. Next slices: 4e-ii WebSockets, 4e-iii CMC charts.

**4e-ii — Live WebSockets:**

- [x] Binance combined kline stream updates the forming candle in real time (reconnect with backoff).
- [x] The live feed reconnects when the workspace's Binance symbols or timeframe change.
- [x] Quarterly/yearly timeframes stay on REST (their monthly source folds, so no stream).
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4e-ii:

- Only Binance streams. Coinbase (matches channel) and Alpaca (IEX stream) are deferred.
- The book-ticker best bid/ask overlay is omitted; only the kline candle is live.
- Hidden tabs keep their stream open (the Swift app disconnects inactive hosts).

Resume by collecting 4e-ii feedback. Next: 4e-iii CMC charts.

**4e-iii — CoinMarketCap index charts:**

- [x] CMC client (pro-api, `X-CMC_PRO_API_KEY` from `DEGENVIEW_CMC_API_KEY`) with envelope/status unwrap.
- [x] Fear & Greed and Altcoin Season historical charts fetched as index points, rendered as lines.
- [x] Add Chart sheet gains a CoinMarketCap tab listing the four chart types.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 4e-iii:

- The two "latest" types are gauges (a single current reading), not time series; they report
  "arrives in a later slice" rather than rendering a gauge.
- No CMC API key without env: the tab lists types but adding one reports the missing key.
- Ranges are fixed (7d altcoin, 1M fear & greed); the Swift app offers range pickers.

Resume by collecting 4e-iii feedback. Stage 4 is then accepted and committed.

Stage 4 accepted and committed; next stage is Stage 5 (chart analysis and drawings).

### Stage 5 — Chart analysis and drawings

Stage 5 is sliced; the ledger row below tracks the whole stage. Slices 5a–5d are implemented.

**5a — Indicators and scale parity:**

- [x] Indicator math: EMA (SMA-seeded), RSI (Wilder), Bollinger (population std dev),
  Supertrend flips (Wilder ATR), computed over the full buffer and trimmed to the visible tail.
- [x] Rendering: volume bars, RSI strip with 70/30 guides, EMA overlay, Bollinger channel, and
  Supertrend flip triangles.
- [x] Per-chart indicator settings (volume/RSI/EMA/Bollinger/flips + EMA period), persisted.
- [x] Chart Settings window from the card header "Settings" button.
- [x] Linear/log scale parity retained.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 5a:

- Volume bars use base-asset volume; the Swift app plots quote turnover on Binance.
- Supertrend flip markers omit the "Bullish"/"Bearish" label pill.
- Indicator settings do not dirty the layout fingerprint yet.
- EMA period choices are fixed (9/20/50/100/200) like the Swift sheet.
- Bollinger channel rendering is deferred: the bands' values are correct but the channel fill
  renders wrong; left as-is at the user's request, to be revisited.

Resume by collecting 5a feedback (volume/RSI/EMA/flips). Next: 5b crosshair + price-axis drag.

**5b — Crosshair and price-axis drag:**

- [x] Price-axis drag: dragging the right price scale zooms the vertical range (yZoom, 0.25–20,
  up narrows the slice). Persisted per chart.
- [x] Cross-chart crosshair: hovering one chart draws the vertical crosshair across every chart
  at the same column; the hovered chart also draws the horizontal line and price pill.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 5b:

- Double-click to reset the price zoom is omitted.
- The price pill is drawn only on the hovered chart, matching the Swift owner model.

Resume by collecting 5b feedback. Next: 5c drawings.

**5c — Drawings and undo/redo:**

- [x] Drawing model: trend line, Fibonacci, brush, ruler (time+price anchors, serde).
- [x] Painter helpers + interactive layer (gestures, hit-testing, handles, draft, commit).
- [x] Tool rail arms trend line / fib / brush / ruler; crosshair is the default.
- [x] Per-workspace undo/redo (⌘Z / ⇧⌘Z); selection scoped to (chart, drawing).
- [x] Drawings persisted per chart; rulers transient (dropped on tool/timeframe change).
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 5c:

- No style editor (colour/width/opacity/lock UI); brush is fixed orange 2 px, trend/fib fixed blue 1.5 px.
- Fib levels fixed to the seven defaults; no per-timeframe visibility.
- Ruler has no hover-only handles yet.

Resume by collecting 5c feedback. Next: 5d replay.

**5d — Historical bar replay:**

- [x] Replay model + engine (status, session, timeline, play/pause/step/seek/speed/interval).
- [x] Replay bar UI (open/select start, transport, timeline slider, speed + interval).
- [x] Charts rewind to the replay timestamp; start picked by clicking a candle.
- [x] Toolbar Replay button opens the bar; return-to-live and Esc cancel selection.
- [x] Build, test, lint, bundle, and launch pass.

Known deliberate gaps in 5d:

- Granular intraday replay data is deferred; intervals all behave like `chartBar` (the chart's
  own candle bars) until `fetch_replay_klines` lands on the service trait.
- The "Date & Time…" sheet is deferred; start is chosen on-chart or via first/random bar.

Resume by collecting 5d feedback. Stage 5 is then accepted and committed.

Stage 5 accepted and committed; next stage is Stage 6 (persistence and watchlists).

### Stage 6 — Persistence and watchlists

Stage 6 is sliced; the ledger row below tracks the whole stage. Slice 6a is implemented.

**6a — SQLite store and tab persistence:**

- [x] `rusqlite` (bundled) store with the Swift document-table schema (id/position/JSON payload)
  plus `setting` and `window_group`; in-memory fallback.
- [x] Tab session moves from eframe JSON storage to the `tab` document table + `session.next_id`
  setting.
- [x] Store unit tests (document round-trip, settings, undecodable-row skip).

Known deliberate gaps in 6a:

- Only the tab session persists to SQLite so far; saved views, watchlists, favorites, drawings,
  and settings tables are schema-only until their stores land.

Resume by collecting 6a feedback. Next: 6b watchlist model + UI, 6c imports.

**6b — Watchlists and quote coordinator:**

- [x] Watchlist domain model (lists, sections, instruments, flags, sort, columns, layout engine,
  import/export text), all serde-tolerant.
- [x] WatchlistStore persistence (`watchlist` document table + `watchlist.flags` setting), Favorites
  bootstrap, full mutation surface.
- [x] Watchlist sidebar UI (list selector, rows, sections, flags, drag reorder, right-click menus,
  import sheet) wired into the workspace.
- [x] Quote coordinator + book (batched per-source polling, Coinbase socket, staleness) with
  per-window demand.
- [x] 6c importer (`run_once`, Swift→Rust `tab` mapping) called at launch.

Known deliberate gaps in 6b:

- Watchlist Import/Export uses the clipboard for export (no save dialog); DEX `chain` not persisted.
- Sidebar has no hover "…" button, no detail panel, no multi-select drag, no keyboard Delete.

Resume by collecting 6b feedback. Stage 6 is then accepted and committed.

## Verification log

Record commands and results for the current stage here.

```text
2026-10-09
- cargo fmt --all -- --check: passed
- cargo test: passed, 4 tests
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1
- visual smoke check: live BTC/USDT 1D Binance candles rendered; toolbar, chart card,
  price/time axes, current-price line, and watchlist shell visible in the macOS window

2026-10-09 — Stage 2
- cargo fmt --all -- --check: passed
- cargo test: passed, 6 tests
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no delegate panic
- native smoke check: one-tab bar and `+` visible; ⌘T created an independent workspace;
  AppKit grouped three workspaces into one window; Window ▸ Show Next Tab selected ETH/USDT 1H
- persistence smoke check: graceful ⌘Q and relaunch restored all three native tabs and their
  BTC/USDT 1D / ETH/USDT 1H workspace settings
- manual feedback gate still open: drag reorder, drag-out detach, and Merge All Windows
- manual feedback gate: passed — user confirmed ⌘T / `+` / drag reorder / drag-out detach / Merge All Windows

2026-10-09 — Stage 3
- cargo fmt --all -- --check: passed
- cargo test: passed, 11 tests (column resolution, layout sizing, snapshot round-trip)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- manual feedback gate still open: add/remove, reorder, new column, focus ring, watchlist switch
- manual feedback gate: passed — user confirmed add/remove, reorder, new column, focus ring, watchlist switch, dirty state

2026-10-09 — Stage 4a
- cargo fmt --all -- --check: passed
- cargo test: passed, 20 tests (identity, coinbase fold/page, coingecko window/fold, layout)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- live endpoint check: Coinbase /products/BTC-USD/candles and CoinGecko /market_chart both
  returned expected shapes
- removed a toolbar source dropdown after review: source selection belongs to the 4b search sheet,
  not a toolbar control absent from the original app
- manual feedback gate: passed — accepted after removing the fabricated toolbar dropdown

2026-10-09 — Stage 4b
- cargo fmt --all -- --check: passed
- cargo test: passed, 23 tests (search parse, geckoterminal window/aggregate, identity, fold)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- live endpoint check: Binance exchangeInfo, Coinbase products, CoinGecko /search,
  DEXScreener /search, GeckoTerminal /search/pools all returned expected shapes
- manual feedback gate still open: search a symbol, results grouped by source, Add commits a chart
- manual feedback gate: passed — user confirmed search, grouped results, and source-qualified add

2026-10-09 — Stage 4c
- cargo fmt --all -- --check: passed
- cargo test: passed, 25 tests (alpaca iso parse + monthly fold, geckoterminal, identity)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- manual feedback gate still open: search sheet Stocks tab shows the Alpaca setup error without
  credentials; crypto search still works and CoinGecko/DEX results are cached across refreshes
- manual feedback gate: passed — user confirmed cache, rate limiter, Alpaca, and the Stocks tab

2026-10-09 — Stage 4d
- cargo fmt --all -- --check: passed
- cargo test: passed, 29 tests (polymarket window, kalshi window + series rank, string-array decode)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- live endpoint check: Polymarket /public-search and Kalshi /series both returned the expected
  shapes (JSON-string arrays, snake_case series fields)
- manual feedback gate still open: Predictions tab search and line-chart rendering
- manual feedback gate: passed — user confirmed Predictions search and line charts

2026-10-09 — Stage 4e-i (icons)
- cargo fmt --all -- --check: passed
- cargo test: passed, 29 tests
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- new dependency: egui_extras (all_loaders) for remote image loading
- manual feedback gate still open: coin icons appear in card headers after a short resolve
- manual feedback gate: passed — user confirmed coin icons render in card headers

2026-10-09 — Stage 4e-ii (WebSockets)
- cargo fmt --all -- --check: passed
- cargo test: passed, 30 tests (kline stream parse)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- new dependency: tungstenite for the WebSocket client
- manual feedback gate still open: forming candle updates live rather than only on the 5 s refresh
- fix: tungstenite defaults to no TLS feature, so the wss handshake failed silently; enabled
  rustls-tls-native-roots
- manual feedback gate: passed — user confirmed live candle updates between REST refreshes

2026-10-09 — Stage 4e-iii (CMC)
- cargo fmt --all -- --check: passed
- cargo test: passed, 31 tests (cmc date parse)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- manual feedback gate still open: CoinMarketCap tab lists the four types; adding a historical one
  without a key reports the missing-key error (gated on DEGENVIEW_CMC_API_KEY)
- manual feedback gate: passed — user confirmed the CoinMarketCap tab and key gating

2026-10-09 — Stage 5a (indicators)
- cargo fmt --all -- --check: passed
- cargo test: passed, 34 tests (ema seed, rsi pin, bollinger ordering)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- manual feedback gate still open: card Settings toggles draw volume/RSI/EMA/Bollinger/flips

2026-10-09 — Stage 5b (crosshair + price-axis drag)
- cargo fmt --all -- --check: passed
- cargo test: passed, 34 tests
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic
- manual feedback gate still open: drag the price axis to zoom, hover to sync the crosshair
- fix: `drag_delta()` is per-frame; switched y-zoom and pan to cumulative `total_drag_delta()`
- manual feedback gate: passed — user confirmed price-axis zoom and crosshair sync

2026-10-09 — Stage 5c (drawings + undo/redo)
- cargo fmt --all -- --check: passed
- cargo test: passed, 54 tests (drawing model + layer geometry round-trip)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- launch: passed with DEGENVIEW_NO_KEYCHAIN=1; no panic (built by second agent)
- manual feedback gate still open: trend line / fib / brush / ruler gestures, Delete, ⌘Z / ⇧⌘Z,
  drawings following timeframe switch / scroll-zoom / y-axis drag
- manual feedback gate: passed — user confirmed all drawing gestures

2026-10-09 — Stage 5d (replay)
- cargo fmt --all -- --check: passed
- cargo test: passed, 65 tests (replay engine steps/seek/restart + bar/overlay)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- manual feedback gate still open: open replay, pick a start, play/pause/step/seek, return to live
- manual feedback gate: passed — user confirmed replay; Stage 5 (5a–5d) accepted

2026-10-09 — Stage 6a (SQLite store)
- cargo fmt --all -- --check: passed
- cargo test: passed, 68 tests (store document/setting round-trip, undecodable-row skip)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- tab session now persists to ~/Library/Application Support/com.degenview.rust/degenview.sqlite

2026-10-09 — Stage 6b+6c (watchlists, quotes, imports)
- cargo fmt --all -- --check: passed
- cargo test: passed, 164 tests (watchlist model/store, layout engine, quote service, import)
- cargo clippy --all-targets --all-features -- -D warnings: passed
- ./tools/bundle.sh: passed, optimized target/DegenView.app created
- manual feedback gate still open: watchlist sidebar (lists/rows/sections/flags/reorder/menus),
  quotes populate, import/export, Swift .sqlite import on first launch
- manual feedback gate: passed — user confirmed watchlists, quotes, and import; Stage 6 accepted
```

## Feedback log

- 2026-10-09: Stage 1 approved; user requested continuation to Stage 2 without changes.
- 2026-10-09: Stage 2 approved; native tabs and windows accepted as-is, continuation to Stage 3.
- 2026-10-09: Stage 3 approved; dashboard composition accepted as-is, continuation to Stage 4.
- 2026-10-09: Stage 4a approved; removed a fabricated toolbar source dropdown (source selection
  belongs to the 4b search sheet), with the guidance to port the original without invention.
- 2026-10-09: Stage 4b approved; search sheet and DEXScreener/GeckoTerminal accepted as-is.
- 2026-10-09: Stage 4c approved; Alpaca, kline cache, and rate limiter accepted as-is.
- 2026-10-09: Stage 4d approved; Polymarket/Kalshi prediction markets accepted as-is.
- 2026-10-09: Stage 4e-i approved; coin icons accepted as-is.
- 2026-10-09: Stage 4e-ii approved; Binance live kline stream accepted after enabling TLS.
- 2026-10-09: Stage 4e-iii approved; CoinMarketCap index charts accepted as-is, completing Stage 4.
- 2026-10-09: Stage 5b approved; crosshair + price-axis drag accepted after fixing a per-frame
  drag-delta bug (now cumulative) that also affected chart pan. Stage 5a indicators committed
  with the Bollinger channel fill deferred.
- 2026-10-09: Stage 5c approved; drawings + undo/redo accepted as-is (built with a second agent).
- 2026-10-09: Stage 5d approved; historical bar replay accepted as-is, completing Stage 5.
- 2026-10-09: Stage 6 approved; watchlists, quote coordinator, and import accepted as-is.
