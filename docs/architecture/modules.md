# Modules

The important boundary is ownership, not a complete file list. Use `rg --files src` for the current
inventory.

## Coordinators

| Module | Owns |
| --- | --- |
| `main.rs` | Store opening, native options, and app startup |
| `app.rs` | Hidden coordinator, tabs, session, hosts, and native-window actions |
| `workspace.rs` | One chart tab: grid, charts, drawings, fetches, and replay |
| `tab_window_host.rs` | Window grouping, active tabs, drag/reorder/detach |
| `native_bridge.rs` | AppKit menus, window behavior, cursor, Keychain/accessibility boundaries |
| `update_host.rs` | Update checks, install progress, and restart state |
| `portfolio_import_flow.rs` | CSV/CoinMarketCap import orchestration |

Feature hosts (`alert_host`, `paper_host`, `pine_host`, `pine_subscription_host`, `portfolio_host`,
`portfolio_card_host`, `webhook_host`, `snapshot_host`, `mcp_host`) bridge pure engines, persistence, workers, and UI.

## Feature directories

| Directory | Contents |
| --- | --- |
| `model/` | Alerts, charts/canvas/types, drawings, indicators, instruments, layouts, paper/portfolio engines, replay, tabs, watchlists, and webhooks |
| `pine/` | Lexer → parser → compiler → session, builtins, strategies, alerts, drawings, libraries, `request.*`, and corpus tests |
| `script_manager/` | File-backed scripts, editor operations/diff/completion, preview, inputs, reports, preferences, and demos |
| `service/` | Market providers, HTTP/WebSocket work, updater, caches, images/icons, FX, notifications, files, Keychain, quotes, and webhooks |
| `store/` | Schema/helpers plus watchlist, layout/template, portfolio, paper, alert, Pine, and webhook persistence |
| `ui/` | Shared chrome plus chart, drawing, watchlist, alert, replay, paper, portfolio, Pine, settings, and webhook renderers |
| `mcp/` | Loopback HTTP transport, auth, shared permission state, app bridge, tools, and wire types |

## Boundaries

- `model/` and `pine/` stay pure and egui-free.
- UI renders supplied state; it does not own network or persistence.
- `service/` performs external I/O; `store/` alone owns SQLite details.
- Top-level hosts are the stateful seams. Do not bypass them from workers or views.
- `tools/bundle.sh` builds the macOS app; `tools/pine-corpus/` holds the optional corpus runner.
