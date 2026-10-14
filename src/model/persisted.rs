//! The per-chart state a saved layout restores. Lives in `model/` (not `workspace.rs`) so the
//! `SavedView` model can reference it without depending on the workspace.

use crate::model::pine_instance::PineInstance;
use crate::model::portfolio_card::PortfolioCard;
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
    /// An optional title shown instead of the provider id (prediction events and DEX pairs).
    #[serde(default)]
    pub title: Option<String>,
    /// Prediction markets: the choices drawn together as lines; empty for a single market.
    #[serde(default)]
    pub pm_series: Vec<super::PmSeries>,
    /// Provider artwork retained so a chart added to a watchlist keeps its image.
    #[serde(default)]
    pub image_url: Option<String>,
    /// A portfolio card (`symbol` is its key and `source` a placeholder); `None` for a market.
    #[serde(default)]
    pub card: Option<PortfolioCard>,
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
            image_url: None,
            card: None,
        }
    }

    #[test]
    fn a_card_chart_round_trips_and_an_old_row_has_none() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        let mut saved = chart();
        saved.card = Some(PortfolioCard::new(CardKind::Allocation, Some(2)));
        let encoded = serde_json::to_string(&saved).unwrap();
        let decoded: PersistedChart = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, saved);

        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("card");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.card, None);
    }

    #[test]
    fn old_chart_without_appearance_uses_swift_defaults() {
        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("appearance");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.appearance, ChartAppearance::default());
    }

    #[test]
    fn the_chart_type_and_its_settings_survive_a_save() {
        use crate::model::chart_type::{ChartType, SizeMethod};
        let mut saved = chart();
        saved.appearance.style.chart_type = ChartType::Renko;
        saved.appearance.style.renko.size.method = SizeMethod::Traditional;
        saved.appearance.style.renko.size.amount = 10.0;
        saved.appearance.style.candles.up_body = Some([9, 8, 7, 255]);
        let restored: PersistedChart =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        assert_eq!(restored.appearance.style.renko.size.amount, 10.0);
        // Switching away and back finds Renko as it was left.
        let mut style = restored.appearance.style;
        style.chart_type = ChartType::Candles;
        style.chart_type = ChartType::Renko;
        assert_eq!(style.renko.size.amount, 10.0);
    }

    #[test]
    fn an_old_appearance_without_a_style_reads_as_plain_candles() {
        let old: ChartAppearance =
            serde_json::from_str(r#"{"bullish":[1,2,3],"bearish":[4,5,6],"axis_decimals":2}"#)
                .unwrap();
        assert_eq!(old.bullish, [1, 2, 3]);
        assert_eq!(old.style, crate::model::ChartStyle::default());
        assert_eq!(old.style.chart_type, crate::model::ChartType::Candles);
    }
}
