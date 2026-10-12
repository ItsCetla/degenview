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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorParameterInfo {
    pub name: String,
    /// Always `integer` today.
    pub value_type: String,
    pub default: i64,
    /// The only values accepted.
    pub allowed_values: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorSpecInfo {
    /// Stable id to pass to `degenview_add_indicator`.
    pub id: String,
    pub title: String,
    pub description: String,
    /// `overlay` (drawn on the price pane) or `pane` (a pane of its own).
    pub placement: String,
    /// Chart types it works on: `candles`, `line`.
    pub chart_types: Vec<String>,
    /// Empty for indicators with fixed settings.
    pub parameters: Vec<IndicatorParameterInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorsResult {
    pub indicators: Vec<IndicatorSpecInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddIndicatorResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub indicator: String,
    /// `added`, `updated` (shown again or EMA period changed) or `already_applied`.
    pub status: String,
    /// The chart's indicators after the change.
    pub indicators: IndicatorsInfo,
}

/// The built-in indicators the app actually has, from the same list the chart menu uses.
pub fn indicator_catalog() -> Vec<IndicatorSpecInfo> {
    use crate::model::{
        BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, BuiltIn, EMA_DEFAULT_PERIOD, EMA_PERIODS,
        RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD, SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
    };
    BuiltIn::ALL
        .into_iter()
        .map(|kind| IndicatorSpecInfo {
            id: kind.id().to_owned(),
            title: kind.menu_title().to_owned(),
            description: match kind {
                BuiltIn::Volume => "Traded volume bars under the price.".to_owned(),
                BuiltIn::Rsi => format!(
                    "Relative Strength Index over {RSI_PERIOD} periods, with {RSI_OVERSOLD}/{RSI_OVERBOUGHT} levels. Fixed settings."
                ),
                BuiltIn::Ema => "Exponential moving average of closes.".to_owned(),
                BuiltIn::Bollinger => format!(
                    "Bollinger Bands: {BOLLINGER_PERIOD}-period average with bands {BOLLINGER_MULTIPLIER} standard deviations out. Fixed settings."
                ),
                BuiltIn::TrendFlips => format!(
                    "Supertrend ({SUPERTREND_PERIOD} periods, {SUPERTREND_MULTIPLIER}x ATR) marking where the trend flips. Fixed settings."
                ),
            },
            placement: if kind.is_overlay() { "overlay" } else { "pane" }.to_owned(),
            chart_types: if kind.supports_line_charts() {
                vec!["candles".to_owned(), "line".to_owned()]
            } else {
                vec!["candles".to_owned()]
            },
            parameters: if kind == BuiltIn::Ema {
                vec![IndicatorParameterInfo {
                    name: "period".to_owned(),
                    value_type: "integer".to_owned(),
                    default: EMA_DEFAULT_PERIOD as i64,
                    allowed_values: EMA_PERIODS.iter().map(|p| *p as i64).collect(),
                }]
            } else {
                Vec::new()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_exactly_the_built_in_indicators() {
        let catalog = indicator_catalog();
        let ids: Vec<_> = catalog.iter().map(|spec| spec.id.as_str()).collect();
        assert_eq!(ids, ["volume", "rsi", "ema", "bollinger", "trend_flips"]);

        let ema = catalog.iter().find(|spec| spec.id == "ema").unwrap();
        assert_eq!(ema.placement, "overlay");
        assert_eq!(ema.parameters.len(), 1);
        assert_eq!(ema.parameters[0].allowed_values, [9, 20, 50, 100, 200]);
        assert!(
            ema.parameters[0]
                .allowed_values
                .contains(&ema.parameters[0].default)
        );

        let volume = catalog.iter().find(|spec| spec.id == "volume").unwrap();
        assert_eq!(volume.placement, "pane");
        assert_eq!(volume.chart_types, ["candles"]);
        for spec in catalog.iter().filter(|spec| spec.id != "ema") {
            assert!(spec.parameters.is_empty(), "{} has no parameters", spec.id);
        }
    }
}
