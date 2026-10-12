//! The stateful bridge between the MCP server, SQLite, the Settings page and the workspaces.
//!
//! The host owns the listener's lifecycle (it exists only while the user has the server
//! enabled), persists settings and credentials through `store::mcp`, keeps the server's shared
//! permission snapshot equal to what is on disk, and hands queued tool requests to the app.
//! Persist first, then publish: a write that fails changes nothing the server can see.

use crate::mcp::McpServer;
use crate::mcp::auth::{generate_token, verifier_hex};
use crate::mcp::bridge::{self, BridgeClient, BridgeInbox, Request};
use crate::mcp::state::{McpShared, McpStatus};
use crate::mcp::types::{
    AddChartResult, ChartInfo, ChartsResult, IndicatorsInfo, PineInstanceInfo, WorkspaceInfo,
};
use crate::model::mcp::{
    MAX_CLIENTS, McpAction, McpClient, McpSettings, Permission, PermissionSet, clean_client_name,
    is_valid_port,
};
use crate::model::watchlist::{new_id, now_ms};
use crate::model::{DataSource, InstrumentId};
use crate::store::SharedStore;
use crate::workspace::{ChartSummary, ChartWorkspace};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Recorded last-use times are written at most this often.
const USAGE_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// A bearer token just issued. Held only until the user dismisses it; never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshToken {
    pub client_id: String,
    pub client_name: String,
    pub token: String,
}

pub struct McpHost {
    store: SharedStore,
    shared: Arc<McpShared>,
    bridge: BridgeClient,
    inbox: BridgeInbox,
    server: Option<McpServer>,
    settings: McpSettings,
    /// Set when persisted MCP state could not be read: the server stays off and nothing is
    /// written, so unreadable credentials or permissions are never replaced by defaults.
    load_error: Option<String>,
    fresh_token: Option<FreshToken>,
    notice: Option<String>,
    last_usage_save: Instant,
    /// Actions from the Settings page, applied by [`McpHost::tick`]. The page runs while the
    /// store is locked, so it can only queue.
    pending: Vec<McpAction>,
}

impl McpHost {
    /// Loads persisted state and, only if the user previously enabled the server, starts it.
    /// `wake` runs a UI frame when a request is queued.
    pub fn load(store: SharedStore, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let shared = McpShared::new();
        let (bridge, inbox) = bridge::channel(shared.clone(), wake);
        let mut host = Self {
            store,
            shared,
            bridge,
            inbox,
            server: None,
            settings: McpSettings::default(),
            load_error: None,
            fresh_token: None,
            notice: None,
            last_usage_save: Instant::now(),
            pending: Vec::new(),
        };
        host.reload();
        if host.load_error.is_none() && host.settings.enabled {
            host.start_server();
        }
        host
    }

    fn reload(&mut self) {
        let loaded = self
            .store
            .lock()
            .map_err(|_| "store unavailable".to_owned())
            .and_then(|store| {
                let settings = store.mcp_settings().map_err(|error| error.to_string())?;
                let clients = store.mcp_clients().map_err(|error| error.to_string())?;
                let global = store
                    .mcp_global_permissions()
                    .map_err(|error| error.to_string())?;
                Ok((settings, clients, global))
            });
        match loaded {
            Ok((settings, clients, global)) => {
                self.settings = settings;
                self.shared.replace(clients, global);
                self.load_error = None;
            }
            Err(error) => {
                // Fail closed: no credentials are loaded, so nothing can authenticate.
                self.shared.replace(Vec::new(), PermissionSet::none());
                self.settings.enabled = false;
                self.load_error = Some(format!(
                    "Saved MCP settings could not be read, so the server stays off ({error})"
                ));
            }
        }
    }

    pub fn shared(&self) -> Arc<McpShared> {
        self.shared.clone()
    }

    pub fn status(&self) -> McpStatus {
        self.shared.status()
    }

    pub fn settings(&self) -> &McpSettings {
        &self.settings
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn fresh_token(&self) -> Option<&FreshToken> {
        self.fresh_token.as_ref()
    }

    pub fn clients(&self) -> Vec<McpClient> {
        self.shared.clients()
    }

    pub fn global_permissions(&self) -> PermissionSet {
        self.shared.global()
    }

    /// Up to `max` queued tool requests for the app thread to run.
    pub fn take_requests(&self, max: usize) -> Vec<Request> {
        self.inbox.take(max)
    }

    pub fn queue(&mut self, action: McpAction) {
        self.pending.push(action);
    }

    /// Applies queued Settings actions, then persists "last used" times (at most every 30
    /// seconds). Called once per frame by the app, with no other lock held.
    pub fn tick(&mut self) {
        for action in std::mem::take(&mut self.pending) {
            self.apply(action);
        }
        if self.load_error.is_some() || self.last_usage_save.elapsed() < USAGE_SAVE_INTERVAL {
            return;
        }
        self.last_usage_save = Instant::now();
        if let Some(clients) = self.shared.take_usage_update()
            && let Ok(store) = self.store.lock()
        {
            let _ = store.replace_mcp_clients(&clients);
        }
    }

    pub fn apply(&mut self, action: McpAction) {
        self.notice = None;
        if self.load_error.is_some() {
            self.notice = Some("MCP settings are unreadable; nothing can be changed".into());
            return;
        }
        match action {
            McpAction::SetEnabled(enabled) => self.set_enabled(enabled),
            McpAction::SetPort(port) => self.set_port(port),
            McpAction::CreateClient(name) => self.create_client(&name),
            McpAction::RevokeClient(id) => self.revoke_client(&id),
            McpAction::SetClientPermission {
                client_id,
                permission,
                allowed,
            } => {
                self.edit_clients(|clients| {
                    if let Some(client) = clients.iter_mut().find(|client| client.id == client_id) {
                        client.permissions.set(permission, allowed);
                    }
                });
            }
            McpAction::SetGlobalPermission {
                permission,
                allowed,
            } => self.set_global_permission(permission, allowed),
            McpAction::DismissToken => self.fresh_token = None,
        }
    }

    fn save_settings(&mut self, settings: McpSettings) -> bool {
        let saved = self
            .store
            .lock()
            .map(|store| store.set_mcp_settings(&settings))
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        match saved {
            Ok(()) => {
                self.settings = settings;
                true
            }
            Err(_) => {
                self.notice = Some("Could not save the MCP settings".into());
                false
            }
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        let settings = McpSettings {
            enabled,
            ..self.settings.clone()
        };
        if !self.save_settings(settings) {
            return;
        }
        self.stop_server();
        if enabled {
            self.start_server();
        }
    }

    fn set_port(&mut self, port: u16) {
        if !is_valid_port(port) {
            self.notice = Some("Choose a port between 1024 and 65535".into());
            return;
        }
        if port == self.settings.port {
            return;
        }
        let settings = McpSettings {
            port,
            ..self.settings.clone()
        };
        if !self.save_settings(settings) {
            return;
        }
        if self.settings.enabled {
            self.stop_server();
            self.start_server();
        }
    }

    fn start_server(&mut self) {
        match McpServer::start(self.shared.clone(), self.bridge.clone(), self.settings.port) {
            Ok(server) => self.server = Some(server),
            // The status already says Failed with the reason.
            Err(message) => self.notice = Some(message),
        }
    }

    fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop();
        } else {
            self.shared.bump_generation();
            self.shared.set_status(McpStatus::Disabled);
        }
    }

    /// Applies `edit` to the clients, saves the result, and only then publishes it.
    fn edit_clients(&mut self, edit: impl FnOnce(&mut Vec<McpClient>)) -> bool {
        let mut clients = self.shared.clients();
        edit(&mut clients);
        let saved = self
            .store
            .lock()
            .map(|store| store.replace_mcp_clients(&clients))
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        match saved {
            Ok(()) => {
                self.shared.replace(clients, self.shared.global());
                true
            }
            Err(_) => {
                self.notice = Some("Could not save the MCP clients".into());
                false
            }
        }
    }

    fn create_client(&mut self, name: &str) {
        let Some(name) = clean_client_name(name) else {
            self.notice = Some("Give the client a short name".into());
            return;
        };
        if self.shared.clients().len() >= MAX_CLIENTS {
            self.notice = Some(format!("At most {MAX_CLIENTS} clients are supported"));
            return;
        }
        let token = match generate_token() {
            Ok(token) => token,
            Err(error) => {
                self.notice = Some(error);
                return;
            }
        };
        let id = new_id().to_string();
        let client = McpClient {
            id: id.clone(),
            name: name.clone(),
            verifier: verifier_hex(&token),
            // A new credential can read; anything that changes the app is opt-in.
            permissions: PermissionSet::of([Permission::ChartsRead]),
            created_at_ms: now_ms(),
            last_used_ms: None,
        };
        if self.edit_clients(|clients| clients.push(client)) {
            self.fresh_token = Some(FreshToken {
                client_id: id,
                client_name: name,
                token,
            });
        }
    }

    fn revoke_client(&mut self, id: &str) {
        if self.edit_clients(|clients| clients.retain(|client| client.id != id))
            && self
                .fresh_token
                .as_ref()
                .is_some_and(|fresh| fresh.client_id == id)
        {
            self.fresh_token = None;
        }
    }

    fn set_global_permission(&mut self, permission: Permission, allowed: bool) {
        let mut global = self.shared.global();
        global.set(permission, allowed);
        let saved = self
            .store
            .lock()
            .map(|store| store.set_mcp_global_permissions(&global))
            .unwrap_or_else(|_| Err(rusqlite::Error::InvalidQuery));
        match saved {
            Ok(()) => self.shared.replace(self.shared.clients(), global),
            Err(_) => self.notice = Some("Could not save the MCP permissions".into()),
        }
    }
}

impl Drop for McpHost {
    fn drop(&mut self) {
        self.stop_server();
    }
}

// MARK: Workspace projections (run on the app thread)

pub fn source_slug(source: DataSource) -> &'static str {
    match source {
        DataSource::Binance => "binance",
        DataSource::Coinbase => "coinbase",
        DataSource::CoinGecko => "coingecko",
        DataSource::DexScreener => "dexscreener",
        DataSource::Alpaca => "alpaca",
        DataSource::Polymarket => "polymarket",
        DataSource::Kalshi => "kalshi",
        DataSource::CoinMarketCap => "coinmarketcap",
    }
}

pub fn workspace_info(workspace: &ChartWorkspace, active: bool) -> WorkspaceInfo {
    WorkspaceInfo {
        workspace_id: workspace.id().to_string(),
        title: workspace.title(),
        chart_count: workspace.chart_count(),
        active,
        timeframe: workspace.timeframe().label().to_owned(),
    }
}

fn chart_info(workspace: &ChartWorkspace, chart: ChartSummary) -> ChartInfo {
    let indicators = chart.indicators;
    let status = if chart.error.is_some() {
        "error"
    } else if chart.loading {
        "loading"
    } else {
        "ready"
    };
    ChartInfo {
        chart_id: chart.id.to_string(),
        instrument: chart.instrument.key(),
        source: source_slug(chart.instrument.source).to_owned(),
        symbol: chart.instrument.symbol.clone(),
        display_symbol: chart.display_symbol,
        timeframe: workspace.timeframe().label().to_owned(),
        chart_type: if chart.is_line_chart {
            "line"
        } else {
            "candles"
        }
        .to_owned(),
        indicators: IndicatorsInfo {
            volume: indicators.show_volume && !indicators.hide_volume,
            rsi: indicators.show_rsi && !indicators.hide_rsi,
            ema: indicators.show_ema && !indicators.hide_ema,
            ema_period: indicators.ema_period,
            bollinger: indicators.show_bollinger && !indicators.hide_bollinger,
            trend_flips: indicators.show_trend_flips && !indicators.hide_trend_flips,
        },
        pine_instances: chart
            .pine_instances
            .into_iter()
            .map(|(id, name)| PineInstanceInfo {
                instance_id: id.to_string(),
                script_name: name,
            })
            .collect(),
        status: status.to_owned(),
        error: chart.error,
        focused: chart.focused,
    }
}

pub fn charts_result(workspace: &ChartWorkspace) -> ChartsResult {
    ChartsResult {
        workspace_id: workspace.id().to_string(),
        charts: workspace
            .chart_summaries()
            .into_iter()
            .map(|chart| chart_info(workspace, chart))
            .collect(),
    }
}

/// Adds the chart through the workspace's own path and describes the outcome.
pub fn add_chart(
    workspace: &mut ChartWorkspace,
    instrument: &InstrumentId,
    ctx: &eframe::egui::Context,
) -> AddChartResult {
    let outcome = workspace.add_chart_for(instrument, ctx);
    let chart = workspace
        .chart_summaries()
        .into_iter()
        .find(|chart| chart.id == outcome.chart_id);
    AddChartResult {
        workspace_id: workspace.id().to_string(),
        chart_id: outcome.chart_id.to_string(),
        instrument: instrument.key(),
        source: source_slug(instrument.source).to_owned(),
        symbol: chart.as_ref().map_or_else(
            || instrument.symbol.clone(),
            |chart| chart.instrument.symbol.clone(),
        ),
        display_symbol: chart.map_or_else(
            || instrument.source.display_symbol(&instrument.symbol),
            |chart| chart.display_symbol,
        ),
        timeframe: workspace.timeframe().label().to_owned(),
        status: if outcome.created {
            "created"
        } else {
            "already_present"
        }
        .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::state::McpStatus;
    use crate::store::Store;
    use std::sync::Mutex;

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    fn host(store: &SharedStore) -> McpHost {
        McpHost::load(store.clone(), Arc::new(|| {}))
    }

    fn enable_on_free_port(host: &mut McpHost, store: &SharedStore) {
        // Tests bind port 0 (any free one), which the Settings page itself never offers.
        store
            .lock()
            .unwrap()
            .set_mcp_settings(&McpSettings {
                enabled: false,
                port: 0,
            })
            .unwrap();
        host.settings.port = 0;
        host.apply(McpAction::SetEnabled(true));
    }

    fn running_port(host: &McpHost) -> u16 {
        match host.status() {
            McpStatus::Running { port } => port,
            other => panic!("not running: {other:?}"),
        }
    }

    #[test]
    fn a_fresh_install_is_off_and_listens_nowhere() {
        let host = host(&store());
        assert_eq!(host.status(), McpStatus::Disabled);
        assert!(!host.settings().enabled);
        assert!(host.clients().is_empty());
        assert_eq!(
            host.global_permissions(),
            crate::model::mcp::default_global_permissions()
        );
        assert!(host.load_error().is_none());
    }

    #[test]
    fn enabling_and_disabling_controls_the_real_listener() {
        let store = store();
        let mut host = host(&store);
        enable_on_free_port(&mut host, &store);
        let port = running_port(&host);
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
        assert!(store.lock().unwrap().mcp_settings().unwrap().enabled);

        host.apply(McpAction::SetEnabled(false));
        assert_eq!(host.status(), McpStatus::Disabled);
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
        assert!(!store.lock().unwrap().mcp_settings().unwrap().enabled);
    }

    #[test]
    fn only_a_previously_enabled_server_starts_on_launch() {
        let store = store();
        store
            .lock()
            .unwrap()
            .set_mcp_settings(&McpSettings {
                enabled: true,
                port: 0,
            })
            .unwrap();
        let host = host(&store);
        assert!(matches!(host.status(), McpStatus::Running { .. }));
        drop(host);
        // Dropping the host (app quit) closes the listener.
    }

    #[test]
    fn credentials_are_shown_once_and_only_their_hash_is_kept() {
        let store = store();
        let mut host = host(&store);
        host.apply(McpAction::CreateClient("  Claude Desktop ".into()));
        let fresh = host.fresh_token().cloned().expect("token shown once");
        assert_eq!(fresh.client_name, "Claude Desktop");
        assert!(fresh.token.starts_with("dvmcp_"));

        let stored = store.lock().unwrap().mcp_clients().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].verifier, verifier_hex(&fresh.token));
        assert!(
            !serde_json::to_string(&stored[0])
                .unwrap()
                .contains(&fresh.token)
        );
        // A new credential can read and nothing more.
        assert_eq!(
            stored[0].permissions,
            PermissionSet::of([Permission::ChartsRead])
        );
        assert_eq!(
            host.shared().authenticate(&fresh.token).unwrap().id,
            stored[0].id
        );

        host.apply(McpAction::DismissToken);
        assert!(host.fresh_token().is_none());
    }

    #[test]
    fn revoking_is_immediate_and_persisted() {
        let store = store();
        let mut host = host(&store);
        host.apply(McpAction::CreateClient("a".into()));
        let token = host.fresh_token().unwrap().token.clone();
        let id = host.clients()[0].id.clone();
        assert!(host.shared().authenticate(&token).is_some());

        host.apply(McpAction::RevokeClient(id));
        assert!(host.shared().authenticate(&token).is_none());
        assert!(host.fresh_token().is_none());
        assert!(store.lock().unwrap().mcp_clients().unwrap().is_empty());
    }

    #[test]
    fn permission_changes_publish_only_after_they_are_saved() {
        let store = store();
        let mut host = host(&store);
        host.apply(McpAction::CreateClient("a".into()));
        let id = host.clients()[0].id.clone();
        host.apply(McpAction::SetClientPermission {
            client_id: id.clone(),
            permission: Permission::ChartsCreate,
            allowed: true,
        });
        // Granted by the client but capped by the global read-only default.
        assert!(!host.shared().allows(&id, Permission::ChartsCreate));
        host.apply(McpAction::SetGlobalPermission {
            permission: Permission::ChartsCreate,
            allowed: true,
        });
        assert!(host.shared().allows(&id, Permission::ChartsCreate));
        assert!(
            store
                .lock()
                .unwrap()
                .mcp_global_permissions()
                .unwrap()
                .contains(Permission::ChartsCreate)
        );
        host.apply(McpAction::SetClientPermission {
            client_id: id.clone(),
            permission: Permission::ChartsCreate,
            allowed: false,
        });
        assert!(!host.shared().allows(&id, Permission::ChartsCreate));
    }

    #[test]
    fn unreadable_saved_state_fails_closed_and_is_never_overwritten() {
        let store = store();
        {
            let guard = store.lock().unwrap();
            guard
                .set_mcp_settings(&McpSettings {
                    enabled: true,
                    port: 0,
                })
                .unwrap();
            guard.insert_raw_mcp_client("x", "{broken");
        }
        let mut host = host(&store);
        assert!(host.load_error().is_some());
        assert_eq!(
            host.status(),
            McpStatus::Disabled,
            "must not start with lost credentials"
        );
        assert!(host.clients().is_empty());

        host.apply(McpAction::CreateClient("a".into()));
        host.apply(McpAction::SetEnabled(true));
        assert_eq!(host.status(), McpStatus::Disabled);
        assert!(host.fresh_token().is_none());
        // The broken row is still there to be recovered, not replaced.
        assert!(store.lock().unwrap().mcp_clients().is_err());
    }

    #[test]
    fn bad_input_is_refused_with_a_notice() {
        let store = store();
        let mut host = host(&store);
        host.apply(McpAction::CreateClient("   ".into()));
        assert!(host.notice().is_some());
        assert!(host.clients().is_empty());
        host.apply(McpAction::SetPort(80));
        assert!(host.notice().is_some());
        assert_eq!(host.settings().port, crate::model::mcp::DEFAULT_PORT);
    }

    #[test]
    fn settings_actions_are_applied_on_the_next_tick() {
        let store = store();
        let mut host = host(&store);
        host.queue(McpAction::CreateClient("queued".into()));
        assert!(host.clients().is_empty());
        host.tick();
        assert_eq!(host.clients().len(), 1);
    }
}
