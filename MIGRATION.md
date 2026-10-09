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
| 3. Dashboard composition | Add/remove charts, columns/stack layouts, drag reorder, responsive cards, toolbar, tool rail, watchlist split, focus behavior, saved layout fingerprints. | accepted | — |
| 4. Market data parity | Provider traits; Binance, Coinbase, CoinGecko, DEXScreener/GeckoTerminal, Alpaca, Polymarket, Kalshi, and CMC; cache; rate limits; live WebSockets; source-qualified identity; icons. | planned | — |
| 5. Chart analysis and drawings | Volume, RSI, EMA, Bollinger, Supertrend, linear/log scale parity, cross-chart crosshair, price-axis drag, trend lines, Fibonacci, brush, ruler, undo/redo, replay. | planned | — |
| 6. Persistence and watchlists | SQLite schema, settings, tabs/layouts, watchlists/sections/flags, quote coordinator, imports, compatibility/import decision for existing Swift data. | planned | — |
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
```

## Feedback log

- 2026-10-09: Stage 1 approved; user requested continuation to Stage 2 without changes.
- 2026-10-09: Stage 2 approved; native tabs and windows accepted as-is, continuation to Stage 3.
- 2026-10-09: Stage 3 approved; dashboard composition accepted as-is, continuation to Stage 4.
