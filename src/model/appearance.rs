use super::ChartStyle;
use serde::{Deserialize, Serialize};

/// Per-chart colors, price-axis precision and chart type. The defaults match SwiftUI's dark system colors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartAppearance {
    pub bullish: [u8; 3],
    pub bearish: [u8; 3],
    /// None chooses precision from the price magnitude; Some(0..=8) fixes the axis precision.
    pub axis_decimals: Option<u8>,
    /// How the bars are drawn, and each chart type's own settings; old rows read as candles.
    pub style: ChartStyle,
}

impl Default for ChartAppearance {
    fn default() -> Self {
        Self {
            bullish: [48, 209, 88],
            bearish: [255, 69, 58],
            axis_decimals: None,
            style: ChartStyle::default(),
        }
    }
}
