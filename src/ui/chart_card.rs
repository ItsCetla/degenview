use crate::format;
use crate::model::paper::PaperOrderSide;
use crate::model::{
    BollingerSeries, DataSource, IndicatorConfig, IndicatorSeries, Kline, Timeframe,
};
use crate::ui::drawing_layer::{DrawingFrame, DrawingLayer, Projection};
use crate::ui::drawing_render::TimeIndex;
use crate::ui::paper_chart_trading::{self, PaperCardState};
use crate::ui::replay_overlay::{self, ReplayChartFrame};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2,
};

const PRICE_AXIS_WIDTH: f32 = 82.0;
const TIME_AXIS_HEIGHT: f32 = 24.0;
const HEADER_HEIGHT: f32 = 45.0;
const MIN_VISIBLE_CANDLES: usize = 20;

/// Signals a chart card emits while it is being drawn. The owning workspace
/// applies them after the frame, so rendering never mutates layout mid-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChartCardAction {
    pub focus_requested: bool,
    pub close_requested: bool,
    pub settings_requested: bool,
    /// A header SELL / BUY pill was clicked: open the order ticket on that side.
    pub quick_trade: Option<PaperOrderSide>,
    /// The header bell was clicked: start a new price alert for this chart's market.
    pub new_alert: bool,
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
    visible_candles: usize,
    right_offset: usize,
    drag_origin_offset: Option<usize>,
    y_zoom: f64,
    y_zoom_drag_start: Option<f64>,
    layer: DrawingLayer,
    /// Paper-trading inputs and outputs for this card; the workspace fills and drains it.
    pub paper: PaperCardState,
}

impl Default for ChartCard {
    fn default() -> Self {
        Self {
            visible_candles: 120,
            right_offset: 0,
            drag_origin_offset: None,
            y_zoom: 1.0,
            y_zoom_drag_start: None,
            layer: DrawingLayer::default(),
            paper: PaperCardState::default(),
        }
    }
}

impl ChartCard {
    pub fn set_y_zoom(&mut self, y_zoom: f64) {
        self.y_zoom = y_zoom;
    }

    pub fn y_zoom(&self) -> f64 {
        self.y_zoom
    }
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

    /// Header row. Only the title block is a drag handle; the buttons sit outside
    /// it so they stay clickable and the layout stays put.
    #[allow(clippy::too_many_arguments)]
    pub fn header(
        &self,
        ui: &mut egui::Ui,
        symbol: &str,
        source: DataSource,
        timeframe: Timeframe,
        candles: &[Kline],
        loading: bool,
        action: &mut ChartCardAction,
        drag_id: u64,
    ) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.dnd_drag_source(egui::Id::new(("chart-drag", drag_id)), drag_id, |ui| {
                    ui.add_space(10.0);
                    if let Some(url) = crate::service::icon::get_url(source, symbol) {
                        ui.add(egui::Image::new(url).max_size(Vec2::splat(19.0)));
                    } else {
                        let glyph_color = if source == DataSource::Binance {
                            theme::BITCOIN
                        } else {
                            Color32::from_gray(190)
                        };
                        ui.label(
                            egui::RichText::new(source.glyph())
                                .color(glyph_color)
                                .strong()
                                .size(19.0),
                        );
                        crate::service::icon::request(source, symbol);
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(source.display_symbol(symbol)).strong());
                            ui.label(egui::RichText::new(source.display_name()).small().weak());
                            ui.label(egui::RichText::new(timeframe.label()).small().weak());
                        });
                        if let Some(last) = candles.last() {
                            ui.label(
                                egui::RichText::new(format!("{} US$", format::price(last.close)))
                                    .monospace()
                                    .small()
                                    .weak(),
                            );
                        } else {
                            ui.label(egui::RichText::new("Fetching market data…").small().weak());
                        }
                    });
                });

                if supports_alerts(source)
                    && ui
                        .small_button("🔔")
                        .on_hover_text("New price alert")
                        .clicked()
                {
                    action.new_alert = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Remove chart").clicked() {
                        action.close_requested = true;
                    }
                    ui.add_space(10.0);
                    if ui
                        .small_button("Settings")
                        .on_hover_text("Chart settings")
                        .clicked()
                    {
                        action.settings_requested = true;
                    }
                    ui.label(egui::RichText::new("Favorite").small().weak());
                    if loading {
                        ui.spinner();
                    }
                    if let (Some(first), Some(last)) = (candles.first(), candles.last()) {
                        let change = (last.close / first.open - 1.0) * 100.0;
                        let color = if change >= 0.0 {
                            theme::UP
                        } else {
                            theme::DOWN
                        };
                        ui.label(
                            egui::RichText::new(format!("{change:+.2}%"))
                                .color(color)
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
        line: bool,
        indicators: &IndicatorConfig,
        crosshair: Option<&CrosshairInfo>,
        chart_id: u64,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        let mut pointer = ChartPointer::default();
        ui.separator();
        let size = ui.available_size().max(Vec2::new(300.0, 260.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, theme::SURFACE);

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
            let display = symbol.strip_suffix("USDT").unwrap_or(symbol);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Loading {display}/USDT candles…"),
                FontId::proportional(13.0),
                ui.visuals().weak_text_color(),
            );
            return pointer;
        }

        let plot = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - PRICE_AXIS_WIDTH, rect.max.y - TIME_AXIS_HEIGHT),
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

        self.draw_grid(ui, &painter, plot, rect, scale);
        if line {
            self.draw_line(&painter, shown, &x_for, scale);
        } else {
            self.draw_candles(&painter, shown, slot_width, &x_for, scale);
        }

        let series = IndicatorSeries::compute(candles, view.start, shown.len(), indicators);
        if !line && indicators.show_volume {
            self.draw_volume(&painter, plot, &x_for, slot_width, shown);
        }
        if indicators.show_ema {
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
                Stroke::new(1.0, theme::GRID),
            );
            painter.text(
                Pos2::new(rect.right() - 7.0, y),
                Align2::RIGHT_CENTER,
                format::price(scale.price(y)),
                FontId::monospace(10.0),
                ui.visuals().weak_text_color(),
            );
        }

        for step in 0..=8 {
            let x = plot.left() + plot.width() * step as f32 / 8.0;
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0, theme::GRID.gamma_multiply(0.65)),
            );
        }

        painter.line_segment(
            [
                Pos2::new(plot.right(), plot.top()),
                Pos2::new(plot.right(), rect.bottom()),
            ],
            Stroke::new(1.0, theme::BORDER),
        );
    }

    fn draw_candles(
        &self,
        painter: &egui::Painter,
        candles: &[Kline],
        slot_width: f32,
        x_for: &dyn Fn(usize) -> f32,
        scale: &PriceScale,
    ) {
        let body_half_width = (slot_width * 0.34).clamp(1.0, 8.0);
        for (index, candle) in candles.iter().enumerate() {
            let x = x_for(index);
            let color = if candle.is_up() {
                theme::UP
            } else {
                theme::DOWN
            };
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
                    theme::ACCENT,
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
            painter.line_segment([pair[0], pair[1]], Stroke::new(1.5, theme::ACCENT));
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
                theme::UP
            } else {
                theme::DOWN
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
                Stroke::new(1.0, theme::GRID),
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
            let color = if *bullish { theme::UP } else { theme::DOWN };
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
        let color = if last.is_up() { theme::UP } else { theme::DOWN };
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
            format::price(last.close),
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
                    format::price(crosshair.price),
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
            theme::UP
        } else {
            theme::DOWN
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
            theme::CHROME.gamma_multiply(0.94),
            Stroke::new(1.0, theme::BORDER),
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
