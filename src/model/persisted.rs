//! The per-chart state a saved layout restores. Lives in `model/` (not `workspace.rs`) so the
//! `SavedView` model can reference it without depending on the workspace.

use crate::model::pine_instance::PineInstance;
use crate::model::{ChartAppearance, DataSource, Drawing, IndicatorConfig};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedChart {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub log_scale: bool,
    pub y_zoom: f64,
    pub indicators: IndicatorConfig,
    #[serde(default)]
    pub appearance: ChartAppearance,
    pub drawings: Vec<Drawing>,
    /// The Pine scripts applied to the chart, in order; old rows decode as none.
    #[serde(default)]
    pub pine_instances: Vec<PineInstance>,
    /// The scroll-zoom candle count at save time; old rows decode as `0` (auto).
    #[serde(default)]
    pub candle_count: i64,
    /// Prediction markets: the title shown instead of the market id.
    #[serde(default)]
    pub title: Option<String>,
    /// Prediction markets: the choices drawn together as lines; empty for a single market.
    #[serde(default)]
    pub pm_series: Vec<super::PmSeries>,
}

impl PersistedChart {
    /// The chart's scripts.
    pub fn instances(&self) -> Vec<PineInstance> {
        self.pine_instances.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart() -> PersistedChart {
        PersistedChart {
            id: 1,
            symbol: "BTCUSDT".into(),
            source: DataSource::Binance,
            log_scale: false,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance: ChartAppearance::default(),
            drawings: Vec::new(),
            pine_instances: Vec::new(),
            candle_count: 0,
            title: None,
            pm_series: Vec::new(),
        }
    }

    #[test]
    fn old_chart_without_appearance_uses_swift_defaults() {
        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("appearance");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.appearance, ChartAppearance::default());
    }
}
