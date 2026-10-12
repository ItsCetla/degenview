//! Settings ▸ MCP Server: the on/off switch, listener status, the global permission cap, and
//! the credentials issued to MCP clients with what each may do.
//!
//! A pure view over [`McpSettingsInput`]; it returns [`McpAction`]s and never touches the
//! server, the store or the Keychain itself.

use crate::mcp::state::McpStatus;
use crate::model::mcp::{
    MAX_CLIENT_NAME_LEN, McpAction, McpClient, McpSettings, Permission, PermissionSet,
};
use crate::ui::alert_center::{Tone, relative_time};
use crate::ui::icons::Icon;
use crate::ui::settings_window::{card, divider, row, section, status_badge, status_message};
use crate::ui::{kit, settings_card, theme};
use eframe::egui::{self, Id};

pub struct McpSettingsInput<'a> {
    pub status: &'a McpStatus,
    pub settings: &'a McpSettings,
    pub clients: &'a [McpClient],
    pub global: &'a PermissionSet,
    /// `(client name, token)` of a credential created this session and not yet dismissed.
    pub fresh_token: Option<(&'a str, &'a str)>,
    pub load_error: Option<&'a str>,
    pub notice: Option<&'a str>,
    pub now_ms: i64,
}

#[derive(Default)]
pub struct McpSettingsState {
    new_client_name: String,
    port_text: String,
    port_synced: Option<u16>,
    confirm_revoke: Option<String>,
}

/// The `mcpServers` entry a client's config file takes. The token is shown only while it is
/// still known; afterwards a placeholder stands in for it.
pub fn connection_json(endpoint: &str, token: Option<&str>) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            "degenview": {
                "type": "http",
                "url": endpoint,
                "headers": {
                    "Authorization": format!("Bearer {}", token.unwrap_or("<YOUR_TOKEN>")),
                },
            }
        }
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

fn status_tone(status: &McpStatus) -> Tone {
    match status {
        McpStatus::Running { .. } => Tone::Good,
        McpStatus::Starting | McpStatus::Stopping => Tone::Warning,
        McpStatus::Failed(_) => Tone::Bad,
        McpStatus::Disabled => Tone::Neutral,
    }
}

fn secondary(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(kit::text(
        text,
        kit::size::BODY,
        kit::REGULAR,
        theme::text_secondary(),
    ));
}

fn grouped(group: &'static str) -> impl Iterator<Item = Permission> {
    Permission::ALL
        .into_iter()
        .filter(move |permission| permission.group() == group)
}

fn group_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(kit::text(
        title,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
    ));
}

pub fn body(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    if let Some(error) = input.load_error {
        status_message(ui, error, true);
        ui.add_space(12.0);
    }
    server_section(ui, state, input, actions);
    ui.add_space(22.0);
    security_section(ui);
    ui.add_space(22.0);
    permissions_section(ui, input, actions);
    ui.add_space(22.0);
    clients_section(ui, state, input, actions);
}

fn server_section(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(ui, "MCP Server", None, |ui| {
        let locked = input.load_error.is_some();
        settings_card::card(
            ui,
            Icon::LockFill,
            "Enable MCP Server",
            Some(
                "Lets AI assistants and automation on this Mac inspect DegenView, within the permissions below. Off until you turn it on.",
            ),
            |ui| {
                let mut enabled = input.settings.enabled;
                if settings_card::switch_enabled(ui, &mut enabled, !locked) {
                    actions.push(McpAction::SetEnabled(enabled));
                }
            },
        );
        ui.add_space(10.0);
        card(ui, |ui| {
            row(ui, "Status", |ui| {
                status_badge(ui, input.status.label(), status_tone(input.status));
            });
            if let McpStatus::Failed(reason) = input.status {
                ui.add(
                    egui::Label::new(kit::text(
                        reason,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        kit::RED,
                    ))
                    .wrap(),
                );
            }
            divider(ui);
            row(ui, "Address", |ui| secondary(ui, "127.0.0.1"));
            divider(ui);
            row(ui, "Port", |ui| {
                port_field(ui, state, input.settings.port, actions)
            });
            divider(ui);
            row(ui, "Endpoint", |ui| {
                let endpoint = input.settings.endpoint();
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(endpoint.clone());
                }
                ui.label(egui::RichText::new(endpoint).monospace().size(12.0));
            });
        });
        if let Some(notice) = input.notice {
            ui.add_space(8.0);
            status_message(ui, notice, true);
        }
    });
}

fn port_field(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    port: u16,
    actions: &mut Vec<McpAction>,
) {
    let id = Id::new("mcp-port");
    let editing = ui.memory(|memory| memory.has_focus(id));
    if !editing && state.port_synced != Some(port) {
        state.port_text = port.to_string();
        state.port_synced = Some(port);
    }
    let parsed = state.port_text.trim().parse::<u16>().ok();
    let changed = parsed.is_some_and(|value| value != port);
    if kit::sheet_button(ui, "Apply", false, changed).clicked()
        && let Some(value) = parsed
    {
        actions.push(McpAction::SetPort(value));
    }
    let response = kit::field_frame(editing)
        .show(ui, |ui| {
            kit::plain_field(
                ui,
                id,
                &mut state.port_text,
                "8765",
                kit::size::BODY,
                kit::REGULAR,
                56.0,
            )
        })
        .inner;
    if response.lost_focus()
        && ui.input(|input| input.key_pressed(egui::Key::Enter))
        && let Some(value) = parsed
        && value != port
    {
        actions.push(McpAction::SetPort(value));
    }
}

fn security_section(ui: &mut egui::Ui) {
    section(ui, "Security", None, |ui| {
        card(ui, |ui| {
            row(ui, "Authentication", |ui| secondary(ui, "Always required"));
            divider(ui);
            row(ui, "Remote connections", |ui| {
                secondary(ui, "Not supported")
            });
            divider(ui);
            row(ui, "Browser pages", |ui| secondary(ui, "Always refused"));
        });
    });
}

fn permissions_section(ui: &mut egui::Ui, input: &McpSettingsInput, actions: &mut Vec<McpAction>) {
    section(
        ui,
        "Maximum permissions",
        Some("The most any client can do. A client's own grants below can only narrow this."),
        |ui| {
            card(ui, |ui| {
                for (index, group) in Permission::GROUPS.into_iter().enumerate() {
                    if index > 0 {
                        divider(ui);
                    }
                    group_heading(ui, group);
                    for permission in grouped(group) {
                        row(ui, permission.title(), |ui| {
                            let mut allowed = input.global.contains(permission);
                            if settings_card::switch(ui, &mut allowed) {
                                actions.push(McpAction::SetGlobalPermission {
                                    permission,
                                    allowed,
                                });
                            }
                            if permission.is_mutating() {
                                secondary(ui, "changes DegenView");
                            }
                        });
                    }
                }
            });
        },
    );
}

fn clients_section(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    section(
        ui,
        "Clients",
        Some("Each MCP client gets its own credential. Revoking one cuts it off immediately."),
        |ui| {
            if let Some((name, token)) = input.fresh_token {
                token_card(ui, name, token, input, actions);
                ui.add_space(10.0);
            }
            if input.clients.is_empty() {
                card(ui, |ui| {
                    secondary(ui, "No clients yet. Generate a credential to connect one.")
                });
            } else {
                for (index, client) in input.clients.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(10.0);
                    }
                    client_card(ui, state, client, input, actions);
                }
            }
            ui.add_space(12.0);
            generate_row(ui, state, input, actions);
        },
    );
}

fn token_card(
    ui: &mut egui::Ui,
    name: &str,
    token: &str,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    card(ui, |ui| {
        ui.label(kit::text(
            format!("Credential for {name}"),
            kit::size::BODY,
            kit::SEMIBOLD,
            theme::text_primary(),
        ));
        secondary(
            ui,
            "Copy it now. It is not stored and cannot be shown again.",
        );
        ui.label(egui::RichText::new(token).monospace().size(12.0));
        ui.horizontal(|ui| {
            if kit::sheet_button(ui, "Copy Credential", true, true).clicked() {
                ui.ctx().copy_text(token.to_owned());
            }
            if kit::sheet_button(ui, "Copy Connection Details", false, true).clicked() {
                ui.ctx()
                    .copy_text(connection_json(&input.settings.endpoint(), Some(token)));
            }
            if kit::sheet_button(ui, "Done", false, true).clicked() {
                actions.push(McpAction::DismissToken);
            }
        });
    });
}

fn client_card(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    client: &McpClient,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    card(ui, |ui| {
        row(ui, &client.name, |ui| {
            if state.confirm_revoke.as_deref() == Some(client.id.as_str()) {
                if kit::destructive_sheet_button(ui, Icon::Xmark, "Revoke").clicked() {
                    actions.push(McpAction::RevokeClient(client.id.clone()));
                    state.confirm_revoke = None;
                }
                if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                    state.confirm_revoke = None;
                }
            } else if kit::sheet_button(ui, "Revoke…", false, true).clicked() {
                state.confirm_revoke = Some(client.id.clone());
            }
        });
        secondary(
            ui,
            format!(
                "Created {} · {}",
                relative_time(input.now_ms, client.created_at_ms),
                client.last_used_ms.map_or_else(
                    || "never used".to_owned(),
                    |used| format!("last used {}", relative_time(input.now_ms, used))
                )
            ),
        );
        let effective = client.effective(input.global);
        for group in Permission::GROUPS {
            divider(ui);
            group_heading(ui, group);
            for permission in grouped(group) {
                row(ui, permission.title(), |ui| {
                    let mut granted = client.permissions.contains(permission);
                    if settings_card::switch(ui, &mut granted) {
                        actions.push(McpAction::SetClientPermission {
                            client_id: client.id.clone(),
                            permission,
                            allowed: granted,
                        });
                    }
                    if client.permissions.contains(permission) && !effective.contains(permission) {
                        secondary(ui, "blocked by maximum");
                    }
                });
            }
        }
    });
}

fn generate_row(
    ui: &mut egui::Ui,
    state: &mut McpSettingsState,
    input: &McpSettingsInput,
    actions: &mut Vec<McpAction>,
) {
    let id = Id::new("mcp-client-name");
    ui.horizontal(|ui| {
        let focused = ui.memory(|memory| memory.has_focus(id));
        let response = kit::field_frame(focused)
            .show(ui, |ui| {
                kit::plain_field(
                    ui,
                    id,
                    &mut state.new_client_name,
                    "Client name, e.g. Claude Desktop",
                    kit::size::BODY,
                    kit::REGULAR,
                    240.0,
                )
            })
            .inner;
        if state.new_client_name.chars().count() > MAX_CLIENT_NAME_LEN {
            state.new_client_name = state
                .new_client_name
                .chars()
                .take(MAX_CLIENT_NAME_LEN)
                .collect();
        }
        let ready = !state.new_client_name.trim().is_empty() && input.load_error.is_none();
        let submitted =
            response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if (kit::sheet_button(ui, "Generate Credential", true, ready).clicked() || submitted)
            && ready
        {
            actions.push(McpAction::CreateClient(std::mem::take(
                &mut state.new_client_name,
            )));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_json_names_the_endpoint_and_never_invents_a_token() {
        let with_token = connection_json("http://127.0.0.1:8765/mcp", Some("dvmcp_abc"));
        let parsed: serde_json::Value = serde_json::from_str(&with_token).unwrap();
        let server = &parsed["mcpServers"]["degenview"];
        assert_eq!(server["url"], "http://127.0.0.1:8765/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer dvmcp_abc");

        let placeholder = connection_json("http://127.0.0.1:8765/mcp", None);
        assert!(placeholder.contains("<YOUR_TOKEN>"));
        assert!(!placeholder.contains("dvmcp_"));
    }

    #[test]
    fn statuses_have_distinct_tones() {
        assert_eq!(status_tone(&McpStatus::Running { port: 1 }), Tone::Good);
        assert_eq!(status_tone(&McpStatus::Failed("x".into())), Tone::Bad);
        assert_eq!(status_tone(&McpStatus::Disabled), Tone::Neutral);
    }
}
