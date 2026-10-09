//! The sheets for a chart's Pine script alerts: create one (`show`), and choose which webhooks an
//! existing one posts to (`show_webhooks`). Pure views over plain values: the Pine alert store
//! and runtime host (which compile the script, hash its source and own the subscriptions) feed
//! the facts in and apply the outcome. A port of `PineAlertEditor` and `PineAlertWebhooksSheet`.

// Transient: the chart card opens this once the Pine session lands.
#![allow(dead_code)]

use crate::model::pine_alert::{self, ArmingFacts, Blocker, CallRow, PineAlertSubscription};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::model::webhook_policy;
use crate::pine::alert_call::AlertCallSite;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Vec2};

/// More call sites than this scroll instead of growing the sheet.
pub const INLINE_CALL_LIMIT: usize = 4;

pub const FOOTNOTE: &str = "Fires only while this chart shows this symbol and timeframe and its live feed is running. \
Editing the script pauses the alert until you re-arm it.";

#[derive(Default)]
pub struct PineAlertEditorState {
    pub note: String,
    pub sends_to_webhooks: bool,
    pub webhook_ids: Vec<EndpointId>,
}

pub struct PineAlertEditorInput<'a> {
    /// The chart's market, "BTC/USDT".
    pub chart_title: &'a str,
    /// "Binance".
    pub source_name: &'a str,
    pub timeframe: &'a str,
    /// The saved script's name, else the declaration's title; empty until known.
    pub script_name: &'a str,
    pub facts: ArmingFacts,
    pub call_sites: &'a [AlertCallSite],
    pub endpoints: &'a [WebhookEndpoint],
}

#[derive(Clone, Debug, PartialEq)]
pub enum PineAlertEditorOutcome {
    /// Arm it with this note and these endpoints (none unless webhooks are switched on).
    Create {
        note: String,
        webhook_endpoint_ids: Vec<EndpointId>,
    },
    Cancel,
}

impl PineAlertEditorState {
    pub fn toggle_webhook(&mut self, endpoint: EndpointId, on: bool) {
        self.webhook_ids.retain(|id| *id != endpoint);
        if on {
            self.webhook_ids.push(endpoint);
        }
    }

    /// The webhooks that will be used: none unless the switch is on.
    pub fn chosen_webhooks(&self) -> Vec<EndpointId> {
        if self.sends_to_webhooks {
            self.webhook_ids.clone()
        } else {
            Vec::new()
        }
    }
}

// MARK: Pure text

pub fn can_create(facts: ArmingFacts) -> bool {
    facts.loaded && pine_alert::blocker(facts).is_none()
}

/// The script name shown in the card: its own, else a placeholder while loading or untitled.
pub fn displayed_script_name(script_name: &str, loaded: bool) -> String {
    if !script_name.is_empty() {
        script_name.to_owned()
    } else if loaded {
        "Untitled script".to_owned()
    } else {
        "Loading…".to_owned()
    }
}

/// "Fires on live bars of BTC/USDT · 1h", shown beside the Create button.
pub fn summary(facts: ArmingFacts, chart_title: &str, timeframe: &str) -> String {
    if can_create(facts) {
        format!("Fires on live bars of {chart_title} · {timeframe}")
    } else {
        "Nothing to arm yet.".to_owned()
    }
}

/// The readiness badge on the script card.
pub fn status_badge(facts: ArmingFacts) -> (&'static str, Color32) {
    if !facts.loaded {
        ("Checking…", Color32::from_gray(150))
    } else if pine_alert::blocker(facts).is_none() {
        ("Ready", theme::UP)
    } else {
        ("Can't arm", theme::REPLAY)
    }
}

/// The webhook switch's subtitle.
pub fn webhook_subtitle(is_on: bool, selected: usize, endpoints: usize) -> String {
    if !is_on {
        "Also post this alert to your own server or bot.".to_owned()
    } else if selected > 0 {
        format!("{selected} selected")
    } else if endpoints == 0 {
        "Add a webhook to choose where to post.".to_owned()
    } else {
        "Choose where to post.".to_owned()
    }
}

// MARK: Create sheet

pub fn show(
    ctx: &egui::Context,
    state: &mut PineAlertEditorState,
    input: &PineAlertEditorInput,
) -> Option<PineAlertEditorOutcome> {
    let mut outcome = None;
    egui::Window::new("Create Script Alert")
        .id(Id::new("pine-alert-editor"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(480.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Create Script Alert").heading());
            ui.label(
                RichText::new(format!("{} · {}", input.chart_title, input.source_name)).weak(),
            );
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .max_height(440.0)
                .auto_shrink([false, true])
                .show(ui, |ui| form(ui, state, input));
            ui.separator();
            footer(ui, state, input, &mut outcome);
        });
    if outcome.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(PineAlertEditorOutcome::Cancel);
    }
    outcome
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(9))
        .stroke(egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.5)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn pill(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(14))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().weak());
        });
}

fn form(ui: &mut egui::Ui, state: &mut PineAlertEditorState, input: &PineAlertEditorInput) {
    script_card(ui, input);
    ui.add_space(14.0);
    triggers(ui, input);
    ui.add_space(14.0);
    ui.add(
        egui::TextEdit::singleline(&mut state.note)
            .hint_text("Add a note (optional)")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(14.0);
    webhooks(ui, state, input.endpoints);
    if let Some(Blocker { title, detail }) = pine_alert::blocker(input.facts) {
        ui.add_space(14.0);
        egui::Frame::new()
            .fill(theme::REPLAY.gamma_multiply(0.14))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(format!("⚠ {title}"))
                        .strong()
                        .color(theme::REPLAY),
                );
                ui.label(RichText::new(detail).small());
            });
    }
    ui.add_space(14.0);
    ui.label(RichText::new(format!("ⓘ {FOOTNOTE}")).small().weak());
    ui.add_space(6.0);
}

fn script_card(ui: &mut egui::Ui, input: &PineAlertEditorInput) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("{ }")
                    .strong()
                    .size(16.0)
                    .color(theme::ACCENT),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(displayed_script_name(input.script_name, input.facts.loaded))
                        .strong()
                        .size(15.0),
                );
                ui.horizontal(|ui| {
                    pill(ui, input.chart_title);
                    pill(ui, input.timeframe);
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = status_badge(input.facts);
                ui.label(
                    RichText::new(format!("● {text}"))
                        .small()
                        .strong()
                        .color(color),
                );
            });
        });
    });
}

fn triggers(ui: &mut egui::Ui, input: &PineAlertEditorInput) {
    ui.label(RichText::new("Alerts in this script").strong());
    ui.add_space(4.0);
    if input.call_sites.is_empty() {
        card(ui, |ui| {
            ui.label(
                RichText::new(if input.facts.loaded {
                    "No alert calls"
                } else {
                    "Reading the script…"
                })
                .small()
                .weak(),
            );
        });
        return;
    }
    let rows = |ui: &mut egui::Ui| {
        for site in input.call_sites {
            call_row(ui, &pine_alert::call_row(site));
            ui.add_space(4.0);
        }
    };
    if input.call_sites.len() > INLINE_CALL_LIMIT {
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, rows);
    } else {
        rows(ui);
    }
}

fn call_row(ui: &mut egui::Ui, row: &CallRow) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("🔔").color(theme::ACCENT));
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(&row.headline).strong()).truncate());
                if let Some(detail) = &row.detail {
                    ui.add(egui::Label::new(RichText::new(detail).small().weak()).truncate());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.vertical(|ui| {
                    pill(ui, row.frequency);
                    ui.label(RichText::new(format!("Line {}", row.line)).small().weak());
                });
            });
        })
        .response
        .on_hover_text(row.help);
    });
}

fn webhooks(ui: &mut egui::Ui, state: &mut PineAlertEditorState, endpoints: &[WebhookEndpoint]) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Send to webhooks").strong());
                ui.label(
                    RichText::new(webhook_subtitle(
                        state.sends_to_webhooks,
                        state.webhook_ids.len(),
                        endpoints.len(),
                    ))
                    .small()
                    .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut state.sends_to_webhooks, "");
            });
        });
        if state.sends_to_webhooks {
            endpoint_list(ui, &mut state.webhook_ids, endpoints);
            ui.label(
                RichText::new(
                    "The alert's message is posted as is. Frequency and re-arming work exactly as they do for notifications.",
                )
                .small()
                .weak(),
            );
        }
    });
}

/// A checkbox per endpoint, naming it and its redacted URL.
fn endpoint_list(ui: &mut egui::Ui, selected: &mut Vec<EndpointId>, endpoints: &[WebhookEndpoint]) {
    if endpoints.is_empty() {
        ui.label(
            RichText::new("No webhooks yet. Add one in Settings ▸ Webhooks.")
                .small()
                .weak(),
        );
    }
    for endpoint in endpoints {
        let mut on = selected.contains(&endpoint.id);
        let label = format!(
            "{}  {}{}",
            endpoint.name,
            webhook_policy::redact(&endpoint.url),
            if endpoint.is_enabled { "" } else { "  (off)" }
        );
        if ui.checkbox(&mut on, label).changed() {
            selected.retain(|id| *id != endpoint.id);
            if on {
                selected.push(endpoint.id);
            }
        }
    }
}

fn footer(
    ui: &mut egui::Ui,
    state: &PineAlertEditorState,
    input: &PineAlertEditorInput,
    outcome: &mut Option<PineAlertEditorOutcome>,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(summary(input.facts, input.chart_title, input.timeframe))
                    .small()
                    .weak(),
            )
            .truncate(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button =
                egui::Button::new(RichText::new("Create Alert").strong().color(Color32::WHITE))
                    .fill(theme::ACCENT);
            if ui.add_enabled(can_create(input.facts), button).clicked() {
                *outcome = Some(PineAlertEditorOutcome::Create {
                    note: state.note.clone(),
                    webhook_endpoint_ids: state.chosen_webhooks(),
                });
            }
            if ui.button("Cancel").clicked() {
                *outcome = Some(PineAlertEditorOutcome::Cancel);
            }
        });
    });
    ui.add_space(4.0);
}

// MARK: Webhooks of an existing alert

pub struct PineAlertWebhooksState {
    pub selection: Vec<EndpointId>,
}

impl PineAlertWebhooksState {
    pub fn new(subscription: &PineAlertSubscription) -> Self {
        Self {
            selection: subscription.webhook_endpoint_ids.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PineAlertWebhooksOutcome {
    Save(Vec<EndpointId>),
    Cancel,
}

/// Chooses which webhook endpoints an existing Pine alert posts to.
pub fn show_webhooks(
    ctx: &egui::Context,
    state: &mut PineAlertWebhooksState,
    subscription: &PineAlertSubscription,
    endpoints: &[WebhookEndpoint],
) -> Option<PineAlertWebhooksOutcome> {
    let mut outcome = None;
    egui::Window::new("Webhooks")
        .id(Id::new("pine-alert-webhooks"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(420.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Webhooks for {}", subscription.script_name)).heading());
            ui.label(
                RichText::new(
                    "The alert's message is posted as is. Frequency and re-arming work exactly as they do for notifications.",
                )
                .small()
                .weak(),
            );
            ui.add_space(10.0);
            endpoint_list(ui, &mut state.selection, endpoints);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save = egui::Button::new(RichText::new("Save").strong().color(Color32::WHITE))
                        .fill(theme::ACCENT);
                    if ui.add(save).clicked() {
                        outcome = Some(PineAlertWebhooksOutcome::Save(state.selection.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(PineAlertWebhooksOutcome::Cancel);
                    }
                });
            });
        });
    if outcome.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(PineAlertWebhooksOutcome::Cancel);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::pine_alert::PineAlertState;
    use crate::pine::alert_frequency::AlertFrequency;
    use crate::pine::token::{SourcePosition, SourceRange};

    fn facts() -> ArmingFacts {
        ArmingFacts {
            loaded: true,
            instance_exists: true,
            source_loaded: true,
            call_sites: 1,
            duplicate_exists: false,
        }
    }

    fn site() -> AlertCallSite {
        AlertCallSite {
            frequency: Some(AlertFrequency::OncePerBarClose),
            is_condition: false,
            range: SourceRange::new(SourcePosition::new(4, 1, 0), SourcePosition::new(4, 5, 4)),
            title: None,
            message: Some("Buy".into()),
        }
    }

    #[test]
    fn creating_needs_a_loaded_unblocked_script() {
        assert!(can_create(facts()));
        assert!(
            !can_create(ArmingFacts {
                loaded: false,
                ..facts()
            }),
            "unknown is not ready"
        );
        assert!(!can_create(ArmingFacts {
            call_sites: 0,
            ..facts()
        }));
        assert!(!can_create(ArmingFacts {
            duplicate_exists: true,
            ..facts()
        }));
    }

    #[test]
    fn the_card_and_footer_say_what_will_happen() {
        assert_eq!(displayed_script_name("RSI", true), "RSI");
        assert_eq!(displayed_script_name("", true), "Untitled script");
        assert_eq!(displayed_script_name("", false), "Loading…");
        assert_eq!(
            summary(facts(), "BTC/USDT", "1h"),
            "Fires on live bars of BTC/USDT · 1h"
        );
        assert_eq!(
            summary(
                ArmingFacts {
                    call_sites: 0,
                    ..facts()
                },
                "BTC/USDT",
                "1h"
            ),
            "Nothing to arm yet."
        );
        assert_eq!(
            status_badge(ArmingFacts {
                loaded: false,
                ..facts()
            })
            .0,
            "Checking…"
        );
        assert_eq!(status_badge(facts()).0, "Ready");
        assert_eq!(
            status_badge(ArmingFacts {
                call_sites: 0,
                ..facts()
            })
            .0,
            "Can't arm"
        );
    }

    #[test]
    fn the_webhook_switch_explains_its_state() {
        assert_eq!(
            webhook_subtitle(false, 3, 3),
            "Also post this alert to your own server or bot."
        );
        assert_eq!(webhook_subtitle(true, 2, 3), "2 selected");
        assert_eq!(
            webhook_subtitle(true, 0, 0),
            "Add a webhook to choose where to post."
        );
        assert_eq!(webhook_subtitle(true, 0, 3), "Choose where to post.");
    }

    #[test]
    fn webhooks_are_used_only_when_the_switch_is_on() {
        let mut state = PineAlertEditorState::default();
        state.toggle_webhook(4, true);
        state.toggle_webhook(5, true);
        state.toggle_webhook(4, true);
        assert_eq!(state.webhook_ids, [5, 4], "no duplicates");
        assert!(
            state.chosen_webhooks().is_empty(),
            "switched off: none, whatever was picked"
        );
        state.sends_to_webhooks = true;
        assert_eq!(state.chosen_webhooks(), [5, 4]);
        state.toggle_webhook(5, false);
        assert_eq!(state.chosen_webhooks(), [4]);
    }

    fn run_frame(ctx: &egui::Context, paint: impl FnOnce(&egui::Context)) {
        let mut paint = Some(paint);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(paint) = paint.take() {
                paint(ui.ctx());
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn both_sheets_draw_headlessly() {
        let ctx = egui::Context::default();
        let sites = [site(), site(), site(), site(), site()];
        for (loaded, calls) in [
            (false, &sites[..0]),
            (true, &sites[..1]),
            (true, &sites[..]),
        ] {
            let mut state = PineAlertEditorState {
                sends_to_webhooks: true,
                ..PineAlertEditorState::default()
            };
            let mut outcome = None;
            run_frame(&ctx, |ctx| {
                outcome = show(
                    ctx,
                    &mut state,
                    &PineAlertEditorInput {
                        chart_title: "BTC/USDT",
                        source_name: "Binance",
                        timeframe: "1h",
                        script_name: "RSI",
                        facts: ArmingFacts {
                            loaded,
                            call_sites: calls.len(),
                            ..facts()
                        },
                        call_sites: calls,
                        endpoints: &[],
                    },
                );
            });
            assert_eq!(outcome, None);
        }
        let subscription = PineAlertSubscription {
            id: 1,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "RSI".into(),
            symbol_key: "BINANCE:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "h".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: vec![2, 3],
        };
        let mut state = PineAlertWebhooksState::new(&subscription);
        assert_eq!(state.selection, [2, 3]);
        let mut outcome = None;
        run_frame(&ctx, |ctx| {
            outcome = show_webhooks(ctx, &mut state, &subscription, &[]);
        });
        assert_eq!(outcome, None);
    }
}
