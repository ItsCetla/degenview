//! The portfolio's front page: what it is worth, how it has moved, and where the money sits.
//! And, beside it, the Statistics tab: performance, value range, activity and P&L by asset.
//! Pure views over the holdings, history and statistics the host computes.
//!
//! Every block here has a fixed height and paints its text at measured positions, so nothing
//! reflows while prices stream in and cards in one row always end level.

// Transient: the Portfolio window hosts these once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{
    PortfolioAsset, PortfolioHolding, PortfolioSnapshot, PortfolioTransaction,
};
use crate::model::portfolio_format::{PortfolioValueFormatter, Tone};
use crate::model::portfolio_statistics::{
    AssetResult, PortfolioHistoryRange, PortfolioPeriodChange, PortfolioStatistics, pnl_bars,
};
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::portfolio_charts::{self, HistoryInput};
use crate::ui::portfolio_table::{self, tone_color, tone_option};
use crate::ui::replay_bar::date_text;
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Galley, Margin, Pos2, Rect, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, pos2, vec2,
};
use std::sync::Arc;

/// How many holdings the "Top Holdings" panel lists.
pub const TOP_COUNT: usize = 6;

/// Space around the page and between its blocks.
const PAGE_INSET: i8 = 20;
const SECTION_GAP: f32 = 16.0;
const GRID_GAP: f32 = 12.0;
/// A stat or performer card: two 12 pt paddings around a 50 pt text block.
const CARD_HEIGHT: f32 = 76.0;
const HERO_HEIGHT: f32 = 92.0;
/// A panel's padding, its title row and the gap under it.
const PANEL_PADDING: f32 = 16.0;
const PANEL_HEADER: f32 = 27.0;
const PANEL_GAP: f32 = 12.0;
const HISTORY_CHART_HEIGHT: f32 = 250.0;
/// Both lower panels size their content to this, so they always end level: six Top Holdings
/// rows fill it exactly.
const PANEL_CONTENT_HEIGHT: f32 = 270.0;
const TOP_ROW_HEIGHT: f32 = 43.0;
const TOP_ROW_GAP: f32 = 2.0;

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

/// The top holdings by value, largest first; unpriced ones last. Equal values keep a fixed
/// order (by market), so rows never swap places between frames.
pub fn top_holdings(holdings: &[PortfolioHolding]) -> Vec<&PortfolioHolding> {
    let mut ranked: Vec<&PortfolioHolding> = holdings.iter().collect();
    ranked.sort_by(|a, b| {
        b.current_value
            .unwrap_or(0.0)
            .total_cmp(&a.current_value.unwrap_or(0.0))
            .then_with(|| a.asset.key.cmp(&b.asset.key))
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

// MARK: Painting helpers

/// One line of text at an SF weight, laid out so it can be measured before it is placed.
fn line(ui: &Ui, text: &str, size: f32, weight: f32, color: Color32) -> Arc<Galley> {
    kit::galley(ui, text, size, weight, color, 4_000.0)
}

fn draw(ui: &Ui, at: Pos2, galley: &Arc<Galley>) {
    ui.painter()
        .galley(at, galley.clone(), Color32::PLACEHOLDER);
}

/// Paints `text` anchored at `pos`; returns the rect it covers.
fn text_at(
    ui: &Ui,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    weight: f32,
    color: Color32,
) -> Rect {
    let galley = line(ui, text, size, weight, color);
    let rect = anchor.anchor_size(pos, galley.size());
    draw(ui, rect.min, &galley);
    rect
}

fn caption_color() -> Color32 {
    theme::text_secondary()
}

/// The tone's colour, or `fallback` when there is none (zero, unknown, or values hidden).
fn tone_or(tone: Option<Tone>, fallback: Color32) -> Color32 {
    tone_option(tone).unwrap_or(fallback)
}

/// A small filled triangle: the direction mark beside a change percentage.
fn paint_arrow(ui: &Ui, center: Pos2, up: bool, color: Color32) {
    let (half_width, half_height) = (3.5, 3.0);
    let points = if up {
        vec![
            pos2(center.x - half_width, center.y + half_height),
            pos2(center.x + half_width, center.y + half_height),
            pos2(center.x, center.y - half_height),
        ]
    } else {
        vec![
            pos2(center.x - half_width, center.y - half_height),
            pos2(center.x + half_width, center.y - half_height),
            pos2(center.x, center.y + half_height),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A panel: fixed outer height, a title row with an optional trailing control, then content.
/// Because the height is given, two panels in a row end level whatever they hold.
fn panel(ui: &mut Ui, content_height: f32, title: &str, content: impl FnOnce(&mut Ui, &mut Ui)) {
    let height = PANEL_PADDING * 2.0 + PANEL_HEADER + PANEL_GAP + content_height;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 12.0, theme::chrome());
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(PANEL_PADDING);
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), PANEL_HEADER));
    text_at(
        ui,
        pos2(header.left(), header.center().y),
        Align2::LEFT_CENTER,
        title,
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
    );
    let mut header_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(header)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let body = Rect::from_min_size(
        pos2(inner.left(), header.bottom() + PANEL_GAP),
        vec2(inner.width(), content_height),
    );
    let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(
        body.expand2(vec2(PANEL_PADDING, 0.0))
            .intersect(ui.clip_rect()),
    );
    content(&mut header_ui, &mut body_ui);
}

/// A tinted, bordered card of a fixed height; `inside` is the content rect.
fn card_frame(ui: &mut Ui, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(rect, 10.0, theme::quaternary(0.4));
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.4)),
        egui::StrokeKind::Inside,
    );
    rect
}

// MARK: Overview

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
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, SECTION_GAP);
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
                    ui.spacing_mut().item_spacing.x = SECTION_GAP;
                    ui.columns(2, |columns| {
                        panel(
                            &mut columns[0],
                            PANEL_CONTENT_HEIGHT,
                            "Allocation",
                            |_, ui| {
                                portfolio_charts::show_allocation(
                                    ui,
                                    input.holdings,
                                    input.formatter,
                                    &mut state.hovered_slice,
                                    PANEL_CONTENT_HEIGHT,
                                );
                            },
                        );
                        top_holdings_panel(&mut columns[1], input, &mut actions);
                    });
                });
        });
    actions
}

fn hero(
    ui: &mut Ui,
    state: &OverviewState,
    input: &OverviewInput,
    stats: &PortfolioStatistics,
    period: Option<PortfolioPeriodChange>,
) {
    let f = input.formatter;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_HEIGHT), Sense::hover());
    let left = rect.left() + 4.0;
    let right = rect.right() - 4.0;

    let name = text_at(
        ui,
        pos2(left, rect.top()),
        Align2::LEFT_TOP,
        input.portfolio_name.unwrap_or("All Portfolios"),
        15.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    text_at(
        ui,
        pos2(name.right() + 6.0, rect.top()),
        Align2::LEFT_TOP,
        &format!("· {}", asset_count_text(input.holdings.len())),
        15.0,
        REGULAR,
        caption_color(),
    );

    let balance_top = rect.top() + 26.0;
    let balance_middle = balance_top + 24.0;
    if input.is_loading_initial {
        ui.put(
            Rect::from_center_size(pos2(left + 8.0, balance_middle), Vec2::splat(16.0)),
            egui::Spinner::new().size(16.0),
        );
        text_at(
            ui,
            pos2(left + 26.0, balance_middle),
            Align2::LEFT_CENTER,
            "Loading portfolio value…",
            12.0,
            REGULAR,
            caption_color(),
        );
    } else {
        text_at(
            ui,
            pos2(left, balance_top),
            Align2::LEFT_TOP,
            &f.money(input.total_value),
            40.0,
            SEMIBOLD,
            theme::text_primary(),
        );
    }

    let caption_middle = rect.bottom() - 6.0;
    let caption = text_at(
        ui,
        pos2(left, caption_middle),
        Align2::LEFT_CENTER,
        input.market_value_caption,
        kit::size::CAPTION,
        REGULAR,
        caption_color(),
    );
    if input.is_updating && !input.is_loading_initial {
        ui.put(
            Rect::from_center_size(
                pos2(caption.right() + 12.0, caption_middle),
                Vec2::splat(10.0),
            ),
            egui::Spinner::new().size(10.0),
        );
        text_at(
            ui,
            pos2(caption.right() + 22.0, caption_middle),
            Align2::LEFT_CENTER,
            "Updating…",
            kit::size::CAPTION,
            REGULAR,
            caption_color(),
        );
    }

    if !input.is_loading_initial {
        let pnl_label = format!("P&L · {}", state.range.period_name());
        let edge = change_figure(
            ui,
            f,
            &pnl_label,
            period.map(|p| p.profit),
            period.and_then(|p| p.percentage),
            pos2(right, rect.bottom()),
        );
        change_figure(
            ui,
            f,
            "24h",
            stats.day_change.map(|d| d.amount),
            stats.day_change.and_then(|d| d.percentage),
            pos2(edge - 28.0, rect.bottom()),
        );
    }
}

/// A signed amount and percentage under a period label: "+$1,204.10  ▲ 3.21%", right-aligned
/// to `anchor` (its bottom-right corner). Returns the figure's left edge.
fn change_figure(
    ui: &Ui,
    f: PortfolioValueFormatter,
    label: &str,
    amount: Option<f64>,
    percentage: Option<f64>,
    anchor: Pos2,
) -> f32 {
    let secondary = caption_color();
    let label = line(ui, label, kit::size::CAPTION, REGULAR, secondary);
    let Some(amount) = amount else {
        let dash = line(ui, "—", 15.0, SEMIBOLD, secondary);
        let width = label.size().x.max(dash.size().x);
        let height = label.size().y + 3.0 + dash.size().y;
        let top = anchor.y - height;
        draw(ui, pos2(anchor.x - label.size().x, top), &label);
        draw(
            ui,
            pos2(anchor.x - dash.size().x, top + label.size().y + 3.0),
            &dash,
        );
        return anchor.x - width;
    };
    let tone = f.tone(Some(amount));
    let value = line(
        ui,
        &f.signed_money(amount),
        15.0,
        SEMIBOLD,
        tone_or(tone, theme::text_primary()),
    );
    let percent = percentage.map(|percentage| {
        line(
            ui,
            &f.percent(percentage.abs(), 2),
            kit::size::CAPTION,
            MEDIUM,
            tone_or(tone, secondary),
        )
    });
    let arrow = !f.privacy && amount != 0.0;
    let arrow_width = if arrow { 11.0 } else { 0.0 };
    let percent_width = percent
        .as_ref()
        .map_or(0.0, |galley| galley.size().x + arrow_width);
    let width = label.size().x.max(value.size().x).max(percent_width);
    let percent_height = percent.as_ref().map_or(0.0, |galley| galley.size().y + 3.0);
    let height = label.size().y + 3.0 + value.size().y + percent_height;
    let mut y = anchor.y - height;
    draw(ui, pos2(anchor.x - label.size().x, y), &label);
    y += label.size().y + 3.0;
    draw(ui, pos2(anchor.x - value.size().x, y), &value);
    y += value.size().y + 3.0;
    if let Some(percent) = percent {
        let x = anchor.x - percent.size().x;
        draw(ui, pos2(x, y), &percent);
        if arrow {
            paint_arrow(
                ui,
                pos2(x - 7.0, y + percent.size().y / 2.0),
                amount > 0.0,
                tone_or(tone, secondary),
            );
        }
    }
    anchor.x - width
}

pub struct StatCard<'a> {
    pub title: &'a str,
    pub value: String,
    pub tone: Option<Tone>,
    pub caption: Option<String>,
    /// Quieter value when there is nothing to report yet.
    pub placeholder: bool,
    pub icon: Option<Icon>,
}

impl<'a> StatCard<'a> {
    pub fn new(title: &'a str, value: String) -> Self {
        Self {
            title,
            value,
            tone: None,
            caption: None,
            placeholder: false,
            icon: None,
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

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

pub fn stat_card(ui: &mut egui::Ui, card: &StatCard) {
    let rect = card_frame(ui, CARD_HEIGHT);
    let tint = card.tone.map_or(theme::SYSTEM_ACCENT, tone_color);
    let mut x = rect.left() + 14.0;
    if let Some(icon) = card.icon {
        let badge = Rect::from_center_size(pos2(x + 17.0, rect.center().y), Vec2::splat(34.0));
        ui.painter()
            .rect_filled(badge, 9.0, tint.gamma_multiply(0.14));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(badge.center(), Vec2::splat(17.0)),
            icon,
            tint,
        );
        x = badge.right() + 12.0;
    }
    let max_width = (rect.right() - 14.0 - x).max(20.0);
    let secondary = caption_color();
    let title = line(ui, card.title, kit::size::CAPTION, REGULAR, secondary);
    let value_color = if card.placeholder {
        secondary
    } else {
        tone_or(card.tone, theme::text_primary())
    };
    let value = kit::galley(ui, &card.value, 15.0, SEMIBOLD, value_color, max_width);
    // Always laid out, so cards with and without a caption share one text block.
    let caption = card.caption.as_deref().map(|caption| {
        kit::galley(
            ui,
            caption,
            kit::size::CAPTION,
            REGULAR,
            tone_or(card.tone, secondary),
            max_width,
        )
    });
    let block = title.size().y + 4.0 + value.size().y + 4.0 + title.size().y;
    let mut y = rect.center().y - block / 2.0;
    draw(ui, pos2(x, y), &title);
    y += title.size().y + 4.0;
    draw(ui, pos2(x, y), &value);
    y += value.size().y + 4.0;
    if let Some(caption) = caption {
        draw(ui, pos2(x, y), &caption);
    }
}

pub fn grid(ui: &mut egui::Ui, cards: &[StatCard]) {
    ui.spacing_mut().item_spacing.x = GRID_GAP;
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
            StatCard::new("Cost Basis", f.money(stats.cost_basis)).icon(Icon::Banknote),
            StatCard::new("All-time P&L", f.signed_money(stats.total_pnl()))
                .tone(f.tone(Some(stats.total_pnl())))
                .icon(Icon::ChartUptrend),
            StatCard::new("Unrealized P&L", f.signed_money(stats.unrealized_pnl))
                .tone(f.tone(Some(stats.unrealized_pnl)))
                .caption(stats.unrealized_percent.map(|p| f.signed_percent(p, 2)))
                .icon(Icon::Clock),
            StatCard::new("Realized P&L", f.signed_money(stats.realized_pnl))
                .tone(f.tone(Some(stats.realized_pnl)))
                .icon(Icon::CheckmarkSeal),
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
    let rect = card_frame(ui, CARD_HEIGHT);
    let left = rect.left() + 14.0;
    let right = rect.right() - 14.0;
    let secondary = caption_color();
    let top = rect.top() + 12.0;
    let heading = text_at(
        ui,
        pos2(left, top),
        Align2::LEFT_TOP,
        title,
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let row = Rect::from_min_max(
        pos2(left, heading.bottom() + 8.0),
        pos2(right, rect.bottom() - 12.0),
    );
    let Some(result) = result else {
        text_at(
            ui,
            pos2(left, row.center().y),
            Align2::LEFT_CENTER,
            "—",
            15.0,
            SEMIBOLD,
            secondary,
        );
        return;
    };

    let icon = Rect::from_min_size(row.min, Vec2::splat(row.height().min(30.0)));
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        portfolio_table::asset_icon(ui, &result.asset, icon.width());
    });

    let percent = result
        .pnl_percent
        .map_or("—".to_owned(), |p| f.signed_percent(p, 2));
    let big = line(
        ui,
        &percent,
        15.0,
        SEMIBOLD,
        tone_or(f.tone(result.pnl_percent), theme::text_primary()),
    );
    let small = line(
        ui,
        &f.signed_money(result.total_pnl),
        kit::size::CAPTION,
        REGULAR,
        tone_or(f.tone(Some(result.total_pnl)), secondary),
    );
    let figures_height = big.size().y + 1.0 + small.size().y;
    let figures_top = row.center().y - figures_height / 2.0;
    draw(ui, pos2(right - big.size().x, figures_top), &big);
    draw(
        ui,
        pos2(right - small.size().x, figures_top + big.size().y + 1.0),
        &small,
    );
    let figures_width = big.size().x.max(small.size().x);

    let text_left = icon.right() + 10.0;
    let text_width = (right - figures_width - 12.0 - text_left).max(30.0);
    let ticker = kit::galley(
        ui,
        &result.asset.display_ticker(),
        kit::size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&result.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let names_top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, names_top), &ticker);
    draw(
        ui,
        pos2(text_left, names_top + ticker.size().y + 1.0),
        &name,
    );
}

fn history_card(ui: &mut egui::Ui, state: &mut OverviewState, input: &OverviewInput) {
    panel(ui, HISTORY_CHART_HEIGHT, "Portfolio Value", |header, ui| {
        history_range_control(header, &mut state.range);
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
            HISTORY_CHART_HEIGHT,
        );
    });
}

fn history_range_control(ui: &mut egui::Ui, selected: &mut PortfolioHistoryRange) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(195.0, 27.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 7.0, theme::quaternary(0.6));
    painter.rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    for (index, range) in PortfolioHistoryRange::ALL.into_iter().enumerate() {
        let segment = Rect::from_min_size(
            Pos2::new(rect.left() + 2.0 + index as f32 * 38.2, rect.top() + 2.0),
            Vec2::new(38.2, 23.0),
        );
        let response = ui
            .interact(
                segment,
                egui::Id::new(("portfolio-history-range", range.title())),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            *selected = range;
        }
        let is_selected = *selected == range;
        if is_selected {
            painter.rect_filled(segment, 5.0, theme::chrome());
            painter.rect_stroke(
                segment,
                5.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, 5.0, theme::ink(12));
        }
        text_at(
            ui,
            segment.center(),
            Align2::CENTER_CENTER,
            range.title(),
            11.5,
            if is_selected { MEDIUM } else { REGULAR },
            if is_selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
    }
}

fn top_holdings_panel(ui: &mut Ui, input: &OverviewInput, actions: &mut Vec<OverviewAction>) {
    let f = input.formatter;
    let show_all = input.holdings.len() > TOP_COUNT;
    panel(ui, PANEL_CONTENT_HEIGHT, "Top Holdings", |header, ui| {
        if show_all {
            let link = header
                .add(
                    egui::Label::new(kit::text(
                        "View All",
                        kit::size::SUBHEADLINE,
                        MEDIUM,
                        theme::SYSTEM_ACCENT,
                    ))
                    .sense(Sense::click()),
                )
                .on_hover_cursor(CursorIcon::PointingHand);
            if link.clicked() {
                actions.push(OverviewAction::ShowHoldings);
            }
        }
        let width = ui.available_width();
        let top = ui.max_rect().top();
        let left = ui.max_rect().left();
        for (index, holding) in top_holdings(input.holdings).into_iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(left, top + index as f32 * (TOP_ROW_HEIGHT + TOP_ROW_GAP)),
                vec2(width, TOP_ROW_HEIGHT),
            );
            top_holding_row(ui, row, holding, input, f, actions);
        }
    });
}

fn top_holding_row(
    ui: &mut Ui,
    row: Rect,
    holding: &PortfolioHolding,
    input: &OverviewInput,
    f: PortfolioValueFormatter,
    actions: &mut Vec<OverviewAction>,
) {
    let response = ui
        .interact(
            row,
            egui::Id::new(("portfolio-top-holding", &holding.asset.key)),
            Sense::click(),
        )
        .on_hover_text("Shows this asset's transactions")
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(row, 8.0, theme::ink(18));
    }
    let secondary = caption_color();
    let icon = Rect::from_center_size(
        pos2(row.left() + 8.0 + 13.0, row.center().y),
        Vec2::splat(26.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
        portfolio_table::asset_icon(ui, &holding.asset, 26.0);
    });

    // Right column: the value over an allocation bar and its percentage.
    let right = row.right() - 8.0;
    let value = line(
        ui,
        &holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        kit::size::SUBHEADLINE,
        REGULAR,
        theme::text_primary(),
    );
    let percent = line(
        ui,
        &f.percent(holding.allocation, 1),
        kit::size::CAPTION,
        REGULAR,
        secondary,
    );
    let right_height = value.size().y + 3.0 + percent.size().y;
    let right_top = row.center().y - right_height / 2.0;
    draw(ui, pos2(right - value.size().x, right_top), &value);
    let line_y = right_top + value.size().y + 3.0;
    draw(ui, pos2(right - percent.size().x, line_y), &percent);
    let bar = Rect::from_center_size(
        pos2(right - 44.0 - 6.0 - 32.0, line_y + percent.size().y / 2.0),
        vec2(64.0, 4.0),
    );
    ui.painter().rect_filled(bar, 2.0, theme::ink(28));
    let filled = Rect::from_min_size(
        bar.min,
        vec2(64.0 * holding.allocation.clamp(0.0, 1.0) as f32, 4.0),
    );
    ui.painter().rect_filled(filled, 2.0, theme::SYSTEM_ACCENT);

    let text_left = icon.right() + 10.0;
    let text_width = (bar.left() - 12.0 - text_left)
        .min(right - value.size().x - 12.0 - text_left)
        .max(30.0);
    let ticker = kit::galley(
        ui,
        &holding.asset.display_ticker(),
        kit::size::SUBHEADLINE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let name = kit::galley(
        ui,
        &(input.subtitle)(&holding.asset),
        kit::size::CAPTION,
        REGULAR,
        secondary,
        text_width,
    );
    let top = row.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    draw(ui, pos2(text_left, top), &ticker);
    draw(ui, pos2(text_left, top + ticker.size().y + 1.0), &name);

    if response.clicked() {
        actions.push(OverviewAction::Select(holding.asset.key.clone()));
    }
    let spoken = format!(
        "{}: {} ({} of portfolio)",
        holding.asset.display_ticker(),
        holding.current_value.map_or("—".to_owned(), |v| f.money(v)),
        f.percent(holding.allocation, 1),
    );
    accessibility::label_as(response, egui::WidgetType::Button, spoken);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::chrome())
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
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
            egui::Frame::new()
                .inner_margin(Margin::same(PAGE_INSET))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GRID_GAP, GRID_GAP);
                    section(ui, "Performance", None, false);
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
                    section(ui, "Value", None, true);
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
                    section(ui, "Activity", None, true);
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
                        true,
                    );
                    card(ui, |ui| pnl_bars_view(ui, &stats.asset_results, f));
                });
        });
}

/// A section heading: a semibold title with an optional secondary note, `SECTION_BREAK` below
/// the previous block when it is not the first.
fn section(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>, spaced: bool) {
    if spaced {
        ui.add_space(STATS_SECTION_GAP - GRID_GAP);
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(title, 15.0, SEMIBOLD, theme::text_primary()));
        if let Some(subtitle) = subtitle {
            ui.label(kit::text(
                subtitle,
                kit::size::SUBHEADLINE,
                REGULAR,
                caption_color(),
            ));
        }
    });
    ui.add_space(-2.0);
}

/// Space between the Statistics sections (Swift's `VStack(spacing: 24)`).
const STATS_SECTION_GAP: f32 = 24.0;
const PNL_ROW_HEIGHT: f32 = 26.0;
const PNL_ROW_GAP: f32 = 4.0;

/// Horizontal bars of each asset's total P&L around a zero line: gains grow right, losses left.
fn pnl_bars_view(ui: &mut egui::Ui, results: &[AssetResult], f: PortfolioValueFormatter) {
    if results.is_empty() {
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), Sense::hover());
        text_at(
            ui,
            area.center(),
            Align2::CENTER_CENTER,
            "Priced holdings appear here.",
            12.0,
            REGULAR,
            caption_color(),
        );
        return;
    }
    let (bars, zero) = pnl_bars(results);
    let secondary = caption_color();
    for (result, bar) in results.iter().zip(&bars) {
        let (row, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), PNL_ROW_HEIGHT), Sense::hover());
        ui.add_space(PNL_ROW_GAP - ui.spacing().item_spacing.y);
        if response.hovered() {
            ui.painter()
                .rect_filled(row.expand2(vec2(8.0, 0.0)), 6.0, theme::ink(10));
        }
        let middle = row.center().y;

        // Asset: icon and ticker in a 100 pt column.
        let icon = Rect::from_center_size(pos2(row.left() + 10.0, middle), Vec2::splat(20.0));
        ui.scope_builder(UiBuilder::new().max_rect(icon), |ui| {
            portfolio_table::asset_icon(ui, &result.asset, 20.0);
        });
        let ticker = kit::galley(
            ui,
            &result.asset.display_ticker(),
            kit::size::SUBHEADLINE,
            MEDIUM,
            theme::text_primary(),
            62.0,
        );
        ui.painter().galley(
            pos2(row.left() + 28.0, middle - ticker.size().y / 2.0),
            ticker,
            Color32::PLACEHOLDER,
        );

        // Figures: amount (120 pt) and percentage (80 pt), right-aligned at the row's end.
        let tone = f.tone(Some(result.total_pnl));
        let percent_tone = f.tone(result.pnl_percent);
        let percent = kit::galley(
            ui,
            &result
                .pnl_percent
                .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
            kit::size::SUBHEADLINE,
            REGULAR,
            tone_or(percent_tone, secondary),
            80.0,
        );
        let amount = kit::galley(
            ui,
            &f.signed_money(result.total_pnl),
            kit::size::SUBHEADLINE,
            REGULAR,
            tone_or(tone, theme::text_primary()),
            120.0,
        );
        let percent_right = row.right();
        let amount_right = percent_right - 80.0 - 10.0;
        ui.painter().galley(
            pos2(
                percent_right - percent.size().x,
                middle - percent.size().y / 2.0,
            ),
            percent,
            Color32::PLACEHOLDER,
        );
        ui.painter().galley(
            pos2(
                amount_right - amount.size().x,
                middle - amount.size().y / 2.0,
            ),
            amount,
            Color32::PLACEHOLDER,
        );

        // The bar track fills what the other columns leave.
        let track = Rect::from_min_max(
            pos2(
                row.left() + 100.0 + 10.0,
                middle - (PNL_ROW_HEIGHT - 8.0) / 2.0,
            ),
            pos2(
                amount_right - 120.0 - 10.0,
                middle + (PNL_ROW_HEIGHT - 8.0) / 2.0,
            ),
        );
        if track.width() > 20.0 {
            let zero_x = track.left() + track.width() * zero as f32;
            ui.painter()
                .vline(zero_x, track.y_range(), Stroke::new(1.0, theme::ink(60)));
            // Neutral while values are hidden: red and green would give each sign away.
            let colour = tone_option(tone).map_or(theme::ink(80), |c| c.gamma_multiply(0.85));
            let left = track.left() + track.width() * bar.start as f32;
            let length = (track.width() * bar.length as f32).max(2.0);
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(left, track.top()), vec2(length, track.height())),
                3.0,
                colour,
            );
        }
        accessibility::label_as(
            response,
            egui::WidgetType::Other,
            format!(
                "{}: {} ({})",
                result.asset.display_ticker(),
                f.signed_money(result.total_pnl),
                result
                    .pnl_percent
                    .map_or("—".to_owned(), |p| f.signed_percent(p, 2)),
            ),
        );
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
