//! The MCP tool surface: the chart tools and the handler that guards them.
//!
//! Tools are thin adapters. They validate their arguments, then ask the app to do the work
//! through the [`bridge`](crate::mcp::bridge); no workspace or SQLite state is touched here.
//! Authorization is enforced in `call_tool` on every call — `tools/list` merely hides what the
//! caller could not use anyway.

use crate::mcp::bridge::{
    AlertUpdate, BridgeClient, McpCommand, McpError, McpReply, NewAlert, REQUEST_TIMEOUT,
};
use crate::mcp::state::{ClientIdentity, McpShared};
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertInfo, AlertsResult, ChartsResult,
    CreateAlertResult, IndicatorsResult, UpdateAlertResult, WorkspacesResult, indicator_catalog,
};
use crate::model::alert::{AlertCurrency, AlertFrequency};
use crate::model::mcp::Permission;
use crate::model::watchlist_text::validate_market;
use crate::model::{BuiltIn, DataSource, EMA_PERIODS};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
    PaginatedRequestParams, ResultType,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub const LIST_WORKSPACES: &str = "degenview_list_workspaces";
pub const LIST_CHARTS: &str = "degenview_list_charts";
pub const ADD_CHART: &str = "degenview_add_chart";
pub const LIST_INDICATORS: &str = "degenview_list_indicators";
pub const ADD_INDICATOR: &str = "degenview_add_indicator";
pub const LIST_ALERTS: &str = "degenview_list_price_alerts";
pub const CREATE_ALERT: &str = "degenview_create_price_alert";
pub const UPDATE_ALERT: &str = "degenview_update_price_alert";
pub const PAUSE_ALERT: &str = "degenview_pause_price_alert";
pub const RESUME_ALERT: &str = "degenview_resume_price_alert";
pub const DELETE_ALERT: &str = "degenview_delete_price_alert";

/// The permission a tool needs, or `None` for a name this server does not have.
pub fn required_permission(tool: &str) -> Option<Permission> {
    match tool {
        LIST_WORKSPACES | LIST_CHARTS | LIST_INDICATORS => Some(Permission::ChartsRead),
        ADD_CHART => Some(Permission::ChartsCreate),
        ADD_INDICATOR => Some(Permission::ChartsIndicatorsWrite),
        LIST_ALERTS => Some(Permission::AlertsRead),
        CREATE_ALERT => Some(Permission::AlertsCreate),
        UPDATE_ALERT => Some(Permission::AlertsEdit),
        PAUSE_ALERT | RESUME_ALERT => Some(Permission::AlertsPause),
        DELETE_ALERT => Some(Permission::AlertsDelete),
        _ => None,
    }
}

/// Providers a chart can be added from by symbol alone. Prediction markets (they need an event)
/// and CoinMarketCap (indices, not markets) are not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpSource {
    Binance,
    Coinbase,
    Coingecko,
    Dexscreener,
    Alpaca,
}

impl McpSource {
    fn data_source(self) -> DataSource {
        match self {
            Self::Binance => DataSource::Binance,
            Self::Coinbase => DataSource::Coinbase,
            Self::Coingecko => DataSource::CoinGecko,
            Self::Dexscreener => DataSource::DexScreener,
            Self::Alpaca => DataSource::Alpaca,
        }
    }
}

/// The built-in indicators, by the ids `degenview_list_indicators` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpIndicator {
    Volume,
    Rsi,
    Ema,
    Bollinger,
    TrendFlips,
}

impl McpIndicator {
    fn built_in(self) -> BuiltIn {
        match self {
            Self::Volume => BuiltIn::Volume,
            Self::Rsi => BuiltIn::Rsi,
            Self::Ema => BuiltIn::Ema,
            Self::Bollinger => BuiltIn::Bollinger,
            Self::TrendFlips => BuiltIn::TrendFlips,
        }
    }
}

/// Settings for the indicator. Only EMA has any.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndicatorInputs {
    /// EMA period: one of 9, 20, 50, 100, 200.
    #[schemars(range(min = 1, max = 1000))]
    pub period: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddIndicatorParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// A `chart_id` from `degenview_list_charts` in that workspace.
    #[schemars(length(max = 20))]
    pub chart_id: String,
    pub indicator: McpIndicator,
    /// Omit for indicators with fixed settings.
    pub inputs: Option<IndicatorInputs>,
}

/// When a price alert fires. Percentage conditions are not offered yet: they need a live
/// reference price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpCondition {
    CrossesAbove,
    CrossesBelow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum McpCurrency {
    Usd,
    Eur,
    Gbp,
    Jpy,
    Chf,
}

impl McpCurrency {
    fn alert_currency(self) -> AlertCurrency {
        match self {
            Self::Usd => AlertCurrency::Usd,
            Self::Eur => AlertCurrency::Eur,
            Self::Gbp => AlertCurrency::Gbp,
            Self::Jpy => AlertCurrency::Jpy,
            Self::Chf => AlertCurrency::Chf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpFrequency {
    Once,
    EveryTime,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAlertParams {
    /// The market data provider.
    pub source: McpSource,
    /// The provider's own symbol, as for `degenview_add_chart`.
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
    pub condition: McpCondition,
    /// The trigger price as a plain decimal string in `currency`, e.g. `"70000"` or `"0.00000278"`.
    #[schemars(length(min = 1, max = 32))]
    pub target_price: String,
    /// Defaults to USD.
    pub currency: Option<McpCurrency>,
    /// `once` (default) fires one time; `every_time` re-arms after each crossing.
    pub frequency: Option<McpFrequency>,
    /// A label shown with the alert.
    #[schemars(length(max = 200))]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAlertParams {
    /// An `alert_id` from `degenview_list_price_alerts`.
    #[schemars(length(max = 20))]
    pub alert_id: String,
    /// Only the settings given change. At least one is required.
    pub condition: Option<McpCondition>,
    /// The new trigger price, as for `degenview_create_price_alert`.
    #[schemars(length(min = 1, max = 32))]
    pub target_price: Option<String>,
    pub currency: Option<McpCurrency>,
    pub frequency: Option<McpFrequency>,
    /// The new note; an empty string clears it.
    #[schemars(length(max = 200))]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertIdParams {
    /// An `alert_id` from `degenview_list_price_alerts`.
    #[schemars(length(max = 20))]
    pub alert_id: String,
}

/// A decimal like `70000` or `0.5`: digits and at most one point, nothing else, so no sign,
/// exponent, `inf` or `NaN`. Returns it trimmed when it is a finite number above zero.
pub fn clean_price_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    let well_formed = !text.is_empty()
        && text.len() <= 32
        && text.chars().all(|c| c.is_ascii_digit() || c == '.')
        && text.matches('.').count() <= 1
        && !text.starts_with('.')
        && !text.ends_with('.');
    let value = text.parse::<f64>().ok()?;
    (well_formed && value.is_finite() && value > 0.0).then(|| text.to_owned())
}

fn clean_note(raw: &str) -> Result<String, CallToolResult> {
    let note = raw.trim();
    if note.chars().any(char::is_control) {
        return Err(invalid("note must not contain control characters"));
    }
    Ok(note.to_owned())
}

/// Whether two renderings of an alert have the same settings (timestamps aside).
fn same_settings(a: &AlertInfo, b: &AlertInfo) -> bool {
    (
        &a.condition,
        &a.target_price,
        &a.percent,
        &a.currency,
        &a.frequency,
        &a.status,
        &a.note,
    ) == (
        &b.condition,
        &b.target_price,
        &b.percent,
        &b.currency,
        &b.frequency,
        &b.status,
        &b.note,
    )
}

fn parse_alert_id(raw: &str) -> Result<u64, CallToolResult> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| invalid("alert_id must be an id returned by degenview_list_price_alerts"))
}

const SETTLE_POLLS: usize = 20;
const SETTLE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChartsParams {
    /// A `workspace_id` from `degenview_list_workspaces`.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddChartParams {
    /// A `workspace_id` from `degenview_list_workspaces`. Must be a chart workspace.
    #[schemars(length(max = 20))]
    pub workspace_id: String,
    /// The market data provider.
    pub source: McpSource,
    /// The provider's own symbol: `BTCUSDT` (binance), `BTC-USD` (coinbase), `bitcoin`
    /// (coingecko), `chain/address` (dexscreener), `AAPL` (alpaca).
    #[schemars(length(min = 1, max = 128))]
    pub symbol: String,
}

fn failure(error: &McpError) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "error": error.code(),
        "message": error.message(),
    }))
}

fn invalid(message: impl Into<String>) -> CallToolResult {
    failure(&McpError::InvalidArgument(message.into()))
}

fn parse_id(raw: &str, what: &str) -> Result<u64, CallToolResult> {
    raw.trim().parse::<u64>().map_err(|_| {
        invalid(format!(
            "{what} must be an id returned by degenview_list_workspaces"
        ))
    })
}

/// The client the HTTP guard authenticated for this request.
fn client_of(parts: &http::request::Parts) -> Result<ClientIdentity, CallToolResult> {
    parts
        .extensions
        .get::<ClientIdentity>()
        .cloned()
        .ok_or_else(|| failure(&McpError::PermissionDenied("Not authenticated".into())))
}

fn unexpected() -> CallToolResult {
    failure(&McpError::AppUnavailable(
        "Unexpected response from DegenView".into(),
    ))
}

#[derive(Clone)]
pub struct DegenViewMcp {
    bridge: BridgeClient,
    shared: Arc<McpShared>,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl DegenViewMcp {
    pub fn new(bridge: BridgeClient, shared: Arc<McpShared>) -> Self {
        Self {
            bridge,
            shared,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "degenview_list_workspaces",
        description = "List DegenView chart workspaces (one per tab). Returns each workspace's stable id, title, chart count and timeframe, and which one is active. Use a workspace_id with the other chart tools.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_workspaces(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<WorkspacesResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsRead,
                McpCommand::ListWorkspaces,
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Workspaces(workspaces)) => Ok(Json(WorkspacesResult { workspaces })),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_charts",
        description = "List the charts in one workspace: market, provider, timeframe, chart type, applied indicators and Pine scripts, and load state. Candle data is not returned.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_charts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<ListChartsParams>,
    ) -> Result<Json<ChartsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsRead,
                McpCommand::ListCharts { workspace_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Charts(charts)) => Ok(Json(charts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_indicators",
        description = "List the built-in chart indicators DegenView supports: id, where it draws, which chart types it works on, and its parameters with defaults and allowed values.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_indicators(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<IndicatorsResult>, CallToolResult> {
        client_of(&parts)?;
        Ok(Json(IndicatorsResult {
            indicators: indicator_catalog(),
        }))
    }

    #[tool(
        name = "degenview_add_indicator",
        description = "Apply a built-in indicator to a chart, or update it if already applied (re-shows it; an EMA takes a period). Each indicator exists once per chart. Returns the chart's indicators afterwards.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_indicator(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AddIndicatorParams>,
    ) -> Result<Json<AddIndicatorResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let chart_id = params
            .chart_id
            .trim()
            .parse::<u64>()
            .map_err(|_| invalid("chart_id must be an id returned by degenview_list_charts"))?;
        let ema_period = match params.inputs.and_then(|inputs| inputs.period) {
            None => None,
            Some(period) if params.indicator != McpIndicator::Ema => {
                return Err(invalid(format!(
                    "{period} given, but only ema takes a period"
                )));
            }
            Some(period) if EMA_PERIODS.contains(&(period as usize)) => Some(period as usize),
            Some(period) => {
                return Err(invalid(format!(
                    "EMA period {period} is not supported; use one of {EMA_PERIODS:?}"
                )));
            }
        };
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsIndicatorsWrite,
                McpCommand::ApplyIndicator {
                    workspace_id,
                    chart_id,
                    indicator: params.indicator.built_in(),
                    ema_period,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::IndicatorApplied(result)) => Ok(Json(result)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_list_price_alerts",
        description = "List DegenView price alerts: market, condition, target price, currency, frequency, status (active, paused, triggered), note and timestamps. Webhook endpoints and their secrets are never included. Alerts only evaluate while DegenView is running.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_price_alerts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<Json<AlertsResult>, CallToolResult> {
        let client = client_of(&parts)?;
        match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsRead,
                McpCommand::ListAlerts { only: None },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::Alerts(alerts)) => Ok(Json(alerts)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }

    #[tool(
        name = "degenview_create_price_alert",
        description = "Create a price alert that fires when a market crosses a price. Uses the same rules as the app's alert sheet; an identical existing alert is returned instead of a duplicate. The alert only evaluates while DegenView is running. Creating one may ask the user to allow notifications.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn create_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<CreateAlertParams>,
    ) -> Result<Json<CreateAlertResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let source = params.source.data_source();
        let instrument = validate_market(source, &params.symbol).map_err(invalid)?;
        let target_text = clean_price_text(&params.target_price).ok_or_else(|| {
            invalid("target_price must be a plain positive decimal such as \"70000\" or \"0.5\"")
        })?;
        let note = clean_note(&params.note.unwrap_or_default())?;
        let request = NewAlert {
            instrument,
            crosses_above: params.condition == McpCondition::CrossesAbove,
            target_text,
            currency: params
                .currency
                .map_or(AlertCurrency::Usd, McpCurrency::alert_currency),
            frequency: match params.frequency {
                Some(McpFrequency::EveryTime) => AlertFrequency::EveryTime,
                _ => AlertFrequency::Once,
            },
            note,
        };
        let mut created = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsCreate,
                McpCommand::CreateAlert(Box::new(request)),
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertCreated(created)) => created,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if created.status == "queued" {
            let id = parse_alert_id(&created.alert.alert_id)?;
            if let Some(stored) = self
                .settle(&client, Permission::AlertsCreate, id, |alerts| {
                    !alerts.is_empty()
                })
                .await
                .and_then(|alerts| alerts.into_iter().next())
            {
                created = CreateAlertResult {
                    status: "created".into(),
                    alert: stored,
                };
            }
        }
        Ok(Json(created))
    }

    #[tool(
        name = "degenview_update_price_alert",
        description = "Edit an existing price alert: its direction, trigger price, currency, frequency or note (only the settings you give change; the market cannot). A paused alert stays paused. Webhook settings are untouched. Percentage alerts only take a new frequency or note.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<UpdateAlertParams>,
    ) -> Result<Json<UpdateAlertResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let alert_id = parse_alert_id(&params.alert_id)?;
        if params.condition.is_none()
            && params.target_price.is_none()
            && params.currency.is_none()
            && params.frequency.is_none()
            && params.note.is_none()
        {
            return Err(invalid(
                "give at least one of condition, target_price, currency, frequency or note",
            ));
        }
        let target_text = match &params.target_price {
            None => None,
            Some(raw) => Some(clean_price_text(raw).ok_or_else(|| {
                invalid(
                    "target_price must be a plain positive decimal such as \"70000\" or \"0.5\"",
                )
            })?),
        };
        let note = match &params.note {
            None => None,
            Some(raw) => Some(clean_note(raw)?),
        };
        let request = AlertUpdate {
            alert_id,
            crosses_above: params.condition.map(|c| c == McpCondition::CrossesAbove),
            target_text,
            currency: params.currency.map(McpCurrency::alert_currency),
            frequency: params.frequency.map(|f| match f {
                McpFrequency::Once => AlertFrequency::Once,
                McpFrequency::EveryTime => AlertFrequency::EveryTime,
            }),
            note,
        };
        let mut updated = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsEdit,
                McpCommand::UpdateAlert(Box::new(request)),
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertUpdated(updated)) => updated,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if updated.status == "queued" {
            let expected = updated.alert.clone();
            if let Some(stored) = self
                .settle(&client, Permission::AlertsEdit, alert_id, |alerts| {
                    alerts
                        .first()
                        .is_some_and(|alert| same_settings(alert, &expected))
                })
                .await
                .and_then(|alerts| alerts.into_iter().next())
            {
                updated = UpdateAlertResult {
                    status: "updated".into(),
                    alert: stored,
                };
            }
        }
        Ok(Json(updated))
    }

    #[tool(
        name = "degenview_pause_price_alert",
        description = "Pause a price alert so it stops evaluating; its settings are kept. Pausing a paused alert changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn pause_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&parts, &params.alert_id, true).await
    }

    #[tool(
        name = "degenview_resume_price_alert",
        description = "Resume a paused or fired price alert; it judges the price afresh and fires on the next crossing. Resuming an active alert changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resume_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        self.set_alert_paused(&parts, &params.alert_id, false).await
    }

    #[tool(
        name = "degenview_delete_price_alert",
        description = "Permanently delete a price alert. This cannot be undone.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_price_alert(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AlertIdParams>,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let alert_id = parse_alert_id(&params.alert_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsDelete,
                McpCommand::DeleteAlert { alert_id },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertChanged(changed)) => changed,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if self
            .settle(&client, Permission::AlertsDelete, alert_id, |alerts| {
                alerts.is_empty()
            })
            .await
            .is_some()
        {
            changed.status = "deleted".into();
        }
        Ok(Json(changed))
    }

    #[tool(
        name = "degenview_add_chart",
        description = "Add a chart for a market to an existing chart workspace. The chart uses the workspace's timeframe and is placed by the normal layout rules. If the workspace already charts that market, that chart is returned instead of adding a duplicate.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn add_chart(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(params): Parameters<AddChartParams>,
    ) -> Result<Json<AddChartResult>, CallToolResult> {
        let client = client_of(&parts)?;
        let workspace_id = parse_id(&params.workspace_id, "workspace_id")?;
        let source = params.source.data_source();
        let instrument = validate_market(source, &params.symbol).map_err(invalid)?;
        if source == DataSource::Alpaca && !alpaca_configured().await {
            return Err(failure(&McpError::Unsupported(
                "Alpaca needs API credentials: add them in DegenView Settings > API Keys".into(),
            )));
        }
        match self
            .bridge
            .call(
                &client.id,
                Permission::ChartsCreate,
                McpCommand::AddChart {
                    workspace_id,
                    instrument,
                },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::ChartAdded(added)) => Ok(Json(added)),
            Ok(_) => Err(unexpected()),
            Err(error) => Err(failure(&error)),
        }
    }
}

impl DegenViewMcp {
    /// The alert runtime applies queued changes on its own schedule. Looks for the change to
    /// show up (briefly), so a reply can say what is true now rather than only "queued".
    /// Returns the alert's entry (empty once deleted) as soon as `done` holds.
    async fn settle(
        &self,
        client: &ClientIdentity,
        permission: Permission,
        alert_id: u64,
        done: impl Fn(&[AlertInfo]) -> bool,
    ) -> Option<Vec<AlertInfo>> {
        for _ in 0..SETTLE_POLLS {
            let reply = self
                .bridge
                .call(
                    &client.id,
                    permission,
                    McpCommand::ListAlerts {
                        only: Some(alert_id),
                    },
                    Duration::from_secs(2),
                )
                .await;
            match reply {
                Ok(McpReply::Alerts(result)) if done(&result.alerts) => return Some(result.alerts),
                Ok(_) => {}
                Err(_) => return None,
            }
            tokio::time::sleep(SETTLE_INTERVAL).await;
        }
        None
    }

    async fn set_alert_paused(
        &self,
        parts: &http::request::Parts,
        raw_id: &str,
        paused: bool,
    ) -> Result<Json<AlertChangeResult>, CallToolResult> {
        let client = client_of(parts)?;
        let alert_id = parse_alert_id(raw_id)?;
        let mut changed = match self
            .bridge
            .call(
                &client.id,
                Permission::AlertsPause,
                McpCommand::SetAlertPaused { alert_id, paused },
                REQUEST_TIMEOUT,
            )
            .await
        {
            Ok(McpReply::AlertChanged(changed)) => changed,
            Ok(_) => return Err(unexpected()),
            Err(error) => return Err(failure(&error)),
        };
        if changed.status == "queued" {
            let wanted = if paused { "paused" } else { "active" };
            if self
                .settle(&client, Permission::AlertsPause, alert_id, |alerts| {
                    alerts.first().is_some_and(|alert| alert.status == wanted)
                })
                .await
                .is_some()
            {
                changed.status = wanted.into();
            }
        }
        Ok(Json(changed))
    }
}

/// The Keychain read stays off the async worker.
async fn alpaca_configured() -> bool {
    tokio::task::spawn_blocking(|| {
        crate::service::keychain_api_keys::AlpacaCredentialsStore::shared().is_configured()
    })
    .await
    .unwrap_or(false)
}

#[tool_handler(
    router = self.tool_router,
    name = "DegenView",
    instructions = "Inspect and add charts in DegenView, a market dashboard. List workspaces first, then list or add charts by workspace_id."
)]
impl ServerHandler for DegenViewMcp {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if let Some(permission) = required_permission(&request.name) {
            let allowed = context
                .extensions
                .get::<http::request::Parts>()
                .and_then(|parts| parts.extensions.get::<ClientIdentity>())
                .is_some_and(|client| self.shared.allows(&client.id, permission));
            if !allowed {
                return Ok(failure(&McpError::PermissionDenied(format!(
                    "This credential lacks the {} permission",
                    permission.id()
                )))
                .into());
            }
        }
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let client = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ClientIdentity>());
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|tool| {
                client.is_some_and(|client| {
                    required_permission(&tool.name)
                        .is_some_and(|permission| self.shared.allows(&client.id, permission))
                })
            })
            .collect();
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            // What a client may use depends on who it is and can change at any moment.
            ttl_ms: Some(0),
            cache_scope: Some(CacheScope::Private),
        })
    }
}
