# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 1 implements one real Binance candlestick `ChartCard` inside an early dashboard shell.
It supports historical data, automatic refresh, timeframe changes, scroll zoom, horizontal
pan, a crosshair, adaptive price labels, and linear/log price scales.

This stage does **not** claim tab, layout, persistence, WebSocket, indicator, or multi-source
parity yet.

## Build and run

Rust is expected in `~/.cargo/bin`:

```bash
~/.cargo/bin/cargo test
~/.cargo/bin/cargo run
```

Build a Finder-launchable app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

The current slice fetches public Binance REST data and therefore needs network access.

