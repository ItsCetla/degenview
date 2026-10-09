//! The strategy backtest report panel: headline metrics, the equity curve, and the closed trades,
//! rendered from a `strategy::StrategyReport`. Pure view — nothing here runs the strategy or
//! touches the store. A port of `PineStrategyReportView` / `PineEquityCurveView` /
//! `PineReportFormat`.

// Transient: the Script Manager hosts this once a strategy produces a report.
#![allow(dead_code)]

use crate::pine::strategy::StrategyReport;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke};

/// Number formatting for the strategy report, ported from the Swift `PineReportFormat`.
pub struct PineReportFormat;

/// Inserts thousands separators into the whole part of a decimal string.
fn group(digits: &str) -> String {
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// Fixed-decimal, grouped formatting of a possibly-negative value, with its sign.
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.decimals$}", value.abs());
    let grouped = match text.split_once('.') {
        Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
        None => group(&text),
    };
    let rounds_to_zero = !grouped.chars().any(|c| c.is_ascii_digit() && c != '0');
    if value < 0.0 && !rounds_to_zero {
        format!("-{grouped}")
    } else {
        grouped
    }
}

impl PineReportFormat {
    /// `1,234.50` — two decimals, grouped.
    pub fn money(value: f64) -> String {
        grouped(value, 2)
    }

    /// `+1,234.50` / `-1,234.50`, always signed.
    pub fn signed_money(value: f64) -> String {
        let body = grouped(value.abs(), 2);
        if value >= 0.0 {
            format!("+{body}")
        } else {
            format!("-{body}")
        }
    }

    /// A fraction as `12.34%`; with `signed`, `+12.34%` / `-12.34%`.
    pub fn percent(fraction: f64, signed: bool) -> String {
        let body = format!("{:.2}%", (fraction * 100.0).abs());
        if signed {
            if fraction >= 0.0 {
                format!("+{body}")
            } else {
                format!("-{body}")
            }
        } else {
            body
        }
    }

    /// A price with up to seven significant digits, grouped: `67,432.1`, `0.00000278`.
    pub fn price(value: f64) -> String {
        let abs = value.abs();
        if !abs.is_finite() || abs == 0.0 {
            return "0".to_owned();
        }
        let exponent = abs.log10().floor() as i32;
        let decimals = (6 - exponent).clamp(0, 12) as usize;
        let text = format!("{abs:.decimals$}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        let grouped = match text.split_once('.') {
            Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
            None => group(text),
        };
        if value < 0.0 {
            format!("-{grouped}")
        } else {
            grouped
        }
    }

    /// A plain ratio with two decimals: `1.23`.
    pub fn ratio(value: f64) -> String {
        format!("{value:.2}")
    }
}

/// Green above zero, red below, neutral at zero.
fn tint(value: f64, neutral: Color32) -> Color32 {
    if value > 0.0 {
        theme::UP
    } else if value < 0.0 {
        theme::DOWN
    } else {
        neutral
    }
}

/// Renders the whole report (metrics, equity curve, trades). A no-op when `report` is `None`.
pub fn show(ui: &mut egui::Ui, report: Option<&StrategyReport>) {
    let Some(report) = report else {
        return;
    };
    let neutral = ui.visuals().text_color();

    ui.vertical(|ui| {
        ui.label(RichText::new("Strategy report").small().strong());
        metric_grid(ui, report, neutral);
        equity_curve(ui, &report.equity, report.settings.initial_capital);
        if !report.trades.is_empty() {
            ui.add_space(4.0);
            trade_list(ui, report, neutral);
        }
    });
}

fn metric_grid(ui: &mut egui::Ui, report: &StrategyReport, neutral: Color32) {
    let drawdown = report.max_drawdown();
    let net = report.net_profit();
    let win_rate = report
        .win_rate()
        .map_or_else(|| "—".to_owned(), |r| PineReportFormat::percent(r, false));
    let profit_factor = report
        .profit_factor()
        .map_or_else(|| "—".to_owned(), PineReportFormat::ratio);
    let avg_trade = report
        .average_trade()
        .map_or_else(|| "—".to_owned(), PineReportFormat::signed_money);

    let metrics: [(&str, String, f64); 8] = [
        (
            "Net profit",
            format!(
                "{}  {}",
                PineReportFormat::signed_money(net),
                PineReportFormat::percent(report.return_fraction(), true)
            ),
            net,
        ),
        ("Closed trades", report.trades.len().to_string(), 0.0),
        ("Win rate", win_rate, 0.0),
        ("Profit factor", profit_factor, 0.0),
        (
            "Max drawdown",
            format!(
                "{}  {}",
                PineReportFormat::money(drawdown.0),
                PineReportFormat::percent(drawdown.1, false)
            ),
            0.0,
        ),
        ("Avg trade", avg_trade, 0.0),
        (
            "Open P&L",
            PineReportFormat::signed_money(report.open_profit),
            report.open_profit,
        ),
        (
            "Final equity",
            PineReportFormat::money(report.final_equity()),
            0.0,
        ),
    ];

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(16.0, 8.0);
        for (title, value, tone) in metrics {
            ui.allocate_ui(egui::vec2(132.0, 38.0), |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).small().weak());
                    ui.label(
                        RichText::new(value)
                            .small()
                            .monospace()
                            .color(tint(tone, neutral)),
                    );
                });
            });
        }
    });
}

fn trade_list(ui: &mut egui::Ui, report: &StrategyReport, neutral: Color32) {
    ui.label(RichText::new("Trades").small().strong());
    egui::ScrollArea::vertical()
        .max_height(150.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for trade in &report.trades {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{}", trade.id)).small().monospace());
                    ui.label(
                        RichText::new(if trade.is_long { "Long" } else { "Short" })
                            .small()
                            .monospace(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} → {}",
                            PineReportFormat::price(trade.entry_price),
                            PineReportFormat::price(trade.exit_price)
                        ))
                        .small()
                        .monospace(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(PineReportFormat::signed_money(trade.profit))
                                .small()
                                .monospace()
                                .color(tint(trade.profit, neutral)),
                        );
                    });
                });
            }
        });
}

/// Draws the equity curve against the starting capital, downsampled to `POINT_LIMIT` points.
fn equity_curve(ui: &mut egui::Ui, equity: &[f64], initial_capital: f64) {
    const POINT_LIMIT: usize = 400;
    const HEIGHT: f32 = 64.0;
    const INSET: f32 = 2.0;

    let values = downsample(equity, POINT_LIMIT);
    if values.len() <= 1 {
        return;
    }
    let rect = ui
        .allocate_exact_size(
            egui::vec2(ui.available_width(), HEIGHT),
            egui::Sense::hover(),
        )
        .0;
    let painter = ui.painter_at(rect);
    let weak = ui.visuals().weak_text_color();

    let low = values
        .iter()
        .copied()
        .fold(initial_capital, f64::min)
        .min(initial_capital);
    let high = values
        .iter()
        .copied()
        .fold(initial_capital, f64::max)
        .max(initial_capital);
    let span = (high - low).max(1e-9);
    let usable = rect.height() - 2.0 * INSET;
    let y = |value: f64| rect.max.y - ((value - low) / span) as f32 * usable - INSET;
    let x = |index: usize| rect.min.x + rect.width() * index as f32 / (values.len() - 1) as f32;

    let baseline_y = y(initial_capital);
    painter.line_segment(
        [
            egui::pos2(rect.min.x, baseline_y),
            egui::pos2(rect.max.x, baseline_y),
        ],
        Stroke::new(1.0, weak),
    );

    let tone = if values.last().copied().unwrap_or(initial_capital) >= initial_capital {
        theme::UP
    } else {
        theme::DOWN
    };
    let points: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, &value)| egui::pos2(x(index), y(value)))
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, tone)));

    painter.text(
        rect.min + egui::vec2(4.0, 4.0),
        Align2::LEFT_TOP,
        "Equity",
        FontId::proportional(11.0),
        weak,
    );
}

/// Keeps the first and last points and evenly spaced ones between.
fn downsample(values: &[f64], limit: usize) -> Vec<f64> {
    if values.len() <= limit || limit <= 1 {
        return values.to_vec();
    }
    (0..limit)
        .map(|index| values[index * (values.len() - 1) / (limit - 1)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_signed_money_group_and_keep_two_decimals() {
        assert_eq!(PineReportFormat::money(1_234.5), "1,234.50");
        assert_eq!(PineReportFormat::money(0.0), "0.00");
        assert_eq!(PineReportFormat::signed_money(12.0), "+12.00");
        assert_eq!(PineReportFormat::signed_money(-12.0), "-12.00");
        assert_eq!(PineReportFormat::signed_money(0.0), "+0.00");
    }

    #[test]
    fn percent_and_ratio_match_the_swift_format() {
        assert_eq!(PineReportFormat::percent(0.233_49, false), "23.35%");
        assert_eq!(PineReportFormat::percent(0.233_49, true), "+23.35%");
        assert_eq!(PineReportFormat::percent(-0.041, true), "-4.10%");
        assert_eq!(PineReportFormat::ratio(1.0), "1.00");
        assert_eq!(PineReportFormat::ratio(0.0), "0.00");
    }

    #[test]
    fn price_keeps_significant_digits_and_groups() {
        assert_eq!(PineReportFormat::price(67_432.1), "67,432.1");
        assert_eq!(PineReportFormat::price(0.000_002_78), "0.00000278");
        assert_eq!(PineReportFormat::price(0.0), "0");
        assert_eq!(PineReportFormat::price(1.5), "1.5");
        assert_eq!(PineReportFormat::price(-3.25), "-3.25");
    }

    #[test]
    fn downsample_keeps_ends_and_spaces_the_rest() {
        let values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let sampled = downsample(&values, 11);
        assert_eq!(sampled.first(), Some(&0.0));
        assert_eq!(sampled.last(), Some(&99.0));
        assert_eq!(sampled.len(), 11);
    }

    #[test]
    fn the_report_draws_headlessly() {
        let report = StrategyReport {
            settings: Default::default(),
            trades: Vec::new(),
            open_trades: Vec::new(),
            equity: vec![1_000_000.0, 1_020_000.0, 1_010_000.0, 1_050_000.0],
            open_profit: 0.0,
        };
        egui::__run_test_ui(|ui| {
            show(ui, Some(&report));
            show(ui, None);
        });
    }
}
