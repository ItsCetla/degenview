//! The MCP tool surface: the chart tools and the handler that guards them.
//!
//! Tools are thin adapters. They validate their arguments, then ask the app to do the work
//! through the [`bridge`](crate::mcp::bridge); no workspace or SQLite state is touched here.
//! Authorization is enforced in `call_tool` on every call — `tools/list` merely hides what the
//! caller could not use anyway.

use crate::mcp::bridge::{BridgeClient, McpCommand, McpError, McpReply, REQUEST_TIMEOUT};
use crate::mcp::state::{ClientIdentity, McpShared};
use crate::mcp::types::{AddChartResult, ChartsResult, WorkspacesResult};
use crate::model::DataSource;
use crate::model::mcp::Permission;
use crate::model::watchlist_text::validate_market;
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

pub const LIST_WORKSPACES: &str = "degenview_list_workspaces";
pub const LIST_CHARTS: &str = "degenview_list_charts";
pub const ADD_CHART: &str = "degenview_add_chart";

/// The permission a tool needs, or `None` for a name this server does not have.
pub fn required_permission(tool: &str) -> Option<Permission> {
    match tool {
        LIST_WORKSPACES | LIST_CHARTS => Some(Permission::ChartsRead),
        ADD_CHART => Some(Permission::ChartsCreate),
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
