//! Host for the app-level Settings window's Webhooks section: owns the window state, the
//! Keychain secret store and the HTTP transport, and applies the webhook actions the pure
//! `settings_window`/`webhook_settings` views return. Actions that reach outside the webhook
//! store (notifications, login items, alert reference bookkeeping) are handed back to the caller.
#![allow(dead_code)] // transient until the app-level settings window is threaded

use crate::model::alert::{AlertNotificationSettings, AlertRuntimeHealth};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::service::keychain_webhook_secrets::KeychainWebhookSecretStore;
use crate::service::webhook_delivery::{self, Mode, UreqTransport, WebhookSecretStore};
use crate::store::Store;
use crate::ui::settings_window::{self, SettingsAction, SettingsInput, SettingsState};
use crate::ui::webhook_settings::{WebhookSettingsAction, WebhookSettingsInput};
use eframe::egui;

/// Something the settings window asked for that the webhook store cannot do on its own.
pub enum WebhookHostAction {
    UpdateNotifications(AlertNotificationSettings),
    RetryRegistration,
    OpenLoginItems,
    OpenNotificationSettings,
    /// An endpoint was deleted: remove its id from every alert that references it.
    EndpointDeleted(EndpointId),
}

#[derive(Default)]
pub struct WebhookHost {
    state: SettingsState,
    secrets: KeychainWebhookSecretStore,
    transport: UreqTransport,
    open: bool,
}

impl WebhookHost {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Opens Settings on the Alpaca page (the Script Manager's "Open Settings").
    pub fn open_alpaca(&mut self) {
        self.state.section = crate::ui::settings_window::SettingsSection::Alpaca;
        self.open = true;
    }

    /// Opens Settings on the Webhooks page (an alert editor's "Add in Settings").
    pub fn open_webhooks(&mut self) {
        self.state.section = crate::ui::settings_window::SettingsSection::Webhooks;
        self.open = true;
    }

    /// Renders the Settings window and applies webhook actions in place. `reference_count`
    /// answers "how many alerts use this endpoint" for the delete confirmation; the alert host
    /// supplies it (returns 0 until it exists). Returns the out-of-store actions for the caller.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        store: &Store,
        notifications: &AlertNotificationSettings,
        health: &AlertRuntimeHealth,
        now_ms: i64,
        reference_count: &dyn Fn(EndpointId) -> usize,
    ) -> Vec<WebhookHostAction> {
        if !self.open {
            return Vec::new();
        }
        let mut out = Vec::new();
        // The page fills the native Settings window itself: no egui window inside it, which
        // drew a second title bar and let the page be dragged around within its own window.
        let endpoints = store.webhook_endpoints();
        let input = SettingsInput {
            notifications,
            health,
            webhooks: WebhookSettingsInput {
                endpoints: &endpoints,
                has_secret: &|id| self.secrets.has_secret(id),
                secret_for_edit: &|id| self.secrets.secret(id),
                reference_count,
                load_failed: false,
            },
            now_ms,
        };
        for action in settings_window::show(ui, &mut self.state, &input) {
            match action {
                SettingsAction::Webhook(action) => {
                    self.apply_webhook(action, store, &endpoints, now_ms, &mut out);
                }
                SettingsAction::UpdateNotifications(settings) => {
                    out.push(WebhookHostAction::UpdateNotifications(settings));
                }
                SettingsAction::RetryRegistration => {
                    out.push(WebhookHostAction::RetryRegistration);
                }
                SettingsAction::OpenLoginItems => {
                    out.push(WebhookHostAction::OpenLoginItems);
                }
                SettingsAction::OpenNotificationSettings => {
                    out.push(WebhookHostAction::OpenNotificationSettings);
                }
            }
        }
        out
    }

    fn apply_webhook(
        &mut self,
        action: WebhookSettingsAction,
        store: &Store,
        endpoints: &[WebhookEndpoint],
        now_ms: i64,
        out: &mut Vec<WebhookHostAction>,
    ) {
        match action {
            WebhookSettingsAction::Test { endpoint, message } => {
                // TODO: run on a worker thread so a 3 s timeout doesn't block the UI.
                let result = webhook_delivery::deliver(
                    &self.transport,
                    &self.secrets,
                    &message,
                    &endpoint,
                    Mode::Test,
                );
                self.state.webhooks.set_test_result(&result);
            }
            WebhookSettingsAction::Toggle(id) => {
                let mut next = endpoints.to_vec();
                for endpoint in &mut next {
                    if endpoint.id == id {
                        endpoint.is_enabled = !endpoint.is_enabled;
                    }
                }
                let _ = store.replace_webhook_endpoints(&next);
            }
            WebhookSettingsAction::Delete(id) => {
                let mut next = endpoints.to_vec();
                next.retain(|endpoint| endpoint.id != id);
                let _ = store.replace_webhook_endpoints(&next);
                self.secrets.remove(id);
                out.push(WebhookHostAction::EndpointDeleted(id));
            }
            WebhookSettingsAction::Save(result) => self.save(result, store, endpoints, now_ms),
        }
    }

    /// Creates (`id == None`) or updates an endpoint, then writes the secret. On a row-write
    /// failure the sheet stays open; on a secret-write failure the row write is rolled back
    /// (Swift parity).
    fn save(
        &mut self,
        result: crate::ui::webhook_editor::WebhookEditorResult,
        store: &Store,
        endpoints: &[WebhookEndpoint],
        now_ms: i64,
    ) {
        let original = endpoints.to_vec();
        let mut next = endpoints.to_vec();
        let target_id = match result.id {
            Some(id) => {
                for endpoint in &mut next {
                    if endpoint.id == id {
                        endpoint.name = result.name.clone();
                        endpoint.description = result.description.clone();
                        endpoint.url = result.url.clone();
                        endpoint.method = result.method;
                        endpoint.headers = result.headers.clone();
                        endpoint.is_enabled = result.is_enabled;
                        endpoint.updated_at_ms = now_ms;
                    }
                }
                id
            }
            None => {
                let mut endpoint =
                    WebhookEndpoint::new(result.name.clone(), result.url.clone(), now_ms);
                endpoint.description = result.description.clone();
                endpoint.method = result.method;
                endpoint.headers = result.headers.clone();
                endpoint.is_enabled = result.is_enabled;
                let id = endpoint.id;
                next.push(endpoint);
                id
            }
        };

        if let Err(error) = store.replace_webhook_endpoints(&next) {
            self.state.webhooks.save_failed(error.to_string());
            return;
        }
        let secret_written = if result.secret.is_empty() {
            self.secrets.remove(target_id);
            Ok(())
        } else {
            self.secrets.set_secret(target_id, &result.secret)
        };
        match secret_written {
            Ok(()) => self.state.webhooks.save_succeeded(),
            Err(error) => {
                let _ = store.replace_webhook_endpoints(&original);
                self.state.webhooks.save_failed(error.0);
            }
        }
    }
}
