//! The data the chart tools return. Serializable and schema-described, free of egui and HTTP,
//! built from the workspace's own summaries — never a second chart model.

use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WorkspaceInfo {
    /// Stable id: survives restarts, never reused. Pass it to the other chart tools.
    pub workspace_id: String,
    pub title: String,
    pub chart_count: usize,
    /// Whether this is the workspace whose window last had focus.
    pub active: bool,
    /// The timeframe every chart in the workspace uses (`1H`, `1D`, `1W`, `1M`, `3M`, `1Y`).
    pub timeframe: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WorkspacesResult {
    pub workspaces: Vec<WorkspaceInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorsInfo {
    pub volume: bool,
    pub rsi: bool,
    pub ema: bool,
    pub ema_period: usize,
    pub bollinger: bool,
    pub trend_flips: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInstanceInfo {
    pub instance_id: String,
    pub script_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChartInfo {
    /// Unique within its workspace.
    pub chart_id: String,
    /// Source-qualified market, e.g. `BINANCE:BTCUSDT`.
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    pub display_symbol: String,
    pub timeframe: String,
    /// `candles` or `line`.
    pub chart_type: String,
    pub indicators: IndicatorsInfo,
    pub pine_instances: Vec<PineInstanceInfo>,
    /// `loading`, `ready` or `error`.
    pub status: String,
    pub error: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChartsResult {
    pub workspace_id: String,
    pub charts: Vec<ChartInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddChartResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    pub display_symbol: String,
    /// The workspace's timeframe, which the new chart uses.
    pub timeframe: String,
    /// `created`, or `already_present` when the workspace already charted this market (that
    /// chart is returned and focused, so a retried request is harmless).
    pub status: String,
}
