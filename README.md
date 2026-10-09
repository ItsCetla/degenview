# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 4e-i adds coin icons: the card header resolves a coin's icon by base symbol (CoinGecko
search with a static-CDN fallback) and renders it, or keeps a letter monogram while resolving.

This stage does **not** yet claim live WebSockets, the CoinMarketCap index charts, disk caching,
or 429 backoff.

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

