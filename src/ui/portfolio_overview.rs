//! The portfolio's front page: what it is worth, how it has moved, and where the money sits.
//! And, beside it, the Statistics tab: performance, value range, activity and P&L by asset.
//! Pure views over the holdings, history and statistics the host computes.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{
    PortfolioAsset, PortfolioHolding, PortfolioSnapshot, PortfolioTransaction,
};
use crate::model::portfolio_format::{PortfolioValueFormatter, Tone};
use crate::model::portfolio_statistics::{
    AssetResult, PortfolioHistoryRange, PortfolioPeriodChange, PortfolioStatistics, pnl_bars,
};
use crate::ui::portfolio_charts::{self, HistoryInput};
use crate::ui::portfolio_table::{tone_color, tone_option};
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

/// How many holdings the "Top Holdings" panel lists.
pub const TOP_COUNT: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum OverviewAction {
    /// Open this asset's transactions.
    Select(String),
    /// "View All" in Top Holdings.
    ShowHoldings,
}

pub struct OverviewInput<'a> {
    /// `None` is "All Portfolios".
    pub portfolio_name: Option<&'a str>,
    pub holdings: &'a [PortfolioHolding],
    /// The value history for the selected portfolio (intraday for 1D when there is some).
    pub history: &'a [PortfolioSnapshot],
    pub total_value: f64,
    pub formatter: PortfolioValueFormatter,
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
    /// "Prices as of 14:03" and the like.
    pub market_value_caption: &'a str,
    pub is_updating: bool,
    pub is_loading_initial: bool,
    pub is_loading_history: bool,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

#[derive(Default)]
pub struct OverviewState {
    pub range: PortfolioHistoryRange,
    hovered_slice: Option<usize>,
}

/// The top holdings by value, largest first; unpriced ones last.
pub fn top_holdings(holdings: &[PortfolioHolding]) -> Vec<&PortfolioHolding> {
    let mut ranked: Vec<&PortfolioHolding> = holdings.iter().collect();
    ranked.sort_by(|a, b| {
        b.current_value
            .unwrap_or(0.0)
            .total_cmp(&a.current_value.unwrap_or(0.0))
    });
    ranked.truncate(TOP_COUNT);
    ranked
}

/// "Portfolio · 3 assets".
pub fn asset_count_text(count: usize) -> String {
    format!("{count} {}", if count == 1 { "asset" } else { "assets" })
}

/// The caption beside a change figure: "▲ 3.21%" (the glyph hidden in privacy mode, and for a
/// move of exactly zero).
pub fn change_caption(formatter: &PortfolioValueFormatter, amount: f64, percentage: f64) -> String {
    let glyph = if formatter.privacy || amount == 0.0 {
        ""
    } else if amount > 0.0 {
        "▲ "
    } else {
        "▼ "
    };
    format!("{glyph}{}", formatter.percent(percentage.abs(), 2))
}

pub fn show_overview(
    ui: &mut egui::Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
) -> Vec<OverviewAction> {
    let mut actions = Vec::new();
    let stats = PortfolioStatistics::new(input.holdings, input.history, &[]);
    let ranged = state.range.filter(input.history, input.now_ms);
    let period = PortfolioPeriodChange::new(&ranged, input.total_value);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.y = 14.0;
            let width = ui.available_width() - 40.0;
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    hero(ui, state, input, &stats, period);
                    summary_cards(ui, input.formatter, &stats);
                    ui.columns(2, |columns| {
                        performer_card(
                            &mut columns[0],
                            "Best Performer",
                            stats.best.as_ref(),
                            input,
                        );
                        performer_card(
                            &mut columns[1],
                            "Worst Performer",
                            stats.worst.as_ref(),
                            input,
                        );
                    });
                    history_card(ui, state, input);
                    ui.columns(2, |columns| {
                        card(&mut columns[0], |ui| {
                            ui.label(RichText::new("Allocation").strong().size(15.0));
                            ui.add_space(6.0);
                            portfolio_charts::show_allocation(
                                ui,
                                input.holdings,
                                input.formatter,
                                &mut state.hovered_slice,
                            );
                        });
                        card(&mut columns[1], |ui| {
                            top_holdings_panel(ui, input, &mut actions);
                        });
                    });
                    ui.add_space(14.0);
                });
            });
        });
    actions
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn hero(
    ui: &mut egui::Ui,
    state: &OverviewState,
    input: &OverviewInput,
    stats: &PortfolioStatistics,
    period: Option<PortfolioPeriodChange>,
) {
    let f = input.formatter;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(input.portfolio_name.unwrap_or("All Portfolios"))
                        .strong()
                        .size(17.0),
                );
                ui.label(
                    RichText::new(format!("· {}", asset_count_text(input.holdings.len())))
                        .size(17.0)
                        .weak(),
                );
            });
            if input.is_loading_initial {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading portfolio value…").weak());
                });
            } else {
                ui.label(
                    RichText::new(f.money(input.total_value))
                        .size(38.0)
                        .strong(),
                );
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(input.market_value_caption).small().weak());
                if input.is_updating && !input.is_loading_initial {
                    ui.spinner();
                    ui.label(RichText::new("Updating…").small().weak());
                }
            });
        });
        if !input.is_loading_initial {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                change_figure(
                    ui,
                    f,
                    &format!("P&L · {}", state.range.period_name()),
                    period.map(|p| p.profit),
                    period.and_then(|p| p.percentage),
                );
                ui.add_space(24.0);
                change_figure(
                    ui,
                    f,
                    "24h",
                    stats.day_change.map(|d| d.amount),
                    stats.day_change.and_then(|d| d.percentage),
                );
            });
        }
    });
}

/// A signed amount and percentage under a period label: "+$1,204.10  ▲ 3.21%".
fn change_figure(
    ui: &mut egui::Ui,
    f: PortfolioValueFormatter,
    label: &str,
    amount: Option<f64>,
    percentage: Option<f64>,
) {
    ui.vertical(|ui| {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
            ui.label(RichText::new(label).small().weak());
            match amount {
                Some(amount) => {
                    let tone = tone_option(f.tone(Some(amount)));
                    let mut text = RichText::new(f.signed_money(amount))
                        .size(16.0)
                        .strong()
                        .monospace();
                    if let Some(color) = tone {
                        text = text.color(color);
                    }
                    ui.label(text);
                    if let Some(percentage) = percentage {
                        let mut caption = RichText::new(change_caption(&f, amount, percentage))
                            .small()
                            .monospace();
                        caption = caption.color(tone.unwrap_or(ui.visuals().weak_text_color()));
                        ui.label(caption);
                    }
                }
                None => {
                    ui.label(RichText::new("—").size(16.0).strong().weak());
                }
            }
        });
    });
}

pub struct StatCard<'a> {
    pub title: &'a str,
    pub value: String,
    pub tone: Option<Tone>,
    pub caption: Option<String>,
    /// Quieter value when there is nothing to report yet.
    pub placeholder: bool,
    pub glyph: Option<&'a str>,
}

impl<'a> StatCard<'a> {
    pub fn new(title: &'a str, value: String) -> Self {
        Self {
            title,
            value,
            tone: None,
            caption: None,
            placeholder: false,
            glyph: None,
        }
    }

    pub fn tone(mut self, tone: Option<Tone>) -> Self {
        self.tone = tone;
        self
    }

    pub fn caption(mut self, caption: Option<String>) -> Self {
        self.caption = caption;
        self
    }

    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn glyph(mut self, glyph: &'a str) -> Self {
        self.glyph = Some(glyph);
        self
    }
}

pub fn stat_card(ui: &mut egui::Ui, card: &StatCard) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let tint = card.tone.map_or(theme::ACCENT, tone_color);
                if let Some(glyph) = card.glyph {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 9.0, tint.gamma_multiply(0.14));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        egui::FontId::proportional(15.0),
                        tint,
                    );
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(card.title).small().weak());
                    let mut value = RichText::new(&card.value).size(17.0).strong().monospace();
                    value = if card.placeholder {
                        value.color(ui.visuals().weak_text_color())
                    } else if let Some(tone) = card.tone {
                        value.color(tone_color(tone))
                    } else {
                        value
                    };
                    ui.label(value);
                    // Always laid out, so cards with and without a caption share one height.
                    let caption = card.caption.as_deref().unwrap_or(" ");
                    let mut text = RichText::new(caption).small().monospace();
                    text = text.color(card.tone.map_or(ui.visuals().weak_text_color(), tone_color));
                    ui.label(text);
                });
            });
        });
}

fn grid(ui: &mut egui::Ui, cards: &[StatCard]) {
    ui.columns(4, |columns| {
        for (index, card) in cards.iter().enumerate() {
            stat_card(&mut columns[index % 4], card);
        }
    });
}

fn summary_cards(ui: &mut egui::Ui, f: PortfolioValueFormatter, stats: &PortfolioStatistics) {
    grid(
        ui,
        &[
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).glyph("￥"),
            StatCard::new("All-time P&L", f.signed_money(stats.total_pnl()))
                .tone(f.tone(Some(stats.total_pnl())))
                .glyph("↗"),
            StatCard::new("Unrealized P&L", f.signed_money(stats.unrealized_pnl))
                .tone(f.tone(Some(stats.unrealized_pnl)))
                .caption(stats.unrealized_percent.map(|p| f.signed_percent(p, 2)))
                .glyph("⏳"),
            StatCard::new("Realized P&L", f.signed_money(stats.realized_pnl))
                .tone(f.tone(Some(stats.realized_pnl)))
                .glyph("✔"),
        ],
    );
}

fn performer_card(
    ui: &mut egui::Ui,
    title: &str,
    result: Option<&AssetResult>,
    input: &OverviewInput,
) {
    let f = input.formatter;
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .stroke(Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).small().weak());
            match result {
                Some(result) => {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(result.asset.display_ticker())
                                    .strong()
                                    .size(15.0),
                            );
                            ui.label(
                                RichText::new((input.subtitle)(&result.asset))
                                    .small()
                                    .weak(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                let percent = result
                                    .pnl_percent
                                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2));
                                let mut big =
                                    RichText::new(percent).size(16.0).strong().monospace();
                                if let Some(color) = tone_option(f.tone(result.pnl_percent)) {
                                    big = big.color(color);
                                }
                                ui.label(big);
                                let mut small = RichText::new(f.signed_money(result.total_pnl))
                                    .small()
                                    .monospace();
                                small = small.color(
                                    tone_option(f.tone(Some(result.total_pnl)))
                                        .unwrap_or(ui.visuals().weak_text_color()),
                                );
                                ui.label(small);
                            });
                        });
                    });
                }
                None => {
                    ui.label(RichText::new("—").size(16.0).strong().weak());
                    ui.label(" ");
                }
            }
        });
}

fn history_card(ui: &mut egui::Ui, state: &mut OverviewState, input: &OverviewInput) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Portfolio Value").strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for range in PortfolioHistoryRange::ALL.into_iter().rev() {
                    ui.selectable_value(&mut state.range, range, range.title());
                }
            });
        });
        ui.add_space(6.0);
        let loading =
            input.is_loading_initial || (input.is_loading_history && input.history.is_empty());
        portfolio_charts::show_history(
            ui,
            &HistoryInput {
                snapshots: input.history,
                current_value: input.total_value,
                currency: input.formatter.currency,
                range: state.range,
                privacy: input.formatter.privacy,
                is_loading: loading,
                loading_message: if input.is_loading_initial {
                    "Loading market data…"
                } else {
                    "Building history…"
                },
                now_ms: input.now_ms,
                utc_offset_secs: input.utc_offset_secs,
            },
            250.0,
        );
    });
}

fn top_holdings_panel(ui: &mut egui::Ui, input: &OverviewInput, actions: &mut Vec<OverviewAction>) {
    let f = input.formatter;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Top Holdings").strong().size(15.0));
        if input.holdings.len() > TOP_COUNT {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.link("View All").clicked() {
                    actions.push(OverviewAction::ShowHoldings);
                }
            });
        }
    });
    ui.add_space(4.0);
    for holding in top_holdings(input.holdings) {
        let row = ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(holding.asset.display_ticker()).strong());
                ui.label(
                    RichText::new((input.subtitle)(&holding.asset))
                        .small()
                        .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(
                                holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
                            )
                            .monospace(),
                        );
                        ui.horizontal(|ui| {
                            let (bar, _) =
                                ui.allocate_exact_size(Vec2::new(64.0, 4.0), Sense::hover());
                            ui.painter()
                                .rect_filled(bar, 2.0, Color32::from_white_alpha(25));
                            let filled = Rect::from_min_size(
                                bar.min,
                                Vec2::new(64.0 * holding.allocation.clamp(0.0, 1.0) as f32, 4.0),
                            );
                            ui.painter().rect_filled(filled, 2.0, theme::ACCENT);
                            ui.label(
                                RichText::new(f.percent(holding.allocation, 1))
                                    .small()
                                    .weak()
                                    .monospace(),
                            );
                        });
                    });
                });
            });
        });
        let response = row.response.interact(Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(
                response.rect.expand(2.0),
                8.0,
                Color32::from_white_alpha(14),
            );
        }
        if response.clicked() {
            actions.push(OverviewAction::Select(holding.asset.key.clone()));
        }
        response.on_hover_text("Shows this asset's transactions");
    }
}

// MARK: Statistics tab

pub struct StatisticsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    pub history: &'a [PortfolioSnapshot],
    /// The converted transactions, for fees and activity.
    pub transactions: &'a [PortfolioTransaction],
    pub formatter: PortfolioValueFormatter,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

/// "3 buys · 1 sell".
pub fn activity_caption(buys: usize, sells: usize) -> String {
    format!(
        "{buys} {} · {sells} {}",
        if buys == 1 { "buy" } else { "buys" },
        if sells == 1 { "sell" } else { "sells" }
    )
}

/// "Tracking since Jan 15, 2024" caption: how long ago, in words.
pub fn tracking_caption(since_ms: i64, now_ms: i64) -> String {
    let days = ((now_ms - since_ms) / 86_400_000).max(0);
    match days {
        0 => "today".to_owned(),
        1 => "1 day ago".to_owned(),
        2..=59 => format!("{days} days ago"),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

pub fn show_statistics(ui: &mut egui::Ui, input: &StatisticsInput) {
    let f = input.formatter;
    let stats = PortfolioStatistics::new(input.holdings, input.history, input.transactions);
    let on = |ms: i64| date_text(ms + input.utc_offset_secs * 1_000);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let width = ui.available_width() - 40.0;
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    section(ui, "Performance", None);
                    grid(
                        ui,
                        &[
                            StatCard::new("Total P&L", f.signed_money(stats.total_pnl()))
                                .tone(f.tone(Some(stats.total_pnl()))),
                            StatCard::new("Unrealized P&L", f.signed_money(stats.unrealized_pnl))
                                .tone(f.tone(Some(stats.unrealized_pnl)))
                                .caption(stats.unrealized_percent.map(|p| f.signed_percent(p, 2))),
                            StatCard::new("Realized P&L", f.signed_money(stats.realized_pnl))
                                .tone(f.tone(Some(stats.realized_pnl))),
                            StatCard::new(
                                "Last 24 Hours",
                                stats
                                    .day_change
                                    .map_or("—".to_owned(), |d| f.signed_money(d.amount)),
                            )
                            .tone(f.tone(stats.day_change.map(|d| d.amount)))
                            .caption(
                                stats
                                    .day_change
                                    .and_then(|d| d.percentage)
                                    .map(|p| f.signed_percent(p, 2)),
                            )
                            .placeholder(stats.day_change.is_none()),
                        ],
                    );
                    section(ui, "Value", None);
                    grid(
                        ui,
                        &[
                            StatCard::new("Current Value", f.money(stats.total_value)),
                            StatCard::new("Cost Basis", f.money(stats.cost_basis)),
                            StatCard::new(
                                "Portfolio High",
                                stats.high.map_or("—".to_owned(), |e| f.money(e.value)),
                            )
                            .caption(stats.high.map(|e| on(e.timestamp)))
                            .placeholder(stats.high.is_none()),
                            StatCard::new(
                                "Portfolio Low",
                                stats.low.map_or("—".to_owned(), |e| f.money(e.value)),
                            )
                            .caption(stats.low.map(|e| on(e.timestamp)))
                            .placeholder(stats.low.is_none()),
                        ],
                    );
                    section(ui, "Activity", None);
                    grid(
                        ui,
                        &[
                            StatCard::new("Transactions", stats.transaction_count.to_string())
                                .caption(Some(activity_caption(stats.buy_count, stats.sell_count))),
                            StatCard::new("Assets", stats.asset_count.to_string()).caption(Some(
                                format!("{} in profit · {} at a loss", stats.winners, stats.losers),
                            )),
                            StatCard::new("Fees Paid", f.money(stats.fees_paid)),
                            StatCard::new(
                                "Tracking Since",
                                stats.first_transaction.map_or("—".to_owned(), on),
                            )
                            .caption(
                                stats
                                    .first_transaction
                                    .map(|t| tracking_caption(t, input.now_ms)),
                            )
                            .placeholder(stats.first_transaction.is_none()),
                        ],
                    );
                    section(
                        ui,
                        "P&L by Asset",
                        Some(&format!("Total, in {}", f.currency.code())),
                    );
                    card(ui, |ui| pnl_bars_view(ui, &stats.asset_results, f));
                    ui.add_space(14.0);
                });
            });
        });
}

fn section(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(16.0));
        if let Some(subtitle) = subtitle {
            ui.label(RichText::new(subtitle).weak());
        }
    });
}

/// Horizontal bars of each asset's total P&L around a zero line: gains grow right, losses left.
fn pnl_bars_view(ui: &mut egui::Ui, results: &[AssetResult], f: PortfolioValueFormatter) {
    if results.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Priced holdings appear here.").weak()));
        ui.add_space(20.0);
        return;
    }
    let (bars, zero) = pnl_bars(results);
    for (result, bar) in results.iter().zip(&bars) {
        ui.horizontal(|ui| {
            ui.set_height(26.0);
            ui.add_sized(
                [100.0, 18.0],
                egui::Label::new(RichText::new(result.asset.display_ticker()).strong()).truncate(),
            );
            let track_width = (ui.available_width() - 120.0 - 80.0 - 20.0).max(40.0);
            let (track, _) = ui.allocate_exact_size(Vec2::new(track_width, 18.0), Sense::hover());
            let painter = ui.painter_at(track);
            let zero_x = track.left() + track.width() * zero as f32;
            painter.line_segment(
                [
                    Pos2::new(zero_x, track.top()),
                    Pos2::new(zero_x, track.bottom()),
                ],
                Stroke::new(1.0, theme::BORDER),
            );
            // Neutral while values are hidden: red and green would give each sign away.
            let colour = tone_option(f.tone(Some(result.total_pnl)))
                .map_or(Color32::from_gray(130).gamma_multiply(0.5), |c| {
                    c.gamma_multiply(0.85)
                });
            let left = track.left() + track.width() * bar.start as f32;
            let length = (track.width() * bar.length as f32).max(2.0);
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(left, track.top() + 3.0), Vec2::new(length, 12.0)),
                3.0,
                colour,
            );
            let amount = RichText::new(f.signed_money(result.total_pnl)).monospace();
            ui.add_sized(
                [120.0, 18.0],
                egui::Label::new(
                    amount.color(
                        tone_option(f.tone(Some(result.total_pnl)))
                            .unwrap_or(ui.visuals().text_color()),
                    ),
                ),
            );
            let percent = RichText::new(
                result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
            )
            .monospace();
            ui.add_sized(
                [80.0, 18.0],
                egui::Label::new(
                    percent.color(
                        tone_option(f.tone(result.pnl_percent))
                            .unwrap_or(ui.visuals().weak_text_color()),
                    ),
                ),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn holding(ticker: &str, value: Option<f64>) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}"),
                symbol: format!("{ticker}/USDT"),
                name: ticker.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: 1.0,
            cost_basis: 1.0,
            average_cost: 1.0,
            realized_pnl: 0.0,
            current_price: value,
            previous_day_price: None,
            current_value: value,
            unrealized_pnl: None,
            allocation: 0.0,
            is_price_stale: false,
        }
    }

    #[test]
    fn top_holdings_are_the_six_largest_with_unpriced_last() {
        let holdings: Vec<PortfolioHolding> = (1..=8)
            .map(|n| holding(&format!("A{n}"), Some(f64::from(n) * 10.0)))
            .chain([holding("NONE", None)])
            .collect();
        let top = top_holdings(&holdings);
        let tickers: Vec<String> = top.iter().map(|h| h.asset.display_ticker()).collect();
        assert_eq!(tickers, ["A8", "A7", "A6", "A5", "A4", "A3"]);
        let few = vec![holding("X", None), holding("Y", Some(1.0))];
        let ranked: Vec<String> = top_holdings(&few)
            .iter()
            .map(|h| h.asset.display_ticker())
            .collect();
        assert_eq!(ranked, ["Y", "X"]);
    }

    #[test]
    fn asset_counts_pluralise() {
        assert_eq!(asset_count_text(0), "0 assets");
        assert_eq!(asset_count_text(1), "1 asset");
        assert_eq!(asset_count_text(12), "12 assets");
    }

    #[test]
    fn the_change_caption_shows_direction_unless_hidden_or_flat() {
        let open = PortfolioValueFormatter::new(PortfolioCurrency::Usd, false);
        let hidden = PortfolioValueFormatter::new(PortfolioCurrency::Usd, true);
        assert_eq!(change_caption(&open, 120.0, 0.0321), "▲ 3.21%");
        assert_eq!(change_caption(&open, -120.0, -0.0321), "▼ 3.21%");
        assert_eq!(change_caption(&open, 0.0, 0.0), "0.00%");
        assert_eq!(
            change_caption(&hidden, 120.0, 0.0321),
            "3.21%",
            "no glyph to give the sign away"
        );
    }

    #[test]
    fn activity_and_tracking_captions_read_naturally() {
        assert_eq!(activity_caption(1, 1), "1 buy · 1 sell");
        assert_eq!(activity_caption(3, 0), "3 buys · 0 sells");
        let day = 86_400_000;
        assert_eq!(tracking_caption(0, 0), "today");
        assert_eq!(tracking_caption(0, day), "1 day ago");
        assert_eq!(tracking_caption(0, 30 * day), "30 days ago");
        assert_eq!(tracking_caption(0, 90 * day), "3 months ago");
        assert_eq!(tracking_caption(0, 800 * day), "2 years ago");
        assert_eq!(
            tracking_caption(10 * day, 0),
            "today",
            "a future first transaction is not negative"
        );
    }

    #[test]
    fn the_default_range_is_a_month() {
        assert_eq!(
            OverviewState::default().range,
            PortfolioHistoryRange::OneMonth
        );
    }
}
