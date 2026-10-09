# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 4c adds the Alpaca stock service (IEX bars + asset search, credential-gated), an in-memory
kline cache, and request rate limiting for CoinGecko/GeckoTerminal. The Add Chart sheet now has
Crypto and Stocks tabs.

This stage does **not** yet claim the Prediction Markets or CoinMarketCap sources, disk caching,
429 backoff, live WebSockets, or icons.

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

