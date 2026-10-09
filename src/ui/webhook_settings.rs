//! Settings ▸ Webhooks: the reusable endpoints that price alerts and Pine alerts can post
//! to. A pure view returning `WebhookSettingsAction`s; the host owns the store, sends the
//! test (through the same `deliver` alerts use, in test mode) and feeds the result back.

// Transient: the Settings window hosts this once the webhook store is wired.
#![allow(dead_code)]

use crate::model::webhook::{
    EndpointId, WebhookDeliveryError, WebhookDeliveryResult, WebhookEndpoint,
};
use crate::model::webhook_policy;
use crate::ui::theme;
use crate::ui::webhook_editor::{
    self, WebhookEditorOutcome, WebhookEditorResult, WebhookEditorState,
};
use eframe::egui::{self, Align2, Color32, Id, RichText, Vec2};
use std::collections::HashMap;

pub const DEFAULT_TEST_PAYLOAD: &str = r#"{"event":"DegenView webhook test"}"#;

/// What the last Test button press found out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WebhookTestState {
    #[default]
    NotTested,
    Testing,
    Delivered {
        status: Option<u16>,
        duration_secs: f64,
    },
    Failed {
        error: WebhookDeliveryError,
        status: Option<u16>,
        duration_secs: f64,
    },
    TimedOut {
        duration_secs: f64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Neutral,
    Good,
    Bad,
}

impl WebhookTestState {
    pub fn from_result(result: &WebhookDeliveryResult) -> Self {
        match result.error {
            None => Self::Delivered {
                status: result.status_code,
                duration_secs: result.duration_secs,
            },
            Some(WebhookDeliveryError::Timeout) => Self::TimedOut {
                duration_secs: result.duration_secs,
            },
            Some(error) => Self::Failed {
                error,
                status: result.status_code,
                duration_secs: result.duration_secs,
            },
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::NotTested => "Not tested",
            Self::Testing => "Testing…",
            Self::Delivered { .. } => "Delivered",
            Self::Failed { .. } => "Failed",
            Self::TimedOut { .. } => "Timed out",
        }
    }

    /// `HTTP 200 · 143 ms`, `3.0 s`, or a short reason. Never a raw system error.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NotTested | Self::Testing => None,
            Self::Delivered {
                status,
                duration_secs,
            } => Some(join(
                status.map(|s| format!("HTTP {s}")),
                milliseconds(*duration_secs),
            )),
            Self::TimedOut { duration_secs } => Some(format!("{duration_secs:.1} s")),
            Self::Failed {
                error,
                status,
                duration_secs,
            } => Some(match error {
                WebhookDeliveryError::HttpFailure => join(
                    status.map(|s| format!("HTTP {s}")),
                    milliseconds(*duration_secs),
                ),
                WebhookDeliveryError::NetworkFailure => "Could not connect".to_owned(),
                WebhookDeliveryError::InvalidUrl
                | WebhookDeliveryError::UnsupportedScheme
                | WebhookDeliveryError::UnsupportedPort => "Invalid address".to_owned(),
                WebhookDeliveryError::SecretUnavailable => {
                    "Secret not found in Keychain".to_owned()
                }
                WebhookDeliveryError::InvalidHeader => "Invalid header".to_owned(),
                WebhookDeliveryError::SecretInHost => "Secret can't be in the host".to_owned(),
                WebhookDeliveryError::MissingEndpoint => "Webhook not found".to_owned(),
                WebhookDeliveryError::DisabledEndpoint => "Webhook is off".to_owned(),
                WebhookDeliveryError::Timeout => format!("{duration_secs:.1} s"),
            }),
        }
    }

    pub fn tone(&self) -> Tone {
        match self {
            Self::NotTested | Self::Testing => Tone::Neutral,
            Self::Delivered { .. } => Tone::Good,
            Self::Failed { .. } | Self::TimedOut { .. } => Tone::Bad,
        }
    }
}

fn milliseconds(duration_secs: f64) -> String {
    format!("{} ms", (duration_secs * 1000.0).round() as i64)
}

fn join(status: Option<String>, timing: String) -> String {
    match status {
        Some(status) => format!("{status} · {timing}"),
        None => timing,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebhookSettingsAction {
    /// Send `message` to this endpoint in test mode, then call `set_test_result`.
    Test {
        endpoint: Box<WebhookEndpoint>,
        message: String,
    },
    Toggle(EndpointId),
    Delete(EndpointId),
    /// Create (`id == None`) or update an endpoint; on failure set the sheet's `failure`.
    Save(WebhookEditorResult),
}

pub struct WebhookSettingsInput<'a> {
    pub endpoints: &'a [WebhookEndpoint],
    /// Whether the Keychain holds a secret for this endpoint.
    pub has_secret: &'a dyn Fn(EndpointId) -> bool,
    /// The secret's value, read only when the editor opens.
    pub secret_for_edit: &'a dyn Fn(EndpointId) -> Option<String>,
    /// How many alerts use this endpoint, for the delete confirmation.
    pub reference_count: &'a dyn Fn(EndpointId) -> usize,
    pub load_failed: bool,
}

pub struct WebhookSettingsState {
    pub test_payload: String,
    editor: Option<WebhookEditorState>,
    pending_delete: Option<EndpointId>,
    test_states: HashMap<EndpointId, WebhookTestState>,
    pub failure: Option<String>,
}

impl Default for WebhookSettingsState {
    fn default() -> Self {
        Self {
            test_payload: DEFAULT_TEST_PAYLOAD.to_owned(),
            editor: None,
            pending_delete: None,
            test_states: HashMap::new(),
            failure: None,
        }
    }
}

impl WebhookSettingsState {
    pub fn set_test_result(&mut self, result: &WebhookDeliveryResult) {
        self.test_states
            .insert(result.endpoint_id, WebhookTestState::from_result(result));
    }

    pub fn test_state(&self, endpoint: EndpointId) -> WebhookTestState {
        self.test_states.get(&endpoint).copied().unwrap_or_default()
    }

    /// A save the store refused: keep the sheet open and say why.
    pub fn save_failed(&mut self, reason: String) {
        if let Some(editor) = &mut self.editor {
            editor.failure = Some(reason);
        }
    }

    /// A save that went through: close the sheet.
    pub fn save_succeeded(&mut self) {
        self.editor = None;
        self.failure = None;
    }

    pub fn forget(&mut self, endpoint: EndpointId) {
        self.test_states.remove(&endpoint);
    }

    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }
}

/// The delete confirmation's wording, with a usage count.
pub fn delete_message(references: usize) -> String {
    match references {
        0 => "This webhook isn’t used by any alert.".to_owned(),
        1 => "This webhook is currently used by 1 alert. Deleting it will stop webhook delivery for that alert."
            .to_owned(),
        count => format!(
            "This webhook is currently used by {count} alerts. Deleting it will stop webhook delivery for those alerts."
        ),
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
) -> Vec<WebhookSettingsAction> {
    let mut actions = Vec::new();

    ui.label(RichText::new("Webhooks").heading());
    ui.label(
        RichText::new(
            "Send alert messages to external HTTP endpoints, such as a trading bot or a chat channel.",
        )
        .weak(),
    );
    ui.add_space(10.0);

    if input.load_failed {
        notice(
            ui,
            "Saved webhooks could not be read",
            "Changes are turned off so nothing is overwritten. Restart DegenView to try again.",
        );
    }

    ui.label(RichText::new("Endpoints").strong());
    ui.label(
        RichText::new(
            "Each alert chooses which of these it posts to. Turning one off pauses it everywhere.",
        )
        .small()
        .weak(),
    );
    ui.add_space(4.0);
    if input.endpoints.is_empty() {
        card(ui, |ui| {
            ui.label(
                RichText::new(
                    "No webhooks yet. Add one to start sending alerts to your own server.",
                )
                .weak(),
            );
        });
    }
    for endpoint in input.endpoints {
        endpoint_row(ui, state, input, endpoint, &mut actions);
        ui.add_space(4.0);
    }

    ui.add_space(8.0);
    ui.label(RichText::new("Test message").strong());
    ui.label(
        RichText::new(
            "Sent by each Test button. Valid JSON goes out as application/json, anything else as text/plain.",
        )
        .small()
        .weak(),
    );
    card(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut state.test_payload)
                .font(egui::TextStyle::Monospace)
                .desired_rows(1)
                .desired_width(f32::INFINITY),
        );
    });

    ui.add_space(8.0);
    ui.label(RichText::new("How it works").strong());
    ui.label(
        RichText::new(
            "Alerts send their final message as the request body. A secret is kept in your Keychain \
             and added to the URL or a header where you place it. Each alert triggers one attempt per \
             webhook, with a 3 second limit and no retries, so a slow or failing server can never send \
             the same message twice.",
        )
        .small()
        .weak(),
    );

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if let Some(failure) = &state.failure {
            ui.label(RichText::new(failure).small().color(theme::DOWN));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!input.load_failed, egui::Button::new("＋ Add Webhook"))
                .clicked()
            {
                state.editor = Some(WebhookEditorState::new_blank());
            }
        });
    });

    if let Some(editor) = &mut state.editor {
        match webhook_editor::show(ui.ctx(), editor) {
            Some(WebhookEditorOutcome::Save(result)) => {
                actions.push(WebhookSettingsAction::Save(result));
            }
            Some(WebhookEditorOutcome::Cancel) => state.editor = None,
            None => {}
        }
    }
    delete_dialog(ui.ctx(), state, input, &mut actions);
    actions
}

fn notice(ui: &mut egui::Ui, title: &str, detail: &str) {
    egui::Frame::new()
        .fill(theme::REPLAY.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, theme::REPLAY.gamma_multiply(0.35)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("⚠ {title}")).strong());
            ui.label(RichText::new(detail).small().weak());
        });
    ui.add_space(8.0);
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn endpoint_row(
    ui: &mut egui::Ui,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
    endpoint: &WebhookEndpoint,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    let test = state.test_state(endpoint.id);
    ui.scope(|ui| {
        if !endpoint.is_enabled {
            ui.multiply_opacity(0.75);
        }
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let dot = if endpoint.is_enabled {
                    theme::UP
                } else {
                    Color32::from_gray(110)
                };
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, dot);
                response.on_hover_text(if endpoint.is_enabled {
                    "Enabled"
                } else {
                    "Disabled: alerts skip this webhook"
                });

                ui.vertical(|ui| {
                    ui.label(RichText::new(&endpoint.name).strong());
                    if !endpoint.description.is_empty() {
                        ui.label(RichText::new(&endpoint.description).small().weak());
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(endpoint.method.as_str())
                                .small()
                                .monospace()
                                .strong()
                                .weak(),
                        );
                        // Never the full URL: it is a credential.
                        ui.label(
                            RichText::new(webhook_policy::redact(&endpoint.url))
                                .small()
                                .monospace()
                                .weak(),
                        );
                        if (input.has_secret)(endpoint.id) {
                            ui.label(RichText::new("🔒").small())
                                .on_hover_text("Uses a secret kept in your Keychain");
                        }
                        if !endpoint.headers.is_empty() {
                            ui.label(
                                RichText::new(if endpoint.headers.len() == 1 {
                                    "1 header".to_owned()
                                } else {
                                    format!("{} headers", endpoint.headers.len())
                                })
                                .small()
                                .weak(),
                            );
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("⋯", |ui| {
                        if ui.button("Edit…").clicked() {
                            state.editor = Some(WebhookEditorState::editing(
                                endpoint,
                                (input.secret_for_edit)(endpoint.id),
                            ));
                            ui.close();
                        }
                        if ui
                            .button(if endpoint.is_enabled {
                                "Disable"
                            } else {
                                "Enable"
                            })
                            .clicked()
                        {
                            actions.push(WebhookSettingsAction::Toggle(endpoint.id));
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(RichText::new("Delete…").color(theme::DOWN))
                            .clicked()
                        {
                            state.pending_delete = Some(endpoint.id);
                            ui.close();
                        }
                    });
                    let testing = test == WebhookTestState::Testing;
                    if ui
                        .add_enabled(
                            !testing,
                            egui::Button::new(if testing { "…" } else { "Test" }),
                        )
                        .on_hover_text(
                            "Send the test payload to this webhook, even if it is disabled",
                        )
                        .clicked()
                    {
                        state
                            .test_states
                            .insert(endpoint.id, WebhookTestState::Testing);
                        actions.push(WebhookSettingsAction::Test {
                            endpoint: Box::new(endpoint.clone()),
                            message: state.test_payload.clone(),
                        });
                    }
                    ui.vertical(|ui| {
                        let color = match test.tone() {
                            Tone::Neutral => ui.visuals().weak_text_color(),
                            Tone::Good => theme::UP,
                            Tone::Bad => theme::DOWN,
                        };
                        ui.label(RichText::new(test.title()).small().strong().color(color));
                        if let Some(detail) = test.detail() {
                            ui.label(RichText::new(detail).small().weak());
                        }
                    });
                });
            });
        });
    });
}

fn delete_dialog(
    ctx: &egui::Context,
    state: &mut WebhookSettingsState,
    input: &WebhookSettingsInput,
    actions: &mut Vec<WebhookSettingsAction>,
) {
    let Some(id) = state.pending_delete else {
        return;
    };
    let Some(endpoint) = input.endpoints.iter().find(|endpoint| endpoint.id == id) else {
        state.pending_delete = None;
        return;
    };
    let mut done = false;
    egui::Window::new(format!("Delete “{}”?", endpoint.name))
        .id(Id::new("webhook-delete"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.label(delete_message((input.reference_count)(id)));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Delete").color(theme::DOWN))
                    .clicked()
                {
                    actions.push(WebhookSettingsAction::Delete(id));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.pending_delete = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(
        error: Option<WebhookDeliveryError>,
        status: Option<u16>,
        secs: f64,
    ) -> WebhookDeliveryResult {
        WebhookDeliveryResult {
            endpoint_id: 1,
            timestamp_ms: 0,
            duration_secs: secs,
            status_code: status,
            error,
        }
    }

    #[test]
    fn a_delivered_test_reports_status_and_timing() {
        let state = WebhookTestState::from_result(&result(None, Some(200), 0.1434));
        assert_eq!(state.title(), "Delivered");
        assert_eq!(state.detail().as_deref(), Some("HTTP 200 · 143 ms"));
        assert_eq!(state.tone(), Tone::Good);
        let no_status = WebhookTestState::from_result(&result(None, None, 0.05));
        assert_eq!(no_status.detail().as_deref(), Some("50 ms"));
    }

    #[test]
    fn failures_get_short_human_reasons_never_raw_errors() {
        use WebhookDeliveryError::*;
        let cases = [
            (HttpFailure, Some(500), "HTTP 500 · 20 ms"),
            (HttpFailure, None, "20 ms"),
            (NetworkFailure, None, "Could not connect"),
            (InvalidUrl, None, "Invalid address"),
            (UnsupportedScheme, None, "Invalid address"),
            (UnsupportedPort, None, "Invalid address"),
            (SecretUnavailable, None, "Secret not found in Keychain"),
            (InvalidHeader, None, "Invalid header"),
            (SecretInHost, None, "Secret can't be in the host"),
            (MissingEndpoint, None, "Webhook not found"),
            (DisabledEndpoint, None, "Webhook is off"),
        ];
        for (error, status, expected) in cases {
            let state = WebhookTestState::from_result(&result(Some(error), status, 0.02));
            assert_eq!(state.title(), "Failed", "{error:?}");
            assert_eq!(state.detail().as_deref(), Some(expected), "{error:?}");
            assert_eq!(state.tone(), Tone::Bad);
        }
    }

    #[test]
    fn a_timeout_is_its_own_state() {
        let state =
            WebhookTestState::from_result(&result(Some(WebhookDeliveryError::Timeout), None, 3.02));
        assert_eq!(state.title(), "Timed out");
        assert_eq!(state.detail().as_deref(), Some("3.0 s"));
        assert_eq!(state.tone(), Tone::Bad);
    }

    #[test]
    fn untested_and_testing_have_no_detail() {
        for state in [WebhookTestState::NotTested, WebhookTestState::Testing] {
            assert_eq!(state.detail(), None);
            assert_eq!(state.tone(), Tone::Neutral);
        }
        assert_eq!(WebhookTestState::NotTested.title(), "Not tested");
        assert_eq!(WebhookTestState::Testing.title(), "Testing…");
    }

    #[test]
    fn the_state_remembers_results_per_endpoint_and_can_forget_them() {
        let mut state = WebhookSettingsState::default();
        assert_eq!(state.test_state(1), WebhookTestState::NotTested);
        state.set_test_result(&result(None, Some(204), 0.2));
        assert_eq!(state.test_state(1).title(), "Delivered");
        assert_eq!(state.test_state(2), WebhookTestState::NotTested);
        state.forget(1);
        assert_eq!(state.test_state(1), WebhookTestState::NotTested);
    }

    #[test]
    fn the_default_test_message_is_json() {
        assert!(webhook_policy::is_json(DEFAULT_TEST_PAYLOAD));
        assert_eq!(
            WebhookSettingsState::default().test_payload,
            DEFAULT_TEST_PAYLOAD
        );
    }

    #[test]
    fn deleting_says_how_many_alerts_lose_their_webhook() {
        assert_eq!(delete_message(0), "This webhook isn’t used by any alert.");
        assert!(
            delete_message(1)
                .contains("1 alert. Deleting it will stop webhook delivery for that alert.")
        );
        let many = delete_message(3);
        assert!(many.contains("3 alerts") && many.contains("those alerts"));
    }

    #[test]
    fn a_failed_save_keeps_the_sheet_open_with_the_reason() {
        let mut state = WebhookSettingsState::default();
        state.save_failed("ignored: nothing open".into());
        assert!(!state.is_editing());
        state.editor = Some(WebhookEditorState::new_blank());
        state.save_failed("Couldn't save".into());
        assert_eq!(
            state.editor.as_ref().and_then(|e| e.failure.as_deref()),
            Some("Couldn't save")
        );
        state.save_succeeded();
        assert!(!state.is_editing());
    }
}
