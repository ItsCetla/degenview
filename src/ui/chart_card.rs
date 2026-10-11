use crate::format;
use crate::model::paper::PaperOrderSide;
use crate::model::pine_instance::{PineInstance, StoredInput};
use crate::model::{
    BollingerSeries, ChartAppearance, DataSource, IndicatorConfig, IndicatorSeries, Kline,
};
use crate::pine::program::{InputValue, SymbolInfo, VisualOutput};
use crate::pine::token::Diagnostic;
use crate::pine_host;
use crate::ui::chart_indicator_strip;
use crate::ui::drawing_layer::{DrawingFrame, DrawingLayer, Projection};
use crate::ui::drawing_render::TimeIndex;
use crate::ui::icons::{self, Icon};
use crate::ui::paper_chart_trading::{self, PaperCardState};
use crate::ui::pine_chart_render::{CandleStyle, PineLayer};
use crate::ui::replay_overlay::{self, ReplayChartFrame};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2,
};
use std::collections::HashMap;
use std::sync::Arc;

const PRICE_AXIS_WIDTH: f32 = 82.0;
const TIME_AXIS_HEIGHT: f32 = 24.0;
const HEADER_HEIGHT: f32 = 45.0;
const MIN_VISIBLE_CANDLES: usize = 20;

/// The header change uses the same candle window as the plot, including pan and zoom.
fn visible_price_change(
    candles: &[Kline],
    visible_count: usize,
    right_offset: usize,
) -> Option<f64> {
    let end = candles.len().saturating_sub(right_offset);
    let start = end.saturating_sub(visible_count);
    let visible = candles.get(start..end)?;
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return None;
    };
    if visible.len() < 2 || first.close == 0.0 {
        return None;
    }
    let change = (last.close - first.close) / first.close * 100.0;
    change.is_finite().then_some(change)
}

/// Half the width of a candle body, matching the real candles the Pine overlay recolours.
fn candle_body_half_width(slot_width: f32) -> f32 {
    (slot_width * 0.34).clamp(1.0, 8.0)
}

/// Signals a chart card emits while it is being drawn. The owning workspace
/// applies them after the frame, so rendering never mutates layout mid-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChartCardAction {
    pub focus_requested: bool,
    pub settings_requested: bool,
    pub indicator_action: Option<chart_indicator_strip::Action>,
    /// A header SELL / BUY pill was clicked: open the order ticket on that side.
    pub quick_trade: Option<PaperOrderSide>,
    /// The header bell was clicked: start a new price alert for this chart's market.
    pub new_alert: bool,
    pub toggle_favorite: bool,
    pub watchlist_change: Option<(u64, bool)>,
}

pub struct WatchlistMembership {
    pub id: u64,
    pub name: String,
    pub contains_market: bool,
}

/// The shared crosshair a tab draws across all its charts. Stored as a fraction
/// of plot width so the same column lines up on every card.
#[derive(Clone, Copy, Debug)]
pub struct CrosshairInfo {
    pub owner_id: u64,
    pub x_fraction: f64,
    pub price: f64,
}

/// What a chart reports back while drawing, plus whether its plot was clicked.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChartPointer {
    pub clicked: bool,
    pub hover_x_fraction: Option<f64>,
    pub hover_price: Option<f64>,
}

pub struct ChartCard {
    appearance: ChartAppearance,
    visible_candles: usize,
    right_offset: usize,
    drag_origin_offset: Option<usize>,
    y_zoom: f64,
    y_zoom_drag_start: Option<f64>,
    layer: DrawingLayer,
    /// Paper-trading inputs and outputs for this card; the workspace fills and drains it.
    pub paper: PaperCardState,
    /// The Pine scripts applied to this chart, in order. Overlay scripts draw over the candles;
    /// the others share a pane under them.
    instances: Vec<PineInstance>,
    /// What each instance's last run produced, by instance id.
    runs: HashMap<u64, Run>,
}

/// The instance the Script Manager's single-script preview chart runs.
const PREVIEW_INSTANCE: u64 = 0;

/// One instance's last run, cached so the card does not recompile every frame.
struct Run {
    source: String,
    inputs: HashMap<String, StoredInput>,
    bar_count: usize,
    output: Arc<VisualOutput>,
    /// The runtime failure that stopped this run early.
    error: Option<Diagnostic>,
    /// The last run that finished without a runtime failure: what stays on screen while a newer
    /// source fails to run.
    last_valid: Option<Arc<VisualOutput>>,
}

/// How one instance is doing, for the chart settings' Scripts tab.
pub struct InstanceStatus<'a> {
    pub output: &'a VisualOutput,
    pub error: Option<&'a Diagnostic>,
}

impl Default for ChartCard {
    fn default() -> Self {
        Self {
            appearance: ChartAppearance::default(),
            visible_candles: 120,
            right_offset: 0,
            drag_origin_offset: None,
            y_zoom: 1.0,
            y_zoom_drag_start: None,
            layer: DrawingLayer::default(),
            paper: PaperCardState::default(),
            instances: Vec::new(),
            runs: HashMap::new(),
        }
    }
}

impl ChartCard {
    pub fn appearance(&self) -> ChartAppearance {
        self.appearance
    }

    pub fn appearance_mut(&mut self) -> &mut ChartAppearance {
        &mut self.appearance
    }

    pub fn set_appearance(&mut self, appearance: ChartAppearance) {
        self.appearance = appearance;
    }

    fn bullish_color(&self) -> Color32 {
        Color32::from_rgb(
            self.appearance.bullish[0],
            self.appearance.bullish[1],
            self.appearance.bullish[2],
        )
    }

    fn bearish_color(&self) -> Color32 {
        Color32::from_rgb(
            self.appearance.bearish[0],
            self.appearance.bearish[1],
            self.appearance.bearish[2],
        )
    }

    fn axis_price(&self, value: f64) -> String {
        format::price_with_decimals(value, self.appearance.axis_decimals)
    }

    pub fn set_y_zoom(&mut self, y_zoom: f64) {
        self.y_zoom = y_zoom;
    }

    pub fn y_zoom(&self) -> f64 {
        self.y_zoom
    }

    /// The scroll-zoom candle count (visible candles).
    pub fn visible_candles(&self) -> usize {
        self.visible_candles
    }

    /// Restores the scroll-zoom candle count. `0` means "keep the default".
    pub fn set_visible_candles(&mut self, count: usize) {
        if count > 0 {
            self.visible_candles = count.clamp(5, 2000);
        }
    }

    // MARK: Pine instances

    pub fn pine_instances(&self) -> &[PineInstance] {
        &self.instances
    }

    /// Replaces every instance (restoring a saved chart).
    pub fn set_pine_instances(&mut self, instances: Vec<PineInstance>) {
        self.instances = instances;
        self.runs.clear();
    }

    /// Applies a script to the chart, after the ones already there. Returns the new instance's id.
    pub fn add_pine_instance(
        &mut self,
        script_id: Option<String>,
        name: &str,
        source: &str,
    ) -> u64 {
        let id = self
            .instances
            .iter()
            .map(|i| i.id)
            .max()
            .map_or(1, |max| max + 1);
        self.instances
            .push(PineInstance::new(id, script_id, name, source));
        id
    }

    pub fn remove_pine_instance(&mut self, id: u64) {
        self.instances.retain(|instance| instance.id != id);
        self.runs.remove(&id);
    }

    /// Moves an instance to `index` (clamped); draw order follows list order.
    pub fn move_pine_instance(&mut self, id: u64, index: usize) {
        if let Some(from) = self.instances.iter().position(|i| i.id == id) {
            let instance = self.instances.remove(from);
            let to = index.min(self.instances.len());
            self.instances.insert(to, instance);
        }
    }

    pub fn set_pine_visible(&mut self, id: u64, visible: bool) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.visible = visible;
        }
    }

    /// Sets an instance's input overrides; its next run uses them.
    pub fn set_pine_instance_inputs(&mut self, id: u64, inputs: &HashMap<String, InputValue>) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.set_input_values(inputs);
        }
    }

    pub fn pine_status(&self, id: u64) -> Option<InstanceStatus<'_>> {
        self.runs.get(&id).map(|run| InstanceStatus {
            output: &run.output,
            error: run.error.as_ref(),
        })
    }

    // The single-script preview (Script Manager): one instance, id 0, that keeps its last good
    // output while a newer source fails to run.

    /// Sets (or clears) the one script the preview chart runs.
    pub fn set_pine_source(&mut self, source: Option<String>) {
        match source {
            Some(source) => match self.instances.first_mut() {
                Some(instance) if instance.id == PREVIEW_INSTANCE => instance.source = source,
                _ => {
                    self.instances = vec![PineInstance::new(
                        PREVIEW_INSTANCE,
                        None,
                        "Preview",
                        &source,
                    )];
                    self.runs.clear();
                }
            },
            None => {
                self.instances.clear();
                self.runs.clear();
            }
        }
    }

    /// Sets the input overrides the preview script runs with.
    pub fn set_pine_inputs(&mut self, inputs: HashMap<String, InputValue>) {
        self.set_pine_instance_inputs(PREVIEW_INSTANCE, &inputs);
    }

    /// The preview run's runtime failure, if it stopped early.
    pub fn pine_error(&self) -> Option<&Diagnostic> {
        self.runs.get(&PREVIEW_INSTANCE)?.error.as_ref()
    }

    /// Whether a preview run has finished cleanly, so a later failure leaves real output up.
    pub fn has_valid_pine_output(&self) -> bool {
        self.runs
            .get(&PREVIEW_INSTANCE)
            .is_some_and(|run| run.last_valid.is_some())
    }

    /// The preview run's output (strategy report, alerts), for the Report tab.
    pub fn pine_visual(&self) -> Option<&VisualOutput> {
        self.runs.get(&PREVIEW_INSTANCE).map(|run| &*run.output)
    }

    /// The visible instances' outputs in order, rerunning only an instance whose source, inputs or
    /// bar count changed.
    fn pine_outputs(
        &mut self,
        symbol_info: &SymbolInfo,
        candles: &[Kline],
    ) -> Vec<Arc<VisualOutput>> {
        let mut outputs = Vec::new();
        for instance in self.instances.iter().filter(|i| i.visible) {
            let fresh = self.runs.get(&instance.id).is_some_and(|run| {
                run.source == instance.source
                    && run.inputs == instance.inputs
                    && run.bar_count == candles.len()
            });
            if !fresh {
                let (output, error) = pine_host::run_script_checked(
                    &instance.source,
                    candles,
                    symbol_info,
                    &instance.input_values(),
                );
                let previous_valid = self
                    .runs
                    .get(&instance.id)
                    .and_then(|run| run.last_valid.clone());
                let output = Arc::new(output);
                let (shown, last_valid) = match (&error, previous_valid) {
                    (None, _) => (Arc::clone(&output), Some(Arc::clone(&output))),
                    // A run that fails leaves the last good output on screen.
                    (Some(_), Some(valid)) => (Arc::clone(&valid), Some(valid)),
                    (Some(_), None) => (Arc::clone(&output), None),
                };
                self.runs.insert(
                    instance.id,
                    Run {
                        source: instance.source.clone(),
                        inputs: instance.inputs.clone(),
                        bar_count: candles.len(),
                        output: shown,
                        error,
                        last_valid,
                    },
                );
            }
            if let Some(run) = self.runs.get(&instance.id) {
                outputs.push(Arc::clone(&run.output));
            }
        }
        outputs
    }
}

/// The own pane under the candles for `overlay=false` scripts, ported from `PineScriptPaneView`:
/// a value axis fitted to the script's outputs, sharing the price chart's horizontal layout so
/// bar slots line up with the candles above (the time axis stays up there).
#[allow(clippy::too_many_arguments)]
fn draw_script_pane(
    ui: &egui::Ui,
    painter: &egui::Painter,
    pane: Rect,
    price_plot: Rect,
    pine: &VisualOutput,
    shown: &[Kline],
    slot_width: f32,
    x_for_index: &dyn Fn(i64) -> f32,
) {
    // Top and bottom leave just enough room for the outermost axis labels.
    let plot = Rect::from_min_max(
        Pos2::new(price_plot.left(), pane.top() + 8.0),
        Pos2::new(price_plot.right(), pane.bottom() - 8.0),
    );
    if plot.height() <= 1.0 {
        return;
    }
    let (low, high) = crate::ui::pine_geometry::pane_value_range(pine, shown.len(), 0.1);
    let span = (high - low).max(f64::EPSILON);
    let y_for = |value: f64| plot.bottom() - ((value - low) / span) as f32 * plot.height();
    painter.line_segment(
        [pane.left_top(), pane.right_top()],
        Stroke::new(1.0, theme::grid()),
    );
    for step in 0..=3 {
        let fraction = step as f32 / 3.0;
        let y = plot.top() + plot.height() * fraction;
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, theme::grid()),
        );
        let value = high - (high - low) * f64::from(fraction);
        painter.text(
            Pos2::new(pane.right() - 7.0, y),
            Align2::RIGHT_CENTER,
            format::price(value),
            FontId::monospace(10.0),
            ui.visuals().weak_text_color(),
        );
    }
    let layer = PineLayer {
        pine,
        candles: shown,
        in_pane: true,
        plot,
        slot_width,
        x_for_index,
        y_for: &y_for,
        candle_style: CandleStyle {
            body_width: candle_body_half_width(slot_width) * 2.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        },
    };
    let clipped = painter.with_clip_rect(plot.expand2(Vec2::new(0.0, 0.0)));
    layer.draw_background(&clipped);
    layer.draw_foreground(&clipped);
}

/// Whether a chart on `source` can raise price alerts: markets with a price quote feed.
pub fn supports_alerts(source: DataSource) -> bool {
    source != DataSource::CoinMarketCap && !source.is_prediction_market()
}

pub(super) struct PriceScale {
    pub(super) minimum: f64,
    pub(super) maximum: f64,
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) logarithmic: bool,
}

impl PriceScale {
    fn project(&self, price: f64) -> f64 {
        if self.logarithmic {
            price.max(f64::MIN_POSITIVE).ln()
        } else {
            price
        }
    }

    pub(super) fn y(&self, price: f64) -> f32 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction =
            ((self.project(price) - minimum) / (maximum - minimum).max(f64::EPSILON)) as f32;
        self.top + self.height * (1.0 - fraction)
    }

    pub(super) fn price(&self, y: f32) -> f64 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction = 1.0 - f64::from((y - self.top) / self.height);
        let projected = minimum + fraction * (maximum - minimum);
        if self.logarithmic {
            projected.exp()
        } else {
            projected
        }
    }
}

/// The visible slice of the data and the scales that map it onto the plot.
struct View {
    start: usize,
    end: usize,
    scale: PriceScale,
    slot_width: f32,
    blank_slots: usize,
}

impl View {
    fn projection<'a>(&'a self, plot: Rect, candles: &'a [Kline]) -> Projection<'a> {
        Projection {
            plot,
            scale: &self.scale,
            times: TimeIndex::new(candles),
            start: self.start,
            slot_width: self.slot_width,
            blank_slots: self.blank_slots,
        }
    }
}

impl ChartCard {
    /// Visible range and price scale for the current pan/zoom, if anything is shown.
    fn view(&self, candles: &[Kline], plot: Rect, log_scale: bool) -> Option<View> {
        let end = candles.len().saturating_sub(self.right_offset);
        let start = end.saturating_sub(self.visible_candles);
        let shown = &candles[start..end];
        if shown.is_empty() {
            return None;
        }
        let low = shown
            .iter()
            .map(|candle| candle.low)
            .fold(f64::MAX, f64::min);
        let high = shown
            .iter()
            .map(|candle| candle.high)
            .fold(f64::MIN, f64::max);
        let padding = (high - low).max(f64::EPSILON) * 0.08;
        let (mut minimum, mut maximum) = (
            if log_scale { low * 0.97 } else { low - padding },
            if log_scale {
                high * 1.03
            } else {
                high + padding
            },
        );
        if self.y_zoom != 1.0 {
            let mid = (minimum + maximum) / 2.0;
            let half = (maximum - minimum) / 2.0 / self.y_zoom;
            minimum = mid - half;
            maximum = mid + half;
        }
        Some(View {
            start,
            end,
            scale: PriceScale {
                minimum,
                maximum,
                top: plot.top(),
                height: plot.height(),
                logarithmic: log_scale,
            },
            slot_width: plot.width() / self.visible_candles as f32,
            blank_slots: self.visible_candles.saturating_sub(shown.len()),
        })
    }

    pub fn reset_view(&mut self) {
        self.visible_candles = 120;
        self.right_offset = 0;
        self.drag_origin_offset = None;
    }

    /// Header row. Reordering has its own handle so clicking the market title
    /// never moves its icon and text into egui's drag preview layer.
    #[allow(clippy::too_many_arguments)]
    pub fn header(
        &self,
        ui: &mut egui::Ui,
        symbol: &str,
        source: DataSource,
        indicators: &IndicatorConfig,
        candles: &[Kline],
        replaying: bool,
        is_favorite: bool,
        watchlists: &[WatchlistMembership],
        action: &mut ChartCardAction,
        drag_id: u64,
    ) {
        let header_width = ui.available_width();
        let price_change = visible_price_change(candles, self.visible_candles, self.right_offset);
        let reserved = 164.0
            + if self.paper.trading_enabled {
                125.0
            } else {
                0.0
            };
        let title_block_width = (header_width - reserved).max(96.0);
        let title_content_width = title_block_width - 18.0;
        let indicator_space = if chart_indicator_strip::has_items(indicators, self, source) {
            60.0
        } else {
            25.0
        };
        let show_replay_badge = replaying && header_width >= 430.0;
        let header_rect =
            Rect::from_min_size(ui.cursor().min, Vec2::new(header_width, HEADER_HEIGHT));
        let header_hovered = ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pos| header_rect.contains(pos));
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.dnd_drag_source(egui::Id::new(("chart-drag", drag_id)), drag_id, |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 20.0), Sense::hover());
                    let color = theme::text_secondary().gamma_multiply(0.7);
                    for x in [-2.0, 2.0] {
                        for y in [-4.0, 0.0, 4.0] {
                            ui.painter()
                                .circle_filled(rect.center() + Vec2::new(x, y), 1.0, color);
                        }
                    }
                })
                .response
                .on_hover_text("Drag to reorder charts");
                ui.vertical(|ui| {
                    ui.set_width(title_content_width);
                    ui.spacing_mut().item_spacing.y = -3.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if let Some(url) = crate::service::icon::get_url(source, symbol) {
                            if ui
                                .add(
                                    egui::Image::new(url)
                                        .fit_to_exact_size(Vec2::splat(20.0))
                                        .sense(Sense::click()),
                                )
                                .on_hover_text("Chart settings")
                                .clicked()
                            {
                                action.settings_requested = true;
                            }
                        } else {
                            let glyph_color = if source == DataSource::Binance {
                                theme::BITCOIN
                            } else {
                                theme::text_secondary()
                            };
                            if ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(source.glyph())
                                            .color(glyph_color)
                                            .strong()
                                            .size(19.0),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text("Chart settings")
                                .clicked()
                            {
                                action.settings_requested = true;
                            }
                            crate::service::icon::request(source, symbol);
                        }
                        let title_max_width = (title_content_width
                            - 26.0
                            - indicator_space
                            - if show_replay_badge { 72.0 } else { 0.0 })
                        .max(20.0);
                        if ui
                            .scope(|ui| {
                                ui.set_max_width(title_max_width);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(source.display_symbol(symbol))
                                            .size(13.0)
                                            .strong(),
                                    )
                                    .truncate()
                                    .sense(Sense::click()),
                                )
                            })
                            .inner
                            .on_hover_text("Chart settings")
                            .clicked()
                        {
                            action.settings_requested = true;
                        }
                        if show_replay_badge {
                            egui::Frame::new()
                                .fill(theme::REPLAY.gamma_multiply(0.14))
                                .corner_radius(999.0)
                                .inner_margin(egui::Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        let (icon, _) = ui
                                            .allocate_exact_size(Vec2::splat(11.0), Sense::hover());
                                        crate::ui::icons::paint(
                                            ui.painter(),
                                            icon,
                                            crate::ui::icons::Icon::ClockArrow,
                                            theme::REPLAY,
                                        );
                                        ui.label(
                                            egui::RichText::new("Replay")
                                                .size(10.0)
                                                .strong()
                                                .color(theme::REPLAY),
                                        );
                                    });
                                });
                        }
                        action.indicator_action = chart_indicator_strip::show(
                            ui,
                            self,
                            indicators,
                            source,
                            header_hovered,
                        );
                    });
                    if let Some(last) = candles.last()
                        && ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format::headline(
                                        last.close,
                                        source.is_prediction_market(),
                                    ))
                                    .size(11.0)
                                    .color(theme::text_secondary()),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Chart settings")
                            .clicked()
                    {
                        action.settings_requested = true;
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(change) = price_change {
                        let color = if change >= 0.0 {
                            theme::UP
                        } else {
                            theme::DOWN
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {:.2}%",
                                if change >= 0.0 { "↗" } else { "↘" },
                                change.abs()
                            ))
                            .color(color)
                            .size(11.0)
                            .strong(),
                        );
                    }
                    if self.paper.trading_enabled
                        && let Some(last) = candles.last()
                    {
                        let price_text = format::price(last.close);
                        if let Some(side) =
                            paper_chart_trading::quick_trade_buttons(ui, &price_text)
                        {
                            action.quick_trade = Some(side);
                        }
                    }
                    if supports_alerts(source)
                        && icons::button(
                            ui,
                            Icon::BellBadge,
                            27.0,
                            theme::text_secondary(),
                            "Create Price Alert",
                        )
                        .clicked()
                    {
                        action.new_alert = true;
                    }
                    let favorite_icon = if is_favorite {
                        Icon::StarFill
                    } else {
                        Icon::Star
                    };
                    let favorite_color = if is_favorite {
                        theme::text_primary()
                    } else {
                        theme::text_secondary()
                    };
                    let favorite_tip = if is_favorite {
                        "Remove from Favorites"
                    } else {
                        "Add to Favorites"
                    };
                    let favorite =
                        icons::button(ui, favorite_icon, 27.0, favorite_color, favorite_tip);
                    favorite.context_menu(|ui| {
                        ui.label(egui::RichText::new("Add to Watchlist").small().weak());
                        for list in watchlists {
                            let mut included = list.contains_market;
                            if ui.checkbox(&mut included, &list.name).changed() {
                                action.watchlist_change = Some((list.id, included));
                                ui.close();
                            }
                        }
                    });
                    if favorite.clicked() {
                        action.toggle_favorite = true;
                    }
                });
            },
        );
    }

    /// Plot body. Reports the pointer (click + hover) back to the workspace.
    #[allow(clippy::too_many_arguments)]
    pub fn body(
        &mut self,
        ui: &mut egui::Ui,
        symbol: &str,
        candles: &[Kline],
        loading: bool,
        error: Option<&str>,
        log_scale: bool,
        source: DataSource,
        indicators: &IndicatorConfig,
        crosshair: Option<&CrosshairInfo>,
        chart_id: u64,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        let line = source.is_line_chart();
        let symbol_info = SymbolInfo {
            ticker: symbol.to_owned(),
            ticker_id: format!("{}:{}", source.watchlist_slug(), symbol),
            currency: "USD".to_owned(),
            kind: if source == DataSource::Alpaca {
                "stock"
            } else {
                "crypto"
            }
            .to_owned(),
            base_currency: None,
        };
        let mut pointer = ChartPointer::default();
        let available = ui.available_size();
        let size = Vec2::new(available.x.max(1.0), available.y.max(50.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let response = crate::ui::accessibility::label_as(
            response,
            egui::WidgetType::Panel,
            format!(
                "{} {} chart",
                source.display_symbol(symbol),
                source.display_name()
            ),
        );
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, theme::surface());

        if let Some(message) = error {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                message,
                FontId::proportional(13.0),
                theme::DOWN,
            );
            return pointer;
        }
        if candles.is_empty() {
            return pointer;
        }
        if source == DataSource::CoinMarketCap && crate::service::cmc::is_latest(symbol) {
            if let Some(reading) = crate::service::cmc::latest_reading(symbol) {
                let description = crate::ui::cmc_gauge::paint(ui, &painter, rect, symbol, &reading);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, &description)
                });
            }
            return pointer;
        }

        // A script that isn't an overlay (RSI, MACD …) gets a pane under the candles: 30% of the
        // chart's height shared between them, the candles keep the rest and the time axis. On a
        // line chart every script draws over the line instead.
        let outputs = self.pine_outputs(&symbol_info, candles);
        let pane_outputs: Vec<&Arc<VisualOutput>> = outputs
            .iter()
            .filter(|output| !output.overlay && !line)
            .collect();
        let overlay_outputs: Vec<&Arc<VisualOutput>> = outputs
            .iter()
            .filter(|output| output.overlay || line)
            .collect();
        let pane_total = if pane_outputs.is_empty() {
            0.0
        } else {
            (rect.height() * 0.3).round()
        };
        let pane_area_top = rect.max.y - pane_total;
        let rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, pane_area_top));
        let pane_rects: Vec<Rect> = (0..pane_outputs.len())
            .map(|index| {
                let each = (pane_total / pane_outputs.len() as f32).round();
                let top = pane_area_top + each * index as f32;
                let bottom = if index + 1 == pane_outputs.len() {
                    pane_area_top + pane_total
                } else {
                    top + each
                };
                Rect::from_min_max(Pos2::new(rect.min.x, top), Pos2::new(rect.max.x, bottom))
            })
            .collect();
        let axis_width = if self.appearance.axis_decimals.is_some() {
            let extrema = candles
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), candle| {
                    (low.min(candle.low), high.max(candle.high))
                });
            let font = FontId::monospace(10.0);
            [extrema.0, extrema.1]
                .into_iter()
                .map(|price| {
                    painter
                        .layout_no_wrap(
                            self.axis_price(price),
                            font.clone(),
                            theme::text_secondary(),
                        )
                        .size()
                        .x
                })
                .fold(PRICE_AXIS_WIDTH, |width, label| width.max(label + 17.0))
                .min(148.0)
        } else {
            PRICE_AXIS_WIDTH
        };
        let plot = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - axis_width, rect.max.y - TIME_AXIS_HEIGHT),
        );
        replay.picked_ms = None;
        if let Some(view) = self.view(candles, plot, log_scale) {
            if replay.selecting {
                self.layer.clear_hover();
            } else {
                self.layer
                    .update(ui, &response, &view.projection(plot, candles), drawing);
            }
        }
        let allow_pan = !self.layer.claims_drag();
        self.handle_input(ui, &response, plot, candles.len(), allow_pan);
        self.handle_price_axis_drag(&response, plot);

        let Some(view) = self.view(candles, plot, log_scale) else {
            return pointer;
        };
        let shown = &candles[view.start..view.end];
        let (scale, slot_width, blank_slots) = (&view.scale, view.slot_width, view.blank_slots);
        let x_for = |index: usize| {
            plot.left() + slot_width * (index as f32 + 0.5) + slot_width * blank_slots as f32
        };
        let body_half_width = candle_body_half_width(slot_width);
        let x_for_index = |index: i64| {
            plot.left() + slot_width * (index as f32 + 0.5) + slot_width * blank_slots as f32
        };
        let y_for = |price: f64| scale.y(price);
        let candle_style = CandleStyle {
            body_width: body_half_width * 2.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        };
        let pine_layers: Vec<PineLayer> = overlay_outputs
            .iter()
            .map(|pine| PineLayer {
                pine,
                candles: shown,
                in_pane: false,
                plot,
                slot_width,
                x_for_index: &x_for_index,
                y_for: &y_for,
                candle_style,
            })
            .collect();

        self.draw_grid(ui, &painter, plot, rect, scale);
        for layer in &pine_layers {
            layer.draw_background(&painter);
        }
        if line {
            self.draw_line(&painter, shown, &x_for, scale);
        } else {
            // The first script that recolours a bar wins it.
            let recolour =
                |index: usize| pine_layers.iter().find_map(|layer| layer.bar_color(index));
            let bar_color: Option<&dyn Fn(usize) -> Option<Color32>> =
                (!pine_layers.is_empty()).then_some(&recolour);
            self.draw_candles(&painter, shown, slot_width, &x_for, scale, bar_color);
        }

        let drawn_indicators = indicators.drawn();
        let series = IndicatorSeries::compute(candles, view.start, shown.len(), &drawn_indicators);
        if !line && drawn_indicators.show_volume {
            self.draw_volume(&painter, plot, &x_for, slot_width, shown);
        }
        if drawn_indicators.show_ema {
            self.draw_ema(&painter, &x_for, scale, &series.ema);
        }
        if let Some(bands) = &series.bollinger {
            self.draw_bollinger(&painter, &x_for, scale, bands);
        }
        if !series.rsi.is_empty() {
            self.draw_rsi(&painter, plot, &x_for, &series.rsi);
        }
        if !series.trend_flips.is_empty() {
            self.draw_trend_flips(&painter, &x_for, scale, shown, &series.trend_flips);
        }
        for layer in &pine_layers {
            layer.draw_foreground(&painter);
        }

        self.layer
            .paint(&painter, &view.projection(plot, candles), drawing);
        if replay.selecting {
            self.handle_start_pick(ui, &response, &painter, plot, candles, &view, replay);
        }
        if replay.active {
            replay_overlay::paint_active(&painter, plot);
        }

        self.draw_last_price(&painter, plot, rect, scale, candles);
        self.draw_time_axis(ui, &painter, rect, shown, &x_for);
        for (pane_rect, pine) in pane_rects.iter().zip(&pane_outputs) {
            draw_script_pane(
                ui,
                &painter,
                *pane_rect,
                plot,
                pine,
                shown,
                slot_width,
                &x_for_index,
            );
        }

        if let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) {
            pointer.hover_x_fraction = Some(((position.x - plot.left()) / plot.width()) as f64);
            pointer.hover_price = Some(scale.price(position.y));
        }

        if let Some(crosshair) = crosshair {
            self.draw_crosshair(
                ui,
                &painter,
                plot,
                rect,
                crosshair,
                chart_id,
                scale,
                shown,
                slot_width,
                blank_slots,
            );
        }

        self.paper.show_markers(ui, plot, scale);

        pointer.clicked = response.clicked();
        let _ = loading;
        pointer
    }

    /// "Click a candle" mode: marker at the candle under the pointer, click picks it.
    #[allow(clippy::too_many_arguments)]
    fn handle_start_pick(
        &self,
        ui: &egui::Ui,
        response: &Response,
        painter: &egui::Painter,
        plot: Rect,
        candles: &[Kline],
        view: &View,
        replay: &mut ReplayChartFrame,
    ) {
        let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) else {
            return;
        };
        let Some(index) = replay_overlay::candle_at(
            position.x,
            plot.left(),
            view.slot_width,
            view.blank_slots,
            view.start,
            view.end,
        ) else {
            return;
        };
        let candle = &candles[index];
        let x = plot.left()
            + view.slot_width * ((index - view.start) as f32 + 0.5)
            + view.slot_width * view.blank_slots as f32;
        replay_overlay::paint_selection(painter, plot, x, candle.open_time);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if response.clicked() {
            replay.picked_ms = Some(candle.open_time);
        }
    }

    fn handle_price_axis_drag(&mut self, response: &Response, plot: Rect) {
        let axis_rect = Rect::from_min_max(
            Pos2::new(plot.right(), plot.top()),
            Pos2::new(response.rect.right(), plot.bottom()),
        );
        if response.drag_started()
            && response
                .interact_pointer_pos()
                .is_some_and(|pointer| axis_rect.contains(pointer))
        {
            self.y_zoom_drag_start = Some(self.y_zoom);
        }
        if response.dragged()
            && self.y_zoom_drag_start.is_some()
            && let Some(total_delta) = response.total_drag_delta()
        {
            // Up (negative egui y) narrows the slice, so candles grow taller.
            let factor = 2_f64.powf((-total_delta.y as f64) / 120.0);
            self.y_zoom = (self.y_zoom_drag_start.unwrap_or(1.0) * factor).clamp(0.25, 20.0);
        }
        if response.drag_stopped() {
            self.y_zoom_drag_start = None;
        }
    }

    fn handle_input(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        plot: Rect,
        total_candles: usize,
        allow_pan: bool,
    ) {
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = (1.0 - scroll * 0.004).clamp(0.5, 2.0);
                let next = (self.visible_candles as f32 * factor).round() as usize;
                self.visible_candles =
                    next.clamp(MIN_VISIBLE_CANDLES, total_candles.max(MIN_VISIBLE_CANDLES));
            }
        }

        if allow_pan && response.drag_started() {
            self.drag_origin_offset = Some(self.right_offset);
        }
        if allow_pan
            && response.dragged()
            && let Some(total_delta) = response.total_drag_delta()
        {
            let slot_width = plot.width() / self.visible_candles as f32;
            let candle_delta = (total_delta.x / slot_width).round() as isize;
            let origin = self.drag_origin_offset.unwrap_or(self.right_offset) as isize;
            let maximum = total_candles.saturating_sub(MIN_VISIBLE_CANDLES) as isize;
            self.right_offset = (origin - candle_delta).clamp(0, maximum) as usize;
        }
        if response.drag_stopped() {
            self.drag_origin_offset = None;
        }
        self.right_offset = self
            .right_offset
            .min(total_candles.saturating_sub(MIN_VISIBLE_CANDLES));
    }

    fn draw_grid(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
    ) {
        for step in 0..=6 {
            let fraction = step as f32 / 6.0;
            let y = plot.top() + plot.height() * fraction;
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.0, theme::grid()),
            );
            painter.text(
                Pos2::new(rect.right() - 7.0, y),
                Align2::RIGHT_CENTER,
                self.axis_price(scale.price(y)),
                FontId::monospace(10.0),
                ui.visuals().weak_text_color(),
            );
        }

        for step in 0..=8 {
            let x = plot.left() + plot.width() * step as f32 / 8.0;
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0, theme::grid().gamma_multiply(0.65)),
            );
        }

        painter.line_segment(
            [
                Pos2::new(plot.right(), plot.top()),
                Pos2::new(plot.right(), rect.bottom()),
            ],
            Stroke::new(1.0, theme::border()),
        );
    }

    fn draw_candles(
        &self,
        painter: &egui::Painter,
        candles: &[Kline],
        slot_width: f32,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        bar_color: Option<&dyn Fn(usize) -> Option<Color32>>,
    ) {
        let body_half_width = candle_body_half_width(slot_width);
        for (index, candle) in candles.iter().enumerate() {
            let x = x_for(index);
            let color = bar_color.and_then(|color| color(index)).unwrap_or_else(|| {
                if candle.is_up() {
                    self.bullish_color()
                } else {
                    self.bearish_color()
                }
            });
            painter.line_segment(
                [
                    Pos2::new(x, scale.y(candle.high)),
                    Pos2::new(x, scale.y(candle.low)),
                ],
                Stroke::new(1.0, color),
            );
            let open_y = scale.y(candle.open);
            let close_y = scale.y(candle.close);
            let body = Rect::from_min_max(
                Pos2::new(x - body_half_width, open_y.min(close_y)),
                Pos2::new(
                    x + body_half_width,
                    open_y.max(close_y).max(open_y.min(close_y) + 1.0),
                ),
            );
            painter.rect_filled(body, 0.5, color);
        }
    }

    fn draw_line(
        &self,
        painter: &egui::Painter,
        candles: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        if candles.len() < 2 {
            if let Some(candle) = candles.first() {
                painter.circle_filled(
                    Pos2::new(x_for(0), scale.y(candle.close)),
                    2.0,
                    self.bullish_color(),
                );
            }
            return;
        }
        let points: Vec<Pos2> = candles
            .iter()
            .enumerate()
            .map(|(index, candle)| Pos2::new(x_for(index), scale.y(candle.close)))
            .collect();
        for pair in points.windows(2) {
            let color = if pair[1].y <= pair[0].y {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            painter.line_segment([pair[0], pair[1]], Stroke::new(1.5, color));
        }
    }

    fn draw_volume(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        slot_width: f32,
        candles: &[Kline],
    ) {
        let peak = candles
            .iter()
            .map(|candle| candle.volume)
            .fold(0.0, f64::max);
        if peak <= 0.0 {
            return;
        }
        let pane_height = plot.height() * 0.22;
        let pane_bottom = plot.bottom();
        let bar_width = (slot_width * 0.6).clamp(1.0, 8.0);
        for (index, candle) in candles.iter().enumerate() {
            if candle.volume <= 0.0 {
                continue;
            }
            let height = (pane_height * (candle.volume / peak) as f32).max(1.0);
            let x = x_for(index);
            let color = if candle.is_up() {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let bar = Rect::from_min_max(
                Pos2::new(x - bar_width / 2.0, pane_bottom - height),
                Pos2::new(x + bar_width / 2.0, pane_bottom),
            );
            painter.rect_filled(bar, 0.0, translucent(color, 89));
        }
    }

    fn draw_ema(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        values: &[Option<f64>],
    ) {
        let stroke = Stroke::new(1.5, theme::EMA);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    fn draw_bollinger(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        bands: &BollingerSeries,
    ) {
        let mut upper_points = Vec::new();
        let mut lower_points = Vec::new();
        for index in 0..bands.upper.len() {
            if let (Some(upper), Some(lower)) = (bands.upper[index], bands.lower[index]) {
                let x = x_for(index);
                upper_points.push(Pos2::new(x, scale.y(upper)));
                lower_points.push(Pos2::new(x, scale.y(lower)));
            }
        }
        if upper_points.len() < 2 {
            return;
        }

        // Tint down the upper rail, back along the lower one.
        let mut channel = upper_points.clone();
        channel.extend(lower_points.iter().rev().copied());
        painter.add(egui::Shape::Path(egui::epaint::PathShape {
            points: channel,
            closed: true,
            fill: translucent(theme::BOLLINGER, 26),
            stroke: Stroke::NONE.into(),
        }));

        let rail = Stroke::new(1.0, translucent(theme::BOLLINGER, 180));
        for pair in upper_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }
        for pair in lower_points.windows(2) {
            painter.line_segment([pair[0], pair[1]], rail);
        }

        let mut previous: Option<Pos2> = None;
        for (index, value) in bands.middle.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), scale.y(*value));
            if let Some(previous) = previous {
                painter.add(egui::Shape::dashed_line(
                    &[previous, point],
                    Stroke::new(1.0, translucent(theme::BOLLINGER, 115)),
                    4.0,
                    3.0,
                ));
            }
            previous = Some(point);
        }
    }

    fn draw_rsi(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        x_for: &dyn Fn(usize) -> f32,
        values: &[Option<f64>],
    ) {
        if !values.iter().any(|value| value.is_some()) {
            return;
        }
        let pane = Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom() - plot.height() * 0.28),
            plot.right_bottom(),
        );
        let y_for = |reading: f64| pane.bottom() - (reading as f32 / 100.0) * pane.height();

        for level in [crate::model::RSI_OVERBOUGHT, crate::model::RSI_OVERSOLD] {
            let y = y_for(level);
            painter.add(egui::Shape::dashed_line(
                &[Pos2::new(pane.left(), y), Pos2::new(pane.right(), y)],
                Stroke::new(1.0, theme::grid()),
                4.0,
                3.0,
            ));
        }

        let stroke = Stroke::new(1.0, theme::RSI);
        let mut previous: Option<Pos2> = None;
        for (index, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let point = Pos2::new(x_for(index), y_for(*value));
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
        }
    }

    fn draw_trend_flips(
        &self,
        painter: &egui::Painter,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
        candles: &[Kline],
        flips: &[Option<bool>],
    ) {
        let half = 4.0;
        for (index, flip) in flips.iter().enumerate() {
            let Some(bullish) = flip else { continue };
            let Some(candle) = candles.get(index) else {
                continue;
            };
            let x = x_for(index);
            let y = if *bullish {
                scale.y(candle.low) + half + 3.0
            } else {
                scale.y(candle.high) - half - 3.0
            };
            let color = if *bullish {
                self.bullish_color()
            } else {
                self.bearish_color()
            };
            let points = if *bullish {
                vec![
                    Pos2::new(x, y - half),
                    Pos2::new(x - half, y + half),
                    Pos2::new(x + half, y + half),
                ]
            } else {
                vec![
                    Pos2::new(x, y + half),
                    Pos2::new(x - half, y - half),
                    Pos2::new(x + half, y - half),
                ]
            };
            painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        }
    }

    fn draw_last_price(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        scale: &PriceScale,
        candles: &[Kline],
    ) {
        let Some(last) = candles.last() else {
            return;
        };
        let y = scale.y(last.close);
        if !plot.y_range().contains(y) {
            return;
        }
        let color = if last.is_up() {
            self.bullish_color()
        } else {
            self.bearish_color()
        };
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, color.gamma_multiply(0.65)),
        );
        let tag = Rect::from_min_max(
            Pos2::new(plot.right() + 3.0, y - 9.0),
            Pos2::new(rect.right() - 3.0, y + 9.0),
        );
        painter.rect_filled(tag, 3.0, color);
        painter.text(
            tag.center(),
            Align2::CENTER_CENTER,
            self.axis_price(last.close),
            FontId::monospace(10.0),
            Color32::WHITE,
        );
    }

    fn draw_time_axis(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        candles: &[Kline],
        x_for: &dyn Fn(usize) -> f32,
    ) {
        let step = (candles.len() / 6).max(1);
        for (index, candle) in candles.iter().enumerate().step_by(step) {
            let label = format::timestamp(candle.open_time);
            painter.text(
                Pos2::new(x_for(index), rect.bottom() - TIME_AXIS_HEIGHT / 2.0),
                Align2::CENTER_CENTER,
                &label[5..],
                FontId::monospace(9.0),
                ui.visuals().weak_text_color(),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_crosshair(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        plot: Rect,
        rect: Rect,
        crosshair: &CrosshairInfo,
        chart_id: u64,
        scale: &PriceScale,
        candles: &[Kline],
        slot_width: f32,
        blank_slots: usize,
    ) {
        let stroke = Stroke::new(1.0, Color32::from_gray(135).gamma_multiply(0.6));
        let x = plot.left() + plot.width() * crosshair.x_fraction as f32;

        // Vertical line — every chart draws it at the shared column.
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            stroke,
        );

        // Horizontal line + price pill — only the chart the pointer is over.
        if crosshair.owner_id == chart_id {
            let y = scale.y(crosshair.price);
            if plot.y_range().contains(y) {
                painter.line_segment(
                    [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    stroke,
                );
                let price_tag = Rect::from_min_max(
                    Pos2::new(plot.right() + 3.0, y - 9.0),
                    Pos2::new(rect.right() - 3.0, y + 9.0),
                );
                painter.rect_filled(price_tag, 3.0, Color32::from_gray(79));
                painter.text(
                    price_tag.center(),
                    Align2::CENTER_CENTER,
                    self.axis_price(crosshair.price),
                    FontId::monospace(10.0),
                    Color32::WHITE,
                );
            }
        }

        let raw_index = ((x - plot.left()) / slot_width).floor() as isize - blank_slots as isize;
        let Some((index, candle)) = usize::try_from(raw_index)
            .ok()
            .and_then(|index| candles.get(index).map(|candle| (index, candle)))
        else {
            return;
        };

        let date = format::timestamp(candle.open_time);
        let time_tag_width = 116.0;
        let time_tag = Rect::from_center_size(
            Pos2::new(x, rect.bottom() - TIME_AXIS_HEIGHT / 2.0),
            Vec2::new(time_tag_width, 18.0),
        )
        .intersect(Rect::from_min_max(
            Pos2::new(plot.left(), plot.bottom()),
            Pos2::new(plot.right(), rect.bottom()),
        ));
        painter.rect_filled(time_tag, 3.0, Color32::from_gray(79));
        painter.text(
            time_tag.center(),
            Align2::CENTER_CENTER,
            date,
            FontId::monospace(9.0),
            Color32::WHITE,
        );

        let color = if candle.is_up() {
            self.bullish_color()
        } else {
            self.bearish_color()
        };
        let details = format!(
            "O {}   H {}   L {}   C {}   Vol {:.2}",
            format::price(candle.open),
            format::price(candle.high),
            format::price(candle.low),
            format::price(candle.close),
            candle.volume
        );
        let detail_rect =
            Rect::from_min_size(plot.min + Vec2::new(8.0, 7.0), Vec2::new(410.0, 23.0));
        painter.rect(
            detail_rect,
            4.0,
            theme::chrome().gamma_multiply(0.94),
            Stroke::new(1.0, theme::border()),
            StrokeKind::Inside,
        );
        painter.text(
            detail_rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            details,
            FontId::monospace(10.0),
            color,
        );

        let _ = (ui, index);
    }
}

/// A color at the given alpha, premultiplied for egui's blending.
fn translucent(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(open: f64, close: f64) -> Kline {
        Kline {
            open_time: 0,
            close_time: 1,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 1.0,
        }
    }

    #[test]
    fn header_change_uses_visible_closes_after_pan_and_zoom() {
        let candles = [
            candle(80.0, 100.0),
            candle(105.0, 110.0),
            candle(95.0, 90.0),
            candle(115.0, 120.0),
        ];
        assert_eq!(
            visible_price_change(&candles, 2, 0),
            Some(30.0 / 90.0 * 100.0)
        );
        assert_eq!(
            visible_price_change(&candles, 2, 1),
            Some(-20.0 / 110.0 * 100.0)
        );
        assert_eq!(visible_price_change(&candles, 1, 0), None);
        assert_eq!(visible_price_change(&[], 2, 0), None);
    }
}
