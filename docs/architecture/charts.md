# Charts, drawings, and replay

See [chart types](../chart-types.md), [chart canvas](../chart-canvas.md), and
[portfolio cards](../portfolio-cards.md) for feature rules.

## Canvas and templates

```text
Canvas controls → ChartAppearance.canvas → CanvasStyle::resolve → painters
                                      └→ model::canvas geometry/ticks
Templates → ChartTemplateStore → ChartCard::apply_template
Apply to all → plan_apply_to_all → each chart's ChartCard
```

Canvas state is saved with charts/layouts and included in layout fingerprints. It is presentation
only: it cannot change candles, constructed bars, indicators, or Pine execution. Shared pure
geometry owns ticks, price range, visible window, and pane splits.

A template contains appearance only—type/settings, colors, and canvas. It excludes markets,
indicators, scripts, drawings, and alerts. `ChartCard::apply_template` preserves data and routes type
changes through `set_chart_type`.

## Drawings and undo

`Drawing` holds trend, Fibonacci, brush, or ruler anchors. Rulers and crosshairs are transient.
`ChartWorkspace` keeps session-only per-workspace undo/redo entries containing one chart id and its
complete pre-command drawing list. Undo restores only that chart. Native Edit-menu actions dispatch
⌘Z/⇧⌘Z.

## Replay

`ReplayEngine` owns the workspace timestamp, playback step, and status. Charts retain immutable
canonical history and expose a binary-searched prefix through `displayed_candles`; persistence never
stores timeline position. The replay bar controls selection, transport, seek, speed, and interval.
Esc/Return to live cancel selection. Until granular replay fetching exists, each interval replays its
own bars.
