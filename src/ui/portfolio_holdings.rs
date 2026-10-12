//! The selected portfolio's positions as a sortable table.
//!
//! The sort column follows the portfolio's saved `PortfolioHoldingsSort`; clicking a header
//! that the saved sort has a case for writes it back. Double-click (or Return) opens the
//! asset's transactions. Pure over `PortfolioHolding`s: the host prices them and persists.

// Transient: the Portfolio window hosts this once the portfolio store is wired.
#![allow(dead_code)]

use crate::model::portfolio::{PortfolioAsset, PortfolioHolding, PortfolioHoldingsSort};
use crate::model::portfolio_format::PortfolioValueFormatter;
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, SEMIBOLD};
use crate::ui::portfolio_charts;
use crate::ui::portfolio_grid::{self, Col, HEADER_HEIGHT, INSET, ROW_INSET};
use crate::ui::portfolio_table as table;
use crate::ui::theme;
use eframe::egui::{self, Color32, Key, Modifiers, Rect, Sense, Stroke, Vec2, pos2, vec2};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldingsColumn {
    Asset,
    Price,
    DayChange,
    Quantity,
    AverageCost,
    Value,
    Allocation,
    ProfitLoss,
}

impl HoldingsColumn {
    pub const ALL: [Self; 8] = [
        Self::Asset,
        Self::Price,
        Self::DayChange,
        Self::Quantity,
        Self::AverageCost,
        Self::Value,
        Self::Allocation,
        Self::ProfitLoss,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Asset => "Asset",
            Self::Price => "Price",
            Self::DayChange => "24h %",
            Self::Quantity => "Quantity",
            Self::AverageCost => "Avg Cost",
            Self::Value => "Value",
            Self::Allocation => "Allocation",
            Self::ProfitLoss => "P&L",
        }
    }

    /// The saved sort a click on this column writes, for the columns the portfolio remembers.
    pub fn saved_sort(self) -> Option<PortfolioHoldingsSort> {
        match self {
            Self::Asset => Some(PortfolioHoldingsSort::Asset),
            Self::DayChange => Some(PortfolioHoldingsSort::DayChange),
            Self::Value => Some(PortfolioHoldingsSort::CurrentValue),
            Self::Allocation => Some(PortfolioHoldingsSort::Allocation),
            Self::ProfitLoss => Some(PortfolioHoldingsSort::ProfitLoss),
            Self::Price | Self::Quantity | Self::AverageCost => None,
        }
    }

    /// The column a saved sort shows. The table has one P&L column; it sorts by amount.
    pub fn for_saved(sort: PortfolioHoldingsSort) -> Self {
        match sort {
            PortfolioHoldingsSort::Asset => Self::Asset,
            PortfolioHoldingsSort::CurrentValue => Self::Value,
            PortfolioHoldingsSort::Allocation => Self::Allocation,
            PortfolioHoldingsSort::DayChange => Self::DayChange,
            PortfolioHoldingsSort::ProfitLoss | PortfolioHoldingsSort::ProfitLossPercent => {
                Self::ProfitLoss
            }
        }
    }

    /// A first click sorts names A to Z and numbers largest first.
    pub fn default_ascending(self) -> bool {
        self == Self::Asset
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoldingRow {
    pub holding: PortfolioHolding,
    pub subtitle: String,
}

impl HoldingRow {
    pub fn ticker(&self) -> String {
        self.holding.asset.display_ticker()
    }

    pub fn key(&self) -> &str {
        &self.holding.asset.key
    }
}

/// Rows matching the search (ticker, symbol, name, subtitle), sorted by `column`. An
/// unpriced value sorts as the lowest: last when largest-first.
pub fn rows(
    holdings: &[PortfolioHolding],
    subtitle: &dyn Fn(&PortfolioAsset) -> String,
    search: &str,
    column: HoldingsColumn,
    ascending: bool,
) -> Vec<HoldingRow> {
    let needle = search.trim().to_lowercase();
    let mut rows: Vec<HoldingRow> = holdings
        .iter()
        .map(|holding| HoldingRow {
            subtitle: subtitle(&holding.asset),
            holding: holding.clone(),
        })
        .filter(|row| {
            needle.is_empty()
                || [
                    row.ticker(),
                    row.holding.asset.symbol.clone(),
                    row.holding.asset.name.clone(),
                    row.subtitle.clone(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&needle))
        })
        .collect();
    rows.sort_by(|a, b| {
        let order = compare(a, b, column);
        if ascending { order } else { order.reverse() }
    });
    rows
}

fn number(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

fn compare(a: &HoldingRow, b: &HoldingRow, column: HoldingsColumn) -> Ordering {
    let (x, y) = (&a.holding, &b.holding);
    match column {
        HoldingsColumn::Asset => a.ticker().to_lowercase().cmp(&b.ticker().to_lowercase()),
        HoldingsColumn::Price => number(x.current_price).total_cmp(&number(y.current_price)),
        HoldingsColumn::DayChange => {
            number(x.day_change_percent()).total_cmp(&number(y.day_change_percent()))
        }
        HoldingsColumn::Quantity => x.quantity.total_cmp(&y.quantity),
        HoldingsColumn::AverageCost => x.average_cost.total_cmp(&y.average_cost),
        HoldingsColumn::Value => number(x.current_value).total_cmp(&number(y.current_value)),
        HoldingsColumn::Allocation => x.allocation.total_cmp(&y.allocation),
        HoldingsColumn::ProfitLoss => number(x.total_pnl()).total_cmp(&number(y.total_pnl())),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HoldingsAction {
    /// Write the portfolio's saved sort.
    SetSort(PortfolioHoldingsSort),
    /// Open this asset's transactions (double-click, or the menu).
    Open(String),
    /// Remap the asset to another market.
    Remap(String),
}

pub struct HoldingsInput<'a> {
    pub holdings: &'a [PortfolioHolding],
    pub saved_sort: PortfolioHoldingsSort,
    pub formatter: PortfolioValueFormatter,
    /// Prices have not arrived yet: rows would be unpriced and reorder when they do.
    pub is_loading: bool,
    /// "Bitcoin", or the stored label.
    pub subtitle: &'a dyn Fn(&PortfolioAsset) -> String,
}

pub struct HoldingsState {
    pub search: String,
    column: HoldingsColumn,
    ascending: bool,
    selected: Option<String>,
    /// Whether the arrow keys and Return act on the table: set by a click on a row, cleared
    /// while the search field has the keyboard.
    table_focused: bool,
    scroll_y: f32,
    viewport_height: f32,
    /// Where the next frame scrolls to, to keep a keyboard-moved selection in view.
    scroll_to: Option<f32>,
    /// The saved sort the table last adopted, so switching portfolios re-applies it.
    adopted: Option<PortfolioHoldingsSort>,
}

impl Default for HoldingsState {
    fn default() -> Self {
        Self {
            search: String::new(),
            column: HoldingsColumn::Value,
            ascending: false,
            selected: None,
            table_focused: false,
            scroll_y: 0.0,
            viewport_height: 0.0,
            scroll_to: None,
            adopted: None,
        }
    }
}

impl HoldingsState {
    pub fn column(&self) -> HoldingsColumn {
        self.column
    }

    pub fn is_ascending(&self) -> bool {
        self.ascending
    }

    /// Follows the portfolio's saved sort when it changes (another portfolio, a write).
    pub fn adopt(&mut self, saved: PortfolioHoldingsSort) {
        if self.adopted == Some(saved) {
            return;
        }
        self.adopted = Some(saved);
        self.column = HoldingsColumn::for_saved(saved);
        self.ascending = self.column.default_ascending();
    }

    /// A click on a header: flips the direction on the sorted column, otherwise sorts by it.
    /// Returns the saved sort to write when this column is one the portfolio remembers and
    /// differs from what is saved.
    pub fn click(
        &mut self,
        column: HoldingsColumn,
        saved: PortfolioHoldingsSort,
    ) -> Option<PortfolioHoldingsSort> {
        if self.column == column {
            self.ascending = !self.ascending;
        } else {
            self.column = column;
            self.ascending = column.default_ascending();
        }
        let sort = column.saved_sort()?;
        (HoldingsColumn::for_saved(saved) != column).then(|| {
            self.adopted = Some(sort);
            sort
        })
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
) -> Vec<HoldingsAction> {
    let mut actions = Vec::new();
    state.adopt(input.saved_sort);

    // Toolbar: the search field, padded like the table beneath it.
    let search_id = egui::Id::new("portfolio-holdings-search");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        kit::search_field(
            ui,
            search_id,
            &mut state.search,
            "Search asset or name",
            246.0,
        );
    });
    ui.add_space(12.0);
    if ui.memory(|memory| memory.has_focus(search_id)) {
        state.table_focused = false;
    }

    let shown = rows(
        input.holdings,
        input.subtitle,
        &state.search,
        state.column,
        state.ascending,
    );
    let table_height = (ui.available_height() - FOOTER_HEIGHT).max(140.0);
    let (outer, _) = ui.allocate_exact_size(
        vec2(ui.available_width() - 2.0 * INSET, table_height),
        Sense::hover(),
    );
    // `allocate_exact_size` starts at the cursor; shift in by the inset.
    let outer = outer.translate(vec2(INSET, 0.0));
    portfolio_grid::paint_frame(ui, outer);
    if input.is_loading && !input.holdings.is_empty() {
        loading_message(ui, outer);
    } else if shown.is_empty() {
        let (icon, title, detail) = if input.holdings.is_empty() {
            (
                Icon::Briefcase,
                "No Holdings".to_owned(),
                "Positions appear once a transaction leaves you holding an asset.",
            )
        } else {
            (
                Icon::Magnifier,
                format!("No Results for \u{201C}{}\u{201D}", state.search.trim()),
                "Check the spelling or try a new search.",
            )
        };
        portfolio_charts::empty_message(ui, outer, icon, &title, detail);
    } else {
        table_view(ui, state, input, outer, &shown, &mut actions);
    }

    footer(ui, shown.len(), input.holdings.len(), input.formatter);
    actions
}

/// A spinner and "Loading prices…" in the middle of the table.
fn loading_message(ui: &mut egui::Ui, area: Rect) {
    let label = kit::galley(
        ui,
        "Loading prices\u{2026}",
        12.0,
        kit::REGULAR,
        theme::text_secondary(),
        area.width(),
    );
    let width = 16.0 + 8.0 + label.size().x;
    let left = area.center().x - width / 2.0;
    ui.put(
        Rect::from_center_size(pos2(left + 8.0, area.center().y), Vec2::splat(16.0)),
        egui::Spinner::new().size(16.0),
    );
    ui.painter().galley(
        pos2(left + 24.0, area.center().y - label.size().y / 2.0),
        label,
        Color32::PLACEHOLDER,
    );
}

/// "8 assets" under the table's left edge, the currency and the double-click hint opposite.
fn footer(ui: &mut egui::Ui, shown: usize, total: usize, formatter: PortfolioValueFormatter) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FOOTER_HEIGHT), Sense::hover());
    let middle = rect.center().y + 1.0;
    let count = table::count_text(shown, total, ("asset", "assets"));
    let secondary = theme::text_secondary();
    let left = kit::galley(
        ui,
        &count,
        kit::size::CAPTION,
        kit::REGULAR,
        secondary,
        400.0,
    );
    ui.painter().galley(
        pos2(rect.left() + INSET + 2.0, middle - left.size().y / 2.0),
        left,
        Color32::PLACEHOLDER,
    );
    let note = format!(
        "Values in {} \u{00B7} Double-click a row for its transactions",
        formatter.currency.code()
    );
    let right = kit::galley(
        ui,
        &note,
        kit::size::CAPTION,
        kit::REGULAR,
        secondary,
        600.0,
    );
    ui.painter().galley(
        pos2(
            rect.right() - INSET - 2.0 - right.size().x,
            middle - right.size().y / 2.0,
        ),
        right,
        Color32::PLACEHOLDER,
    );
}

const FOOTER_HEIGHT: f32 = 34.0;
const ROW_HEIGHT: f32 = 46.0;
/// Space between the header rule and the first row, and under the last.
const BODY_PADDING: f32 = 4.0;
const MENU_BUTTON: f32 = 24.0;

/// The columns, in `HoldingsColumn::ALL` order. Widths follow the Swift table's ideal widths.
const COLUMNS: [Col; 8] = [
    Col::new("Asset", 130.0, 150.0, false),
    Col::new("Price", 84.0, 100.0, true),
    Col::new("24h %", 64.0, 70.0, true),
    Col::new("Quantity", 70.0, 90.0, true),
    Col::new("Avg Cost", 84.0, 100.0, true),
    Col::new("Value", 84.0, 105.0, true),
    Col::new("Allocation", 76.0, 76.0, true),
    // Row actions sit at the end of the last column, as in the Transactions table.
    Col::new("P&L", 130.0, 140.0, true).trailing(MENU_BUTTON + 6.0),
];

fn table_view(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    input: &HoldingsInput,
    outer: Rect,
    shown: &[HoldingRow],
    actions: &mut Vec<HoldingsAction>,
) {
    let f = input.formatter;
    let inner_left = outer.left() + ROW_INSET;
    let inner_width = outer.width() - 2.0 * ROW_INSET;

    // Header: sticky above the scrolling rows.
    let header_rects = portfolio_grid::column_rects(
        &COLUMNS,
        inner_left,
        outer.top(),
        HEADER_HEIGHT,
        inner_width,
    );
    let mut clicked_header = None;
    for ((column, rect), kind) in COLUMNS.iter().zip(&header_rects).zip(HoldingsColumn::ALL) {
        let sorted = (state.column == kind).then_some(state.ascending);
        if portfolio_grid::header_cell(
            ui,
            egui::Id::new(("portfolio-holdings-header", kind.title())),
            *rect,
            column,
            sorted,
        ) {
            clicked_header = Some(kind);
        }
    }
    ui.painter().hline(
        outer.x_range().shrink(1.0),
        outer.top() + HEADER_HEIGHT,
        Stroke::new(1.0, theme::separator()),
    );

    // Keyboard: arrows move the selection, Return opens it.
    let keyboard = state.table_focused && ui.memory(|memory| memory.focused().is_none());
    if keyboard {
        let step = ui.input_mut(|input| {
            i32::from(input.consume_key(Modifiers::NONE, Key::ArrowDown))
                - i32::from(input.consume_key(Modifiers::NONE, Key::ArrowUp))
        });
        if step != 0 {
            let current = state
                .selected
                .as_deref()
                .and_then(|key| shown.iter().position(|row| row.key() == key));
            let next = match current {
                Some(index) => (index as i32 + step).clamp(0, shown.len() as i32 - 1) as usize,
                None if step > 0 => 0,
                None => shown.len() - 1,
            };
            state.selected = Some(shown[next].key().to_owned());
            let top = next as f32 * ROW_HEIGHT;
            if top < state.scroll_y {
                state.scroll_to = Some(top);
            } else if top + ROW_HEIGHT > state.scroll_y + state.viewport_height {
                state.scroll_to = Some(top + ROW_HEIGHT - state.viewport_height);
            }
        }
        if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
            && let Some(key) = &state.selected
            && shown.iter().any(|row| row.key() == key)
        {
            actions.push(HoldingsAction::Open(key.clone()));
        }
    }

    // Body.
    let body = Rect::from_min_max(
        pos2(outer.left(), outer.top() + HEADER_HEIGHT + BODY_PADDING),
        pos2(outer.right(), outer.bottom() - BODY_PADDING),
    );
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(body.intersect(ui.clip_rect()));
    body_ui.spacing_mut().item_spacing = Vec2::ZERO;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("portfolio-holdings-rows")
        .auto_shrink([false, false])
        .max_height(body.height());
    if let Some(target) = state.scroll_to.take() {
        area = area.vertical_scroll_offset(target.max(0.0));
    }
    let output = area.show_rows(&mut body_ui, ROW_HEIGHT, shown.len(), |ui, range| {
        for index in range {
            let item = &shown[index];
            holding_row(ui, state, item, index, f, inner_left, inner_width, actions);
        }
    });
    state.scroll_y = output.state.offset.y;
    state.viewport_height = output.inner_rect.height();

    if let Some(column) = clicked_header
        && let Some(sort) = state.click(column, input.saved_sort)
    {
        actions.push(HoldingsAction::SetSort(sort));
    }
}

#[allow(clippy::too_many_arguments)]
fn holding_row(
    ui: &mut egui::Ui,
    state: &mut HoldingsState,
    item: &HoldingRow,
    index: usize,
    f: PortfolioValueFormatter,
    inner_left: f32,
    inner_width: f32,
    actions: &mut Vec<HoldingsAction>,
) {
    let holding = &item.holding;
    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let row = Rect::from_min_max(
        pos2(inner_left, slot.top() + 1.0),
        pos2(inner_left + inner_width, slot.bottom() - 1.0),
    );
    let selected = state.selected.as_deref() == Some(item.key());
    if selected {
        ui.painter()
            .rect_filled(row, 7.0, theme::with_alpha(theme::SYSTEM_ACCENT, 0.16));
    } else if response.hovered() {
        ui.painter().rect_filled(row, 7.0, theme::ink(12));
    } else if index % 2 == 1 {
        ui.painter().rect_filled(row, 7.0, theme::ink(6));
    }

    let cells =
        portfolio_grid::column_rects(&COLUMNS, row.left(), row.top(), row.height(), row.width());
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    // Asset: icon, ticker over name.
    let asset = cells[0];
    let icon = Rect::from_center_size(
        pos2(
            asset.left() + portfolio_grid::CELL_PAD + 12.0,
            asset.center().y,
        ),
        Vec2::splat(24.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(icon), |ui| {
        table::asset_icon(ui, &holding.asset, 24.0);
    });
    let text_left = icon.right() + 10.0;
    let text_width = (asset.right() - text_left - 6.0).max(20.0);
    let ticker = kit::galley(
        ui,
        &item.ticker(),
        kit::size::BODY,
        SEMIBOLD,
        primary,
        text_width,
    );
    let name = kit::galley(
        ui,
        &item.subtitle,
        kit::size::CAPTION,
        kit::REGULAR,
        secondary,
        text_width,
    );
    let top = asset.center().y - (ticker.size().y + 1.0 + name.size().y) / 2.0;
    let name_y = top + ticker.size().y + 1.0;
    ui.painter()
        .galley(pos2(text_left, top), ticker, Color32::PLACEHOLDER);
    ui.painter()
        .galley(pos2(text_left, name_y), name, Color32::PLACEHOLDER);

    let dash = || "\u{2014}".to_owned();
    portfolio_grid::figure(
        ui,
        cells[1],
        &COLUMNS[1],
        &holding.current_price.map_or_else(dash, |v| f.price(v)),
        primary,
    );
    let day = holding.day_change_percent();
    portfolio_grid::figure(
        ui,
        cells[2],
        &COLUMNS[2],
        &day.map_or_else(dash, |v| f.signed_percent(v, 2)),
        table::tone_option(f.tone(day)).unwrap_or(secondary),
    );
    portfolio_grid::figure(
        ui,
        cells[3],
        &COLUMNS[3],
        &f.quantity(holding.quantity, Default::default()),
        primary,
    );
    portfolio_grid::figure(
        ui,
        cells[4],
        &COLUMNS[4],
        &f.price(holding.average_cost),
        primary,
    );
    portfolio_grid::cell_text(
        ui,
        cells[5],
        &COLUMNS[5],
        &holding.current_value.map_or_else(dash, |v| f.money(v)),
        kit::size::BODY,
        kit::MEDIUM,
        primary,
    );
    portfolio_grid::figure(
        ui,
        cells[6],
        &COLUMNS[6],
        &f.percent(holding.allocation, 1),
        secondary,
    );
    let pnl = holding.total_pnl();
    portfolio_grid::cell_text(
        ui,
        cells[7],
        &COLUMNS[7],
        &pnl.map_or_else(dash, |v| f.signed_money(v)),
        kit::size::BODY,
        kit::MEDIUM,
        table::tone_option(f.tone(pnl)).unwrap_or(primary),
    );

    // The row's actions menu, at the end of the last column.
    let button = Rect::from_center_size(
        pos2(
            cells[7].right() - 8.0 - MENU_BUTTON / 2.0,
            cells[7].center().y,
        ),
        Vec2::splat(MENU_BUTTON),
    );
    let menu = ui
        .interact(
            button,
            egui::Id::new(("portfolio-holding-menu", item.key())),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), menu.id.with("popup"));
    if menu.hovered() || open {
        ui.painter().rect_filled(button, 6.0, theme::ink(18));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(button.center(), Vec2::splat(15.0)),
        Icon::Ellipsis,
        if menu.hovered() || open {
            primary
        } else {
            secondary
        },
    );
    let ticker_text = item.ticker();
    let menu = accessibility::label(menu, format!("Actions for {ticker_text}"));
    egui::Popup::menu(&menu)
        .id(menu.id.with("popup"))
        .frame(kit::menu_frame())
        .gap(4.0)
        .show(|ui| row_menu(ui, item.key(), actions));

    if response.clicked() || response.secondary_clicked() {
        state.selected = Some(item.key().to_owned());
        state.table_focused = true;
    }
    if response.double_clicked() {
        actions.push(HoldingsAction::Open(item.key().to_owned()));
    }
    egui::Popup::context_menu(&response)
        .frame(kit::menu_frame())
        .show(|ui| row_menu(ui, item.key(), actions));
    let spoken = format!(
        "{ticker_text}, {}, {}",
        item.subtitle,
        holding.current_value.map_or_else(dash, |v| f.money(v)),
    );
    accessibility::label_as(response, egui::WidgetType::Button, spoken);
}

fn row_menu(ui: &mut egui::Ui, key: &str, actions: &mut Vec<HoldingsAction>) {
    ui.set_width(190.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    if kit::menu_item(ui, Icon::List, "View Transactions", true, false) {
        actions.push(HoldingsAction::Open(key.to_owned()));
        ui.close();
    }
    if kit::menu_item(ui, Icon::ArrowLeftRight, "Remap Asset\u{2026}", true, false) {
        actions.push(HoldingsAction::Remap(key.to_owned()));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::PortfolioCurrency;
    use std::collections::HashMap;

    fn holding(
        ticker: &str,
        name: &str,
        value: Option<f64>,
        cost: f64,
        qty: f64,
    ) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{ticker}USDT"),
                symbol: format!("{ticker}/USDT"),
                name: name.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity: qty,
            cost_basis: cost,
            average_cost: if qty == 0.0 { 0.0 } else { cost / qty },
            realized_pnl: 0.0,
            current_price: value.map(|v| v / qty),
            previous_day_price: value.map(|v| v / qty / 1.1),
            current_value: value,
            unrealized_pnl: value.map(|v| v - cost),
            allocation: value.unwrap_or(0.0) / 1_000.0,
            is_price_stale: false,
        }
    }

    fn fixtures() -> Vec<PortfolioHolding> {
        vec![
            holding("BTC", "Bitcoin", Some(600.0), 500.0, 0.01),
            holding("ETH", "Ethereum", Some(300.0), 350.0, 0.1),
            holding("DOGE", "Dogecoin", None, 50.0, 400.0),
            holding("ADA", "Cardano", Some(100.0), 60.0, 200.0),
        ]
    }

    fn tickers(rows: &[HoldingRow]) -> Vec<String> {
        rows.iter().map(HoldingRow::ticker).collect()
    }

    fn subtitle(asset: &PortfolioAsset) -> String {
        asset.name.clone()
    }

    #[test]
    fn value_sorts_largest_first_with_unpriced_assets_last() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, false);
        assert_eq!(tickers(&found), ["BTC", "ETH", "ADA", "DOGE"]);
        let reversed = rows(&fixtures(), &subtitle, "", HoldingsColumn::Value, true);
        assert_eq!(tickers(&reversed), ["DOGE", "ADA", "ETH", "BTC"]);
    }

    #[test]
    fn assets_sort_a_to_z_ignoring_case() {
        let found = rows(&fixtures(), &subtitle, "", HoldingsColumn::Asset, true);
        assert_eq!(tickers(&found), ["ADA", "BTC", "DOGE", "ETH"]);
    }

    #[test]
    fn profit_and_loss_sorts_by_amount_with_unknown_last() {
        let found = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::ProfitLoss,
            false,
        );
        assert_eq!(tickers(&found), ["BTC", "ADA", "ETH", "DOGE"]);
    }

    #[test]
    fn quantity_and_allocation_sort_numerically() {
        let by_quantity = rows(&fixtures(), &subtitle, "", HoldingsColumn::Quantity, false);
        assert_eq!(tickers(&by_quantity), ["DOGE", "ADA", "ETH", "BTC"]);
        let by_allocation = rows(
            &fixtures(),
            &subtitle,
            "",
            HoldingsColumn::Allocation,
            false,
        );
        assert_eq!(by_allocation[0].ticker(), "BTC");
    }

    #[test]
    fn search_matches_ticker_pair_name_and_subtitle() {
        let all = fixtures();
        for (needle, expected) in [
            ("btc", vec!["BTC"]),
            ("COIN", vec!["BTC", "DOGE"]),
            ("usdt", vec!["BTC", "ETH", "DOGE", "ADA"]),
            ("  card ", vec!["ADA"]),
            ("zzz", vec![]),
        ] {
            let mut found = tickers(&rows(&all, &subtitle, needle, HoldingsColumn::Value, false));
            found.sort();
            let mut want: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
            want.sort();
            assert_eq!(found, want, "{needle:?}");
        }
    }

    #[test]
    fn saved_sorts_map_onto_columns_and_back() {
        use PortfolioHoldingsSort::*;
        for (sort, column) in [
            (Asset, HoldingsColumn::Asset),
            (CurrentValue, HoldingsColumn::Value),
            (Allocation, HoldingsColumn::Allocation),
            (DayChange, HoldingsColumn::DayChange),
            (ProfitLoss, HoldingsColumn::ProfitLoss),
            (ProfitLossPercent, HoldingsColumn::ProfitLoss),
        ] {
            assert_eq!(HoldingsColumn::for_saved(sort), column);
        }
        assert_eq!(HoldingsColumn::Price.saved_sort(), None);
        assert_eq!(HoldingsColumn::Quantity.saved_sort(), None);
        assert_eq!(HoldingsColumn::AverageCost.saved_sort(), None);
        assert_eq!(HoldingsColumn::Value.saved_sort(), Some(CurrentValue));
    }

    #[test]
    fn a_header_click_flips_or_switches_and_writes_back_only_remembered_columns() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::CurrentValue);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Value, false)
        );

        // Clicking the sorted column flips direction; the saved sort already names it.
        assert_eq!(
            state.click(HoldingsColumn::Value, PortfolioHoldingsSort::CurrentValue),
            None
        );
        assert!(state.is_ascending());

        // A new remembered column is written back, names A to Z, numbers largest first.
        assert_eq!(
            state.click(HoldingsColumn::Asset, PortfolioHoldingsSort::CurrentValue),
            Some(PortfolioHoldingsSort::Asset)
        );
        assert!(state.is_ascending());
        assert_eq!(
            state.click(HoldingsColumn::Allocation, PortfolioHoldingsSort::Asset),
            Some(PortfolioHoldingsSort::Allocation)
        );
        assert!(!state.is_ascending());

        // A transient column sorts the view but writes nothing.
        assert_eq!(
            state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Allocation),
            None
        );
        assert_eq!(state.column(), HoldingsColumn::Price);
    }

    #[test]
    fn switching_portfolios_adopts_the_other_ones_saved_sort_once() {
        let mut state = HoldingsState::default();
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(
            (state.column(), state.is_ascending()),
            (HoldingsColumn::Asset, true)
        );
        state.click(HoldingsColumn::Price, PortfolioHoldingsSort::Asset);
        // The same saved sort again must not undo the user's transient choice...
        state.adopt(PortfolioHoldingsSort::Asset);
        assert_eq!(state.column(), HoldingsColumn::Price);
        // ...but a different one (another portfolio) is followed.
        state.adopt(PortfolioHoldingsSort::DayChange);
        assert_eq!(state.column(), HoldingsColumn::DayChange);
        assert!(!state.is_ascending());
    }
}
