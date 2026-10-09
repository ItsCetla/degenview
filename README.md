# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 3 adds dashboard composition: multiple charts per tab laid out in reorderable columns,
add/remove via the toolbar, tool rail, or card close button, a trailing drop zone for new
columns, focus behavior with a focus ring, responsive card sizing, and an interactive watchlist
that switches the focused chart. A layout fingerprint drives the "My Chart *" dirty indicator.

This stage does **not** claim multi-source data, indicators, drawings, persistence beyond the
per-tab layout, WebSocket live feeds, or named-view save/load yet.

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

