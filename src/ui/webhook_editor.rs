//! The "Add Webhook" / "Edit Webhook" sheet: a name, where to post, an optional secret
//! and headers, and whether it is on. A pure view: it edits a `WebhookEditorState` the
//! host owns and reports `Save` or `Cancel`. All validation lives in `WebhookEditorState`
//! (and in `webhook_policy`), so the sheet cannot disagree with the store.

// Transient: the Settings window hosts this once the webhook store is wired.
#![allow(dead_code)]

use crate::model::webhook::{EndpointId, WebhookEndpoint, WebhookHeader, WebhookHttpMethod};
use crate::model::webhook_policy::{
    self, MAXIMUM_HEADERS, SECRET_TOKEN, SecretPlacement, WebhookHeaderError, WebhookSecretError,
};
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, Id, RichText, Vec2};

/// What the sheet hands back when the user saves: already trimmed and stripped of blank
/// header rows, ready for the store.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookEditorResult {
    /// `None` creates a new endpoint.
    pub id: Option<EndpointId>,
    pub name: String,
    pub description: String,
    pub url: String,
    pub method: WebhookHttpMethod,
    pub headers: Vec<WebhookHeader>,
    /// Empty means "no secret"; the store removes any saved one.
    pub secret: String,
    pub is_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebhookEditorOutcome {
    Save(WebhookEditorResult),
    Cancel,
}

/// The one-line status under the secret field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecretStatus {
    Problem(WebhookSecretError),
    /// `{{secret}}` is used but no value is entered.
    MissingValue,
    Used(String),
    /// A value is entered but nothing uses it.
    NotUsedYet,
    Idle,
}

#[derive(Clone, Debug)]
pub struct WebhookEditorState {
    existing_id: Option<EndpointId>,
    pub name: String,
    pub description: String,
    pub url: String,
    pub method: WebhookHttpMethod,
    pub secret: String,
    pub headers: Vec<WebhookHeader>,
    pub headers_expanded: bool,
    pub is_enabled: bool,
    /// Set by the host when saving failed, shown under the form.
    pub failure: Option<String>,
    secret_revealed: bool,
    /// A half-typed address is not an error yet: the message waits for the field to lose focus.
    url_has_focus: bool,
    focus_name: bool,
}

impl WebhookEditorState {
    pub fn new_blank() -> Self {
        Self {
            existing_id: None,
            name: String::new(),
            description: String::new(),
            url: String::new(),
            method: WebhookHttpMethod::Post,
            secret: String::new(),
            headers: Vec::new(),
            headers_expanded: false,
            is_enabled: true,
            failure: None,
            secret_revealed: false,
            url_has_focus: false,
            focus_name: true,
        }
    }

    /// `secret` is the value currently in the Keychain for this endpoint, if any.
    pub fn editing(endpoint: &WebhookEndpoint, secret: Option<String>) -> Self {
        Self {
            existing_id: Some(endpoint.id),
            name: endpoint.name.clone(),
            description: endpoint.description.clone(),
            url: endpoint.url.clone(),
            method: endpoint.method,
            secret: secret.unwrap_or_default(),
            headers: endpoint.headers.clone(),
            headers_expanded: !endpoint.headers.is_empty(),
            is_enabled: endpoint.is_enabled,
            failure: None,
            secret_revealed: false,
            url_has_focus: false,
            focus_name: false,
        }
    }

    pub fn is_new(&self) -> bool {
        self.existing_id.is_none()
    }

    // MARK: Validation

    pub fn trimmed_url(&self) -> &str {
        self.url.trim()
    }

    pub fn filled_headers(&self) -> Vec<WebhookHeader> {
        self.headers
            .iter()
            .filter(|header| !header.is_blank())
            .cloned()
            .collect()
    }

    /// Why the address is unacceptable, in the editor's wording; `None` when empty or fine.
    pub fn url_problem(&self) -> Option<&'static str> {
        let url = self.trimmed_url();
        if url.is_empty() {
            return None;
        }
        webhook_policy::validate_templates(url, &[]).map(webhook_policy::url_message)
    }

    pub fn url_is_valid(&self) -> bool {
        !self.trimmed_url().is_empty() && self.url_problem().is_none()
    }

    fn visible_url_problem(&self) -> Option<&'static str> {
        if self.url_has_focus {
            None
        } else {
            self.url_problem()
        }
    }

    pub fn uses_secret(&self) -> bool {
        webhook_policy::references_secret(self.trimmed_url(), &self.filled_headers())
    }

    pub fn secret_problem(&self) -> Option<WebhookSecretError> {
        if self.secret.is_empty() {
            None
        } else {
            webhook_policy::validate_secret(&self.secret)
        }
    }

    pub fn secret_is_valid(&self) -> bool {
        self.secret_problem().is_none() && (!self.uses_secret() || !self.secret.is_empty())
    }

    pub fn name_is_valid(&self) -> bool {
        !self.name.trim().is_empty()
    }

    pub fn can_save(&self) -> bool {
        self.name_is_valid()
            && self.url_is_valid()
            && webhook_policy::headers_are_valid(&self.filled_headers())
            && self.secret_is_valid()
    }

    /// The first problem with each filled header row, by id.
    pub fn header_errors(&self) -> Vec<(u64, WebhookHeaderError)> {
        webhook_policy::header_errors(&self.filled_headers())
    }

    pub fn secret_status(&self) -> SecretStatus {
        if let Some(problem) = self.secret_problem() {
            return SecretStatus::Problem(problem);
        }
        let url_uses = webhook_policy::contains_secret(&self.url);
        let header_uses = self
            .headers
            .iter()
            .filter(|header| webhook_policy::contains_secret(&header.value))
            .count();
        if url_uses || header_uses > 0 {
            if self.secret.is_empty() {
                return SecretStatus::MissingValue;
            }
            let mut places = Vec::new();
            if url_uses {
                places.push("the URL".to_owned());
            }
            if header_uses > 0 {
                places.push(if header_uses == 1 {
                    "1 header".to_owned()
                } else {
                    format!("{header_uses} headers")
                });
            }
            return SecretStatus::Used(format!("Used in {}", places.join(" and ")));
        }
        if self.secret.is_empty() {
            SecretStatus::Idle
        } else {
            SecretStatus::NotUsedYet
        }
    }

    /// A header that looks like a credential, typed in plain while no secret is saved.
    pub fn plain_credential(&self) -> Option<WebhookHeader> {
        if !self.secret.is_empty() {
            return None;
        }
        let filled = self.filled_headers();
        webhook_policy::plain_credential_header(&filled).cloned()
    }

    // MARK: Edits

    pub fn move_to_secret(&mut self, header_id: u64) {
        let Some(header) = self
            .headers
            .iter_mut()
            .find(|header| header.id == header_id)
        else {
            return;
        };
        let (secret, value) = webhook_policy::moving_to_secret(header);
        header.value = value;
        self.secret = secret;
    }

    /// The "Use secret…" shortcuts.
    pub fn apply_placement(&mut self, choice: &SecretPlacement) {
        let edit = webhook_policy::apply_placement(choice, &self.url, &self.headers);
        self.url = edit.url;
        self.headers = edit.headers;
        if edit.touches_headers {
            self.headers_expanded = true;
        }
    }

    pub fn add_header(&mut self) {
        if self.headers.len() < MAXIMUM_HEADERS {
            self.headers.push(WebhookHeader::new("", ""));
            self.headers_expanded = true;
        }
    }

    pub fn remove_header(&mut self, header_id: u64) {
        self.headers.retain(|header| header.id != header_id);
    }

    /// The saved form of this sheet, or `None` while it cannot be saved.
    pub fn result(&self) -> Option<WebhookEditorResult> {
        self.can_save().then(|| WebhookEditorResult {
            id: self.existing_id,
            name: self.name.trim().to_owned(),
            description: self.description.trim().to_owned(),
            url: self.trimmed_url().to_owned(),
            method: self.method,
            headers: self.filled_headers(),
            secret: self.secret.clone(),
            is_enabled: self.is_enabled,
        })
    }
}

// MARK: View

const FORM_WIDTH: f32 = 480.0;
const MAXIMUM_FORM_HEIGHT: f32 = 440.0;

pub fn show(ctx: &egui::Context, state: &mut WebhookEditorState) -> Option<WebhookEditorOutcome> {
    let mut outcome = None;
    let title = if state.is_new() {
        "Add Webhook"
    } else {
        "Edit Webhook"
    };
    egui::Window::new(title)
        .id(Id::new("webhook-editor"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(Vec2::new(FORM_WIDTH, 0.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Alerts post their message here over HTTP.")
                    .small()
                    .weak(),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .max_height(MAXIMUM_FORM_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| form(ui, state));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if state.is_new() {
                        "Add Webhook"
                    } else {
                        "Save Changes"
                    };
                    let save = ui.add_enabled(state.can_save(), egui::Button::new(label));
                    if save.clicked() {
                        outcome = state.result().map(WebhookEditorOutcome::Save);
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(WebhookEditorOutcome::Cancel);
                    }
                });
            });
        });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = Some(WebhookEditorOutcome::Cancel);
    }
    outcome
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

fn form(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    ui.spacing_mut().item_spacing.y = 6.0;

    section_title(ui, "Name");
    let name = ui.add(
        egui::TextEdit::singleline(&mut state.name)
            .hint_text("Trading Bot")
            .desired_width(f32::INFINITY),
    );
    if state.focus_name {
        name.request_focus();
        state.focus_name = false;
    }
    ui.add_space(8.0);

    url_section(ui, state);
    ui.add_space(8.0);
    secret_section(ui, state);

    if let Some(credential) = state.plain_credential() {
        credential_nudge(ui, state, &credential);
    }
    ui.add_space(8.0);
    headers_section(ui, state);
    ui.add_space(8.0);

    section_title(ui, "Description");
    ui.add(
        egui::TextEdit::singleline(&mut state.description)
            .hint_text("Optional")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);

    egui::Frame::new()
        .fill(Color32::from_white_alpha(10))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Enabled").strong());
                    ui.label(
                        RichText::new(if state.is_enabled {
                            "Alerts that use this webhook will post to it."
                        } else {
                            "Paused: alerts skip this webhook."
                        })
                        .small()
                        .weak(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut state.is_enabled, "");
                });
            });
        });

    if let Some(failure) = &state.failure {
        ui.add_space(6.0);
        ui.label(RichText::new(failure).small().color(theme::DOWN));
    }
}

fn url_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    section_title(ui, "Webhook URL");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("webhook-method")
            .selected_text(RichText::new(state.method.as_str()).monospace().strong())
            .width(70.0)
            .show_ui(ui, |ui| {
                for method in WebhookHttpMethod::ALL {
                    ui.selectable_value(&mut state.method, method, method.as_str());
                }
            })
            .response
            .on_hover_text("HTTP method");
        let width = ui.available_width() - 28.0;
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.url)
                .hint_text("https://example.com/webhook")
                .font(egui::TextStyle::Monospace)
                .desired_width(width.max(60.0)),
        );
        state.url_has_focus = response.has_focus();
        if state.url_is_valid() {
            ui.label(RichText::new("✔").color(theme::UP))
                .on_hover_text("Valid address");
        }
    });
    if let Some(problem) = state.visible_url_problem() {
        ui.label(
            RichText::new(format!("⚠ {problem}"))
                .small()
                .color(theme::DOWN),
        );
    } else if state.url_is_valid() && webhook_policy::contains_secret(state.trimmed_url()) {
        // What will actually be requested, with the secret hidden.
        ui.label(
            RichText::new(format!(
                "↳ {}",
                webhook_policy::masked_url(state.trimmed_url())
            ))
            .small()
            .monospace()
            .weak(),
        );
    }
}

fn secret_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    ui.horizontal(|ui| {
        section_title(ui, "Secret");
        ui.label(RichText::new("Optional").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new("⊕ Use secret").small().strong(), |ui| {
                let choices = [
                    (
                        "Add to the URL as ?token=",
                        SecretPlacement::UrlParameter("token".to_owned()),
                    ),
                    ("Send as a Bearer token", SecretPlacement::BearerToken),
                    ("Send as an X-API-Key header", SecretPlacement::ApiKey),
                    ("Send in a custom header…", SecretPlacement::CustomHeader),
                ];
                for (label, choice) in choices {
                    if ui.button(label).clicked() {
                        state.apply_placement(&choice);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Put the secret in the URL or in a header");
        });
    });
    ui.horizontal(|ui| {
        let width = ui.available_width() - 30.0;
        ui.add(
            egui::TextEdit::singleline(&mut state.secret)
                .password(!state.secret_revealed)
                .hint_text("API token or key")
                .desired_width(width.max(60.0)),
        );
        let eye = if state.secret_revealed {
            "🙈"
        } else {
            "👁"
        };
        if ui
            .add(egui::Button::new(eye).frame(false))
            .on_hover_text(if state.secret_revealed {
                "Hide"
            } else {
                "Show"
            })
            .clicked()
        {
            state.secret_revealed = !state.secret_revealed;
        }
    });
    match state.secret_status() {
        SecretStatus::Problem(problem) => {
            ui.label(
                RichText::new(format!("⚠ {}", webhook_policy::secret_message(problem)))
                    .small()
                    .color(theme::DOWN),
            );
        }
        SecretStatus::MissingValue => {
            ui.label(
                RichText::new(format!(
                    "⚠ Add the secret value that {SECRET_TOKEN} stands for."
                ))
                .small()
                .color(theme::DOWN),
            );
        }
        SecretStatus::Used(places) => {
            ui.label(
                RichText::new(format!("✔ {places}"))
                    .small()
                    .color(theme::UP),
            );
        }
        SecretStatus::NotUsedYet => {
            ui.label(
                RichText::new("Not used yet. Choose where it goes with Use secret.")
                    .small()
                    .color(theme::REPLAY),
            );
        }
        SecretStatus::Idle => {
            ui.label(
                RichText::new(
                    "Kept in your Keychain and inserted wherever you place it, in the URL or a header.",
                )
                .small()
                .weak(),
            );
        }
    }
}

fn credential_nudge(ui: &mut egui::Ui, state: &mut WebhookEditorState, header: &WebhookHeader) {
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(theme::REPLAY.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, theme::REPLAY.gamma_multiply(0.35)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("“{}” looks like a credential", header.name)).strong());
            ui.label(
                RichText::new(
                    "Save it as the secret so it is kept in your Keychain instead of in plain text.",
                )
                .small()
                .weak(),
            );
            if ui.small_button("Move to Secret").clicked() {
                state.move_to_secret(header.id);
            }
        });
}

fn headers_section(ui: &mut egui::Ui, state: &mut WebhookEditorState) {
    let errors = state.header_errors();
    let arrow = if state.headers_expanded { "⏷" } else { "⏵" };
    ui.horizontal(|ui| {
        let title = format!(
            "{arrow} Headers{}",
            if state.headers.is_empty() {
                String::new()
            } else {
                format!("  {}", state.headers.len())
            }
        );
        if ui
            .add(egui::Button::new(RichText::new(title).strong()).frame(false))
            .clicked()
        {
            state.headers_expanded = !state.headers_expanded;
        }
        if !state.headers_expanded && !errors.is_empty() {
            ui.label(RichText::new("⚠").color(theme::DOWN));
        }
        ui.label(RichText::new("Optional").small().weak());
    });
    if !state.headers_expanded {
        return;
    }
    if state.headers.is_empty() {
        ui.label(RichText::new("No custom headers yet.").small().weak());
    }
    let mut remove = None;
    let mut insert_secret = None;
    for header in &mut state.headers {
        let error = errors
            .iter()
            .find(|(id, _)| *id == header.id)
            .map(|(_, error)| *error);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut header.name)
                    .hint_text("Header-Name")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(150.0),
            );
            let width = ui.available_width() - 56.0;
            ui.add(
                egui::TextEdit::singleline(&mut header.value)
                    .hint_text("Value")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(width.max(60.0)),
            );
            if ui
                .add(egui::Button::new("🔑").frame(false))
                .on_hover_text("Insert the secret")
                .clicked()
            {
                insert_secret = Some(header.id);
            }
            if ui
                .add(egui::Button::new("🗑").frame(false))
                .on_hover_text("Remove header")
                .clicked()
            {
                remove = Some(header.id);
            }
        });
        if let Some(error) = error {
            ui.label(
                RichText::new(webhook_policy::header_message(error))
                    .small()
                    .color(theme::DOWN),
            );
        }
    }
    if let Some(id) = insert_secret
        && let Some(header) = state.headers.iter_mut().find(|header| header.id == id)
    {
        header.value.push_str(SECRET_TOKEN);
    }
    if let Some(id) = remove {
        state.remove_header(id);
    }
    let can_add = state.headers.len() < MAXIMUM_HEADERS;
    if ui
        .add_enabled(can_add, egui::Button::new("＋ Add header").small())
        .on_disabled_hover_text(format!("At most {MAXIMUM_HEADERS} headers"))
        .clicked()
    {
        state.add_header();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> WebhookEditorState {
        let mut state = WebhookEditorState::new_blank();
        state.name = "  Trading Bot ".into();
        state.url = " https://example.com/hook ".into();
        state
    }

    #[test]
    fn a_blank_sheet_cannot_be_saved_and_a_name_plus_url_can() {
        let mut state = WebhookEditorState::new_blank();
        assert!(state.is_new());
        assert!(!state.can_save());
        state.name = "Bot".into();
        assert!(!state.can_save(), "no address yet");
        state.url = "https://example.com/hook".into();
        assert!(state.can_save());
        state.name = "   ".into();
        assert!(!state.can_save(), "a blank name is no name");
    }

    #[test]
    fn the_result_is_trimmed_and_drops_blank_header_rows() {
        let mut state = filled();
        state.description = "  my bridge ".into();
        state.headers = vec![
            WebhookHeader::new("X-Trace", "abc"),
            WebhookHeader::new("", ""),
            WebhookHeader::new("  ", " "),
        ];
        let result = state.result().unwrap();
        assert_eq!(result.id, None);
        assert_eq!(result.name, "Trading Bot");
        assert_eq!(result.description, "my bridge");
        assert_eq!(result.url, "https://example.com/hook");
        assert_eq!(result.headers.len(), 1);
        assert!(result.is_enabled);
        assert_eq!(result.method, WebhookHttpMethod::Post);
    }

    #[test]
    fn address_problems_use_the_editors_wording() {
        let mut state = filled();
        state.url = "ftp://example.com".into();
        assert_eq!(
            state.url_problem(),
            Some("Use an http:// or https:// address.")
        );
        assert!(!state.url_is_valid());
        state.url = "https://{{secret}}.example.com".into();
        assert!(state.url_problem().unwrap().contains("host"));
        state.url = "".into();
        assert_eq!(state.url_problem(), None, "empty is not an error yet");
        assert!(!state.url_is_valid());
    }

    #[test]
    fn a_half_typed_address_stays_quiet_until_the_field_loses_focus() {
        let mut state = filled();
        state.url = "htt".into();
        state.url_has_focus = true;
        assert_eq!(state.visible_url_problem(), None);
        state.url_has_focus = false;
        assert!(state.visible_url_problem().is_some());
    }

    #[test]
    fn using_the_secret_requires_a_value_and_a_value_must_be_valid() {
        let mut state = filled();
        state.url = "https://example.com/hook?token={{secret}}".into();
        assert!(state.uses_secret());
        assert_eq!(state.secret_status(), SecretStatus::MissingValue);
        assert!(!state.can_save());
        state.secret = "abc".into();
        assert_eq!(
            state.secret_status(),
            SecretStatus::Used("Used in the URL".into())
        );
        assert!(state.can_save());
        state.secret = "a\nb".into();
        assert_eq!(
            state.secret_status(),
            SecretStatus::Problem(WebhookSecretError::ControlCharacters)
        );
        assert!(!state.can_save());
    }

    #[test]
    fn an_unused_secret_is_allowed_but_flagged() {
        let mut state = filled();
        assert_eq!(state.secret_status(), SecretStatus::Idle);
        state.secret = "abc".into();
        assert_eq!(state.secret_status(), SecretStatus::NotUsedYet);
        assert!(state.can_save());
    }

    #[test]
    fn the_status_names_every_place_the_secret_is_used() {
        let mut state = filled();
        state.secret = "abc".into();
        state.apply_placement(&SecretPlacement::UrlParameter("token".into()));
        state.apply_placement(&SecretPlacement::BearerToken);
        state.apply_placement(&SecretPlacement::ApiKey);
        assert_eq!(
            state.secret_status(),
            SecretStatus::Used("Used in the URL and 2 headers".into())
        );
        assert!(
            state.headers_expanded,
            "a header placement opens the headers"
        );
        assert!(state.url.contains("?token={{secret}}"));
    }

    #[test]
    fn use_secret_shortcuts_do_not_open_headers_for_a_url_choice() {
        let mut state = filled();
        state.apply_placement(&SecretPlacement::UrlParameter("key".into()));
        assert!(!state.headers_expanded);
        assert!(state.headers.is_empty());
    }

    #[test]
    fn header_errors_block_saving() {
        let mut state = filled();
        state.headers = vec![
            WebhookHeader::new("Content-Type", "x"),
            WebhookHeader::new("X-Fine", "y"),
        ];
        assert_eq!(state.header_errors().len(), 1);
        assert!(!state.can_save());
        state.headers.remove(0);
        assert!(state.can_save());
        // A header whose value uses the secret counts as using it.
        state
            .headers
            .push(WebhookHeader::new("X-Key", "{{secret}}"));
        assert!(!state.can_save(), "no secret value yet");
    }

    #[test]
    fn header_rows_are_capped_and_removable() {
        let mut state = filled();
        for _ in 0..15 {
            state.add_header();
        }
        assert_eq!(state.headers.len(), MAXIMUM_HEADERS);
        let id = state.headers[3].id;
        state.remove_header(id);
        assert_eq!(state.headers.len(), MAXIMUM_HEADERS - 1);
        assert!(state.headers.iter().all(|header| header.id != id));
    }

    #[test]
    fn a_plain_credential_can_be_moved_into_the_secret() {
        let mut state = filled();
        state.headers = vec![WebhookHeader::new("Authorization", "Bearer abc123")];
        let credential = state.plain_credential().unwrap();
        assert_eq!(credential.name, "Authorization");
        state.move_to_secret(credential.id);
        assert_eq!(state.secret, "abc123");
        assert_eq!(state.headers[0].value, "Bearer {{secret}}");
        assert!(
            state.plain_credential().is_none(),
            "nothing left to nudge about"
        );
        assert!(state.can_save());
        // Once a secret exists the nudge stays away.
        state.headers.push(WebhookHeader::new("X-Token", "plain"));
        assert!(state.plain_credential().is_none());
    }

    #[test]
    fn editing_starts_from_the_saved_endpoint_and_keeps_its_id() {
        let mut endpoint = WebhookEndpoint::new("Bot", "https://example.com/a?t={{secret}}", 0);
        endpoint
            .headers
            .push(WebhookHeader::new("X-Key", "{{secret}}"));
        endpoint.method = WebhookHttpMethod::Put;
        endpoint.is_enabled = false;
        let state = WebhookEditorState::editing(&endpoint, Some("tok".into()));
        assert!(!state.is_new());
        assert!(
            state.headers_expanded,
            "a webhook that already has headers opens them"
        );
        assert_eq!(state.secret, "tok");
        let result = state.result().unwrap();
        assert_eq!(result.id, Some(endpoint.id));
        assert_eq!(result.method, WebhookHttpMethod::Put);
        assert!(!result.is_enabled);
        // No saved secret: the field starts empty and the sheet will not save until filled.
        let missing = WebhookEditorState::editing(&endpoint, None);
        assert!(!missing.can_save());
    }
}
