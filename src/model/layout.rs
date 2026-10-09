use crate::model::{DataSource, Timeframe};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One ordered column in the chart grid. Chart membership is keyed by the
/// stable chart id, not by a symbol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChartColumn {
    pub id: u64,
    pub chart_ids: Vec<u64>,
}

impl ChartColumn {
    pub fn new(id: u64, chart_ids: Vec<u64>) -> Self {
        Self { id, chart_ids }
    }

    /// Resolve persisted columns against the charts that actually exist. With no
    /// columns yet, lay the charts out as a two-column row-major grid. Stale
    /// membership is repaired without dropping charts. `next_id` mints ids for
    /// any columns created here.
    pub fn resolved(
        persisted: Option<&[ChartColumn]>,
        chart_ids: &[u64],
        next_id: &mut u64,
    ) -> Vec<ChartColumn> {
        if chart_ids.is_empty() {
            return Vec::new();
        }

        let Some(persisted) = persisted.filter(|columns| !columns.is_empty()) else {
            let count = chart_ids.len().min(2);
            let mut columns = (0..count)
                .map(|_| {
                    let id = *next_id;
                    *next_id += 1;
                    ChartColumn::new(id, Vec::new())
                })
                .collect::<Vec<_>>();
            for (index, chart_id) in chart_ids.iter().enumerate() {
                columns[index % count].chart_ids.push(*chart_id);
            }
            return columns;
        };

        let valid: HashSet<u64> = chart_ids.iter().copied().collect();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut columns: Vec<ChartColumn> = persisted
            .iter()
            .filter_map(|column| {
                let mut repaired = column.clone();
                repaired
                    .chart_ids
                    .retain(|id| valid.contains(id) && seen.insert(*id));
                (!repaired.chart_ids.is_empty()).then_some(repaired)
            })
            .collect();

        for chart_id in chart_ids.iter().copied().filter(|id| !seen.contains(id)) {
            if columns.len() < chart_ids.len().min(2) {
                let id = *next_id;
                *next_id += 1;
                columns.push(ChartColumn::new(id, vec![chart_id]));
            } else if let Some(shortest) = columns
                .iter()
                .map(|column| column.chart_ids.len())
                .enumerate()
                .min_by_key(|(index, len)| (*len, *index))
                .map(|(index, _)| index)
            {
                columns[shortest].chart_ids.push(chart_id);
            } else {
                let id = *next_id;
                *next_id += 1;
                columns.push(ChartColumn::new(id, vec![chart_id]));
            }
        }
        columns
    }
}

/// The per-chart configuration that participates in a layout fingerprint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChartConfig {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub log_scale: bool,
}

/// The part of a workspace a saved layout owns, reduced to what decides
/// "has this changed?". Zoom, focus, and watchlist edits are excluded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayoutSnapshot {
    pub timeframe: Timeframe,
    pub configs: Vec<ChartConfig>,
    pub columns: Vec<Vec<u64>>,
}

/// Grid sizing math, ported from the Swift `ChartLayout` constants.
pub struct ChartLayout;

impl ChartLayout {
    pub const GRID_MIN_COLUMN_WIDTH: f32 = 280.0;
    pub const GRID_OUTER_PADDING: f32 = 4.0;
    pub const GRID_CARD_PADDING: f32 = 4.0;

    /// Equal card height after accounting for all grid and item padding.
    pub fn grid_card_height(available: f32, card_count: usize, column_count: usize) -> f32 {
        if card_count == 0 {
            return available.max(0.0);
        }
        let column_count = column_count.max(1);
        let row_count = card_count.div_ceil(column_count);
        let padding = Self::grid_outer_inset(available, card_count, column_count) * 2.0
            + row_count as f32 * Self::grid_card_inset(available, card_count, column_count) * 2.0;
        ((available - padding) / row_count as f32).max(0.0)
    }

    pub fn grid_outer_inset(available: f32, card_count: usize, column_count: usize) -> f32 {
        Self::GRID_OUTER_PADDING * Self::grid_padding_scale(available, card_count, column_count)
    }

    pub fn grid_card_inset(available: f32, card_count: usize, column_count: usize) -> f32 {
        Self::GRID_CARD_PADDING * Self::grid_padding_scale(available, card_count, column_count)
    }

    fn grid_padding_scale(available: f32, card_count: usize, column_count: usize) -> f32 {
        if card_count == 0 {
            return 1.0;
        }
        let column_count = column_count.max(1);
        let row_count = card_count.div_ceil(column_count);
        let nominal =
            Self::GRID_OUTER_PADDING * 2.0 + row_count as f32 * Self::GRID_CARD_PADDING * 2.0;
        if nominal <= 0.0 {
            return 1.0;
        }
        (available.max(0.0) / nominal).min(1.0)
    }

    /// Total height occupied by the rows and their padding. Consumed by tests
    /// to assert the grid never overflows its available height.
    #[allow(dead_code)]
    pub fn grid_occupancy(available: f32, card_count: usize, column_count: usize) -> f32 {
        if card_count == 0 {
            return 0.0;
        }
        let column_count = column_count.max(1);
        let row_count = card_count.div_ceil(column_count);
        Self::grid_outer_inset(available, card_count, column_count) * 2.0
            + row_count as f32
                * (Self::grid_card_inset(available, card_count, column_count) * 2.0
                    + Self::grid_card_height(available, card_count, column_count))
    }

    /// A trailing column may be created only while every column remains usable.
    pub fn can_add_column(available_width: f32, current_column_count: usize) -> bool {
        let candidate = current_column_count + 1;
        if candidate == 0 {
            return false;
        }
        let usable = (available_width - Self::GRID_OUTER_PADDING * 2.0).max(0.0);
        usable / candidate as f32 >= Self::GRID_MIN_COLUMN_WIDTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved_ids(persisted: Option<&[ChartColumn]>, ids: &[u64]) -> Vec<Vec<u64>> {
        let mut next_id = 1000;
        ChartColumn::resolved(persisted, ids, &mut next_id)
            .into_iter()
            .map(|column| column.chart_ids)
            .collect()
    }

    #[test]
    fn no_persisted_columns_gives_two_column_row_major_grid() {
        let ids: Vec<u64> = (1..=5).collect();
        assert_eq!(resolved_ids(None, &ids), vec![vec![1, 3, 5], vec![2, 4]]);
    }

    #[test]
    fn resolution_removes_stale_duplicate_and_empty_membership() {
        let ids: Vec<u64> = vec![1, 2, 3, 4];
        let columns = vec![
            ChartColumn::new(10, vec![1, 2, 99]),
            ChartColumn::new(11, vec![2]),
            ChartColumn::new(12, vec![3]),
        ];
        let resolved = resolved_ids(Some(&columns), &ids);
        let flat: HashSet<u64> = resolved.iter().flatten().copied().collect();
        assert_eq!(flat, ids.iter().copied().collect());
        assert_eq!(resolved.iter().flatten().count(), ids.len());
        assert!(!resolved.iter().any(|column| column.is_empty()));
        assert!(!resolved.iter().flatten().any(|id| *id == 99));
    }

    #[test]
    fn uneven_dynamic_columns_size_for_tallest_column() {
        let height = ChartLayout::grid_card_height(500.0, 9, 3);
        assert!((height - 156.0).abs() < 0.0001);
        assert!(ChartLayout::grid_occupancy(500.0, 9, 3) <= 500.0);
    }

    #[test]
    fn responsive_column_limit_uses_minimum_usable_width() {
        assert!(ChartLayout::can_add_column(900.0, 2));
        assert!(!ChartLayout::can_add_column(839.0, 2));
    }

    #[test]
    fn layout_snapshot_round_trips() {
        let snapshot = LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![
                ChartConfig {
                    id: 1,
                    symbol: "BTCUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: false,
                },
                ChartConfig {
                    id: 2,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Binance,
                    log_scale: true,
                },
            ],
            columns: vec![vec![1], vec![2]],
        };
        let encoded = serde_json::to_string(&snapshot).expect("serialize");
        let decoded: LayoutSnapshot = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, snapshot);
    }
}
