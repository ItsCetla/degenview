//! Settings ▸ Alpaca and Settings ▸ CoinMarketCap: the API keys that unlock US stock data and
//! raise the market-index rate limits. Pages over the shared key stores; saving, removing and the
//! CoinMarketCap connection test happen here, since the stores are process-wide, but every rule
//! (what may be saved, what the status says) is a plain function tested against in-memory stores.

// Transient: reachable once the Settings window is shown.
#![allow(dead_code)]

use crate::service::keychain_api_keys::{
    AlpacaCredentials, AlpacaCredentialsStore, CoinMarketCapKeyStore,
};
use crate::ui::alert_center::Tone;
use crate::ui::settings_window::{card, page_header, status_badge};
use crate::ui::theme;
use eframe::egui::{self, Color32, RichText};
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Debug, PartialEq)]
pub struct StatusMessage {
    pub text: String,
    pub is_error: bool,
}

impl StatusMessage {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

#[derive(Default)]
pub struct AlpacaForm {
    pub key_id: String,
    pub secret_key: String,
    /// Whether the saved credentials have been read into the fields yet (reading may prompt).
    loaded: bool,
    pub is_saved: bool,
    pub status: Option<StatusMessage>,
}

#[derive(Default)]
pub struct CmcForm {
    pub api_key: String,
    loaded: bool,
    pub configured: bool,
    pub status: Option<StatusMessage>,
    test: Option<Receiver<Result<(), String>>>,
}

#[derive(Default)]
pub struct ApiKeysState {
    pub alpaca: AlpacaForm,
    pub cmc: CmcForm,
}

// MARK: Alpaca rules

impl AlpacaForm {
    /// Fills the fields from the saved credentials, once.
    pub fn load(&mut self, store: &AlpacaCredentialsStore) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.is_saved = store.is_configured();
        if self.is_saved {
            let saved = store.credentials();
            self.key_id = saved.key_id;
            self.secret_key = saved.secret_key;
        }
    }

    pub fn can_save(&self) -> bool {
        !self.key_id.trim().is_empty() && !self.secret_key.trim().is_empty()
    }

    pub fn badge(&self) -> (&'static str, Tone) {
        if self.is_saved {
            ("Keys saved", Tone::Good)
        } else {
            ("Not configured", Tone::Neutral)
        }
    }

    pub fn save(&mut self, store: &AlpacaCredentialsStore) {
        let credentials = AlpacaCredentials {
            key_id: self.key_id.clone(),
            secret_key: self.secret_key.clone(),
        };
        self.status = Some(match store.save(&credentials) {
            Ok(()) => {
                self.is_saved = true;
                StatusMessage::ok("Saved securely in Keychain.")
            }
            Err(error) => StatusMessage::error(format!("Could not save: {}", error.0)),
        });
    }

    pub fn remove(&mut self, store: &AlpacaCredentialsStore) {
        self.status = Some(match store.save(&AlpacaCredentials::default()) {
            Ok(()) => {
                self.key_id.clear();
                self.secret_key.clear();
                self.is_saved = false;
                StatusMessage::ok("Keys removed. Stock charts need keys to load.")
            }
            Err(error) => StatusMessage::error(format!("Could not remove the keys: {}", error.0)),
        });
    }
}

// MARK: CoinMarketCap rules

impl CmcForm {
    pub fn load(&mut self, store: &CoinMarketCapKeyStore) {
        if !self.loaded {
            self.loaded = true;
            self.configured = store.is_configured();
        }
    }

    pub fn can_save(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub fn is_testing(&self) -> bool {
        self.test.is_some()
    }

    pub fn badge(&self) -> (&'static str, Tone) {
        if self.configured {
            ("API key configured", Tone::Good)
        } else {
            ("Public API mode", Tone::Neutral)
        }
    }

    pub fn field_title(&self) -> &'static str {
        if self.configured {
            "Replace API Key"
        } else {
            "API Key"
        }
    }

    pub fn field_hint(&self) -> &'static str {
        if self.configured {
            "A key is saved in your Keychain. Enter a new one to replace it."
        } else {
            "Stored in your Mac's Keychain, never in a file."
        }
    }

    pub fn save(&mut self, store: &CoinMarketCapKeyStore) {
        self.status = Some(match store.save(&self.api_key) {
            Ok(()) => {
                self.configured = true;
                self.api_key.clear();
                StatusMessage::ok("Saved securely in Keychain.")
            }
            Err(error) => StatusMessage::error(format!("Could not save: {}", error.0)),
        });
    }

    pub fn remove(&mut self, store: &CoinMarketCapKeyStore) {
        self.status = Some(match store.remove() {
            Ok(()) => {
                self.api_key.clear();
                self.configured = false;
                StatusMessage::ok("Key removed. Public API mode is active.")
            }
            Err(error) => StatusMessage::error(format!("Could not remove the key: {}", error.0)),
        });
    }

    /// Runs `test` off the UI thread; `poll_test` picks up the answer.
    pub fn start_test(&mut self, test: impl FnOnce() -> Result<(), String> + Send + 'static) {
        if self.test.is_some() {
            return;
        }
        self.status = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.test = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(test());
        });
    }

    /// Takes the connection test's answer when it has arrived.
    pub fn poll_test(&mut self) {
        let Some(receiver) = &self.test else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("The connection test stopped unexpectedly.".to_owned())
            }
        };
        self.test = None;
        self.status = Some(match outcome {
            Ok(()) => StatusMessage::ok("Connection successful."),
            Err(message) => StatusMessage::error(message),
        });
    }
}

// MARK: Views

fn field(
    ui: &mut egui::Ui,
    title: &str,
    hint: Option<&str>,
    prompt: &str,
    text: &mut String,
    secret: bool,
) {
    ui.label(RichText::new(title).small().strong().weak());
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(prompt)
            .password(secret)
            .desired_width(f32::INFINITY),
    );
    if let Some(hint) = hint {
        ui.label(RichText::new(hint).small().weak());
    }
}

fn status_line(ui: &mut egui::Ui, status: &Option<StatusMessage>) {
    if let Some(status) = status {
        let color = if status.is_error {
            theme::DOWN
        } else {
            theme::UP
        };
        ui.label(RichText::new(&status.text).small().color(color));
    }
}

fn step(ui: &mut egui::Ui, number: usize, text: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(number.to_string())
                .small()
                .strong()
                .color(theme::ACCENT),
        );
        ui.label(RichText::new(text).weak());
    });
}

pub fn show_alpaca(ui: &mut egui::Ui, state: &mut ApiKeysState, store: &AlpacaCredentialsStore) {
    let form = &mut state.alpaca;
    form.load(store);
    page_header(
        ui,
        "Alpaca",
        "Connect Alpaca to chart US stocks and ETFs in DegenView.",
    );
    let (text, tone) = form.badge();
    status_badge(ui, text, tone);
    ui.add_space(12.0);

    ui.label(RichText::new("API credentials").strong().size(15.0));
    ui.add_space(4.0);
    card(ui, |ui| {
        field(ui, "API Key ID", None, "PK…", &mut form.key_id, false);
        ui.add_space(8.0);
        field(
            ui,
            "Secret Key",
            Some("Stored in your Mac's Keychain, never in a file."),
            "",
            &mut form.secret_key,
            true,
        );
    });
    ui.add_space(14.0);
    ui.label(RichText::new("Getting your keys").strong().size(15.0));
    ui.add_space(4.0);
    card(ui, |ui| {
        step(ui, 1, "Create a free Alpaca account.");
        step(
            ui,
            2,
            "Open the API Keys section of the dashboard and generate a key.",
        );
        step(
            ui,
            3,
            "Copy both values here — Alpaca only shows the secret once.",
        );
        ui.hyperlink_to("Open the Alpaca dashboard ↗", "https://app.alpaca.markets/");
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        status_line(ui, &form.status);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = egui::Button::new(RichText::new("Save").strong().color(Color32::WHITE))
                .fill(theme::ACCENT);
            if ui.add_enabled(form.can_save(), save).clicked() {
                form.save(store);
            }
            if form.is_saved
                && ui
                    .button(RichText::new("Remove Keys").color(theme::DOWN))
                    .clicked()
            {
                form.remove(store);
            }
        });
    });
}

pub fn show_coinmarketcap(
    ui: &mut egui::Ui,
    state: &mut ApiKeysState,
    store: &CoinMarketCapKeyStore,
) {
    let form = &mut state.cmc;
    form.load(store);
    form.poll_test();
    if form.is_testing() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }
    page_header(
        ui,
        "CoinMarketCap",
        "Market sentiment charts, with optional higher API rate limits.",
    );
    let (text, tone) = form.badge();
    status_badge(ui, text, tone);
    ui.add_space(12.0);

    ui.label(RichText::new("API access").strong().size(15.0));
    ui.add_space(4.0);
    let (title, hint) = (form.field_title(), form.field_hint());
    card(ui, |ui| {
        field(ui, title, Some(hint), "", &mut form.api_key, true);
    });
    ui.add_space(12.0);
    card(ui, |ui| {
        ui.label(
            RichText::new("ⓘ A key is optional")
                .strong()
                .color(Color32::from_rgb(10, 132, 255)),
        );
        ui.label(
            RichText::new(
                "CoinMarketCap works without an API key. Adding one gives DegenView higher API rate limits.",
            )
            .small()
            .weak(),
        );
    });
    ui.add_space(8.0);
    ui.hyperlink_to("Get an API key ↗", "https://pro.coinmarketcap.com/signup/");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        status_line(ui, &form.status);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save = egui::Button::new(RichText::new("Save").strong().color(Color32::WHITE))
                .fill(theme::ACCENT);
            if ui.add_enabled(form.can_save(), save).clicked() {
                form.save(store);
            }
            let testing = form.is_testing();
            let label = if testing {
                "Testing…"
            } else {
                "Test Connection"
            };
            if ui.add_enabled(!testing, egui::Button::new(label)).clicked() {
                form.start_test(crate::service::cmc::test_connection);
            }
            if form.configured
                && ui
                    .button(RichText::new("Remove Key").color(theme::DOWN))
                    .clicked()
            {
                form.remove(store);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::keychain_api_keys::MemoryBackend;
    use std::time::{Duration, Instant};

    fn alpaca_store() -> AlpacaCredentialsStore {
        AlpacaCredentialsStore::with_backend(Box::new(MemoryBackend::default()))
    }

    fn cmc_store() -> CoinMarketCapKeyStore {
        CoinMarketCapKeyStore::with_backend(Box::new(MemoryBackend::default()))
    }

    #[test]
    fn alpaca_saves_only_with_both_fields_and_reports_it() {
        let store = alpaca_store();
        let mut form = AlpacaForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Not configured", Tone::Neutral));
        assert!(!form.can_save());
        form.key_id = " PK1 ".into();
        assert!(!form.can_save(), "the secret is missing too");
        form.secret_key = "S1".into();
        assert!(form.can_save());
        form.save(&store);
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Saved securely in Keychain."))
        );
        assert_eq!(form.badge(), ("Keys saved", Tone::Good));
        assert_eq!(store.credentials().key_id, "PK1", "saved trimmed");
    }

    #[test]
    fn alpaca_loads_saved_keys_into_the_fields_once_and_can_remove_them() {
        let store = alpaca_store();
        store
            .save(&AlpacaCredentials {
                key_id: "PK9".into(),
                secret_key: "S9".into(),
            })
            .unwrap();
        let mut form = AlpacaForm::default();
        form.load(&store);
        assert!(form.is_saved);
        assert_eq!(
            (form.key_id.as_str(), form.secret_key.as_str()),
            ("PK9", "S9")
        );
        form.key_id = "typed".into();
        form.load(&store);
        assert_eq!(
            form.key_id, "typed",
            "a second load does not overwrite edits"
        );
        form.remove(&store);
        assert!(!form.is_saved && form.key_id.is_empty() && form.secret_key.is_empty());
        assert_eq!(
            form.status,
            Some(StatusMessage::ok(
                "Keys removed. Stock charts need keys to load."
            ))
        );
        assert!(!store.is_configured());
    }

    #[test]
    fn coinmarketcap_saves_clears_the_field_and_flips_the_badge() {
        let store = cmc_store();
        let mut form = CmcForm::default();
        form.load(&store);
        assert_eq!(form.badge(), ("Public API mode", Tone::Neutral));
        assert_eq!(form.field_title(), "API Key");
        assert!(!form.can_save());
        form.api_key = "  abc ".into();
        assert!(form.can_save());
        form.save(&store);
        assert!(
            form.api_key.is_empty(),
            "the secret does not linger in the field"
        );
        assert_eq!(form.badge(), ("API key configured", Tone::Good));
        assert_eq!(form.field_title(), "Replace API Key");
        assert!(form.field_hint().contains("saved in your Keychain"));
        assert_eq!(store.api_key().as_deref(), Some("abc"));
        form.remove(&store);
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Key removed. Public API mode is active."))
        );
        assert!(!form.configured && store.api_key().is_none());
    }

    fn wait_for_test(form: &mut CmcForm) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while form.is_testing() && Instant::now() < deadline {
            form.poll_test();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_connection_test_runs_in_the_background_and_reports_either_way() {
        let mut form = CmcForm {
            status: Some(StatusMessage::error("old")),
            ..CmcForm::default()
        };
        form.start_test(|| Ok(()));
        assert!(form.is_testing());
        assert_eq!(form.status, None, "a new test clears the old message");
        form.start_test(|| Err("ignored: one test at a time".into()));
        wait_for_test(&mut form);
        assert!(!form.is_testing());
        assert_eq!(
            form.status,
            Some(StatusMessage::ok("Connection successful."))
        );

        form.start_test(|| Err("Invalid API key".into()));
        wait_for_test(&mut form);
        assert_eq!(form.status, Some(StatusMessage::error("Invalid API key")));
    }
}
