# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 6b+6c adds SQLite persistence: tab sessions, watchlists (lists/sections/flags), a batched
quote coordinator, and a one-time importer for the Swift app's saved data. The static watchlist
is replaced by a full sidebar.

This stage does **not** yet claim portfolios, paper trading, alerts, or Pine.

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

