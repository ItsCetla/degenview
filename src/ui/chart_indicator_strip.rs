//! Compact, Swift-style controls for the indicators applied to a chart.

use super::chart_card::ChartCard;
use super::icons::{self, Icon};
use super::theme;
use crate::model::{DataSource, EMA_PERIODS, IndicatorConfig, RSI_PERIOD};
use eframe::egui::{self, RichText, Sense, Ui, Vec2};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BuiltIn {
    Volume,
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl BuiltIn {
    const ALL: [Self; 5] = [
        Self::Volume,
        Self::Rsi,
        Self::Ema,
        Self::Bollinger,
        Self::TrendFlips,
    ];

    fn menu_title(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Rsi => "RSI",
            Self::Ema => "EMA",
            Self::Bollinger => "Bollinger Bands",
            Self::TrendFlips => "Trend Flips",
        }
    }

    fn chip_title(self, config: &IndicatorConfig) -> String {
        match self {
            Self::Volume => "Vol".into(),
            Self::Rsi => format!("RSI {RSI_PERIOD}"),
            Self::Ema => format!("EMA {}", config.ema_period),
            Self::Bollinger => "BB".into(),
            Self::TrendFlips => "Trend Flips".into(),
        }
    }

    pub fn applied(self, config: &IndicatorConfig) -> bool {
        match self {
            Self::Volume => config.show_volume,
            Self::Rsi => config.show_rsi,
            Self::Ema => config.show_ema,
            Self::Bollinger => config.show_bollinger,
            Self::TrendFlips => config.show_trend_flips,
        }
    }

    fn hidden(self, config: &IndicatorConfig) -> bool {
        match self {
            Self::Volume => config.hide_volume,
            Self::Rsi => config.hide_rsi,
            Self::Ema => config.hide_ema,
            Self::Bollinger => config.hide_bollinger,
            Self::TrendFlips => config.hide_trend_flips,
        }
    }

    pub fn set_applied(self, config: &mut IndicatorConfig, applied: bool) {
        match self {
            Self::Volume => config.show_volume = applied,
            Self::Rsi => config.show_rsi = applied,
            Self::Ema => config.show_ema = applied,
            Self::Bollinger => config.show_bollinger = applied,
            Self::TrendFlips => config.show_trend_flips = applied,
        }
        if applied {
            self.set_hidden(config, false);
        }
    }

    fn set_hidden(self, config: &mut IndicatorConfig, hidden: bool) {
        match self {
            Self::Volume => config.hide_volume = hidden,
            Self::Rsi => config.hide_rsi = hidden,
            Self::Ema => config.hide_ema = hidden,
            Self::Bollinger => config.hide_bollinger = hidden,
            Self::TrendFlips => config.hide_trend_flips = hidden,
        }
    }

    pub fn toggle_visible(self, config: &mut IndicatorConfig) {
        self.set_hidden(config, !self.hidden(config));
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Kind {
    BuiltIn(BuiltIn),
    Script(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Add(BuiltIn),
    ToggleVisible(Kind),
    Remove(BuiltIn),
    EmaPeriod(usize),
    ConfigureScript(u64),
    OpenIndicatorSettings,
    OpenScriptSettings,
}

struct Item {
    kind: Kind,
    title: String,
    visible: bool,
    error: bool,
}

fn items(config: &IndicatorConfig, card: &ChartCard, source: DataSource) -> Vec<Item> {
    let mut items: Vec<Item> = BuiltIn::ALL
        .into_iter()
        .filter(|kind| !(source.is_line_chart() && *kind == BuiltIn::Volume))
        .filter(|kind| kind.applied(config))
        .map(|kind| Item {
            kind: Kind::BuiltIn(kind),
            title: kind.chip_title(config),
            visible: !kind.hidden(config),
            error: false,
        })
        .collect();
    if !source.is_line_chart() {
        items.extend(card.pine_instances().iter().map(|instance| {
            Item {
                kind: Kind::Script(instance.id),
                title: instance.name.clone(),
                visible: instance.visible,
                error: card
                    .pine_status(instance.id)
                    .is_some_and(|status| status.error.is_some()),
            }
        }));
    }
    items
}

pub fn has_items(config: &IndicatorConfig, card: &ChartCard, source: DataSource) -> bool {
    !items(config, card, source).is_empty()
}

fn chip_width(item: &Item) -> f32 {
    (item.title.chars().count() as f32 * 6.0).min(110.0) + 16.0
}

/// Returns a deferred edit so the workspace can mutate the chart after drawing it.
pub fn show(
    ui: &mut Ui,
    card: &ChartCard,
    config: &IndicatorConfig,
    source: DataSource,
) -> Option<Action> {
    let items = items(config, card, source);
    let mut action = None;
    ui.spacing_mut().item_spacing.x = 4.0;
    let show_add = items.is_empty() || ui.available_width() >= 48.0;
    // Keep room for the add button and one chip's hover controls.
    let budget = (ui.available_width() - if show_add { 72.0 } else { 40.0 }).max(0.0);
    let mut inline = 0;
    for count in (1..=items.len().min(3)).rev() {
        let overflow = items.len() > count;
        let width: f32 = items.iter().take(count).map(chip_width).sum::<f32>()
            + (count.saturating_sub(1) as f32 * 4.0)
            + if overflow { 40.0 } else { 0.0 };
        if width <= budget {
            inline = count;
            break;
        }
    }
    for item in items.iter().take(inline) {
        chip(ui, item, config, &mut action);
    }
    if inline < items.len() {
        let label = if inline == 0 {
            items.len().to_string()
        } else {
            format!("+{}", items.len() - inline)
        };
        let summary = egui::Frame::new()
            .fill(theme::quaternary(0.35))
            .corner_radius(999.0)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(label).size(10.0)).sense(Sense::click()))
            })
            .inner;
        egui::Popup::menu(&summary).show(|ui| {
            ui.label(RichText::new("Indicators").small().weak());
            for item in items.iter().skip(inline) {
                ui.menu_button(&item.title, |ui| item_menu(ui, item, config, &mut action));
            }
        });
    }

    if show_add {
        let add = ui
            .add_sized(
                Vec2::splat(20.0),
                egui::Button::new(RichText::new("+").size(13.0).strong()).frame(false),
            )
            .on_hover_text("Add Indicator");
        egui::Popup::menu(&add).show(|ui| {
            for kind in BuiltIn::ALL {
                if (source.is_line_chart() && kind == BuiltIn::Volume) || kind.applied(config) {
                    continue;
                }
                if ui.button(kind.menu_title()).clicked() {
                    action = Some(Action::Add(kind));
                    ui.close();
                }
            }
            if !source.is_line_chart() {
                ui.separator();
                if ui.button("Scripts…").clicked() {
                    action = Some(Action::OpenScriptSettings);
                    ui.close();
                }
            }
            if ui.button("All Indicator Settings…").clicked() {
                action = Some(Action::OpenIndicatorSettings);
                ui.close();
            }
        });
    }
    action
}

fn chip(ui: &mut Ui, item: &Item, config: &IndicatorConfig, action: &mut Option<Action>) {
    let id = ui.id().with(("indicator-chip", item.kind));
    let hovered = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id).unwrap_or(false));
    let fill = theme::quaternary(if hovered { 0.55 } else { 0.35 });
    let inner = egui::Frame::new()
        .fill(fill)
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(7, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.horizontal(|ui| {
                let text_color = if item.visible {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_TERTIARY
                };
                let title = ui
                    .scope(|ui| {
                        ui.set_max_width(110.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&item.title).size(10.0).color(text_color),
                            )
                            .truncate()
                            .sense(Sense::click()),
                        )
                    })
                    .inner;
                if title.clicked() {
                    match item.kind {
                        Kind::BuiltIn(BuiltIn::Ema) => {
                            // The period menu is anchored to the name below.
                        }
                        Kind::Script(id) => *action = Some(Action::ConfigureScript(id)),
                        _ => *action = Some(Action::ToggleVisible(item.kind)),
                    }
                }
                if item.kind == Kind::BuiltIn(BuiltIn::Ema) {
                    egui::Popup::menu(&title).show(|ui| {
                        ema_menu(ui, config, action);
                    });
                }
                if item.error {
                    ui.label(RichText::new("!").size(10.0).color(theme::ORANGE));
                }
                if hovered {
                    if mini_icon(
                        ui,
                        if item.visible {
                            Icon::Eye
                        } else {
                            Icon::EyeSlash
                        },
                        if item.visible { "Hide" } else { "Show" },
                    ) {
                        *action = Some(Action::ToggleVisible(item.kind));
                    }
                    if item.kind == Kind::BuiltIn(BuiltIn::Ema)
                        && mini_icon(ui, Icon::Sliders, "EMA period")
                    {
                        *action = Some(Action::OpenIndicatorSettings);
                    }
                    if let Kind::Script(id) = item.kind
                        && mini_icon(ui, Icon::Sliders, "Configure script")
                    {
                        *action = Some(Action::ConfigureScript(id));
                    }
                    if let Kind::BuiltIn(kind) = item.kind
                        && mini_icon(ui, Icon::Xmark, "Remove indicator")
                    {
                        *action = Some(Action::Remove(kind));
                    }
                }
            });
        });
    let under_pointer = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| inner.response.rect.contains(pos));
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, under_pointer));
    inner
        .response
        .context_menu(|ui| item_menu(ui, item, config, action));
}

fn mini_icon(ui: &mut Ui, icon: Icon, tooltip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::click());
    icons::paint(ui.painter(), rect, icon, theme::TEXT_SECONDARY);
    response.on_hover_text(tooltip).clicked()
}

fn ema_menu(ui: &mut Ui, config: &IndicatorConfig, action: &mut Option<Action>) {
    ui.label(RichText::new("EMA period").small().weak());
    for period in EMA_PERIODS {
        if ui
            .selectable_label(config.ema_period == period, period.to_string())
            .clicked()
        {
            *action = Some(Action::EmaPeriod(period));
            ui.close();
        }
    }
}

fn item_menu(ui: &mut Ui, item: &Item, config: &IndicatorConfig, action: &mut Option<Action>) {
    if ui
        .button(if item.visible { "Hide" } else { "Show" })
        .clicked()
    {
        *action = Some(Action::ToggleVisible(item.kind));
        ui.close();
    }
    match item.kind {
        Kind::BuiltIn(BuiltIn::Ema) => {
            ui.menu_button("EMA period", |ui| ema_menu(ui, config, action));
        }
        Kind::Script(id) if ui.button("Configure…").clicked() => {
            *action = Some(Action::ConfigureScript(id));
            ui.close();
        }
        _ => {}
    }
    if let Kind::BuiltIn(kind) = item.kind {
        ui.separator();
        if ui.button("Remove").clicked() {
            *action = Some(Action::Remove(kind));
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_visibility_keeps_the_applied_chip() {
        let mut config = IndicatorConfig {
            show_ema: true,
            ..IndicatorConfig::default()
        };
        BuiltIn::Ema.toggle_visible(&mut config);
        assert!(BuiltIn::Ema.applied(&config));
        assert!(!config.drawn().show_ema);
        BuiltIn::Ema.toggle_visible(&mut config);
        assert!(config.drawn().show_ema);
    }

    #[test]
    fn strip_contains_hidden_built_ins_and_applied_scripts() {
        let config = IndicatorConfig {
            show_ema: true,
            hide_ema: true,
            ..IndicatorConfig::default()
        };
        let mut card = ChartCard::default();
        let script_id = card.add_pine_instance(None, "My script", "plot(close)");
        let items = items(&config, &card, DataSource::Binance);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "EMA 20");
        assert!(!items[0].visible);
        assert_eq!(items[1].kind, Kind::Script(script_id));
    }

    #[test]
    fn older_indicator_configs_default_to_visible() {
        let json = r#"{"show_volume":false,"show_rsi":false,"show_ema":true,"ema_period":50,"show_bollinger":false,"show_trend_flips":false}"#;
        let config: IndicatorConfig = serde_json::from_str(json).unwrap();
        assert!(config.drawn().show_ema);
        assert!(!config.hide_ema);
    }
}
