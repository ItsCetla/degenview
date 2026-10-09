//! The per-chart state a saved layout restores. Lives in `model/` (not `workspace.rs`) so the
//! `SavedView` model can reference it without depending on the workspace.

use crate::model::{DataSource, Drawing, IndicatorConfig};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedChart {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub log_scale: bool,
    pub y_zoom: f64,
    pub indicators: IndicatorConfig,
    pub drawings: Vec<Drawing>,
    /// A Pine script applied to the chart; old rows decode as `None`.
    #[serde(default)]
    pub pine_source: Option<String>,
    /// The scroll-zoom candle count at save time; old rows decode as `0` (auto).
    #[serde(default)]
    pub candle_count: i64,
}
