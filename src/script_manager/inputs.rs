//! A script's `input.*` declarations as editable cards, and the per-script cache of the values
//! tried in the preview. Ported from `PineInputsView`, `SettingsCardRow` and
//! `ScriptPreviewInputsStore`.

use crate::model::pine_instance::StoredInput as Stored;
use crate::pine::program::{InputDefinition, InputSchema, InputValue};
use crate::pine::token::ValueType;
use crate::ui::icons::{self, Icon};
use crate::ui::settings_card;
use crate::ui::theme;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Layout, Margin, Rect, RichText, Sense, Stroke, Ui,
    Vec2, pos2, vec2,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Override map by input id; an input with no entry shows its default.
pub type InputValues = HashMap<String, InputValue>;

/// What a card changed.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEdit {
    Set(String, InputValue),
    Reset,
}

// MARK: Persistence

/// The input values a user tried in the preview, per script: a small cache beside the scripts,
/// not part of them. Nothing here is applied to a chart.
pub struct InputsStore {
    path: Option<PathBuf>,
    entries: HashMap<String, HashMap<String, Stored>>,
}

impl InputsStore {
    pub fn open(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn inputs(&self, script_id: &str) -> InputValues {
        self.entries
            .get(script_id)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), v.clone().into()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// An empty map forgets the script, so defaults are what the next session starts from.
    pub fn set_inputs(&mut self, script_id: &str, inputs: &InputValues) {
        let stored: HashMap<String, Stored> =
            inputs.iter().map(|(k, v)| (k.clone(), v.into())).collect();
        let before = self.entries.get(script_id);
        if stored.is_empty() {
            if before.is_none() {
                return;
            }
            self.entries.remove(script_id);
        } else {
            if before == Some(&stored) {
                return;
            }
            self.entries.insert(script_id.to_owned(), stored);
        }
        self.save();
    }

    /// Drops the values of scripts that no longer exist.
    pub fn prune(&mut self, keeping: &[String]) {
        let before = self.entries.len();
        self.entries.retain(|id, _| keeping.contains(id));
        if self.entries.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        if let Some(path) = &self.path
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.entries)
        {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Keeps only the overrides whose input still exists with the same kind of value.
pub fn prune_values(values: &InputValues, schema: &InputSchema) -> InputValues {
    values
        .iter()
        .filter(|(id, value)| {
            schema.inputs.iter().any(|input| {
                input.id == **id
                    && std::mem::discriminant(&input.default_value) == std::mem::discriminant(value)
            })
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Applies a card's change: a value equal to the default is no override at all.
pub fn apply_edit(values: &mut InputValues, schema: &InputSchema, edit: InputEdit) {
    match edit {
        InputEdit::Reset => values.clear(),
        InputEdit::Set(id, value) => {
            let default = schema
                .inputs
                .iter()
                .find(|input| input.id == id)
                .map(|input| input.default_value.clone());
            if default.as_ref() == Some(&value) {
                values.remove(&id);
            } else {
                values.insert(id, value);
            }
        }
    }
}

// MARK: Layout

/// Inputs sectioned by their `group`, in the order each group first appears.
pub fn groups(schema: &InputSchema) -> Vec<(Option<String>, Vec<&InputDefinition>)> {
    let mut groups: Vec<(Option<String>, Vec<&InputDefinition>)> = Vec::new();
    for input in &schema.inputs {
        match groups.iter_mut().find(|g| g.0 == input.group) {
            Some(group) => group.1.push(input),
            None => groups.push((input.group.clone(), vec![input])),
        }
    }
    groups
}

/// An options input's choices: the stored value and the title the menu shows for it.
pub fn option_titles(input: &InputDefinition) -> Vec<(String, String)> {
    input
        .options
        .iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, option)| {
            let InputValue::String(text) = option else {
                return None;
            };
            let title = input
                .option_titles
                .as_ref()
                .and_then(|titles| titles.get(index))
                .cloned()
                .unwrap_or_else(|| text.clone());
            Some((text.clone(), title))
        })
        .collect()
}

/// Width cap for an options dropdown: a long title would crowd the card's name and hint.
pub fn menu_width(titles: &[String]) -> Option<f32> {
    (titles.iter().map(|t| t.chars().count()).max().unwrap_or(0) > 20).then_some(95.0)
}

fn color_from_rgba(rgba: u32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    )
}

fn rgba_from_color(color: Color32) -> u32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | u32::from(a)
}

// MARK: View

/// The Inputs tab: cards sectioned by group, then "Reset to Defaults".
pub fn show(ui: &mut Ui, schema: &InputSchema, values: &InputValues) -> Vec<InputEdit> {
    let mut edits = Vec::new();
    if schema.inputs.is_empty() {
        super::drawer::placeholder(
            ui,
            "No inputs",
            "Declare one with input.int(), input.float(), input.bool() and friends, and it appears here.",
        );
        return edits;
    }
    egui::ScrollArea::vertical()
        .id_salt("script-inputs-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    for (title, inputs) in groups(schema) {
                        if let Some(title) = title {
                            ui.add_space(6.0 - 12.0f32.min(6.0));
                            ui.label(
                                RichText::new(title)
                                    .size(11.0)
                                    .variation("wght", crate::ui::kit::SEMIBOLD)
                                    .color(theme::TEXT_SECONDARY),
                            );
                        }
                        for input in inputs {
                            if let Some(value) = card(ui, input, values.get(&input.id)) {
                                edits.push(InputEdit::Set(input.id.clone(), value));
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!values.is_empty(), egui::Button::new("Reset to Defaults"))
                            .clicked()
                        {
                            edits.push(InputEdit::Reset);
                        }
                    });
                });
        });
    edits
}

/// One script input as a card; returns the new value when the control changed.
fn card(
    ui: &mut Ui,
    input: &InputDefinition,
    override_value: Option<&InputValue>,
) -> Option<InputValue> {
    let title = input.title.clone().unwrap_or_else(|| input.id.clone());
    let current = override_value.unwrap_or(&input.default_value).clone();
    let icon = match (&input.kind, &current) {
        (_, InputValue::Bool(_)) => Icon::Switch,
        (ValueType::Time, InputValue::Int(_)) => Icon::Calendar,
        (_, InputValue::Int(_) | InputValue::Float(_)) => Icon::Hash,
        (_, InputValue::String(_)) if input.options.is_some() => Icon::List,
        (_, InputValue::String(_)) => Icon::TextFormat,
        (_, InputValue::Color(_)) => Icon::Palette,
        (_, InputValue::Source(_)) => Icon::ChartLine,
    };
    settings_card::card(ui, icon, &title, input.tooltip.as_deref(), |ui| {
        control(ui, input, &title, &current)
    })
}

fn control(
    ui: &mut Ui,
    input: &InputDefinition,
    title: &str,
    current: &InputValue,
) -> Option<InputValue> {
    match (&input.kind, current) {
        (_, InputValue::Bool(value)) => {
            let mut on = *value;
            settings_card::switch(ui, &mut on).then_some(InputValue::Bool(on))
        }
        (ValueType::Time, InputValue::Int(ms)) => time_field(ui, input, *ms),
        (_, InputValue::Int(value)) => int_stepper(ui, input, *value),
        (_, InputValue::Float(value)) => float_field(ui, input, *value),
        (_, InputValue::String(value)) if input.options.is_some() => {
            let titles = option_titles(input);
            let shown = titles
                .iter()
                .find(|(v, _)| v == value)
                .map_or(value.clone(), |(_, t)| t.clone());
            let width = menu_width(&titles.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
            let mut picked = None;
            let mut combo = egui::ComboBox::from_id_salt(Id::new(("input-combo", &input.id)))
                .selected_text(shown.clone());
            if let Some(width) = width {
                combo = combo.width(width);
            }
            combo.show_ui(ui, |ui| {
                for (stored, label) in &titles {
                    if ui.selectable_label(stored == value, label).clicked() {
                        picked = Some(InputValue::String(stored.clone()));
                    }
                }
            });
            picked
        }
        (_, InputValue::String(value)) => {
            let mut text = value.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text(title)
                    .desired_width(140.0)
                    .margin(Margin::symmetric(6, 4)),
            );
            response.changed().then_some(InputValue::String(text))
        }
        (_, InputValue::Color(value)) => {
            let mut color = color_from_rgba(*value);
            let response = egui::color_picker::color_edit_button_srgba(
                ui,
                &mut color,
                egui::color_picker::Alpha::OnlyBlend,
            );
            response
                .changed()
                .then(|| InputValue::Color(rgba_from_color(color)))
        }
        (_, InputValue::Source(value)) => {
            let mut picked = None;
            egui::ComboBox::from_id_salt(Id::new(("input-source", &input.id)))
                .selected_text(capitalized(value))
                .show_ui(ui, |ui| {
                    for name in ["open", "high", "low", "close", "volume"] {
                        if ui
                            .selectable_label(value == name, capitalized(name))
                            .clicked()
                        {
                            picked = Some(InputValue::Source(name.to_owned()));
                        }
                    }
                });
            picked
        }
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn int_stepper(ui: &mut Ui, input: &InputDefinition, value: i64) -> Option<InputValue> {
    let lower = input.min_value.map_or(value.min(1), |v| v as i64);
    let upper = input.max_value.map_or(value.max(10_000), |v| v as i64);
    let step = input.step.map_or(1, |s| (s as i64).max(1));
    let mut next = None;
    // Right-to-left layout: the stepper first (it is rightmost), then the value text.
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 5.0, Color32::from_rgb(63, 72, 77));
    ui.painter().rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, theme::SEPARATOR),
        egui::StrokeKind::Inside,
    );
    for (index, (icon, delta)) in [(Icon::ChevronUp, step), (Icon::ChevronDown, -step)]
        .into_iter()
        .enumerate()
    {
        let half = Rect::from_min_size(
            pos2(rect.min.x, rect.min.y + index as f32 * 11.0),
            vec2(16.0, 11.0),
        );
        let response = ui.interact(half, Id::new(("stepper", &input.id, index)), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(half.shrink(1.0), 4.0, theme::white_alpha(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(half.center(), Vec2::splat(7.0)),
            icon,
            theme::TEXT_PRIMARY,
        );
        if response.clicked() {
            next = Some((value + delta).clamp(lower, upper));
        }
    }
    ui.label(
        RichText::new(value.to_string())
            .monospace()
            .size(13.0)
            .color(theme::TEXT_SECONDARY),
    );
    next.filter(|n| *n != value).map(InputValue::Int)
}

fn float_field(ui: &mut Ui, input: &InputDefinition, value: f64) -> Option<InputValue> {
    let id = Id::new(("input-float", &input.id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| trim_float(value));
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id.with("edit"))
            .desired_width(100.0)
            .horizontal_align(Align::Max)
            .margin(Margin::symmetric(6, 4)),
    );
    let mut result = None;
    if response.changed()
        && let Ok(parsed) = text.trim().replace(',', ".").parse::<f64>()
        && parsed.is_finite()
    {
        result = Some(InputValue::Float(parsed));
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    result
}

fn trim_float(value: f64) -> String {
    let text = format!("{value}");
    text
}

// MARK: Time input

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2024-03-09 14:30` in the system time zone, like the date picker it stands in for.
pub fn format_time(ms: i64) -> String {
    let secs = (ms + crate::service::timezone::local_utc_offset_secs(ms) * 1000).div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60
    )
}

pub fn parse_time(text: &str) -> Option<i64> {
    let (date, time) = text
        .trim()
        .split_once(' ')
        .unwrap_or((text.trim(), "00:00"));
    let mut d = date.split('-');
    let (y, m, day) = (
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
        d.next()?.parse::<i64>().ok()?,
    );
    let mut t = time.split(':');
    let (h, min) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    if !(1..=12).contains(&m)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&min)
    {
        return None;
    }
    let local_ms = (days_from_civil(y, m, day) * 86_400 + h * 3600 + min * 60) * 1000;
    Some(local_ms - crate::service::timezone::local_utc_offset_secs(local_ms) * 1000)
}

fn time_field(ui: &mut Ui, input: &InputDefinition, ms: i64) -> Option<InputValue> {
    let id = Id::new(("input-time", &input.id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| format_time(ms));
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id.with("edit"))
            .desired_width(130.0)
            .hint_text("YYYY-MM-DD HH:MM")
            .margin(Margin::symmetric(6, 4)),
    );
    let mut result = None;
    if response.changed()
        && let Some(parsed) = parse_time(&text)
    {
        result = Some(InputValue::Int(parsed));
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    result
}

#[allow(dead_code)]
fn _keep(_: Align2, _: FontId) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, kind: ValueType, default: InputValue, group: Option<&str>) -> InputDefinition {
        InputDefinition {
            id: id.into(),
            kind,
            default_value: default,
            title: None,
            tooltip: None,
            group: group.map(str::to_owned),
            inline: None,
            confirm: false,
            min_value: None,
            max_value: None,
            step: None,
            options: None,
            option_titles: None,
        }
    }

    #[test]
    fn groups_keep_first_appearance_order() {
        let schema = InputSchema {
            inputs: vec![
                def("a", ValueType::Int, InputValue::Int(1), Some("B")),
                def("b", ValueType::Int, InputValue::Int(1), None),
                def("c", ValueType::Int, InputValue::Int(1), Some("B")),
                def("d", ValueType::Int, InputValue::Int(1), Some("A")),
            ],
        };
        let titles: Vec<Option<String>> = groups(&schema).into_iter().map(|g| g.0).collect();
        assert_eq!(titles, [Some("B".into()), None, Some("A".into())]);
        assert_eq!(groups(&schema)[0].1.len(), 2);
    }

    #[test]
    fn option_titles_fall_back_to_the_value_and_long_titles_cap_the_menu() {
        let mut input = def("m", ValueType::String, InputValue::String("a".into()), None);
        input.options = Some(vec![
            InputValue::String("a".into()),
            InputValue::String("b".into()),
        ]);
        input.option_titles = Some(vec!["Alpha".into()]);
        assert_eq!(
            option_titles(&input),
            [("a".into(), "Alpha".into()), ("b".into(), "b".into())]
        );
        assert_eq!(menu_width(&["short".into()]), None);
        assert_eq!(menu_width(&["x".repeat(21)]), Some(95.0));
    }

    #[test]
    fn the_cache_round_trips_and_forgets_empty_maps() {
        let dir = std::env::temp_dir().join(format!("degenview-inputs-{}", std::process::id()));
        let path = dir.join("script_preview_inputs.json");
        let mut store = InputsStore::open(Some(path.clone()));
        let mut values = InputValues::new();
        values.insert("len".into(), InputValue::Int(30));
        values.insert("c".into(), InputValue::Color(0xff00_00ff));
        store.set_inputs("s1", &values);
        assert_eq!(InputsStore::open(Some(path.clone())).inputs("s1"), values);
        store.set_inputs("s1", &InputValues::new());
        assert!(InputsStore::open(Some(path)).inputs("s1").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn values_for_removed_or_retyped_inputs_are_pruned() {
        let schema = InputSchema {
            inputs: vec![def("len", ValueType::Int, InputValue::Int(1), None)],
        };
        let mut values = InputValues::new();
        values.insert("len".into(), InputValue::Int(9));
        values.insert("gone".into(), InputValue::Int(1));
        let kept = prune_values(&values, &schema);
        assert_eq!(kept.len(), 1);
        values.insert("len".into(), InputValue::Float(1.0));
        assert!(prune_values(&values, &schema).is_empty(), "type changed");
    }

    #[test]
    fn time_text_round_trips() {
        let ms = parse_time("2024-03-09 14:30").unwrap();
        assert_eq!(format_time(ms), "2024-03-09 14:30");
        assert_eq!(parse_time("1970-01-01 00:00"), Some(0));
        assert_eq!(parse_time("2024-13-01 00:00"), None);
        assert_eq!(parse_time("nonsense"), None);
    }

    #[test]
    fn rgba_converts_both_ways() {
        // Opaque colours round-trip exactly; translucent ones lose at most a unit to premultiplying.
        assert_eq!(
            rgba_from_color(color_from_rgba(0x11_22_33_ff)),
            0x11_22_33_ff
        );
        let c = color_from_rgba(0x11_22_33_80);
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        assert_eq!(a, 0x80);
        assert!(r.abs_diff(0x11) <= 1 && g.abs_diff(0x22) <= 1 && b.abs_diff(0x33) <= 1);
    }

    #[test]
    fn a_value_equal_to_the_default_is_no_override() {
        let schema = InputSchema {
            inputs: vec![def("len", ValueType::Int, InputValue::Int(14), None)],
        };
        let mut values = InputValues::new();
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(30)),
        );
        assert_eq!(values.get("len"), Some(&InputValue::Int(30)));
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(14)),
        );
        assert!(values.is_empty(), "back at the default");
        apply_edit(
            &mut values,
            &schema,
            InputEdit::Set("len".into(), InputValue::Int(5)),
        );
        apply_edit(&mut values, &schema, InputEdit::Reset);
        assert!(values.is_empty());
    }
}
