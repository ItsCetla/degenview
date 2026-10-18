//! Table view: the bars a chart draws as rows, newest first, with the values of the indicators
//! and scripts it shows.
//!
//! A docked panel under the charts. Only the rows on screen are laid out, so a long history costs
//! no more than a short one; the numbers are written by `model::chart_table`, the way the chart
//! writes them.

use crate::model::PriceFormat;
use crate::model::chart_table::{Column as Field, Order, TableFeed};
use crate::ui::icons::Icon;
use crate::ui::portfolio_table::{HEADER_HEIGHT, header_cell};
use crate::ui::{icons, kit, theme};
use eframe::egui::{self, Align, Layout, RichText};
use egui_extras::{Column, TableBuilder};

const ROW_HEIGHT: f32 = 24.0;

/// What the panel keeps between frames.
#[derive(Default)]
pub struct TableViewState {
    pub order: Order,
}

/// What the panel asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableAction {
    Close,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut TableViewState,
    title: &str,
    feed: Option<&TableFeed>,
    price: PriceFormat,
) -> Option<TableAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(kit::text(
            "Table view",
            kit::size::HEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        let detail = match feed {
            Some(feed) => format!("{title} · {} · {} bars", feed.bars_label, feed.len()),
            None => title.to_owned(),
        };
        ui.label(kit::text(
            &detail,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            let close = icons::button(
                ui,
                Icon::Xmark,
                24.0,
                theme::text_secondary(),
                "Close table view",
            );
            if close.clicked() {
                action = Some(TableAction::Close);
            }
        });
    });
    ui.add_space(4.0);
    let Some(feed) = feed.filter(|feed| !feed.is_empty()) else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No bars to show yet.").weak());
        });
        return action;
    };
    let columns = feed.columns();
    let height = ui.available_height();
    let mut builder = TableBuilder::new(ui)
        .id_salt("chart-table")
        .striped(true)
        .max_scroll_height(height)
        .column(Column::initial(140.0).at_least(120.0));
    for _ in columns.iter().skip(1) {
        builder = builder.column(Column::initial(96.0).at_least(70.0));
    }
    builder
        .header(HEADER_HEIGHT, |mut header| {
            for (index, column) in columns.iter().enumerate() {
                header.col(|ui| {
                    let sorted =
                        (*column == Field::Time).then_some(state.order == Order::OldestFirst);
                    let sortable = *column == Field::Time;
                    if header_cell(ui, &feed.title(*column), sorted, sortable, index > 0) {
                        state.order = state.order.flipped();
                    }
                });
            }
        })
        .body(|body| {
            let order = state.order;
            body.rows(ROW_HEIGHT, feed.len(), |mut row| {
                let Some(bar) = feed.bar_at(row.index(), order) else {
                    return;
                };
                for column in &columns {
                    let text = feed.cell(bar, *column, price);
                    row.col(|ui| {
                        let color = match column {
                            Field::Close => Some(if feed.bars[bar].is_up() {
                                theme::UP
                            } else {
                                theme::DOWN
                            }),
                            _ => None,
                        };
                        cell(ui, &text, *column != Field::Time, color);
                    });
                }
            });
        });
    action
}

fn cell(ui: &mut egui::Ui, text: &str, right: bool, color: Option<egui::Color32>) {
    let color = color.unwrap_or_else(theme::text_primary);
    let label = kit::text(text, kit::size::BODY, kit::REGULAR, color);
    if right {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(label);
        });
    } else {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.label(label);
        });
    }
}
