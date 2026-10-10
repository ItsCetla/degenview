//! The per-chart state a saved layout restores. Lives in `model/` (not `workspace.rs`) so the
//! `SavedView` model can reference it without depending on the workspace.

use crate::model::pine_instance::PineInstance;
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
    /// The one Pine script older rows carried; read on restore as a single instance, never
    /// written any more.
    #[serde(default)]
    pub pine_source: Option<String>,
    /// The Pine scripts applied to the chart, in order; old rows decode as none.
    #[serde(default)]
    pub pine_instances: Vec<PineInstance>,
    /// The scroll-zoom candle count at save time; old rows decode as `0` (auto).
    #[serde(default)]
    pub candle_count: i64,
}

impl PersistedChart {
    /// The chart's scripts: the saved instances, or for a row from before multiple scripts the one
    /// `pine_source` as a single instance.
    pub fn instances(&self) -> Vec<PineInstance> {
        if !self.pine_instances.is_empty() {
            return self.pine_instances.clone();
        }
        self.pine_source
            .iter()
            .map(|source| PineInstance::new(1, None, "Script", source))
            .collect()
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
            drawings: Vec::new(),
            pine_source: None,
            pine_instances: Vec::new(),
            candle_count: 0,
        }
    }

    #[test]
    fn instances_come_from_the_new_field_or_migrate_the_old_source() {
        assert!(chart().instances().is_empty());
        let mut legacy = chart();
        legacy.pine_source = Some("plot(close)".into());
        let migrated = legacy.instances();
        assert_eq!(migrated.len(), 1);
        assert_eq!(
            (migrated[0].id, migrated[0].source.as_str()),
            (1, "plot(close)")
        );
        let mut modern = legacy.clone();
        modern.pine_instances = vec![
            PineInstance::new(7, None, "A", "x"),
            PineInstance::new(9, None, "B", "y"),
        ];
        assert_eq!(
            modern.instances().len(),
            2,
            "the new field wins over the legacy source"
        );
        // A row written before either field existed decodes to no scripts.
        let mut row = serde_json::to_value(chart()).unwrap();
        let object = row.as_object_mut().unwrap();
        object.remove("pine_source");
        object.remove("pine_instances");
        let decoded: PersistedChart = serde_json::from_value(row).unwrap();
        assert!(decoded.instances().is_empty());
    }
}
