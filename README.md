# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 4b adds ticker search across Binance, Coinbase, CoinGecko, and DEXScreener, with an Add
Chart search sheet (Crypto tab) that debounces the query, groups results by source, and commits
a source-qualified chart. DEX pairs chart through GeckoTerminal OHLCV.

This stage does **not** yet claim the Stocks, Prediction Markets, or CoinMarketCap tabs, the
remaining sources (Alpaca, Polymarket, Kalshi, CoinMarketCap), caching, rate limiting, live
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

