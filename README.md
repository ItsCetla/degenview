# DegenView

A native macOS market dashboard for watching crypto, stocks, prediction markets, and
CoinMarketCap market-wide indices in customizable charts. This is the Rust rewrite of
[DegenView](https://github.com/degenview/degenview), porting the same feature set onto
`eframe`/`egui` with a small AppKit bridge for native windows, tabs, menus, and Keychain.

> **Status**: the rewrite is feature-complete and in parity hardening.

## Features

### Markets and live data

- **Crypto** — Search Binance, Coinbase, CoinGecko, and DEXScreener side by side; DEX pairs
  use GeckoTerminal for historical OHLCV.
- **Stocks** — US equities through Alpaca's IEX feed (credentials stored securely in Keychain).
- **Prediction markets** — Polymarket and Kalshi events, charted as probabilities. No key needed.
- **CoinMarketCap indices** — Altcoin Season and Crypto Fear and Greed, historical or latest.
- **Live updates** — Binance and Coinbase crypto receive WebSocket updates; other sources
  refresh automatically.
- **Timeframes** — 1H, 1D, 1W, 1M, 3M, and 1Y candles with scroll-wheel zoom.
- **Multi-source identity** — The same symbol can be added from different sources without
  being treated as a duplicate.

### Charts and analysis

- Hand-drawn OHLC candlesticks, line series, grid, time axis, adaptive price precision,
  current-price overlay, and price-change readout — no third-party chart library.
- **Indicators** — Volume, RSI, configurable EMA, Bollinger Bands, and Supertrend flip markers.
- **Local Pine scripts** — A Pine Script v6 subset, lexed, parsed, and evaluated locally over
  OHLCV bars; source is never sent to TradingView. Plots, shapes, backgrounds, and candle
  colors render on the chart, and typed `input.*` controls re-evaluate without recompiling.
- **Script Manager** — A `.pine` file library with a code editor and a live preview chart that
  re-runs as you type.
- **Trend lines, Fibonacci retracements, and brush strokes** — anchored to time and price, with
  per-window undo/redo (⌘Z / ⇧⌘Z).
- **Ruler** — Temporary move measurement (percent, price, duration, bar count).
- **Crosshair** — Synchronized vertical inspection across all charts in a tab.
- **Price zoom** — Drag a chart's Y-axis to adjust its vertical scale.

### Portfolio tracking

- **Multiple portfolios** with a transaction ledger and weighted-average accounting.
- **Live analytics** — value, allocation, 24-hour movement, cost basis, realized/unrealized
  P&L, and best/worst performers.
- **Multi-currency reporting** — USD, EUR, GBP, JPY, CHF, and BTC, with cached FX conversion.
- **Portfolio history** — transaction-aware value snapshots across 1D–ALL ranges.
- **CSV import/export**, including CoinMarketCap transaction exports with automatic asset
  mapping.

### Alerts

- **Local price alerts** — absolute or percentage crosses-above/below, once or repeating,
  evaluated locally with no account or remote service.
- **Pine script alerts** — notify when a script's `alert()` fires on a live bar.
- **Delivery** — in-app banner and macOS notification, independently configurable.
- **Alerts center** — search and filter active, triggered, paused, and history records.

### Paper trading

A local, persistent simulator with an order ticket, market/limit/stop orders, net-position
tracking with weighted-average cost, and position/order markers on the chart. Paper orders
never route to a live exchange.

### Historical bar replay

Pick a historical candle and step through the market with no lookahead, with play/pause, a
timeline scrubber, speed and interval controls, and shared clock across every chart in a tab.

### Dashboard and persistence

- **Native macOS tabs and windows** — independent charts, timeframe, layout, zoom, and drawing
  state per tab; drag to reorder, detach, or merge.
- **Restored sessions** — tabs, order, and window grouping return on relaunch.
- **Flexible layouts** — vertical stacks or multi-column grids with drag reorder and drop
  columns.
- **Saved views** — named ticker sets with their timeframe, arrangement, zoom, source,
  indicators, and appearance.
- **Favorites sidebar** — an app-wide, reorderable watchlist.
- **Appearance** — System, Light, and Dark themes.

## Requirements

- macOS
- [Rust](https://rustup.rs) toolchain (`cargo` on your `PATH`)
- A free Alpaca account and API keys only if you want stock data
- No CoinMarketCap API key is required; adding one in Settings is optional

## Build and run

```bash
cargo run
```

Build a Finder-launchable app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

Releases: push a tag like `v0.1.0` and the Release workflow builds macOS (arm64 + Intel),
Windows, and Linux binaries and publishes a GitHub release.

## Documentation

- [Architecture](docs/architecture.md): project structure and data flow

## License

DegenView is licensed under the [GNU General Public License v3.0 only](LICENSE)
(`GPL-3.0-only`). Copyright © 2026 Nico Oelgart.
