# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stage 5a adds built-in indicators — volume bars, RSI, EMA, Bollinger bands, and Supertrend
flips — toggled per chart from a Chart Settings window, with faithful indicator math.

This stage does **not** yet claim cross-chart crosshair, price-axis drag, drawings, or replay.

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

