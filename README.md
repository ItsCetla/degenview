# DegenView Rust

A staged Rust rewrite of the native macOS DegenView market dashboard.

The project is intentionally migrated in reviewable vertical slices. See
[MIGRATION.md](MIGRATION.md) for the architecture, stage gates, current status, and the next
work item.

## Current stage

Stages 0–9 are complete and committed: charts, native tabs, dashboard composition, market data
parity (eight providers + live WebSockets), chart analysis and drawings, SQLite persistence +
watchlists + Swift-data import, portfolio/paper trading, alerts + webhooks, and the full Pine
engine (language, runtime, Script Manager with editor and live preview, chart plots, `request.*`, alert subscriptions).

Stage 10 — parity hardening and cutover — is in progress. egui + native accessibility labels are
landed; keyboard commands, performance/energy, packaging/signing, the full behaviour checklist and
the release candidate remain. A saved-layout controller is also in flight as a Stage-6 backfill.

Nothing has had a manual GUI pass yet — see `SMOKE_TEST.md`.

## Build and run

Install Rust and make sure `cargo` is on your `PATH`:

```bash
cargo test
cargo run
```

Build a Finder-launchable app bundle:

```bash
./tools/bundle.sh
open target/DegenView.app
```

The current slice fetches public Binance REST data and therefore needs network access.

