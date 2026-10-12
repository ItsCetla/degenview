//! The stateful bridge between the MCP server, SQLite, the Settings page and the workspaces.
//!
//! The host owns the listener's lifecycle (it exists only while the user has the server
//! enabled), persists settings and credentials through `store::mcp`, keeps the server's shared
//! permission snapshot equal to what is on disk, and hands queued tool requests to the app.
//! Persist first, then publish: a write that fails changes nothing the server can see.

use crate::alert_host::AlertHost;
use crate::mcp::McpServer;
use crate::mcp::auth::{generate_token, verifier_hex};
use crate::mcp::bridge::{
    self, AlertUpdate, BridgeClient, BridgeInbox, McpError, NewAlert, Request,
};
use crate::mcp::state::{McpShared, McpStatus};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertInfo, AlertsResult, ChartInfo, ChartsResult,
    IndicatorsInfo, PineInstanceInfo, WorkspaceInfo,
};
use crate::model::alert::{
    AlertCondition, AlertCurrency, AlertFrequency, AlertId, AlertState, PriceAlert,
};
use crate::model::mcp::{
    MAX_CLIENTS, McpAction, McpClient, McpSettings, Permission, PermissionSet, clean_client_name,
    is_valid_port,
};
use crate::model::watchlist::{new_id, now_ms};
use crate::model::{BuiltIn, DataSource, IndicatorConfig, InstrumentId};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::store::SharedStore;
use crate::ui::alert_editor::{Direction, PriceAlertEditorState};
use crate::workspace::{ChartSummary, ChartWorkspace, IndicatorError, IndicatorOutcome};
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
        indicators: indicators_info(indicators),
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

/// What the chart draws now: applied and not hidden with its eye.
fn indicators_info(config: IndicatorConfig) -> IndicatorsInfo {
    let drawn = config.drawn();
    IndicatorsInfo {
        volume: drawn.show_volume,
        rsi: drawn.show_rsi,
        ema: drawn.show_ema,
        ema_period: config.ema_period,
        bollinger: drawn.show_bollinger,
        trend_flips: drawn.show_trend_flips,
    }
}

/// Applies a built-in indicator through the workspace's own method and describes the result.
pub fn apply_indicator(
    workspace: &mut ChartWorkspace,
    chart_id: u64,
    indicator: BuiltIn,
    ema_period: Option<usize>,
) -> Result<AddIndicatorResult, McpError> {
    let workspace_id = workspace.id();
    match workspace.apply_indicator(chart_id, indicator, ema_period) {
        Ok((outcome, config)) => Ok(AddIndicatorResult {
            workspace_id: workspace_id.to_string(),
            chart_id: chart_id.to_string(),
            indicator: indicator.id().to_owned(),
            status: match outcome {
                IndicatorOutcome::Added => "added",
                IndicatorOutcome::Updated => "updated",
                IndicatorOutcome::AlreadyApplied => "already_applied",
            }
            .to_owned(),
            indicators: indicators_info(config),
        }),
        Err(IndicatorError::NoSuchChart) => Err(McpError::NotFound(format!(
            "No chart {chart_id} in workspace {workspace_id}"
        ))),
        Err(IndicatorError::UnsupportedOnLineChart) => Err(McpError::Unsupported(format!(
            "{} is not available on line charts",
            indicator.menu_title()
        ))),
        Err(IndicatorError::NoParameters) => Err(McpError::InvalidArgument(format!(
            "{} has no parameters",
            indicator.menu_title()
        ))),
        Err(IndicatorError::InvalidEmaPeriod) => Err(McpError::InvalidArgument(format!(
            "EMA period must be one of {:?}",
            crate::model::EMA_PERIODS
        ))),
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

// MARK: Price alerts (run on the app thread, against the one `AlertHost`)

/// Alerts one `degenview_list_price_alerts` call returns.
const ALERT_PAGE: usize = 200;
/// No further alert is accepted from a client beyond this many in total.
const MAX_ALERTS: usize = 500;

/// A number as a plain decimal (`70000`, `0.00000278`, `2.5`): rounded to 12 significant digits,
/// which drops float noise (`110.00000000000001`) but not any digit a quote carries, and never
/// written in exponent form.
fn decimal(value: f64) -> String {
    let rounded = format!("{value:.11e}").parse::<f64>().unwrap_or(value);
    format!("{rounded}")
}

pub fn alert_info(alert: &PriceAlert, last_triggered_ms: Option<i64>) -> AlertInfo {
    let currency = alert.currency;
    let price = decimal;
    let (condition, percent, reference) = match &alert.condition {
        AlertCondition::CrossesAbove { .. } => ("crosses_above", None, None),
        AlertCondition::CrossesBelow { .. } => ("crosses_below", None, None),
        AlertCondition::RisesBy {
            percent, reference, ..
        } => ("rises_by", Some(decimal(*percent)), Some(price(*reference))),
        AlertCondition::FallsBy {
            percent, reference, ..
        } => ("falls_by", Some(decimal(*percent)), Some(price(*reference))),
        AlertCondition::Unsupported { .. } => ("unsupported", None, None),
    };
    AlertInfo {
        alert_id: alert.id.to_string(),
        instrument: alert.asset.key(),
        source: source_slug(alert.asset.source()).to_owned(),
        symbol: alert.asset.instrument.symbol.clone(),
        condition: condition.to_owned(),
        target_price: alert.condition.target().map(price),
        percent,
        reference_price: reference,
        currency: currency.code().to_owned(),
        frequency: match alert.frequency {
            AlertFrequency::Once => "once",
            AlertFrequency::EveryTime => "every_time",
        }
        .to_owned(),
        status: match alert.state {
            AlertState::Active => "active",
            AlertState::Paused => "paused",
            AlertState::Triggered => "triggered",
            AlertState::Unsupported => "unsupported",
        }
        .to_owned(),
        note: alert.note.clone(),
        sends_to_webhooks: !alert.webhook_endpoint_ids.is_empty(),
        created_at: crate::format::iso8601(alert.created_at_ms),
        updated_at: crate::format::iso8601(alert.updated_at_ms),
        last_triggered_at: last_triggered_ms.map(crate::format::iso8601),
    }
}

fn last_triggered(host: &AlertHost, id: AlertId) -> Option<i64> {
    host.snapshot()
        .history
        .iter()
        .filter(|event| event.alert_id == id)
        .map(|event| event.timestamp_ms)
        .max()
}

fn info_for(host: &AlertHost, alert: &PriceAlert) -> AlertInfo {
    alert_info(alert, last_triggered(host, alert.id))
}

/// Alerts as the runtime holds them: the page, or just the one asked for.
pub fn list_alerts(host: &AlertHost, only: Option<AlertId>) -> AlertsResult {
    let alerts = &host.snapshot().alerts;
    if let Some(id) = only {
        let found: Vec<_> = alerts
            .iter()
            .filter(|alert| alert.id == id)
            .map(|alert| info_for(host, alert))
            .collect();
        return AlertsResult {
            total: found.len(),
            truncated: false,
            alerts: found,
        };
    }
    AlertsResult {
        total: alerts.len(),
        truncated: alerts.len() > ALERT_PAGE,
        alerts: alerts
            .iter()
            .take(ALERT_PAGE)
            .map(|alert| info_for(host, alert))
            .collect(),
    }
}

pub enum CreatePlan {
    /// An identical alert exists already; hand it back instead of adding a duplicate.
    Existing(Box<AlertInfo>),
    New {
        alert: Box<PriceAlert>,
        /// The market now, in the alert's currency, when a live price is known.
        baseline: Option<f64>,
    },
}

fn require_loaded(host: &AlertHost) -> Result<(), McpError> {
    match host.load_error() {
        None => Ok(()),
        Some(_) => Err(McpError::AppUnavailable(
            "Alert data could not be loaded, so alerts cannot be changed".into(),
        )),
    }
}

/// Builds the alert exactly as the price-alert sheet does (its own parsing and rules), without
/// saving it. The caller commits a `New` plan through the same path the sheet's Save uses.
pub fn plan_create_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &NewAlert,
    now_ms: i64,
) -> Result<CreatePlan, McpError> {
    require_loaded(host)?;
    if host.snapshot().alerts.len() >= MAX_ALERTS {
        return Err(McpError::ResourceLimit(format!(
            "There are already {MAX_ALERTS} price alerts; delete some first"
        )));
    }
    let asset =
        ChartWorkspace::alert_asset_for(&request.instrument.symbol, request.instrument.source);
    let mut editor = PriceAlertEditorState::new(asset, None, Some(request.currency));
    editor.direction = if request.crosses_above {
        Direction::Above
    } else {
        Direction::Below
    };
    editor.target = request.target_text.clone();
    editor.frequency = request.frequency;
    editor.note = request.note.clone();
    editor.feed_from_book(book, rates);
    let alert = editor.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    if let Some(existing) = host.find_identical(&alert) {
        return Ok(CreatePlan::Existing(Box::new(info_for(host, existing))));
    }
    Ok(CreatePlan::New {
        baseline: editor.current_price(),
        alert: Box::new(alert),
    })
}

pub enum UpdatePlan {
    /// The request matches the alert as it is: nothing to queue.
    Unchanged(Box<AlertInfo>),
    Change {
        alert: Box<PriceAlert>,
        baseline: Option<f64>,
    },
}

/// Edits an alert as the Alerts window's Edit sheet does (its parsing, currency and arming rules),
/// without saving it. A remote edit never resumes a paused alert, and an alert that already fired
/// only becomes active again when its condition, currency or frequency changed. Webhook settings
/// are carried over untouched.
pub fn plan_update_alert(
    host: &AlertHost,
    book: &WatchlistQuoteBook,
    rates: &std::collections::HashMap<AlertCurrency, f64>,
    request: &AlertUpdate,
    now_ms: i64,
) -> Result<UpdatePlan, McpError> {
    require_loaded(host)?;
    let existing = find_alert(host, request.alert_id)?.clone();
    let percentage = matches!(
        existing.condition,
        AlertCondition::RisesBy { .. } | AlertCondition::FallsBy { .. }
    );
    if existing.condition.target().is_none() {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    if percentage
        && (request.crosses_above.is_some()
            || request.target_text.is_some()
            || request.currency.is_some())
    {
        return Err(McpError::Unsupported(
            "A percentage alert can only have its frequency and note changed".into(),
        ));
    }
    let mut editor = PriceAlertEditorState::new(
        existing.asset.clone(),
        Some(existing.clone()),
        Some(existing.currency),
    );
    if let Some(currency) = request.currency {
        editor.set_currency(currency);
    }
    if let Some(above) = request.crosses_above {
        editor.direction = if above {
            Direction::Above
        } else {
            Direction::Below
        };
    }
    if let Some(target) = &request.target_text {
        editor.target = target.clone();
    }
    if let Some(frequency) = request.frequency {
        editor.frequency = frequency;
    }
    if let Some(note) = &request.note {
        editor.note = note.clone();
    }
    editor.feed_from_book(book, rates);
    let mut alert = editor.result(now_ms).ok_or_else(|| {
        McpError::InvalidArgument("target_price must be a positive number".into())
    })?;
    let settings_changed = alert.condition != existing.condition
        || alert.currency != existing.currency
        || alert.frequency != existing.frequency;
    alert.state = match existing.state {
        AlertState::Paused => AlertState::Paused,
        AlertState::Triggered if !settings_changed => AlertState::Triggered,
        _ => AlertState::Active,
    };
    if !settings_changed && alert.note == existing.note && alert.state == existing.state {
        return Ok(UpdatePlan::Unchanged(Box::new(info_for(host, &existing))));
    }
    if let Some(other) = host.find_identical(&alert) {
        return Err(McpError::Conflict(format!(
            "Price alert {} already has exactly these settings",
            other.id
        )));
    }
    Ok(UpdatePlan::Change {
        baseline: editor.current_price(),
        alert: Box::new(alert),
    })
}

pub enum StatePlan {
    /// Already in the requested state: nothing to queue.
    Already,
    Change {
        id: AlertId,
        state: AlertState,
    },
}

fn find_alert(host: &AlertHost, id: AlertId) -> Result<&PriceAlert, McpError> {
    host.snapshot()
        .alerts
        .iter()
        .find(|alert| alert.id == id)
        .ok_or_else(|| McpError::NotFound(format!("No price alert {id}")))
}

/// Pause or resume, deciding as the Alerts window's toggle does: pausing an active alert, and
/// resuming anything else (a fired once-alert is re-armed).
pub fn plan_pause_alert(
    host: &AlertHost,
    id: AlertId,
    paused: bool,
) -> Result<StatePlan, McpError> {
    require_loaded(host)?;
    let alert = find_alert(host, id)?;
    if alert.state == AlertState::Unsupported {
        return Err(McpError::Unsupported(
            "This alert uses a condition this version of DegenView does not support".into(),
        ));
    }
    let already = if paused {
        alert.state == AlertState::Paused
    } else {
        alert.state == AlertState::Active
    };
    if already {
        return Ok(StatePlan::Already);
    }
    Ok(StatePlan::Change {
        id,
        state: if paused {
            AlertState::Paused
        } else {
            AlertState::Active
        },
    })
}

pub fn plan_delete_alert(host: &AlertHost, id: AlertId) -> Result<AlertId, McpError> {
    require_loaded(host)?;
    find_alert(host, id).map(|alert| alert.id)
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

    // MARK: Price alerts against the real alert host

    use crate::model::alert::{AlertAsset, AlertRuntimeCommand, AlertRuntimeCommandPayload};
    use crate::model::watchlist_quote::{Freshness, WatchlistQuote};
    use std::collections::HashMap;

    const NOW: i64 = 1_760_000_000_000;

    fn alert_host() -> (Store, AlertHost) {
        let store = Store::open_in_memory().unwrap();
        let host = AlertHost::load(&store, NOW);
        (store, host)
    }

    fn command(store: &Store, host: &mut AlertHost, payload: AlertRuntimeCommandPayload) {
        store
            .enqueue_alert_command(&AlertRuntimeCommand::new(payload, NOW))
            .unwrap();
        host.drain_commands(store, NOW);
    }

    fn request(crosses_above: bool, target: &str) -> NewAlert {
        NewAlert {
            instrument: InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            crosses_above,
            target_text: target.to_owned(),
            currency: AlertCurrency::Usd,
            frequency: AlertFrequency::Once,
            note: String::new(),
        }
    }

    fn book_with_price(last: f64) -> WatchlistQuoteBook {
        let book = WatchlistQuoteBook::default();
        book.apply(HashMap::from([(
            InstrumentId::new(DataSource::Binance, "BTCUSDT").key(),
            WatchlistQuote {
                last: Some(last),
                freshness: Freshness::Live,
                ..WatchlistQuote::default()
            },
        )]));
        book
    }

    /// Creates through the same two steps the app takes: plan, then queue the Save.
    fn create(
        store: &Store,
        host: &mut AlertHost,
        book: &WatchlistQuoteBook,
        request: &NewAlert,
    ) -> Result<CreatePlan, McpError> {
        let plan = plan_create_alert(host, book, &HashMap::new(), request, NOW)?;
        if let CreatePlan::New { alert, baseline } = &plan {
            command(
                store,
                host,
                AlertRuntimeCommandPayload::Save {
                    alert: alert.clone(),
                    baseline: *baseline,
                },
            );
        }
        Ok(plan)
    }

    #[test]
    fn a_created_alert_is_built_by_the_sheets_rules_and_armed_from_the_live_price() {
        let (store, mut host) = alert_host();
        // Price below the target: the next cross upward counts.
        let below = book_with_price(65_000.0);
        let plan = create(&store, &mut host, &below, &request(true, "70000")).unwrap();
        let CreatePlan::New { alert, baseline } = plan else {
            panic!("expected a new alert");
        };
        assert_eq!(baseline, Some(65_000.0));
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesAbove { target: 70_000.0 }
        );
        assert_eq!(alert.state, AlertState::Active);
        assert_eq!(alert.currency, AlertCurrency::Usd);
        assert_eq!(alert.frequency, AlertFrequency::Once);
        assert_eq!(alert.asset.key(), "BINANCE:BTCUSDT");
        assert!(alert.webhook_endpoint_ids.is_empty());
        assert!(host.snapshot().alerts[0].armed);

        // Price already above the target: it must first fall back and cross again.
        let above = book_with_price(80_000.0);
        create(&store, &mut host, &above, &request(true, "75000")).unwrap();
        let second = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.condition.target() == Some(75_000.0))
            .unwrap();
        assert!(!second.armed);

        // No live price: still created, and armed by default.
        let none = WatchlistQuoteBook::default();
        let plan = create(&store, &mut host, &none, &request(false, "50000")).unwrap();
        assert!(matches!(plan, CreatePlan::New { baseline: None, .. }));
    }

    #[test]
    fn listing_reports_the_alert_and_never_exposes_webhooks() {
        let (store, mut host) = alert_host();
        let mut alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::CrossesBelow { target: 0.00000278 },
            NOW,
        );
        alert.note = "dip".into();
        alert.webhook_endpoint_ids = vec![424_242];
        let id = alert.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
        );
        let listed = list_alerts(&host, None);
        assert_eq!((listed.total, listed.truncated), (1, false));
        let info = &listed.alerts[0];
        assert_eq!(info.alert_id, id.to_string());
        assert_eq!(info.condition, "crosses_below");
        assert_eq!(info.target_price.as_deref(), Some("0.00000278"));
        assert_eq!(
            (info.status.as_str(), info.frequency.as_str()),
            ("active", "once")
        );
        assert_eq!(info.note, "dip");
        assert!(info.sends_to_webhooks);
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains("424242") && !json.contains("webhook_endpoint"),
            "{json}"
        );
        assert!(info.created_at.ends_with('Z'));

        assert_eq!(list_alerts(&host, Some(id)).alerts.len(), 1);
        assert!(list_alerts(&host, Some(id + 1)).alerts.is_empty());
    }

    #[test]
    fn an_identical_alert_is_returned_instead_of_a_duplicate() {
        let (store, mut host) = alert_host();
        let book = WatchlistQuoteBook::default();
        create(&store, &mut host, &book, &request(true, "70000")).unwrap();
        let id = host.snapshot().alerts[0].id.to_string();

        let again = create(&store, &mut host, &book, &request(true, "70000")).unwrap();
        assert!(matches!(&again, CreatePlan::Existing(info) if info.alert_id == id));
        assert_eq!(host.snapshot().alerts.len(), 1);

        // A different level, direction, currency or frequency is a different alert.
        for different in [
            request(true, "71000"),
            request(false, "70000"),
            NewAlert {
                currency: AlertCurrency::Eur,
                ..request(true, "70000")
            },
            NewAlert {
                frequency: AlertFrequency::EveryTime,
                ..request(true, "70000")
            },
        ] {
            let plan = create(&store, &mut host, &book, &different).unwrap();
            assert!(matches!(plan, CreatePlan::New { .. }));
        }
        assert_eq!(host.snapshot().alerts.len(), 5);
    }

    #[test]
    fn an_unparseable_target_is_refused() {
        let (_store, host) = alert_host();
        let book = WatchlistQuoteBook::default();
        for bad in ["", "0", "-1", "abc"] {
            let result = plan_create_alert(&host, &book, &HashMap::new(), &request(true, bad), NOW);
            assert!(
                matches!(result, Err(McpError::InvalidArgument(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_alert_count_is_capped() {
        let (store, mut host) = alert_host();
        for index in 0..MAX_ALERTS {
            let alert = PriceAlert::new(
                AlertAsset::new(
                    InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                    "BTC",
                    "Bitcoin",
                ),
                AlertCondition::CrossesAbove {
                    target: 1.0 + index as f64,
                },
                NOW,
            );
            store
                .enqueue_alert_command(&AlertRuntimeCommand::new(
                    AlertRuntimeCommandPayload::Save {
                        alert: Box::new(alert),
                        baseline: None,
                    },
                    NOW,
                ))
                .unwrap();
        }
        host.drain_commands(&store, NOW);
        assert_eq!(host.snapshot().alerts.len(), MAX_ALERTS);
        let result = plan_create_alert(
            &host,
            &WatchlistQuoteBook::default(),
            &HashMap::new(),
            &request(true, "999999"),
            NOW,
        );
        assert!(matches!(result, Err(McpError::ResourceLimit(_))));
        assert_eq!(list_alerts(&host, None).alerts.len(), ALERT_PAGE);
        assert!(list_alerts(&host, None).truncated);
    }

    #[test]
    fn pause_resume_and_delete_follow_the_alert_windows_semantics() {
        let (store, mut host) = alert_host();
        create(
            &store,
            &mut host,
            &WatchlistQuoteBook::default(),
            &request(true, "70000"),
        )
        .unwrap();
        let id = host.snapshot().alerts[0].id;

        // Active: pausing changes it, resuming is a no-op.
        assert!(matches!(
            plan_pause_alert(&host, id, false),
            Ok(StatePlan::Already)
        ));
        let Ok(StatePlan::Change {
            id: change_id,
            state,
        }) = plan_pause_alert(&host, id, true)
        else {
            panic!("expected a change");
        };
        assert_eq!((change_id, state), (id, AlertState::Paused));
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state,
                baseline: None,
            },
        );
        assert_eq!(list_alerts(&host, Some(id)).alerts[0].status, "paused");
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Ok(StatePlan::Already)
        ));

        // A fired once-alert is resumed (re-armed) like the Toggle button does.
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Triggered,
                baseline: None,
            },
        );
        assert!(matches!(
            plan_pause_alert(&host, id, false),
            Ok(StatePlan::Change {
                state: AlertState::Active,
                ..
            })
        ));

        // Delete.
        assert_eq!(plan_delete_alert(&host, id).unwrap(), id);
        command(&store, &mut host, AlertRuntimeCommandPayload::Delete(id));
        assert!(list_alerts(&host, None).alerts.is_empty());
        assert!(matches!(
            plan_delete_alert(&host, id),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Err(McpError::NotFound(_))
        ));
    }

    #[test]
    fn an_alert_with_a_condition_from_a_newer_version_cannot_be_toggled() {
        let (store, mut host) = alert_host();
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::Unsupported {
                tag: "touchesVwap".into(),
            },
            NOW,
        );
        let id = alert.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(alert),
                baseline: None,
            },
        );
        assert!(matches!(
            plan_pause_alert(&host, id, true),
            Err(McpError::Unsupported(_))
        ));
        let info = &list_alerts(&host, Some(id)).alerts[0];
        assert_eq!(
            (info.condition.as_str(), info.status.as_str()),
            ("unsupported", "unsupported")
        );
        assert_eq!(info.target_price, None);
    }

    #[test]
    fn decimals_drop_float_noise_but_keep_real_digits() {
        assert_eq!(decimal(70_000.0), "70000");
        assert_eq!(decimal(110.000_000_000_000_01), "110");
        assert_eq!(decimal(0.000_002_78), "0.00000278");
        assert_eq!(decimal(61_234.567_89), "61234.56789");
        assert_eq!(decimal(2.5), "2.5");
        assert_eq!(decimal(1e-8), "0.00000001");
    }

    #[test]
    fn percent_conditions_report_their_reference() {
        let alert = PriceAlert::new(
            AlertAsset::new(
                InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                "BTC",
                "Bitcoin",
            ),
            AlertCondition::rises_by(10.0, 100.0),
            NOW,
        );
        let info = alert_info(&alert, Some(NOW));
        assert_eq!(info.condition, "rises_by");
        assert_eq!(info.percent.as_deref(), Some("10"));
        assert_eq!(info.reference_price.as_deref(), Some("100"));
        assert_eq!(info.target_price.as_deref(), Some("110"));
        assert!(info.last_triggered_at.is_some());
    }

    fn update(id: AlertId) -> AlertUpdate {
        AlertUpdate {
            alert_id: id,
            crosses_above: None,
            target_text: None,
            currency: None,
            frequency: None,
            note: None,
        }
    }

    fn apply_update(
        store: &Store,
        host: &mut AlertHost,
        book: &WatchlistQuoteBook,
        request: &AlertUpdate,
    ) -> Result<UpdatePlan, McpError> {
        let plan = plan_update_alert(host, book, &HashMap::new(), request, NOW)?;
        if let UpdatePlan::Change { alert, baseline } = &plan {
            command(
                store,
                host,
                AlertRuntimeCommandPayload::Save {
                    alert: alert.clone(),
                    baseline: *baseline,
                },
            );
        }
        Ok(plan)
    }

    fn seeded() -> (Store, AlertHost, AlertId) {
        let (store, mut host) = alert_host();
        create(
            &store,
            &mut host,
            &WatchlistQuoteBook::default(),
            &request(true, "70000"),
        )
        .unwrap();
        let id = host.snapshot().alerts[0].id;
        (store, host, id)
    }

    #[test]
    fn editing_changes_only_the_given_settings_and_keeps_the_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        let created_at = host.snapshot().alerts[0].created_at_ms;

        let mut edit = update(id);
        edit.target_text = Some("72500.5".into());
        edit.note = Some("breakout".into());
        assert!(matches!(
            apply_update(&store, &mut host, &book, &edit),
            Ok(UpdatePlan::Change { .. })
        ));
        let alert = &host.snapshot().alerts[0];
        assert_eq!(host.snapshot().alerts.len(), 1);
        assert_eq!(alert.id, id);
        assert_eq!(alert.created_at_ms, created_at);
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesAbove { target: 72_500.5 }
        );
        assert_eq!(alert.note, "breakout");
        assert_eq!(alert.frequency, AlertFrequency::Once);

        let mut edit = update(id);
        edit.crosses_above = Some(false);
        edit.currency = Some(AlertCurrency::Eur);
        edit.frequency = Some(AlertFrequency::EveryTime);
        apply_update(&store, &mut host, &book, &edit).unwrap();
        let alert = &host.snapshot().alerts[0];
        assert_eq!(
            alert.condition,
            AlertCondition::CrossesBelow { target: 72_500.5 }
        );
        assert_eq!(
            (alert.currency, alert.frequency),
            (AlertCurrency::Eur, AlertFrequency::EveryTime)
        );
        assert_eq!(alert.note, "breakout");
    }

    #[test]
    fn an_edit_that_changes_nothing_queues_nothing() {
        let (_store, host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        let mut same = update(id);
        same.target_text = Some("70000".into());
        same.crosses_above = Some(true);
        same.frequency = Some(AlertFrequency::Once);
        same.note = Some(String::new());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &same, NOW),
            Ok(UpdatePlan::Unchanged(_))
        ));
    }

    #[test]
    fn an_edit_is_rearmed_from_the_live_price_like_the_sheet() {
        let (store, mut host, id) = seeded();
        // Move the target below a price of 80,000: it must fall back and cross up again.
        let mut edit = update(id);
        edit.target_text = Some("75000".into());
        apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
        assert!(!host.snapshot().alerts[0].armed);
        let mut edit = update(id);
        edit.target_text = Some("90000".into());
        apply_update(&store, &mut host, &book_with_price(80_000.0), &edit).unwrap();
        assert!(host.snapshot().alerts[0].armed);
    }

    #[test]
    fn a_remote_edit_never_resumes_a_paused_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Paused,
                baseline: None,
            },
        );
        let mut edit = update(id);
        edit.target_text = Some("71000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Paused);
        assert_eq!(host.snapshot().alerts[0].condition.target(), Some(71_000.0));
    }

    #[test]
    fn a_fired_alert_is_rearmed_only_by_a_real_change() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::SetState {
                id,
                state: AlertState::Triggered,
                baseline: None,
            },
        );
        // A note alone leaves it fired.
        let mut edit = update(id);
        edit.note = Some("seen".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Triggered);
        // A new level makes it live again.
        let mut edit = update(id);
        edit.target_text = Some("99000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        assert_eq!(host.snapshot().alerts[0].state, AlertState::Active);
    }

    #[test]
    fn an_edit_cannot_duplicate_another_alert() {
        let (store, mut host, id) = seeded();
        let book = WatchlistQuoteBook::default();
        create(&store, &mut host, &book, &request(true, "80000")).unwrap();
        let mut edit = update(id);
        edit.target_text = Some("80000".into());
        let result = plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW);
        assert!(matches!(result, Err(McpError::Conflict(_))));
        assert_eq!(
            host.snapshot()
                .alerts
                .iter()
                .filter(|a| a.condition.target() == Some(80_000.0))
                .count(),
            1
        );
    }

    #[test]
    fn edits_keep_webhooks_and_refuse_what_cannot_be_edited() {
        let (store, mut host) = alert_host();
        let book = WatchlistQuoteBook::default();
        let asset = AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        );

        let mut hooked = PriceAlert::new(
            asset.clone(),
            AlertCondition::CrossesAbove { target: 70_000.0 },
            NOW,
        );
        hooked.webhook_endpoint_ids = vec![7];
        hooked.webhook_message = Some("{{price}}".into());
        let hooked_id = hooked.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(hooked),
                baseline: None,
            },
        );
        let mut edit = update(hooked_id);
        edit.target_text = Some("71000".into());
        apply_update(&store, &mut host, &book, &edit).unwrap();
        let alert = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.id == hooked_id)
            .unwrap();
        assert_eq!(alert.webhook_endpoint_ids, vec![7]);
        assert_eq!(alert.webhook_message.as_deref(), Some("{{price}}"));

        // A percentage alert takes a note and frequency, not a new level.
        let percent = PriceAlert::new(asset.clone(), AlertCondition::rises_by(10.0, 100.0), NOW);
        let percent_id = percent.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(percent),
                baseline: None,
            },
        );
        let mut level = update(percent_id);
        level.target_text = Some("150".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &level, NOW),
            Err(McpError::Unsupported(_))
        ));
        let mut note = update(percent_id);
        note.note = Some("watch".into());
        note.frequency = Some(AlertFrequency::EveryTime);
        apply_update(&store, &mut host, &book, &note).unwrap();
        let alert = host
            .snapshot()
            .alerts
            .iter()
            .find(|a| a.id == percent_id)
            .unwrap();
        assert!(
            matches!(alert.condition, AlertCondition::RisesBy { percent, reference, .. } if percent == 10.0 && reference == 100.0)
        );
        assert_eq!(
            (alert.note.as_str(), alert.frequency),
            ("watch", AlertFrequency::EveryTime)
        );

        // A condition from a newer version, and an alert that does not exist.
        let future = PriceAlert::new(asset, AlertCondition::Unsupported { tag: "x".into() }, NOW);
        let future_id = future.id;
        command(
            &store,
            &mut host,
            AlertRuntimeCommandPayload::Save {
                alert: Box::new(future),
                baseline: None,
            },
        );
        let mut edit = update(future_id);
        edit.note = Some("x".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &edit, NOW),
            Err(McpError::Unsupported(_))
        ));
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &update(1), NOW),
            Err(McpError::NotFound(_))
        ));
        let mut bad = update(hooked_id);
        bad.target_text = Some("0".into());
        assert!(matches!(
            plan_update_alert(&host, &book, &HashMap::new(), &bad, NOW),
            Err(McpError::InvalidArgument(_))
        ));
    }
}
