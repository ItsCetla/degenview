# Chart guidance

Use [chart types](../chart-types.md), [chart canvas](../chart-canvas.md), and
[portfolio cards](../portfolio-cards.md) for detailed behavior.

## Rendering and prices

- `ui::chart_card::ChartCard` owns the painter loop. `ui::series_render` draws every main series;
  `model::chart_look` decides bar appearance. Do not add type-specific drawing to `chart_card.rs`.
- Chart state lives in `workspace::Chart`. Draw grid, main series, overlays/indicators, then labels.
  The price transform supports linear and log scales.
- Every chart price uses `ChartAppearance::price_format()` and `PriceFormat`, never
  `format::price`. This includes axes, tags, crosshair/OHLC, ruler/Fibonacci, and high/low markers.
- Automatic precision follows the Swift app and counts leading zeros below `0.001` (`0.0₅278`).
  `axis_decimals` (0–12) and `axis_small_prices` stay chart-local across template application.
  Grid ticks use `format_tick`; `ui::price_text` paints the Unicode subscript count. Other screens
  may use `format::price`; watchlist prices use `PriceFormat::AUTO`.

## Types, canvas, and templates

- Never modify canonical candles. `model::synthetic` builds price-based types; the cached
  `constructed_series` feeds scale, indicators, Pine, alerts, drawings, and replay. Standard types
  keep real OHLC for scripts.
- Type and settings have one persistence path: `ChartAppearance::style`. New fields need
  `#[serde(default)]`. Cosmetic restyling must not change `synthetic::Construction`.
- Builders are pure and prefix-preserving because replay depends on it. Add a fixture for changed
  construction rules and document anything unverified against TradingView.
- Pine's `plotcandle`, `plotbar`, and `barcolor` remain independent layers ordered by
  `PineInstance::visual_order`.
- `ChartAppearance::canvas` is presentation-only. Canvas changes must not alter candles, rebuild
  constructed bars, or rerun indicators/Pine. New colors are `Option<Rgba>` so `None` follows the
  theme.
- Resolve canvas/theme once per frame with `CanvasStyle::resolve`; painters must not fetch canvas
  colors from `theme::*`. Grid and axis labels share `canvas::{price_ticks,time_ticks}`. Keep price
  range, visible-window, and pane-split geometry pure in `model/canvas.rs`.
- Pane order and collapse live on `PineInstance`; maximize is card view state. Buttons report
  clicks and `ChartCard` applies them.
- Templates snapshot `ChartAppearance`. Apply with `ChartCard::apply_template`, and plan all
  apply-to-all changes before applying any, so type changes pass through `set_chart_type`.
  `ChartTemplateStore` re-reads before writes and refuses to overwrite undecodable rows.

## Portfolio cards

- A portfolio card is a `workspace::Chart` with `card: Some(...)`; its symbol is a key and source is
  an unread placeholder. `portfolio_card::CardKind` is not Pine's `ChartKind`. Every market-data
  loop must skip non-market cards via `Chart::is_market()`: fetching, live feeds, paper,
  scripts/alerts, replay, MCP, apply-to-all, and layout previews. `Chart::fetch` is a no-op for cards.
- `PortfolioCardEnv::view` alone reads ledgers, quotes, and history. Painters accept `CardData` and
  never access hosts/network. Each `CardRuntime` owns and drops a sibling quote feed demand.
- Favorites use `WatchlistInstrument.card` and `PortfolioCard::instrument_id()`. The card field,
  not provider, controls display/open/export behavior. Opening focuses or adds the card; it never
  replaces a market in the focused chart.
- Top Performers ranks each market's candles without FX and scales today's value by return,
  assuming unchanged quantity.
