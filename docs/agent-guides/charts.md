# Chart guidance

Use [chart types](../chart-types.md), [chart canvas](../chart-canvas.md),
[drawing tools](../drawing-tools.md), and [portfolio cards](../portfolio-cards.md) for detailed
behavior.

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

## Navigation

- `model::viewport::ChartViewport` is the only chart view state (bars, fractional offset from the
  newest bar, manual price range). Gestures live in `ui/chart_card/navigation.rs` and edit it;
  never keep another offset or range in a painter, the drawing layer or Pine rendering.
- The window is continuous, so bars at the plot's edge may be cut: clip what you draw to the plot
  and place bars with `Window::lead`, never an integer count of blank slots.
- A pan must not rewrite drawings, emit `DrawingCommand`s, rerun scripts (only a script reading
  `chart.*_visible_bar_time` depends on the range) or write the Canvas. Viewport state is
  chart/layout state and is not part of a template.
- See [chart navigation](../chart-navigation.md) for gestures, history paging and what is unverified.

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

## Drawings

- A drawing is pinned to time+price. The four original tools keep their serialized form; every
  newer tool is `Drawing::Shape { geometry: Geometry, style, text, locked, hidden, visibility }`.
  New fields need `#[serde(default)]`. Nothing stores a pixel or a derived corner.
- A tool is one row of `Tool::spec()` (`Placement`, `Caps`) and `Tool::default_style()`. Add a
  tool there, a `Geometry` variant, its `shape::shape` projection and an icon; the controller,
  toolbar, dialog and tests (`every_tool_…` loop over `Tool::GROUPED`) pick it up. Do not add
  `match tool` to pointer handling.
- Geometry lives in `ui/drawing_geometry.rs` as pure functions on `Pos2` with their own tests.
  Polygons fill by the even-odd rule; translucent freehand strokes are painted from a coverage map
  (`rasterize_stroke`), never overlapping triangles.
- The layer never mutates drawings: it returns `DrawingCommand`s in the `DrawingFrame`, and
  `drawings::apply_commands` applies them. One batch is one undo step; use `Style` for restyles
  (merged in bursts), never one command per pointer move. Previews and cancelled drafts are not
  commands.
- `PersistedChart.drawings` decodes leniently: a row this version cannot read stays as
  `Drawing::Unreadable` and is written back untouched. Skip it in anything that draws, picks or
  edits.
- Drawings are main-pane only and use the bars the chart draws (constructed and replay-cut); never
  index candles directly.

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
