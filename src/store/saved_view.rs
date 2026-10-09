//! Persistence for named saved layouts, under the `saved_view` document table.

// Transient: the saved-layout controller wires these once it lands.
#![allow(dead_code)]

use crate::model::SavedView;
use crate::store::{Document, Store};

impl Document for SavedView {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

impl Store {
    pub fn saved_views(&self) -> Vec<SavedView> {
        self.documents("saved_view")
    }

    pub fn replace_saved_views(&self, views: &[SavedView]) -> rusqlite::Result<()> {
        self.replace_documents("saved_view", views)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Timeframe;

    fn view(id: u64) -> SavedView {
        SavedView {
            id,
            name: "BTC watch".into(),
            timeframe: Timeframe::Hour,
            created_at_ms: 1_000,
            charts: Vec::new(),
            columns: None,
            candle_count: 0,
            last_opened_at_ms: None,
            autosave: None,
        }
    }

    #[test]
    fn saved_views_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        store
            .replace_saved_views(&[view(7), view(9)])
            .expect("write");
        let loaded = store.saved_views();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, 7);
        assert_eq!(loaded[1].id, 9);
    }
}
