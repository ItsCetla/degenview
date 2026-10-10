//! The chart settings sheet: Appearance, Indicators and Scripts above "Remove Chart" and "Done".
//! Every control writes through to the chart as it changes.

use super::chart_card::ChartCard;
use super::icons::{self, Icon};
use super::settings_card;
use super::theme;
use crate::model::pine_instance::PineInstance;
use crate::model::{
    BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, DataSource, EMA_PERIODS, IndicatorConfig,
    RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD, SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
};
use crate::pine::compiler;
use crate::pine::program::InputSchema;
use crate::pine::token::Limits;
use crate::script_manager::drawer;
use crate::script_manager::inputs;
use crate::script_manager::model::{LocalScript, ScriptType};
use crate::script_manager::registry::LibraryRegistry;
use crate::script_manager::store::ScriptStore;
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Margin, Rect, RichText, Sense,
    Stroke, Ui, Vec2, pos2, vec2,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const WIDTH: f32 = 560.0;
const PAGE_HEIGHT: f32 = 440.0;
const SCRIPT_REFRESH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Indicators,
    Scripts,
    Appearance,
}

impl Tab {
    const ALL: [Self; 3] = [Self::Indicators, Self::Scripts, Self::Appearance];

    fn title(self) -> &'static str {
        match self {
            Self::Indicators => "Indicators",
            Self::Scripts => "Scripts",
            Self::Appearance => "Appearance",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Indicators => Icon::Waveform,
            Self::Scripts => Icon::Curlybraces,
            Self::Appearance => Icon::Palette,
        }
    }
}

#[derive(Default)]
pub struct ChartSettingsState {
    pub tab: Tab,
    editing: Option<u64>,
    confirming_removal: bool,
    scripts: Vec<LocalScript>,
    scripts_read_at: Option<Instant>,
    /// Each instance's input schema, by id, with the source it was compiled from.
    schemas: HashMap<u64, (String, InputSchema)>,
}

/// What the sheet reads and edits for one chart.
pub struct ChartSettingsContext<'a> {
    pub title: &'a str,
    pub symbol: &'a str,
    pub source: DataSource,
    pub indicators: &'a mut IndicatorConfig,
    pub card: &'a mut ChartCard,
}

/// What the sheet asks the workspace to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChartSettingsAction {
    Done,
    RemoveChart,
    OpenScriptManager,
}

pub fn show(
    ctx: &egui::Context,
    state: &mut ChartSettingsState,
    mut c: ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    egui::Modal::new(Id::new("chart-settings")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.add_space(24.0);
        egui::Frame::new()
            .inner_margin(Margin::symmetric(24, 0))
            .show(ui, |ui| header(ui, &c));
        ui.add_space(16.0);
        tab_bar(ui, state);
        ui.add_space(16.0);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt(("chart-settings-page", state.tab as u8))
            .max_height(PAGE_HEIGHT)
            .min_scrolled_height(PAGE_HEIGHT)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(Margin::same(24))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 48.0 - 14.0);
                        match state.tab {
                            Tab::Indicators => indicators_page(ui, &mut c),
                            Tab::Scripts => {
                                if let Some(next) = scripts_page(ui, state, &mut c) {
                                    action = Some(next);
                                }
                            }
                            Tab::Appearance => appearance_page(ui, &mut c),
                        }
                    });
            });
        ui.separator();
        egui::Frame::new()
            .inner_margin(Margin::symmetric(24, 0))
            .show(ui, |ui| {
                if let Some(next) = footer(ui, state, c.title) {
                    action = Some(next);
                }
            });
        ui.add_space(10.0);
    });
    if action.is_none()
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        && !state.confirming_removal
    {
        action = Some(ChartSettingsAction::Done);
    }
    action
}

fn header(ui: &mut Ui, c: &ChartSettingsContext) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        match crate::service::icon::get_url(c.source, c.symbol) {
            Some(url) => {
                egui::Image::new(url)
                    .fit_to_exact_size(Vec2::splat(44.0))
                    .paint_at(ui, rect.shrink(1.0));
            }
            None => {
                crate::service::icon::request(c.source, c.symbol);
                ui.painter()
                    .circle_filled(rect.center(), 22.0, theme::white_alpha(30));
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    c.title.chars().next().unwrap_or('?').to_string(),
                    FontId::proportional(18.0),
                    theme::TEXT_PRIMARY,
                );
            }
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(c.title)
                    .size(17.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.label(
                RichText::new(format!(
                    "{} · Changes apply as you make them",
                    c.source.display_name()
                ))
                .size(11.0)
                .color(theme::TEXT_SECONDARY),
            );
        });
    });
}

/// The icon tab bar: one pill per page, the selected one tinted.
fn tab_bar(ui: &mut Ui, state: &mut ChartSettingsState) {
    ui.horizontal(|ui| {
        let font = FontId::proportional(12.0);
        let widths: Vec<f32> = Tab::ALL
            .iter()
            .map(|tab| {
                ui.painter()
                    .layout_no_wrap(tab.title().to_owned(), font.clone(), Color32::WHITE)
                    .size()
                    .x
                    + 44.0
            })
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 6.0 * (Tab::ALL.len() - 1) as f32;
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        ui.spacing_mut().item_spacing.x = 6.0;
        for (tab, width) in Tab::ALL.into_iter().zip(widths) {
            let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
            let selected = state.tab == tab;
            if selected {
                ui.painter()
                    .rect_filled(rect, 8.0, theme::with_alpha(theme::SYSTEM_ACCENT, 0.16));
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 8.0, theme::white_alpha(12));
            }
            let color = if selected {
                theme::SYSTEM_ACCENT
            } else {
                theme::TEXT_SECONDARY
            };
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(rect.min.x + 18.0, rect.center().y), Vec2::splat(14.0)),
                tab.icon(),
                color,
            );
            ui.painter().text(
                pos2(rect.min.x + 32.0, rect.center().y),
                Align2::LEFT_CENTER,
                tab.title(),
                font.clone(),
                color,
            );
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                state.tab = tab;
            }
        }
    });
}

fn footer(ui: &mut Ui, state: &mut ChartSettingsState, title: &str) -> Option<ChartSettingsAction> {
    let mut action = None;
    ui.add_space(10.0);
    if state.confirming_removal {
        ui.label(
            RichText::new(format!("Remove {title}?"))
                .size(13.0)
                .strong()
                .color(theme::TEXT_PRIMARY),
        );
        ui.label(
            RichText::new(
                "The chart leaves this tab; its drawings are kept. To change a chart's market, remove it and add a new one.",
            )
            .size(11.0)
            .color(theme::TEXT_SECONDARY),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Remove Chart").color(Color32::WHITE))
                        .fill(theme::syntax::STRING)
                        .min_size(vec2(110.0, 28.0)),
                )
                .clicked()
            {
                action = Some(ChartSettingsAction::RemoveChart);
            }
            if ui
                .add(egui::Button::new("Cancel").min_size(vec2(80.0, 28.0)))
                .clicked()
            {
                state.confirming_removal = false;
            }
        });
    } else {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("🗑  Remove Chart").color(theme::syntax::STRING),
                    )
                    .min_size(vec2(120.0, 28.0)),
                )
                .clicked()
            {
                state.confirming_removal = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("Done").color(Color32::WHITE))
                            .fill(theme::SYSTEM_ACCENT)
                            .min_size(vec2(80.0, 28.0)),
                    )
                    .clicked()
                {
                    action = Some(ChartSettingsAction::Done);
                }
            });
        });
    }
    ui.add_space(4.0);
    action
}

// MARK: Appearance

fn appearance_page(ui: &mut Ui, c: &mut ChartSettingsContext) {
    let line = c.source.is_line_chart();
    settings_card::section_title(ui, "Colors", Some("Used for rising and falling prices."));
    settings_card::card(
        ui,
        Icon::ChartUptrend,
        if line { "Line up" } else { "Bullish candle" },
        Some(if line {
            "Stretches where the price rises."
        } else {
            "Price closed higher than it opened."
        }),
        |ui| {
            egui::color_picker::color_edit_button_srgb(ui, &mut c.card.appearance_mut().bullish);
        },
    );
    settings_card::card(
        ui,
        Icon::ChartLine,
        if line { "Line down" } else { "Bearish candle" },
        Some(if line {
            "Stretches where the price falls."
        } else {
            "Price closed lower than it opened."
        }),
        |ui| {
            egui::color_picker::color_edit_button_srgb(ui, &mut c.card.appearance_mut().bearish);
        },
    );
    let appearance = c.card.appearance();
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .stroke(Stroke::new(1.0, theme::with_alpha(theme::SEPARATOR, 0.45)))
        .corner_radius(12.0)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(90.0, 42.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, 8.0, theme::EDITOR_BACKGROUND);
                let up = Color32::from_rgb(
                    appearance.bullish[0],
                    appearance.bullish[1],
                    appearance.bullish[2],
                );
                let down = Color32::from_rgb(
                    appearance.bearish[0],
                    appearance.bearish[1],
                    appearance.bearish[2],
                );
                if line {
                    let y = rect.center().y;
                    ui.painter().line_segment(
                        [
                            pos2(rect.left() + 10.0, y + 9.0),
                            pos2(rect.center().x, y - 8.0),
                        ],
                        Stroke::new(2.0, up),
                    );
                    ui.painter().line_segment(
                        [
                            pos2(rect.center().x, y - 8.0),
                            pos2(rect.right() - 10.0, y + 5.0),
                        ],
                        Stroke::new(2.0, down),
                    );
                } else {
                    for (x, color, rise) in [
                        (rect.left() + 25.0, up, true),
                        (rect.left() + 59.0, down, false),
                    ] {
                        ui.painter().line_segment(
                            [pos2(x, rect.top() + 7.0), pos2(x, rect.bottom() - 7.0)],
                            Stroke::new(1.0, color),
                        );
                        let top = if rise {
                            rect.top() + 12.0
                        } else {
                            rect.top() + 19.0
                        };
                        ui.painter().rect_filled(
                            Rect::from_min_size(pos2(x - 5.0, top), vec2(10.0, 12.0)),
                            1.0,
                            color,
                        );
                    }
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new("Preview").strong());
                    ui.label(
                        RichText::new("The chart updates as you pick.")
                            .small()
                            .weak(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Reset Colors").clicked() {
                        let defaults = crate::model::ChartAppearance::default();
                        let appearance = c.card.appearance_mut();
                        appearance.bullish = defaults.bullish;
                        appearance.bearish = defaults.bearish;
                    }
                });
            });
        });

    ui.add_space(18.0);
    let decimals = c.card.appearance().axis_decimals;
    let hint = decimals.map_or_else(
        || "Automatically chooses precision based on price range.".to_owned(),
        |places| {
            format!(
                "Shows {places} decimal place{} on the Y-axis.",
                if places == 1 { "" } else { "s" }
            )
        },
    );
    settings_card::section_title(ui, "Price axis", None);
    settings_card::card(ui, Icon::Hash, "Decimal places", Some(&hint), |ui| {
        egui::ComboBox::from_id_salt("chart-axis-decimals")
            .selected_text(decimals.map_or_else(|| "Auto".to_owned(), |n| n.to_string()))
            .width(78.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut c.card.appearance_mut().axis_decimals, None, "Auto");
                for places in 0..=8 {
                    ui.selectable_value(
                        &mut c.card.appearance_mut().axis_decimals,
                        Some(places),
                        places.to_string(),
                    );
                }
            });
    });
}

// MARK: Indicators

fn indicators_page(ui: &mut Ui, c: &mut ChartSettingsContext) {
    settings_card::section_title(
        ui,
        "Indicators",
        Some("Add context to the chart without changing its underlying data."),
    );
    ui.spacing_mut().item_spacing.y = 8.0;
    let config = &mut *c.indicators;
    // Line sources report one price per timestamp and no turnover at all.
    if !c.source.is_line_chart() {
        settings_card::card(
            ui,
            Icon::BarChart,
            "Volume Bars",
            Some("Turnover per candle, drawn along the bottom of the chart."),
            |ui| settings_card::switch(ui, &mut config.show_volume),
        );
    }
    settings_card::card(
        ui,
        Icon::Waveform,
        &format!("RSI ({RSI_PERIOD})"),
        Some(&format!(
            "Momentum from 0–100 across the bottom, with guides at {} and {}.",
            RSI_OVERSOLD as i64, RSI_OVERBOUGHT as i64
        )),
        |ui| settings_card::switch(ui, &mut config.show_rsi),
    );
    settings_card::card(
        ui,
        Icon::ChartLine,
        "EMA",
        Some("Exponential moving average over the closes, drawn on the price scale."),
        |ui| {
            settings_card::switch(ui, &mut config.show_ema);
            ui.add_enabled_ui(config.show_ema, |ui| {
                egui::ComboBox::from_id_salt("chart-settings-ema")
                    .selected_text(config.ema_period.to_string())
                    .width(52.0)
                    .show_ui(ui, |ui| {
                        for period in EMA_PERIODS {
                            ui.selectable_value(&mut config.ema_period, period, period.to_string());
                        }
                    });
            });
        },
    );
    settings_card::card(
        ui,
        Icon::Bands,
        "Bollinger Bands",
        Some(&format!(
            "{BOLLINGER_PERIOD}-period average with bands {} standard deviations either side — wide when volatile, tight when calm.",
            BOLLINGER_MULTIPLIER as i64
        )),
        |ui| settings_card::switch(ui, &mut config.show_bollinger),
    );
    settings_card::card(
        ui,
        Icon::Cycle,
        "Trend Flips",
        Some(&format!(
            "Bullish and bearish markers from {SUPERTREND_PERIOD}-period ATR × {}. Signals appear after the candle closes and can whipsaw in sideways markets.",
            SUPERTREND_MULTIPLIER as i64
        )),
        |ui| settings_card::switch(ui, &mut config.show_trend_flips),
    );
}

// MARK: Scripts

/// What one row of the applied list shows, copied out so drawing needs no borrow of the chart.
struct RowData {
    id: u64,
    title: String,
    is_strategy: bool,
    /// `None` until the script has run.
    overlay: Option<bool>,
    visible: bool,
    failure: Option<String>,
    evaluating: bool,
    inputs: HashMap<String, crate::pine::program::InputValue>,
}

enum RowEvent {
    ToggleVisible(u64),
    Remove(u64),
    MoveTo(u64, usize),
    ToggleInputs(u64),
    Input(u64, inputs::InputEdit),
}

fn refresh_scripts(state: &mut ChartSettingsState) {
    if cfg!(test) {
        return;
    }
    if state
        .scripts_read_at
        .is_some_and(|at| at.elapsed() < SCRIPT_REFRESH)
    {
        return;
    }
    state.scripts_read_at = Some(Instant::now());
    if let Some(mut store) = ScriptStore::default_location()
        && let Ok(mut scripts) = store.all_scripts()
    {
        scripts.retain(|s| s.script_type != ScriptType::Library);
        scripts.sort_by_key(|s| s.name.to_lowercase());
        state.scripts = scripts;
    }
}

/// An instance's declared inputs, recompiled only when its source changed.
fn ensure_schema(state: &mut ChartSettingsState, instance: &PineInstance) {
    let stale = state
        .schemas
        .get(&instance.id)
        .is_none_or(|(source, _)| *source != instance.source);
    if stale {
        let program = compiler::compile(&instance.source, Limits::default(), &LibraryRegistry);
        state
            .schemas
            .insert(instance.id, (instance.source.clone(), program.input_schema));
    }
}

fn scripts_page(
    ui: &mut Ui,
    state: &mut ChartSettingsState,
    c: &mut ChartSettingsContext,
) -> Option<ChartSettingsAction> {
    refresh_scripts(state);
    let mut action = None;
    let mut add: Option<LocalScript> = None;
    let has_saved = !state.scripts.is_empty();

    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            settings_card::section_title(
                ui,
                "Scripts",
                Some("Pine scripts running on this chart."),
            );
        });
        if has_saved {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let button = ui.add(
                    egui::Button::new(RichText::new("+  Add Script").size(12.0))
                        .min_size(vec2(110.0, 30.0)),
                );
                egui::Popup::menu(&button).show(|ui| {
                    ui.set_min_width(200.0);
                    for (heading, kind) in [
                        ("Indicators", ScriptType::Indicator),
                        ("Strategies", ScriptType::Strategy),
                    ] {
                        let scripts: Vec<&LocalScript> = state
                            .scripts
                            .iter()
                            .filter(|s| s.script_type == kind)
                            .collect();
                        if scripts.is_empty() {
                            continue;
                        }
                        ui.label(
                            RichText::new(heading)
                                .size(10.0)
                                .color(theme::TEXT_SECONDARY),
                        );
                        for script in scripts {
                            if ui.button(&script.name).clicked() {
                                add = Some(script.clone());
                                ui.close();
                            }
                        }
                        ui.separator();
                    }
                    if ui.button("Script Manager…").clicked() {
                        action = Some(ChartSettingsAction::OpenScriptManager);
                        ui.close();
                    }
                });
            });
        }
    });
    if let Some(script) = add {
        c.card
            .add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
    }

    let instances: Vec<PineInstance> = c.card.pine_instances().to_vec();
    if instances.is_empty() {
        empty_state(ui, has_saved, &mut action);
        return action;
    }

    // Copy what the rows show out of the chart, so drawing needs no borrow of it.
    let rows: Vec<RowData> = instances
        .iter()
        .map(|instance| {
            let status = c.card.pine_status(instance.id);
            RowData {
                id: instance.id,
                title: instance.name.clone(),
                is_strategy: status.as_ref().is_some_and(|s| s.output.strategy.is_some()),
                overlay: status.as_ref().map(|s| s.output.overlay),
                visible: instance.visible,
                failure: status
                    .as_ref()
                    .and_then(|s| s.error.map(|e| e.message.clone())),
                evaluating: instance.visible && status.is_none(),
                inputs: instance.input_values(),
            }
        })
        .collect();
    let bars = instances
        .iter()
        .filter_map(|i| c.card.pine_status(i.id))
        .map(|s| s.output.bar_count)
        .max()
        .filter(|bars| *bars > 0);
    for instance in &instances {
        ensure_schema(state, instance);
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Applied")
                .size(11.0)
                .strong()
                .color(theme::TEXT_PRIMARY),
        );
        count_chip(ui, &rows.len().to_string());
        if let Some(bars) = bars {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                status_badge(ui, &format!("{bars} bars"), theme::TEXT_SECONDARY);
            });
        }
    });
    ui.add_space(6.0);

    let mut events: Vec<RowEvent> = Vec::new();
    egui::Frame::new()
        .fill(theme::quaternary(0.45))
        .stroke(Stroke::new(1.0, theme::with_alpha(theme::SEPARATOR, 0.45)))
        .corner_radius(12.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = rows.len();
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    let y = ui.cursor().min.y;
                    let rect = ui.max_rect();
                    ui.painter().line_segment(
                        [pos2(rect.min.x + 54.0, y), pos2(rect.max.x, y)],
                        Stroke::new(1.0, theme::with_alpha(theme::SEPARATOR, 0.45)),
                    );
                }
                let schema = state.schemas.get(&row.id).map(|(_, schema)| schema.clone());
                instance_row(
                    ui,
                    row,
                    index,
                    count,
                    state.editing == Some(row.id),
                    schema.as_ref(),
                    &mut events,
                );
            }
        });

    for event in events {
        match event {
            RowEvent::ToggleVisible(id) => {
                let visible = instances
                    .iter()
                    .find(|i| i.id == id)
                    .is_some_and(|i| i.visible);
                c.card.set_pine_visible(id, !visible);
            }
            RowEvent::Remove(id) => {
                c.card.remove_pine_instance(id);
                state.schemas.remove(&id);
                if state.editing == Some(id) {
                    state.editing = None;
                }
            }
            RowEvent::MoveTo(id, index) => c.card.move_pine_instance(id, index),
            RowEvent::ToggleInputs(id) => {
                state.editing = if state.editing == Some(id) {
                    None
                } else {
                    Some(id)
                };
            }
            RowEvent::Input(id, edit) => {
                if let (Some(instance), Some((_, schema))) = (
                    instances.iter().find(|i| i.id == id),
                    state.schemas.get(&id),
                ) {
                    let mut values = instance.input_values();
                    inputs::apply_edit(&mut values, schema, edit);
                    c.card.set_pine_instance_inputs(id, &values);
                }
            }
        }
    }

    // One backtest report per applied strategy, so several never hide behind "the first one".
    let strategies: Vec<&PineInstance> = instances
        .iter()
        .filter(|i| {
            c.card
                .pine_status(i.id)
                .is_some_and(|s| s.output.strategy.is_some())
        })
        .collect();
    if !strategies.is_empty() {
        ui.add_space(20.0);
        settings_card::section_title(ui, "Backtest", None);
        for instance in &strategies {
            if strategies.len() > 1 {
                ui.label(
                    RichText::new(&instance.name)
                        .size(11.0)
                        .strong()
                        .color(theme::TEXT_SECONDARY),
                );
            }
            if let Some(status) = c.card.pine_status(instance.id) {
                drawer::report_content(ui, Some(status.output));
            }
            ui.add_space(8.0);
        }
    }
    action
}

fn count_chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::monospace(10.0),
        theme::TEXT_SECONDARY,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, theme::quaternary(1.0));
    ui.painter().galley(
        pos2(rect.min.x + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::TEXT_SECONDARY,
    );
}

fn status_badge(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(10.0), color);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 20.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 10.0, theme::with_alpha(color, 0.16));
    ui.painter().galley(
        pos2(rect.min.x + 9.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
}

fn empty_state(ui: &mut Ui, has_saved: bool, action: &mut Option<ChartSettingsAction>) {
    ui.add_space(12.0);
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .stroke(Stroke::new(1.0, theme::with_alpha(theme::SEPARATOR, 0.5)))
        .corner_radius(12.0)
        .inner_margin(Margin::symmetric(24, 36))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let (disc, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                ui.painter()
                    .circle_filled(disc.center(), 24.0, theme::quaternary(1.3));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(disc.center(), Vec2::splat(20.0)),
                    if has_saved {
                        Icon::ChartLine
                    } else {
                        Icon::Curlybraces
                    },
                    theme::TEXT_SECONDARY,
                );
                ui.add_space(8.0);
                let (title, message) = if has_saved {
                    (
                        "No scripts on this chart",
                        "Choose Add Script to overlay an indicator or strategy on the candles.",
                    )
                } else {
                    (
                        "No scripts yet",
                        "Write one in the Script Manager and it shows up here.",
                    )
                };
                ui.label(
                    RichText::new(title)
                        .size(13.0)
                        .strong()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new(message)
                        .size(11.0)
                        .color(theme::TEXT_SECONDARY),
                );
                if !has_saved {
                    ui.add_space(10.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Open Script Manager").color(Color32::WHITE),
                            )
                            .fill(theme::SYSTEM_ACCENT)
                            .min_size(vec2(160.0, 30.0)),
                        )
                        .clicked()
                    {
                        *action = Some(ChartSettingsAction::OpenScriptManager);
                    }
                }
            });
        });
}

/// One applied script: what it is, how it is doing, and what can be done to it.
fn instance_row(
    ui: &mut Ui,
    row: &RowData,
    index: usize,
    count: usize,
    editing: bool,
    schema: Option<&InputSchema>,
    events: &mut Vec<RowEvent>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let failing = row.failure.is_some();
    let tint = if failing {
        theme::ORANGE
    } else {
        theme::SYSTEM_ACCENT
    };
    let dim = if row.visible { 1.0 } else { 0.55 };

    let badge = Rect::from_center_size(pos2(rect.min.x + 27.0, rect.center().y), Vec2::splat(30.0));
    ui.painter()
        .rect_filled(badge, 8.0, theme::with_alpha(tint, 0.12));
    let kind_icon = if failing {
        Icon::TriangleFill
    } else if row.is_strategy {
        Icon::Cycle
    } else {
        Icon::ChartLine
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(15.0)),
        kind_icon,
        tint,
    );

    // Controls, right to left: remove, settings, eye.
    let mut x = rect.max.x - 12.0;
    let mut button = |ui: &mut Ui, icon: Icon, tip: &str, destructive: bool| -> egui::Response {
        let hit = Rect::from_center_size(pos2(x - 12.0, rect.center().y), Vec2::splat(24.0));
        x -= 26.0;
        let response = ui.interact(
            hit,
            Id::new(("instance-button", row.id, tip.to_owned())),
            Sense::click(),
        );
        if response.hovered() {
            ui.painter().rect_filled(hit, 6.0, theme::white_alpha(18));
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let color = if destructive && response.hovered() {
            theme::syntax::STRING
        } else {
            theme::TEXT_SECONDARY
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(hit.center(), Vec2::splat(13.0)),
            icon,
            color,
        );
        response.on_hover_text(tip.to_owned())
    };
    if button(
        ui,
        Icon::Xmark,
        &format!("Remove {} from this chart", row.title),
        true,
    )
    .clicked()
    {
        events.push(RowEvent::Remove(row.id));
    }
    let settings = button(
        ui,
        Icon::Sliders,
        &format!("Edit {} settings", row.title),
        false,
    );
    if settings.clicked() {
        events.push(RowEvent::ToggleInputs(row.id));
    }
    let eye_tip = if row.visible {
        format!("Hide {} on the chart", row.title)
    } else {
        format!("Show {} on the chart", row.title)
    };
    let eye = if row.visible {
        Icon::Eye
    } else {
        Icon::EyeSlash
    };
    if button(ui, eye, &eye_tip, false).clicked() {
        events.push(RowEvent::ToggleVisible(row.id));
    }
    let text_right = x - 4.0;

    let title = ui.painter().layout_no_wrap(
        row.title.clone(),
        FontId::proportional(11.0),
        theme::TEXT_PRIMARY.gamma_multiply(dim),
    );
    let meta_text = row.failure.clone().unwrap_or_else(|| {
        if row.evaluating {
            "Evaluating…".to_owned()
        } else {
            let mut parts = vec![
                if row.is_strategy {
                    "Strategy"
                } else {
                    "Indicator"
                }
                .to_owned(),
            ];
            if !row.is_strategy
                && let Some(overlay) = row.overlay
            {
                parts.push(if overlay { "On chart" } else { "Own pane" }.to_owned());
            }
            if !row.visible {
                parts.push("Hidden".to_owned());
            }
            parts.join(" · ")
        }
    });
    let meta_color = if failing {
        theme::ORANGE
    } else {
        theme::TEXT_SECONDARY
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        meta_text,
        FontId::proportional(10.0),
        meta_color.gamma_multiply(dim),
    );
    job.wrap.max_width = (text_right - (badge.max.x + 12.0)).max(40.0);
    job.wrap.max_rows = 1;
    let meta = ui.painter().layout_job(job);
    let top = rect.center().y - (title.size().y + 2.0 + meta.size().y) / 2.0;
    let left = badge.max.x + 12.0;
    ui.painter()
        .galley(pos2(left, top), title.clone(), theme::TEXT_PRIMARY);
    ui.painter()
        .galley(pos2(left, top + title.size().y + 2.0), meta, meta_color);

    response.context_menu(|ui| {
        let label = if row.visible {
            "Hide on Chart"
        } else {
            "Show on Chart"
        };
        if ui.button(label).clicked() {
            events.push(RowEvent::ToggleVisible(row.id));
            ui.close();
        }
        if ui.button("Settings…").clicked() {
            events.push(RowEvent::ToggleInputs(row.id));
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(index > 0, egui::Button::new("Move Up"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, index - 1));
            ui.close();
        }
        if ui
            .add_enabled(index + 1 < count, egui::Button::new("Move Down"))
            .clicked()
        {
            events.push(RowEvent::MoveTo(row.id, index + 1));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new("Remove from Chart").color(theme::syntax::STRING))
            .clicked()
        {
            events.push(RowEvent::Remove(row.id));
            ui.close();
        }
    });

    if editing && let Some(schema) = schema {
        let popup_id = Id::new(("instance-inputs", row.id));
        let shown = egui::Popup::from_response(&settings)
            .id(popup_id)
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .align(egui::RectAlign::BOTTOM_END)
            .gap(6.0)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .show(|ui| {
                ui.set_width(400.0);
                ui.set_max_height(380.0);
                for edit in inputs::show(ui, schema, &row.inputs) {
                    events.push(RowEvent::Input(row.id, edit));
                }
            });
        // A click outside the popover, or Esc, puts it away again.
        let pointer = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        let outside = ui.input(|i| i.pointer.any_click())
            && shown
                .as_ref()
                .is_some_and(|s| !s.response.rect.contains(pointer))
            && !settings.rect.contains(pointer);
        if shown.is_none() || outside {
            events.push(RowEvent::ToggleInputs(row.id));
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(
        state: &mut ChartSettingsState,
        card: &mut ChartCard,
        indicators: &mut IndicatorConfig,
    ) {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let _ = show(
                    ui.ctx(),
                    state,
                    ChartSettingsContext {
                        title: "BTC/USDT",
                        symbol: "BTCUSDT",
                        source: DataSource::Binance,
                        indicators,
                        card,
                    },
                );
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_sheet_draws_each_page_empty_and_with_scripts() {
        let mut state = ChartSettingsState::default();
        let mut card = ChartCard::default();
        let mut indicators = IndicatorConfig::default();
        for tab in Tab::ALL {
            state.tab = tab;
            draw(&mut state, &mut card, &mut indicators);
        }
        card.add_pine_instance(
            None,
            "RSI",
            "//@version=6\nindicator(\"x\")\nlen = input.int(14)\nplot(close)\n",
        );
        card.add_pine_instance(
            None,
            "Strat",
            "//@version=6\nstrategy(\"s\")\nplot(close)\n",
        );
        card.set_pine_visible(2, false);
        state.tab = Tab::Scripts;
        state.editing = Some(1);
        draw(&mut state, &mut card, &mut indicators);
        assert!(
            state.schemas.contains_key(&1),
            "the inputs popover's schema was compiled"
        );
        assert_eq!(state.schemas[&1].1.inputs.len(), 1);
    }

    #[test]
    fn tabs_have_titles() {
        assert_eq!(
            Tab::ALL.map(Tab::title),
            ["Indicators", "Scripts", "Appearance"]
        );
    }
}
