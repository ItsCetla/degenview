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

`Drawing` holds the original trend line, Fibonacci, brush and ruler plus one `Shape` variant that
carries a typed `Geometry` for every newer tool, all with time+price anchors. The interactive layer
(`ui::drawing_layer`) is one controller driven by each tool's `Placement`; it reports
`DrawingCommand`s and never mutates drawings. See [drawing tools](../drawing-tools.md).
Rulers and crosshairs are transient.

`ChartWorkspace` keeps session-only per-workspace undo/redo entries containing one chart id and its
complete pre-command drawing list. Undo restores only that chart. A batch that changes nothing is no
step, and restyles in quick succession merge into one. ⌘Z/⇧⌘Z are egui key events handled in
`ChartWorkspace::handle_keyboard` (there is no native Edit menu) and are skipped while a text field
has the keyboard.

## Replay

`ReplayEngine` owns the workspace timestamp, playback step, and status. Charts retain immutable
canonical history and expose a binary-searched prefix through `displayed_candles`; persistence never
stores timeline position. The replay bar controls selection, transport, seek, speed, and interval.
Esc/Return to live cancel selection. Until granular replay fetching exists, each interval replays its
own bars.
