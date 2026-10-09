use crate::model::{Kline, Timeframe};
use crate::service;
use crate::ui::{self, ChartCard};
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CANDLE_LIMIT: u32 = 500;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct DegenViewApp {
    symbol: String,
    timeframe: Timeframe,
    candles: Vec<Kline>,
    loading: bool,
    error: Option<String>,
    last_fetch: Instant,
    inbox: Option<Receiver<Result<Vec<Kline>, String>>>,
    chart: ChartCard,
    watchlist_visible: bool,
}

impl DegenViewApp {
    pub fn new(creation: &eframe::CreationContext<'_>) -> Self {
        ui::theme::install(&creation.egui_ctx);
        let mut app = Self {
            symbol: "BTCUSDT".to_owned(),
            timeframe: Timeframe::Day,
            candles: Vec::new(),
            loading: false,
            error: None,
            last_fetch: Instant::now(),
            inbox: None,
            chart: ChartCard::default(),
            watchlist_visible: true,
        };
        app.fetch(&creation.egui_ctx);
        app
    }

    fn fetch(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let symbol = self.symbol.clone();
        let interval = self.timeframe.binance_interval();
        let repaint = ctx.clone();

        std::thread::spawn(move || {
            let result = service::binance::fetch_klines(&symbol, interval, CANDLE_LIMIT);
            let _ = sender.send(result);
            repaint.request_repaint();
        });

        self.inbox = Some(receiver);
        self.loading = true;
        self.last_fetch = Instant::now();
    }

    fn receive_fetch(&mut self) {
        let Some(receiver) = &self.inbox else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };

        self.loading = false;
        self.inbox = None;
        match result {
            Ok(candles) => {
                self.candles = candles;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_centered(|ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("DV")
                    .color(ui::theme::ACCENT)
                    .strong()
                    .size(12.0),
            );
            ui.label(egui::RichText::new("DegenView").strong().size(14.0));
            ui.separator();

            let mut changed = false;
            ui.spacing_mut().item_spacing.x = 2.0;
            for timeframe in Timeframe::ALL {
                let selected = self.timeframe == timeframe;
                if ui
                    .selectable_label(selected, timeframe.label())
                    .on_hover_text(timeframe.description())
                    .clicked()
                {
                    self.timeframe = timeframe;
                    self.chart.reset_view();
                    changed = true;
                }
            }

            if changed {
                self.candles.clear();
                self.fetch(ctx);
            }

            ui.separator();
            ui.toggle_value(&mut self.chart.log_scale, "Log");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+  Add Chart").clicked() {
                    self.error =
                        Some("Add Chart arrives with dashboard composition in Stage 3".into());
                }
                ui.toggle_value(&mut self.watchlist_visible, "Watchlist");
                let _ = ui.button("My Chart  v");
                if self.loading {
                    ui.spinner();
                }
            });
        });
    }

    fn show_tool_rail(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            for (glyph, hint) in [
                ("+", "Add chart (Stage 3)"),
                ("/", "Trend line (Stage 5)"),
                ("Fib", "Fibonacci retracement (Stage 5)"),
                ("Pen", "Brush (Stage 5)"),
                ("Rul", "Ruler (Stage 5)"),
            ] {
                if ui
                    .add_sized([32.0, 32.0], egui::Button::new(glyph).frame(false))
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.error = Some(format!("{hint} is not migrated in this stage"));
                }
                ui.add_space(4.0);
            }
        });
    }

    fn show_watchlist(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("v  Crypto").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("...");
                ui.label("Search");
                ui.label("+");
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Symbol").small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Chg%").small().weak());
                ui.add_space(26.0);
                ui.label(egui::RichText::new("Last").small().weak());
            });
        });

        let latest = self.candles.last();
        let first = self.candles.first();
        let change = latest
            .zip(first)
            .map(|(latest, first)| (latest.close / first.open - 1.0) * 100.0);
        let last = latest.map_or("—".into(), |candle| crate::format::price(candle.close));
        let change_text = change.map_or("—".into(), |value| format!("{value:+.2}%"));
        let change_color = if change.unwrap_or_default() >= 0.0 {
            ui::theme::UP
        } else {
            ui::theme::DOWN
        };

        egui::Frame::new()
            .fill(ui::theme::SURFACE_SELECTED)
            .corner_radius(5.0)
            .inner_margin(egui::Margin::symmetric(7, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("B")
                            .color(ui::theme::BITCOIN)
                            .strong()
                            .size(17.0),
                    );
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("BTC/USDT").strong().size(12.0));
                        ui.label(egui::RichText::new("Binance").weak().small());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(change_text)
                                .color(change_color)
                                .strong(),
                        );
                        ui.label(egui::RichText::new(last).monospace());
                    });
                });
            });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Meme Coins").small().strong().weak());
        for (symbol, source, price, change) in [
            ("DOGE/USDT", "Binance", "0.2478", "−1.93%"),
            ("PENGU/USDT", "Binance", "0.008525", "−0.35%"),
            ("PEPE/USDT", "Binance", "0.000010", "+0.06%"),
        ] {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(symbol).strong().size(11.0));
                    ui.label(egui::RichText::new(source).weak().small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let color = if change.starts_with('+') {
                        ui::theme::UP
                    } else {
                        ui::theme::DOWN
                    };
                    ui.label(egui::RichText::new(change).color(color).size(11.0));
                    ui.label(egui::RichText::new(price).monospace().size(11.0));
                });
            });
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.label(
                egui::RichText::new("Stage 1 · visual context only")
                    .small()
                    .weak(),
            );
        });
    }
}

impl eframe::App for DegenViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_fetch();
        if self.inbox.is_none() && self.last_fetch.elapsed() >= REFRESH_INTERVAL {
            self.fetch(&ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(250));

        egui::Panel::top("workspace_toolbar")
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
            )
            .show(ui, |ui| self.show_toolbar(ui, &ctx));

        egui::Panel::left("tool_rail")
            .exact_size(48.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::CHROME)
                    .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
            )
            .show(ui, |ui| self.show_tool_rail(ui));

        if self.watchlist_visible {
            egui::Panel::right("watchlist")
                .default_size(260.0)
                .min_size(220.0)
                .max_size(420.0)
                .frame(
                    egui::Frame::new()
                        .fill(ui::theme::CHROME)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .stroke(egui::Stroke::new(1.0, ui::theme::BORDER)),
                )
                .show(ui, |ui| self.show_watchlist(ui));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui::theme::BACKGROUND)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                self.chart.show(
                    ui,
                    &self.symbol,
                    self.timeframe,
                    &self.candles,
                    self.loading,
                    self.error.as_deref(),
                );
            });
    }
}
