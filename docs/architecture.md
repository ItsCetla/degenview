# DegenView Rust Architecture

A staged Rust rewrite of the native macOS DegenView dashboard. This document is the
Rust-side analogue of the Swift app's `docs/architecture.md`: it maps the same
features onto the Rust module layout and describes how data flows through it.

## Project structure

```text
src/
├── main.rs                          # Entry: eframe options, Dock icon, store open
├── app.rs                           # Hidden coordinator: tab lifecycle, session, host wiring
├── format.rs                        # Price and timestamp presentation
├── native_bridge.rs                 # AppKit menus, tab grouping, native actions
├── workspace.rs                     # One chart workspace: grid, charts, drawings, replay
├── alert_host.rs                    # In-app price-alert runtime host
├── paper_host.rs                    # Paper-trading host: engine <-> SQLite <-> panel views
├── pine_host.rs                     # Pine runtime host for applied chart scripts
├── pine_subscription_host.rs        # Pine alert-subscription runtime host
├── portfolio_host.rs                # Portfolio host + dedicated Portfolio tab state
├── webhook_host.rs                  # Settings Webhooks host: keychain, transport, actions
├── mcp_host.rs                      # MCP host: listener lifecycle, credentials, chart projections
├── mcp/                             # Local MCP server (the only async runtime; no egui types)
│   ├── mod.rs                       # McpServer: loopback listener on its own thread
│   ├── transport.rs                 # axum + rmcp Streamable HTTP, limits, Host/Origin checks
│   ├── auth.rs                      # Bearer tokens: CSPRNG, SHA-256 verifier, request guard
│   ├── state.rs                     # Shared clients/permissions/status read per request
│   ├── bridge.rs                    # Tool worker -> app-thread request queue
│   ├── tools.rs                     # degenview_* tools + per-call permission check
│   └── types.rs                     # Tool result shapes
├── portfolio_import_flow.rs         # CSV/CMC portfolio import orchestration
├── model/
│   ├── alert.rs                     # Price-alert domain: rules, assets, currencies, commands
│   ├── alert_engine.rs              # Pure price-crossing state machine
│   ├── alert_feed.rs                # Alert quote feed
│   ├── alert_message.rs             # Trigger message text
│   ├── alert_notification.rs        # Notification payload
│   ├── appearance.rs                # ChartAppearance: candle colours and styles
│   ├── mcp.rs                       # MCP settings, clients, Permission/PermissionSet
│   ├── drawings.rs                  # Anchor, Drawing (trend/fib/brush/ruler), Tool
│   ├── indicators.rs                # EMA, RSI, Bollinger, Supertrend series + config
│   ├── instrument.rs                # InstrumentId: source-qualified market identity
│   ├── kline.rs                     # Kline OHLCV value
│   ├── layout.rs                    # ChartColumn, ChartConfig, ChartLayout, LayoutSnapshot
│   ├── paper.rs                     # Paper accounts, orders, fills, positions, quotes
│   ├── paper_engine.rs              # Paper execution engine (order/fill/position state machine)
│   ├── paper_format.rs              # Paper number/label formatting
│   ├── paper_ticket.rs              # Order-ticket draft and decimal input parsing
│   ├── persisted.rs                 # PersistedChart: what a saved workspace stores per chart
│   ├── pine_alert.rs                # PineAlertSubscription + creation blockers
│   ├── pine_instance.rs             # PineInstance: one applied script + its inputs
│   ├── portfolio.rs                 # Portfolios, transactions, holdings, snapshots, quotes
│   ├── portfolio_accounting.rs      # Pure weighted-average basis and P&L
│   ├── portfolio_csv.rs             # Native and CoinMarketCap CSV import/export
│   ├── portfolio_editor.rs          # Transaction editing helpers
│   ├── portfolio_format.rs          # Portfolio currency/quantity formatting
│   ├── portfolio_ledger.rs          # Serialized ledger mutation surface
│   ├── portfolio_statistics.rs      # Derived stats (best/worst, 24h, extremes, fees)
│   ├── replay.rs                    # ReplayEngine, status, interval, speed, timeline
│   ├── saved_view.rs                # SavedView: named dashboard snapshot
│   ├── source.rs                    # DataSource enum (eight providers)
│   ├── timeframe.rs                 # Timeframe and its Binance interval token
│   ├── watchlist.rs                 # Watchlist, sections, instruments, flags, layout engine
│   ├── watchlist_layout.rs          # Watchlist layout engine
│   ├── watchlist_quote.rs           # Quote value and Freshness
│   ├── watchlist_text.rs            # Watchlist import/export text
│   ├── webhook.rs                   # WebhookEndpoint + delivery-attempt domain
│   └── webhook_policy.rs            # URL/header policy, {{secret}} placement, redaction
├── pine/                            # Pine Script language, runtime, and broker
│   ├── lexer.rs                     # Lexer (+ tokens, source positions, limits)
│   ├── token.rs                     # Token, TokenKind, Diagnostics, ValueType
│   ├── ast.rs                       # Expressions, statements, operators, traversal
│   ├── parser.rs                    # Parser (declarations, statements, expressions)
│   ├── compiler.rs                  # Source -> CompiledProgram (fold, inputs, checks, linker)
│   ├── program.rs                   # Compiled program, declaration, inputs, bar events, visuals
│   ├── structure_validator.rs       # Structural checks
│   ├── type_checker.rs              # Type inference and checking
│   ├── type_constructor.rs          # Type construction helpers
│   ├── builtin_types.rs             # Builtin type tables
│   ├── symbol_catalog.rs            # Builtin variables/constants/functions/namespaces
│   ├── session.rs                   # Bar-by-bar interpreter state machine
│   ├── state.rs                     # Per-bar runtime state
│   ├── broker.rs                    # Backtest broker for strategy() scripts
│   ├── strategy.rs                  # Strategy mode support
│   ├── outputs.rs                   # VisualOutput: renderer-neutral visuals + alerts
│   ├── visuals.rs                   # Typed visual primitives
│   ├── runtime_value.rs             # Runtime values
│   ├── runtime_types.rs             # Runtime type wrappers
│   ├── runtime_support.rs           # Session support helpers
│   ├── ta.rs                        # ta.* builtins (atr, rsi, ema, …)
│   ├── math.rs                      # Pure math builtins
│   ├── strings.rs                   # String builtins
│   ├── array_builtins.rs            # array.* builtins
│   ├── matrix_builtins.rs           # matrix.* builtins
│   ├── map_builtins.rs              # map.* builtins
│   ├── scalar_builtins.rs           # Scalar builtins
│   ├── strategy_builtins.rs         # strategy.* builtins
│   ├── alert_builtins.rs            # alert/alertcondition builtins
│   ├── alert_call.rs                # Call-site discovery for alert()
│   ├── alert_frequency.rs           # Frequency constants
│   ├── operators.rs                 # Operator handlers
│   ├── containers.rs                # Collection helpers
│   ├── number_format.rs             # tostring() / number formatting
│   ├── time.rs                      # time()/timestamp()/timenow builtins
│   ├── timestamp.rs                 # Timestamp parsing/formatting
│   ├── calendar.rs                  # Calendar math
│   ├── trading_session.rs           # Session-open determination
│   ├── security_call.rs             # request.security / request.securities
│   ├── library_call.rs              # import resolution and library linkage
│   ├── drawings.rs                  # Pine line/label/table drawing primitives
│   ├── market_values.rs             # OHLC/volume symbol values
│   ├── identifier_rules.rs          # Identifier validation
│   ├── style_overrides.rs           # Chart style overrides
│   ├── regex_lite.rs                # Minimal regex for Pine patterns
│   ├── ignored_features.rs          # Documented unsupported-feature surfaces
│   ├── builtins.rs                  # Builtin registration
│   ├── corpus_tests.rs              # Pine corpus/regression runner
│   └── mod.rs                       # Re-exports: Lexer, Parser, Compiler, TypeChecker, Token
├── script_manager/                  # The Script Manager window
│   ├── mod.rs                       # ScriptManager: sidebar + editor + preview + drawer
│   ├── store.rs                     # ScriptStore: file-backed library, metadata, drafts
│   ├── model.rs                     # LocalScript, ScriptId, ScriptType, compile record
│   ├── registry.rs                  # LibraryRegistry for import resolution
│   ├── sidebar.rs                   # Script list sidebar
│   ├── editor.rs                    # Line-numbered code editor state
│   ├── edit_ops.rs                  # Editing operations
│   ├── completion.rs                # Autocomplete engine
│   ├── signature.rs                 # Signature help
│   ├── find.rs                      # Find bar
│   ├── syntax.rs                    # Token colours / syntax classification
│   ├── preview.rs                   # Preview chart state
│   ├── drawer.rs                    # Inputs / Report / Problems drawer
│   ├── inputs.rs                    # input.* control rendering
│   ├── sheets.rs                    # New / delete / create-alert sheets
│   ├── name.rs                      # Script-name validation
│   ├── prefs.rs                     # Layout preferences (split, chart position)
│   ├── demo.rs                      # Example-script seeding
│   └── workspace.rs                 # PreviewState and split workspace
├── service/
│   ├── mod.rs                       # MarketSource trait, for_source dispatch, helpers
│   ├── binance.rs                   # Binance REST klines + search
│   ├── coinbase.rs                  # Coinbase REST candles (folded intervals) + search
│   ├── coingecko.rs                 # CoinGecko OHLC + search
│   ├── dexscreener.rs               # DEXScreener pair discovery/search
│   ├── geckoterminal.rs             # GeckoTerminal DEX-pair OHLCV
│   ├── alpaca.rs                    # Alpaca stock bars + search
│   ├── polymarket.rs                # Polymarket search + probability history
│   ├── kalshi.rs                    # Kalshi series index + search + candles
│   ├── cmc.rs                       # CoinMarketCap client and index charts
│   ├── http.rs                      # Shared HTTP transport (ureq)
│   ├── cache.rs                     # In-memory kline cache with per-source TTL
│   ├── rate_limit.rs                # Minimum-gap rate limiter
│   ├── work_queue.rs                # Thread pools for blocking requests (general, images)
│   ├── update_check.rs              # Settings ▸ About release check: GitHub latest release vs running version
│   ├── tag_signature.rs             # Release tag must be GitHub-verified and tagged by the maintainer
│   ├── self_update.rs               # In-app update: verify tag, download, checksum, unpack, swap bundle, relaunch
│   ├── disk_cache.rs                # On-disk cache files: atomic writes, stats, prune, clear
│   ├── lookup_cache.rs              # Persisted keyed lookups with TTL (JSON per namespace)
│   ├── catalog.rs                   # Provider symbol lists: memory + disk, stale on error
│   ├── image_loader.rs              # egui bytes loader: memory → disk → network for logos
│   ├── cache_control.rs             # Launch setup, cache stats and Clear Cache
│   ├── ws.rs                        # WebSocket connect, health, reconnect backoff
│   ├── live.rs                      # BinanceLiveFeed: combined kline stream
│   ├── icon.rs                      # Multi-source artwork lookup, persisted answers
│   ├── fx_rates.rs                  # Frankfurter FX + Bitstamp BTC history
│   ├── timezone.rs                  # Timezone helpers
│   ├── file_dialog.rs               # osascript-backed save/open dialogs
│   ├── keychain_api_keys.rs         # Alpaca/CMC secrets in Keychain
│   ├── keychain_webhook_secrets.rs  # Webhook secret in Keychain
│   ├── macos_notifications.rs       # macOS notification delivery
│   ├── webhook_delivery.rs          # Webhook HTTP delivery + claim rows
│   ├── asset_auto_mapper.rs         # Currency-pair asset resolution
│   ├── portfolio_quotes.rs          # Batched per-source portfolio quotes
│   └── watchlist_quotes/            # Watchlist quote coordinator, book, provider, socket
│       ├── mod.rs                   # Coordinator (per-window demand, merged polling)
│       ├── book.rs                  # WatchlistQuoteBook
│       ├── engine.rs                # Polling loop
│       ├── provider.rs              # LiveProvider + per-source parsers
│       └── coinbase_ws.rs           # Shared Coinbase ticker socket
├── store/
│   ├── mod.rs                       # Store: SQLite schema, document/setting helpers
│   ├── import.rs                    # One-way Swift SQLite import (run_once)
│   ├── watchlist.rs                 # WatchlistStore
│   ├── saved_view.rs                # Saved-layout persistence
│   ├── portfolio.rs                 # Portfolio ledger tables
│   ├── paper.rs                     # Paper-trading tables
│   ├── alert.rs                     # Price-alert snapshot, command queue, seen stamp
│   ├── pine.rs                      # Pine alert-subscription documents
│   └── webhook.rs                   # Webhook endpoints + delivery history
└── ui/
    ├── mod.rs                       # Re-exports: ChartCard, DrawingFrame, ReplayChartFrame
    ├── theme.rs                     # AppTheme, palette, colour tokens
    ├── kit.rs                       # Shared component chrome
    ├── sheet.rs                     # Sheet chrome
    ├── popover.rs                   # Popover helper
    ├── split.rs                     # Resizable two-pane SplitContainer
    ├── icons.rs                     # Icon glyph set
    ├── accessibility.rs             # egui + native accessibility labels
    ├── chart_card.rs                # Card header, candlesticks, axes, crosshair
    ├── chart_indicator_strip.rs     # Header chips + indicator/script add menu
    ├── chart_settings.rs            # Chart settings sheet (indicators/scripts/appearance)
    ├── drawing_layer.rs             # Interactive drawing layer (gestures, hit-test)
    ├── drawing_render.rs            # Painter helpers for drawings
    ├── cmc_gauge.rs                 # CMC gauge rendering
    ├── empty_state.rs               # Empty states
    ├── grid_drop.rs                 # Column-aware chart drag/drop
    ├── layout_prompts.rs            # Saved-layout name prompt
    ├── tool_sidebar.rs              # Crosshair / trend-line / fib / brush / ruler tools
    ├── workspace_toolbar.rs         # Chart-workspace toolbar
    ├── add_chart_sheet.rs           # Add Chart search sheet (crypto/stocks/predictions/CMC)
    ├── watchlist_sidebar.rs         # Watchlist sidebar
    ├── watchlist_rows.rs            # Watchlist rows
    ├── watchlist_style.rs           # Watchlist styling
    ├── alert_banner.rs              # Global trigger banner
    ├── alert_center.rs              # Alerts window (price + script tabs)
    ├── alert_editor.rs              # Price-alert editor
    ├── pine_alert_editor.rs         # Create Script Alert sheet
    ├── script_alert_list.rs         # Scripts tab of the alerts center
    ├── replay_bar.rs                # Replay control bar
    ├── replay_date_sheet.rs         # Replay date/time picker
    ├── replay_overlay.rs            # Replay chart overlay
    ├── paper_grid.rs                # Paper panel layout
    ├── paper_header.rs              # Paper panel header
    ├── paper_tables.rs              # Paper positions/orders/history/journal tables
    ├── paper_order_ticket.rs        # Order ticket sheet
    ├── paper_account_sheet.rs       # Create/reset account sheet
    ├── paper_chart_trading.rs       # Position/order chart markers
    ├── paper_form.rs                # Paper form fields
    ├── portfolio_overview.rs        # Portfolio overview tab
    ├── portfolio_holdings.rs        # Holdings table
    ├── portfolio_transactions.rs    # Transactions table
    ├── portfolio_transaction_editor.rs # Transaction editor
    ├── portfolio_asset_detail.rs    # Asset detail sheet
    ├── portfolio_asset_picker.rs    # Asset picker
    ├── portfolio_charts.rs          # Portfolio value/allocation charts
    ├── portfolio_cmc_import.rs      # CMC import step 1
    ├── portfolio_import_preview.rs  # Import step 2
    ├── portfolio_sheets.rs          # Portfolio sheets
    ├── portfolio_table.rs           # Shared table chrome
    ├── pine_chart_render.rs         # Pine chart layer rendering
    ├── pine_geometry.rs             # Pine drawing geometry
    ├── pine_report.rs               # Strategy report
    ├── pine_settings_popover.rs     # Per-instance Inputs/Style popover
    ├── pine_style_view.rs           # Pine style view
    ├── settings_window.rs           # Settings window shell
    ├── settings_card.rs             # Settings card row
    ├── settings_mcp.rs              # Settings ▸ MCP Server
    ├── settings_about.rs            # Settings ▸ About
    ├── settings_api_keys.rs         # Settings ▸ API keys
    ├── settings_appearance.rs       # Settings ▸ Appearance
    ├── settings_storage.rs          # Settings ▸ Storage (cache sizes, Clear Cache)
    ├── webhook_editor.rs            # Webhook endpoint editor
    ├── webhook_picker.rs            # Webhook picker
    ├── webhook_settings.rs          # Settings ▸ Webhooks
    └── source_logo.rs               # Source badge
tools/
├── bundle.sh                        # Finder-launchable macOS .app bundle
└── pine-corpus/                     # Pine corpus runner
```

## Concurrency model

There is no async runtime outside `mcp/` (see [MCP server](#mcp-server)). The app is single-threaded for UI and delegates
blocking work to background threads that report back through `std::sync::mpsc`
channels:

- `service::work_queue` is a small shared thread pool for blocking, replaceable
  REST requests. A chart's fetch submits a closure that sends `Result<Vec<Kline>, String>`
  back over an mpsc channel and requests a repaint; a fetch-generation counter
  (`fetch_generation`) makes a stale response ignorable.
- Live feeds own their threads: `BinanceLiveFeed` reconnects with backoff and pushes
  `(symbol, kline)` onto a receiver; `WatchlistQuoteCoordinator` and the Coinbase
  watchlist socket do the same for quotes.
- Shared app-wide state is `Arc<Mutex<…>>`. The `SharedStore` (SQLite), the
  watchlist store, the portfolio/paper/alert/webhook/Pine hosts, and the alert/settings
  UI states are all `Arc<Mutex>` behind `DegenViewApp`. Per-tab content is an
  `Arc<Mutex<TabContent>>` inside a `TabSlot`.
- The store mutex is only ever held for short database operations; `PortfolioHost`
  and `PaperHost` compute candidate snapshots outside the lock and persist them inside
  one transaction.

## MCP server

A local Model Context Protocol server, **off by default**, controlled from Settings ▸ MCP Server.
It uses `rmcp` (Streamable HTTP, served statelessly) on `axum`, with a current-thread tokio runtime
confined to one `mcp-server` thread; nothing else in the app is async.

```text
MCP client ──HTTP──▶ 127.0.0.1:<port>/mcp
   └─ bearer guard (auth.rs) ─ Host/Origin/body-size checks (rmcp) ─ concurrency + deadline
        └─ tools.rs: call_tool ── permission check (state.rs, read fresh every call)
             └─ bridge.rs: Request{client, permission, command} ──queue──▶ app thread
                  └─ DegenViewApp::tick_mcp ── Request::run re-checks, then executes
                       └─ ChartWorkspace::{chart_summaries, add_chart_for}  (the UI's own path)
```

- **Loopback only.** The listener binds `127.0.0.1`; `Host` must be `127.0.0.1:<port>` or
  `localhost:<port>`, and any `Origin` header (a web page) is refused. Nothing is answered
  without a valid token, not even a 404.
- **Credentials.** A token is 256 random bits (`dvmcp_<hex>`). Only its SHA-256 verifier is stored
  (`mcp_client` document table); the token is shown once in Settings. Verifiers are compared in
  constant time. Revoking removes the row and the next request fails.
- **Permissions.** `model::mcp::Permission` (`charts.read`, `charts.create`, `charts.indicators.write`, `alerts.read`, `alerts.create`,
  `alerts.edit`, `alerts.pause`, `alerts.delete`, `pine.scripts.list`, `pine.scripts.read`,
  `pine.scripts.write`, `pine.scripts.save`, `pine.diagnostics.read`, `pine.inputs.read`, `pine.styles.read`, `pine.reports.read`, `pine.alerts.read`) — reading and each mutation are separate; deleting is its own grant. A client can do
  what its own grants *and* the global cap allow; both are read fresh per call, `tools/list` is
  filtered per client, and `call_tool` re-checks regardless. A new credential gets `charts.read`
  only; the global cap defaults to read-only.
- **App-thread execution.** Tool workers never touch workspaces or SQLite. A `Request` carries a
  deadline, a stop-generation and a cancel switch; `Request::run` skips it if abandoned, expired,
  queued before a stop, or the client lost the permission meanwhile — so a timed-out call is never
  executed later. A request the app already started reports its real outcome.
- **Settings run under the store lock**, so `McpHost` only *queues* their actions
  (`McpHost::queue`); `tick_mcp` applies them each frame with no other lock held.
- **Fail closed.** If saved MCP settings or credentials cannot be decoded the server stays off and
  the page says why; nothing is rewritten.
- **Protocol.** `rmcp` 3.5 (MCP `2025-03-26` … `2026-07-28`); the server runs stateless
  (`legacy_session_mode = false`, JSON responses), so every request is authorized on its own.

Tools: `degenview_list_workspaces`, `degenview_list_charts`, `degenview_add_chart`,
`degenview_list_indicators`, `degenview_add_indicator`, `degenview_list_price_alerts`,
`degenview_create_price_alert`, `degenview_update_price_alert`, `degenview_pause_price_alert`, `degenview_resume_price_alert`,
`degenview_delete_price_alert`, and the read-only Pine tools: `degenview_list_pine_scripts`,
`degenview_get_pine_script`, `degenview_update_pine_script`, `degenview_save_pine_script`,
`degenview_compile_pine_script`, `degenview_get_pine_inputs`,
`degenview_list_chart_pine_instances`, `degenview_get_pine_instance`, `degenview_get_pine_styles`,
`degenview_get_pine_report`, `degenview_list_pine_alerts`. The indicator catalog is `model::BuiltIn` — the
same list the chart's indicator menu uses (each indicator once per chart; only EMA takes a period, one
of `EMA_PERIODS`; Volume is refused on line charts) — and `ChartWorkspace::apply_indicator` is the one
mutation path. Price alerts go through the alert runtime's own
command queue (`alert_command`): the app thread plans the change from `AlertHost` (`mcp_host::plan_*`,
built with `PriceAlertEditorState` so parsing and arming match the sheet; `commit_price_alert` is the
sheet's Save), queues `Save`/`SetState`/`Delete`, and whichever process owns the runtime applies it. A
tool then briefly polls (`settle`) so its reply says what is true (`created`, `paused`, `deleted`) or
`queued`. Alert ids are strings (they exceed 2^53). Edits (`plan_update_alert`) go through the
same editor state: only the given settings change, the market cannot, webhook settings are carried over,
a paused alert stays paused, a fired once-alert goes live again only if its condition, currency or
frequency changed, and an edit that would duplicate another alert is a `CONFLICT`. Only
`crosses_above`/`crosses_below` can be created or re-levelled for now (percentage alerts take a new
frequency or note); percentage conditions need a live reference price. Webhook endpoints are never exposed or
settable. Pine tools come in two kinds. **Script-level** tools (`mcp/pine.rs`) run on a worker thread against
`ScriptStore` (the owner of script files) and the real compiler, at most two at a time: saved and draft
text are distinct and never substituted for each other (a draft equal to the saved text is not a draft),
a compile is a pure function of the source (no run, no record, no save), and inputs come straight from
`CompiledProgram::input_schema`. No file path ever leaves the module and script ids are validated as
plain id characters. **Instance-level** tools (instances, styles, report) go through the app-thread
queue because a chart's run state is private to its `ChartCard`: `ChartWorkspace::pine_instance_views`
returns what the chart has computed, never runs anything, and a script that has not run (its chart not
drawn yet) is reported as `not_run` / an `available: false` result with a reason — never a stale or
invented report. Reports are the chart's own `StrategyReport` numbers with the context that produced
them (instrument, timeframe, bar count, inputs, source hash) and a `script_status` against the saved
script; trades are paged (max 200) and the equity curve is thinned. DegenView stores no log of fired
script alerts, so `degenview_list_pine_alerts` returns subscriptions only
(`event_history_available: false`).

**Editing Pine source** is two separate tools and two permissions (`pine.scripts.write` for drafts,
`pine.scripts.save` for the saved source; neither is implied by reading). `update_pine_script` writes
only the *draft* (`ScriptStore::save_draft`): the saved source, charts running the script and alert
subscriptions are untouched and nothing runs. `save_pine_script` promotes the draft through
`ScriptStore::save` (the editor's own Save: a revision is recorded) and refuses a draft with compile
errors (`COMPILATION_FAILED`, draft kept). Both use optimistic concurrency: `expected_revision` is the
SHA-256 `source_hash` of the text the caller read and must equal the *working text* — the open Script
Manager editor's live text if the script is open (it can be ahead of anything on disk), else the
draft, else the saved source — otherwise `CONFLICT`, nothing written, and the reply names the current
revision. The check, the write and the push into the open editor
(`ScriptManager::adopt_external_text`) all happen on the app thread, so no frame can type in between;
the editor then shows the change as typed text ("— Edited") with its diagnostics recomputed. Edits are
capped at 200,000 characters and must be text (no control characters other than tab/newline).

Workspace ids
are the persisted tab ids (`ChartWorkspace::id`); Portfolio and Script Manager tabs share that id
space and are rejected as chart targets. A chart uses its workspace's timeframe (timeframe is
workspace-wide), and adding a market the workspace already charts returns that chart.

The Settings ▸ MCP Server page (`ui/settings_mcp.rs`) keeps itself short: status and port, Security
(with the *maximum permissions* row), a Credentials card to generate one, and one row per client. A
client row and the maximum each open the same **permissions sheet** — presets (Read only, Everything,
None) and the switches grouped under Charts, Price Alerts and Pine Scripts, each with a one-line
description — so seventeen switches never sit in the page. Presets are one write
(`McpAction::SetClientPermissions` / `SetGlobalPermissions`). Every button and text field is 28 pt
(`CONTROL_HEIGHT`) so a field and its button always line up; a test pins this and the height of the
credential card.

What is still planned for the MCP server is in [mcp-remaining-work.md](mcp-remaining-work.md).

## Data flow

1. `main` opens the default store, builds `eframe::NativeOptions` with the Dock icon,
   and runs `DegenViewApp` in the root (hidden) viewport. Each visible workspace is a
   real peer `NSWindow` created by `native_bridge`, so `NSWindow.tabGroup` remains the
   authority for tab layout. `DegenViewApp` owns the `Vec<TabSlot>`; a slot wraps
   either a `ChartWorkspace`, a `PortfolioTab`, or a `ScriptManager`.
2. Each chart owns its `Chart` state: symbol, `DataSource`, candles, fetch result, and
   a `ChartCard` view. Tradable charts fetch `Kline` through the `MarketSource` trait
   selected by `for_source(source)`; CoinMarketCap cards carry typed index points
   (Fear & Greed / Altcoin Season) and never force them into OHLCV.
3. Indicator values are computed over the full candle buffer and trimmed to the visible
   tail before the `egui::Painter` renderer draws them. Volume bars, RSI strip, EMA,
   Bollinger, and Supertrend flips render from `IndicatorSeries` values.
4. Binance's combined kline stream updates the forming candle in place via
   `BinanceLiveFeed`; a five-second refresh path covers other sources and recovery.
   `merge_fetched_candles` lets a live bar (or a later REST snapshot) win over an older
   response. Hidden or occluded workspaces suspend their feeds.
5. Candle responses are cached by source, symbol, and interval with per-source TTLs
   (`service::cache`). CoinGecko and GeckoTerminal requests share per-host
   `RateLimiter`s; blocking requests run on `service::work_queue`.
   Regenerable network data also persists in the OS cache folder (see below).
6. User data lives in one SQLite database, `degenview.sqlite` in Application Support
   (`~/Library/Application Support/com.degenview.rust/degenview.sqlite`), opened through
   `store::Store` (`rusqlite`, bundled). Document tables are `id/position/payload`
   JSON rows; nested models evolve through serde defaults instead of schema changes.
   `setting` and `window_group` tables carry keyed values and window grouping. The schema
   is a single idempotent `create_schema`; there are no migrations. `replace_documents`
   refuses to rewrite a collection containing a row this version cannot decode, so corrupt
   data is never silently replaced by an empty value. The Swift app's data is brought over
   once by `store::import::run_once`, gated per table on the Rust counterpart being empty.
7. Portfolio and paper trading persist whole snapshots: `PortfolioLedger` and
   `PaperTradingEngine` each publish a complete `…Snapshot`, and the host persists it in
   one transaction so a mid-write failure rolls everything back.
8. During replay, each chart retains its immutable canonical history and exposes only a
   binary-searched prefix (`displayed_candles`) through `ReplayEngine`. The engine owns
   the workspace's sole timestamp and one playback step; timeline position, `step_backward`
   and `seek` derive from its in-memory timeline, not from `PersistedWorkspace`.
9. Portfolio mutations are serialized by `PortfolioLedger`, persisted as one transaction,
   and replayed by `portfolio_accounting` (pure weighted-average basis and P&L). Quote
   ticks update live valuation without replaying static accounting; historical edits
   invalidate only the affected snapshot suffix. `PortfolioQuoteFeed` refreshes prices
   one batched call per source; daily history snapshots and `FxRateService` (Frankfurter
   fiat + Bitstamp BTC) convert into the reporting currency once and cache the projection.
10. Paper trading is a `PaperTradingEngine` state machine over `PaperTradingSnapshot`.
    `PaperHost` persists and publishes every candidate state — including a domain-error
    result for a rejected order — while refusing all writes after a failed initial load.
    `PaperPanel` holds the per-window presentation over that app-wide host.
11. Price alerts are evaluated by the pure `AlertEngine` (a crossing state machine that
    takes `now_ms` from the caller) inside `AlertHost`, the in-app owner. The host loads
    the persisted snapshot, drains the `alert_command` queue, and exposes the evaluated
    snapshot. There is no separate login-item agent yet. Every new trigger the app learns
    of updates the unseen count behind the sidebar bell; a trigger banner renders over
    every workspace. The sheet that creates a price alert opens over the chart window that
    asked for it (and holds a quote demand while open); only Edit opens it in the Alerts
    window.
12. Pine scripts run through `pine::compiler::compile` — `Lexer` → `Parser` →
    `Compiler` (constant folding, input collection, `structure_validator`, `type_checker`,
    library linking) — into a `CompiledProgram`, which `pine::session::Session` interprets
    bar by bar. `pine_host::run_script` runs a chart's applied instances over its bars and
    returns the renderer-neutral `VisualOutput`; `pine_subscription_host` runs each active
    subscription's source over its symbol's bars and returns the `alert()` events that
    fired. A `strategy()` broker emulator is a value inside the session's per-bar state, so
    realtime rollback restores its orders, positions, and trades with everything else.
    `request.*` resolves through `pine_host::KlineSecurityDataProvider` (blocking REST with
    a ~30 s candle cache); library `import`s resolve through the Script Manager's
    `LibraryRegistry`.
13. The Script Manager is a file-backed library (`Scripts/*.pine` + `ScriptMetadata/`,
    id in an extended attribute) with a sidebar, a line-numbered editor, and a preview
    chart split left/top/bottom, plus an Inputs / Report / Problems drawer. A source edit
    recompiles and feeds diagnostics back to the editor; drafts persist separately from
    last-saved source so invalid edits do not remove the active result.
14. Webhooks are opt-in, global, reusable endpoints that alerts reference by id. Price
    alerts and Pine alerts keep their own pipelines and share only configuration and
    transport. `webhook_policy` owns the URL/header rules and redaction; `webhook_delivery`
    claims a `webhook_delivery` row before the request goes out, so one trigger reaches one
    endpoint once even across processes. The secret lives only in Keychain
    (`keychain_webhook_secrets`) and is substituted as `{{secret}}` at request build time.
    Delivery is currently osascript-backed rather than a native HTTP client.
15. Drawing state is `Drawing` (trend line, Fibonacci, brush, or ruler) with time+price
    anchors, persisted per chart; rulers are transient and never persisted or undone. The
    interactive layer (`drawing_layer`) handles gestures, hit-testing, and draft/commit;
    `drawing_render` projects financial coordinates through the current plot each frame.
16. Settings and the alerts window run in their own native windows; `DegenViewApp`
    synchronises their viewports and routes their actions back into the shared hosts.

## Undo and redo

Drawing history is session-only and scoped to the workspace where an edit originated.
Each chart carries its drawings in the `Chart` state; `ChartWorkspace` keeps `undo` and
`redo` stacks of `UndoEntry { chart_id, drawings }` — the full drawing list of one chart
captured before a batch of drawing commands ran. Applying an inverse restores only that
chart's captured list through the chart's own state, never a whole workspace snapshot.
Rulers are excluded (`is_ruler`) because they are transient. Crosshairs never enter
history. `⌘Z` / `⇧⌘Z` are handled through the native bridge's Edit-menu actions.

## Replay flow

`ReplayEngine` owns the workspace's sole timestamp, one playback step, and the status.
Each chart retains its immutable canonical history; the visible prefix is derived by
binary search over `open_time` (`displayed_candles`) rather than by rewriting candles.
The replay bar (open/select start, transport, timeline slider, speed + interval) drives
the engine; a start is picked on-chart or via the date sheet. Return-to-live and Esc
cancel selection. Granular intraday replay data is deferred: all intervals behave like
the chart's own candle bars until a granular `fetch_replay_klines` lands on the service
trait.

## Pine execution

```text
source text
    │
    ▼
Lexer ──► Parser ──► Compiler (constant folding, inputs, structure validator, type checker,
    │                     library linking) ──► CompiledProgram
    │                                             │
    └── LibraryRegistry (Script Manager library scripts)  │
                                                          ▼
                                                   Session (bar-by-bar interpreter)
                                                          │
                              ┌───────────────────────────┼──────────────────────────┐
                              ▼                           ▼                          ▼
                     VisualOutput (chart)          strategy() Broker (per-bar)   alert() events
```

The session's call router maps each builtin name or namespace to one handler in a
per-family module (`math`, `strings`, `array_builtins`, `strategy_builtins`, `ta`, …);
the pure modules hold no session state. `run_script` runs a chart instance's source once
over history and returns the visual output; `run_script_alerts` replays the last bar as a
realtime close so `alert()`/`alertcondition()` calls fire, and returns the fired alerts.

## Webhook delivery flow

```
Settings ▸ Webhooks ── store/webhook.rs ──┬─ webhook_endpoint (SQLite: name, URL + header templates)
                                          └─ keychain_webhook_secrets (Keychain: the one secret)

price alert triggers                         Pine alert() / alertcondition()
  AlertEngine → AlertHost (in-app owner)       Session → run_script_alerts → PineSubscriptionHost
  → webhook_delivery claim                     → webhook_delivery claim
        └─────────────── one row per trigger + endpoint ───────────────┘
                                      ▼
                       webhook_delivery (osascript-backed HTTP; no retry, no redirects)
                       webhook_policy: {{secret}} → URL (percent-encoded) / headers (raw)
                                      ▼
              webhook_delivery result (status, duration, typed error; no URL or body stored)
```

## Cache

Everything that can be downloaded again lives in the OS cache folder, not in the data folder
(`store::cache_dir`: `~/Library/Caches/com.degenview.rust`, `%LOCALAPPDATA%\com.degenview.rust\Cache`,
`$XDG_CACHE_HOME` or `~/.cache`; `DEGENVIEW_CACHE_DIR` overrides it). Nothing in it is user data:
a missing, unreadable or undecodable file is a miss, never an error, and Settings ▸ Storage can
delete all of it.

```text
images/<aa>/<sha256-of-url>   logo bytes                       (service::image_loader)
lookups/<namespace>.json      icon_url, coin_index, catalog_*   (service::lookup_cache)
fx/*.json                     FX and BTC history                (service::fx_rates)
```

- **Images.** `image_loader::CachedImageLoader` is registered in front of egui's HTTP loader, so
  every `egui::Image::new("https://…")` is served from memory, then disk (read synchronously, so a
  relaunch paints logos on the first frame), then the network on a dedicated worker pool. A copy
  older than 30 days is shown and refreshed in the background; the body must sniff as an image
  before it is stored; a failed download retries after three minutes.
- **Which logo.** `icon` persists each market's resolved artwork URL (30 days; a clean miss one
  day; a failed lookup never) and the CoinGecko top-250 snapshot (one day, including coin names),
  so a normal launch makes no CoinGecko calls for logos.
- **Search lists.** Binance, Coinbase and Alpaca have no search endpoint, only a full list.
  `catalog::Catalog` downloads it once a day, searches it locally, and serves the previous list
  if a refresh fails.
- **Bounds.** `disk_cache::prune` runs ten seconds after launch: images over 90 days go, then the
  oldest until the folder is under 75% of 128 MB. Lookup entries expire after 90 days.
- **Clearing.** `cache_control::clear` deletes the three folders above (only those), bumps the
  cache generation so downloads that were in flight drop their writes, empties every in-memory
  layer, and tells egui to forget its textures. Settings ▸ Storage runs it off the UI thread.
- Candles (`service::cache`) are volatile market data and stay in memory only.
