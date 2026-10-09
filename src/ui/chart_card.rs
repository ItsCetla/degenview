use crate::format;
use crate::model::{Kline, Timeframe};
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
}

pub struct ChartCard {
    visible_candles: usize,
    right_offset: usize,
    drag_origin_offset: Option<usize>,
}

impl Default for ChartCard {
    fn default() -> Self {
        Self {
            visible_candles: 120,
            right_offset: 0,
            drag_origin_offset: None,
        }
    }
}

struct PriceScale {
    minimum: f64,
    maximum: f64,
    top: f32,
    height: f32,
    logarithmic: bool,
}

impl PriceScale {
    fn project(&self, price: f64) -> f64 {
        if self.logarithmic {
            price.max(f64::MIN_POSITIVE).ln()
        } else {
            price
        }
    }

    fn y(&self, price: f64) -> f32 {
        let minimum = self.project(self.minimum);
        let maximum = self.project(self.maximum);
        let fraction =
            ((self.project(price) - minimum) / (maximum - minimum).max(f64::EPSILON)) as f32;
        self.top + self.height * (1.0 - fraction)
    }

    fn price(&self, y: f32) -> f64 {
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

impl ChartCard {
    pub fn reset_view(&mut self) {
        self.visible_candles = 120;
        self.right_offset = 0;
        self.drag_origin_offset = None;
    }

    /// Header row, rendered inside the workspace's drag source so it doubles as
    /// the card's drag handle. Requests focus/close through `action`.
    pub fn header(
        &self,
        ui: &mut egui::Ui,
        symbol: &str,
        timeframe: Timeframe,
        candles: &[Kline],
        loading: bool,
        action: &mut ChartCardAction,
    ) {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("B")
                        .color(theme::BITCOIN)
                        .strong()
                        .size(19.0),
                );
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let display = symbol.strip_suffix("USDT").unwrap_or(symbol);
                        ui.label(egui::RichText::new(format!("{display}/USDT")).strong());
                        ui.label(egui::RichText::new("Binance").small().weak());
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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Remove chart").clicked() {
                        action.close_requested = true;
                    }
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Settings").small().weak());
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
                });
            },
        );
    }

    /// Plot body. Returns true when the plot area was clicked, which the
    /// workspace turns into a focus request.
    pub fn body(
        &mut self,
        ui: &mut egui::Ui,
        symbol: &str,
        candles: &[Kline],
        loading: bool,
        error: Option<&str>,
        log_scale: bool,
    ) -> bool {
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
            return false;
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
            return false;
        }

        let plot = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.max.x - PRICE_AXIS_WIDTH, rect.max.y - TIME_AXIS_HEIGHT),
        );
        self.handle_input(ui, &response, plot, candles.len());

        let end = candles.len().saturating_sub(self.right_offset);
        let start = end.saturating_sub(self.visible_candles);
        let shown = &candles[start..end];
        if shown.is_empty() {
            return false;
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
        let scale = PriceScale {
            minimum: if log_scale { low * 0.97 } else { low - padding },
            maximum: if log_scale {
                high * 1.03
            } else {
                high + padding
            },
            top: plot.top(),
            height: plot.height(),
            logarithmic: log_scale,
        };
        let slot_width = plot.width() / self.visible_candles as f32;
        let blank_slots = self.visible_candles.saturating_sub(shown.len());
        let x_for = |index: usize| {
            plot.left() + slot_width * (index as f32 + 0.5) + slot_width * blank_slots as f32
        };

        self.draw_grid(ui, &painter, plot, rect, &scale);
        self.draw_candles(&painter, shown, slot_width, &x_for, &scale);
        self.draw_last_price(&painter, plot, rect, &scale, candles);
        self.draw_time_axis(ui, &painter, rect, shown, &x_for);

        if let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) {
            self.draw_crosshair(
                ui,
                &painter,
                plot,
                rect,
                position,
                &scale,
                shown,
                slot_width,
                blank_slots,
            );
        }

        let _ = loading;
        response.clicked()
    }

    fn handle_input(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        plot: Rect,
        total_candles: usize,
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

        if response.drag_started() {
            self.drag_origin_offset = Some(self.right_offset);
        }
        if response.dragged() {
            let slot_width = plot.width() / self.visible_candles as f32;
            let candle_delta = (response.drag_delta().x / slot_width).round() as isize;
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
        position: Pos2,
        scale: &PriceScale,
        candles: &[Kline],
        slot_width: f32,
        blank_slots: usize,
    ) {
        let stroke = Stroke::new(1.0, Color32::from_gray(135).gamma_multiply(0.6));
        painter.line_segment(
            [
                Pos2::new(plot.left(), position.y),
                Pos2::new(plot.right(), position.y),
            ],
            stroke,
        );
        painter.line_segment(
            [
                Pos2::new(position.x, plot.top()),
                Pos2::new(position.x, plot.bottom()),
            ],
            stroke,
        );

        let price_tag = Rect::from_min_max(
            Pos2::new(plot.right() + 3.0, position.y - 9.0),
            Pos2::new(rect.right() - 3.0, position.y + 9.0),
        );
        painter.rect_filled(price_tag, 3.0, Color32::from_gray(79));
        painter.text(
            price_tag.center(),
            Align2::CENTER_CENTER,
            format::price(scale.price(position.y)),
            FontId::monospace(10.0),
            Color32::WHITE,
        );

        let raw_index =
            ((position.x - plot.left()) / slot_width).floor() as isize - blank_slots as isize;
        let Some((index, candle)) = usize::try_from(raw_index)
            .ok()
            .and_then(|index| candles.get(index).map(|candle| (index, candle)))
        else {
            return;
        };

        let date = format::timestamp(candle.open_time);
        let time_tag_width = 116.0;
        let time_tag = Rect::from_center_size(
            Pos2::new(position.x, rect.bottom() - TIME_AXIS_HEIGHT / 2.0),
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
