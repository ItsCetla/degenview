# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 4d adds Polymarket and Kalshi prediction markets: search, YES-probability price history
downsampled with a current-ask overlay, and line-chart rendering. The Add Chart sheet now has
Crypto, Stocks, and Predictions tabs.

This stage does **not** yet claim the CoinMarketCap source, disk caching, 429 backoff, live
WebSockets, or icons.

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

