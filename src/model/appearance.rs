use serde::{Deserialize, Serialize};

/// Per-chart colors and price-axis precision. The defaults match SwiftUI's dark system colors.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ChartAppearance {
    pub bullish: [u8; 3],
    pub bearish: [u8; 3],
    /// None chooses precision from the price magnitude; Some(0..=8) fixes the axis precision.
    pub axis_decimals: Option<u8>,
}

impl Default for ChartAppearance {
    fn default() -> Self {
        Self {
            bullish: [48, 209, 88],
            bearish: [255, 69, 58],
            axis_decimals: None,
        }
    }
}
