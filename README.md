# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 4e-iii adds CoinMarketCap index charts (Fear & Greed and Altcoin Season historical),
key-gated behind `DEGENVIEW_CMC_API_KEY`, with a CoinMarketCap tab in the Add Chart sheet.
This completes the market-data sources.

This stage does **not** yet claim the CMC "latest" gauges, range pickers, or disk caching.

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

