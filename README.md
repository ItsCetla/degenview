# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 2 adds real AppKit window tab groups through a small macOS bridge: add tabs with ⌘T or
`+`, reorder, detach, and merge windows, with per-tab independent market/timeframe/fetch state,
native title sync, and basic session restoration on relaunch.

This stage does **not** claim dashboard composition, persistence, WebSocket, indicator, or
multi-source parity yet.

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

