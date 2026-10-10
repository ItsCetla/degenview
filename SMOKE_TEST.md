# Manual GUI smoke test

One pass over the app on screen, before the Stage 10 cutover. Nothing here needs a test
framework — launch the app, click through, and tick what behaves. Each line is one action
followed by the expected result.

Stages 1–6 were GUI-accepted in Swift and re-checked as the Rust port landed. **Stages 7, 8
and 9 are the ones nobody has GUI-tested** — prioritize those.

---

## Stage 1 — Chart card

- [ ] Search "BTCUSDT" on Binance, add it → a candle chart draws with grid, axes and a last-price marker.
- [ ] Add "BTC-USD" from Coinbase → candles draw (Coinbase is a different instrument, not a duplicate).
- [ ] Add "BTC" from CoinGecko → candles draw (same symbol, different source is not rejected).
- [ ] Switch timeframe 1m → 1h → 1d → the axis labels and candle spacing change; the chart re-fetches.
- [ ] Toggle log scale → the y-axis switches to log spacing.
- [ ] Drag the chart left/right → candles pan; the price axis tracks the visible range.
- [ ] Scroll on the chart → the candle count zooms in/out (scroll-zoom).
- [ ] Hover the chart → a crosshair shows the bar's OHLC and the price follows the pointer.
- [ ] A Coinbase chart with live ticks → the last candle's body moves on each trade.

## Stage 2 — Tabs and windows

- [ ] With one tab open, the tab bar and its `+` are still visible.
- [ ] ⌘T (and the `+`) opens a new empty tab named "Unnamed" — never a second view onto an existing tab.
- [ ] Give two tabs different timeframes → neither follows the other; scrolling one doesn't zoom the other.
- [ ] Drag a tab out to detach it → it becomes its own window; drag it back onto the bar to merge.
- [ ] Quit and relaunch → same tabs, same order, same window grouping.

## Stage 3 — Dashboard

- [ ] Add several charts → they lay out in columns; drag a card to reorder.
- [ ] Remove a chart with `×` → it closes without disturbing the others.
- [ ] Click a chart → a faint accent ring shows the focused card.
- [ ] The tool rail (crosshair, trend line, Fib, pen, ruler) sits with the toolbar.
- [ ] Toggle the watchlist split → the sidebar shows/hides; the chart grid re-flows.

## Stage 4 — Market data

- [ ] The search sheet lists Binance, Coinbase, CoinGecko, DEXScreener, Alpaca, Polymarket, Kalshi, CMC.
- [ ] Search "BONK" on DEXScreener → a DEX pair adds and draws.
- [ ] Coin icons resolve in card headers; a symbol without an icon falls back to a monogram (header stays aligned).

## Stage 5 — Drawings and replay

- [ ] Arm the ruler, drag a rectangle up (green) and down (red), right-to-left → the percent matches the price axis and the bar count matches the candles inside.
- [ ] Hover a ruler corner/edge → handles and the move/resize cursor appear; drag to resize/move.
- [ ] Arm the brush, drag a circle, a V and a zigzag → corners stay sharp, curves smooth; a click leaves a dot.
- [ ] ⌘Z removes a whole brush stroke, ⇧⌘Z brings it back.
- [ ] Switch timeframe or scroll-zoom → drawings stay on the same price action.
- [ ] Delete removes the selected drawing; Esc cancels a draft.
- [ ] The Replay button opens the bar; click a candle → the card gets an orange border + "Replay" badge.
- [ ] Play/pause/step the replay; "Live" returns to the latest bar.

## Stage 6 — Watchlists and persistence

- [ ] Relaunch keeps old favorites in "Favorites", in order.
- [ ] Create a list, add BINANCE:BTCUSDT / BINANCE:ETHUSDT / COINBASE:BTC-USD → rows show price + Chg%.
- [ ] Add sections; drag symbols between sections; a heading reorders with its section.
- [ ] Sort by Chg% (click, click to flip, click again for Manual) → manual order returns.
- [ ] Filter "btc" and the flag filter → the panel narrows; drag is off while filtered.
- [ ] Hide the sidebar or minimise the window → quote requests stop.
- [ ] ⋯ ▸ Export to File then Import Symbols → round-trips with a skipped-rows summary.

## Stage 7 — Portfolio and paper trading

- [ ] Open the Portfolio tab → overview, holdings, transactions and asset detail render.
- [ ] Add a transaction → the ledger/accounting numbers update.
- [ ] Import a CSV and a CMC export → the preview sheet shows rows before commit.
- [ ] Add a chart, enable paper trading → BUY/SELL pills appear in the header.
- [ ] Place a paper order → a position opens and markers draw on the chart at the fill price.

## Stage 8 — Alerts and webhooks

- [ ] The bell opens the Alerts window with Active / Triggered / Paused / All / History / Scripts tabs.
- [ ] Create a price alert above the current price → it lists under Active.
- [ ] The alert fires (price crosses) → a red count appears on the sidebar bell; opening the window clears it.
- [ ] History groups by day; prices read `$67,432.19` / `$0.00000278` — no long decimal tail.
- [ ] Settings ▸ Webhooks: add a local receiver (`python3 -m http.server`), "Test" → the receiver logs `{"event":"DegenView webhook test"}`.
- [ ] Add a secret via "Use secret" and a `?token={{secret}}` query → the receiver sees the percent-encoded token; a Bearer header shows it raw.
- [ ] A price alert with two webhooks (one dead) → the banner reports `Webhooks 1/2 delivered`.

## Stage 9 — Pine

- [ ] The Scripts toolbar button opens the Script Manager tab (singleton — focuses an existing one); the first launch seeds the nine example scripts (or copies the Swift app's scripts when its folder exists).
- [ ] Sidebar: search narrows by name or source; sections Favorites / Indicators / Strategies / Libraries collapse; ★ favorites; a slow second click renames inline (Return commits, Esc cancels); the context menu has Favorite, Rename, Duplicate, Show in Finder, Export…, Delete (confirms, moves the file to the Trash).
- [ ] New Script: the sheet rejects empty, `.`-leading, `/ \ :` names and duplicates with the Swift messages; Create selects the new script.
- [ ] Edit a `.pine` file in another editor → the list picks the change up within a couple of seconds.
- [ ] Type in the editor → the chip updates at once (Not compiled / No problems / N warnings / N errors); the preview chart takes the new source after a pause; unsaved edits show "— Edited" in the title and survive a relaunch as a draft; ⌘S saves.
- [ ] Break the syntax → red underline with a hover message, the stale banner over the chart, the last good plot stays; clicking a Problems row jumps to and flashes the spot.
- [ ] A script that fails at run time (for example one calling an unsupported function) shows "Runtime failed" in the drawer bar and the failure under Problems; the last good output stays on the chart.
- [ ] Editing keys: ⌘/ toggles comments, ⌥↑/⌥↓ move lines, ⇧⌥↑/⇧⌥↓ duplicate, Return keeps/opens indentation, Tab/⇧Tab indent/outdent, `( [ " '` pair and wrap a selection.
- [ ] Toolbar: ⌃⌘S hides the list, ⌥⌘P hides the chart, the position menu moves it left/above/below code; dragging a divider resizes, double-click resets; all of it survives a relaunch.
- [ ] Preview market button opens the picker (Crypto / Stocks, search, Recent); the timeframe control reloads; scroll zooms; Stocks without Alpaca keys explains and offers Open Settings.
- [ ] Inputs tab lists the script's `input.*` cards by group; changing one reruns the chart; "Reset to Defaults" clears the overrides; values come back after a relaunch.
- [ ] Report tab: a strategy shows its metrics, equity curve and trades; an indicator that raises alerts lists them under a collapsed Alerts box.
- [ ] Right-click a script ▸ "Add to Chart" applies it to the focused chart (until the chart settings get their Scripts tab).

---

## Notes

- Stages 1–6 map to the Swift acceptance gates and should already pass; re-tick on a fresh checkout.
- Stage 9 Pine has known gaps: editor completion, signature help and the find bar are not ported,
  and the chart settings have no Scripts tab — don't test those.
- Webhook delivery is osascript-backed, so the notification shows as coming from Script Editor.
