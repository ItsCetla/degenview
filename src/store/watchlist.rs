//! Persistence and mutation for the tab's watchlists. Lists live in the `watchlist`
//! document table; per-market flags live in the `watchlist.flags` setting.

use crate::model::InstrumentId;
use crate::model::watchlist::{
    EntryId, Watchlist, WatchlistColumn, WatchlistFlag, WatchlistInstrument, WatchlistSort,
};
use crate::model::watchlist_layout::Flags;
use crate::model::watchlist_text::ImportReport;
use crate::store::{Document, SharedStore};

impl Document for Watchlist {
    fn document_id(&self) -> String {
        self.id.to_string()
    }
}

// `#[allow(dead_code)]`: wired into the workspace sidebar next; the test below
// exercises it until then.
#[allow(dead_code)]
pub struct WatchlistStore {
    store: SharedStore,
    lists: Vec<Watchlist>,
    flags: Flags,
}

#[allow(dead_code)]
impl WatchlistStore {
    pub fn new(store: SharedStore) -> Self {
        let (mut lists, flags) = {
            let store = store.lock().expect("store lock poisoned");
            (
                store.documents::<Watchlist>("watchlist"),
                store
                    .setting::<Flags>("watchlist.flags")
                    .unwrap_or_default(),
            )
        };
        if !lists.iter().any(|list| list.is_favorites) {
            let mut favorites = Watchlist::new("Favorites");
            favorites.is_favorites = true;
            lists.push(favorites);
        }
        let this = Self {
            store,
            lists,
            flags,
        };
        this.persist();
        this
    }

    pub fn lists(&self) -> &[Watchlist] {
        &self.lists
    }

    pub fn flags(&self) -> &Flags {
        &self.flags
    }

    pub fn load_failed(&self) -> bool {
        false
    }

    fn persist(&self) {
        let store = self.store.lock().expect("store lock poisoned");
        let _ = store.replace_documents("watchlist", &self.lists);
    }

    fn persist_flags(&self) {
        let store = self.store.lock().expect("store lock poisoned");
        let _ = store.set_setting("watchlist.flags", &self.flags);
    }

    fn list_mut(&mut self, id: EntryId) -> Option<&mut Watchlist> {
        self.lists.iter_mut().find(|list| list.id == id)
    }

    /// The Favorites list id (always present after construction).
    pub fn favorites(&self) -> EntryId {
        self.lists
            .iter()
            .find(|list| list.is_favorites)
            .map(|list| list.id)
            .unwrap_or_else(|| self.lists[0].id)
    }

    // MARK: Lists

    pub fn create_list(
        &mut self,
        name: &str,
        with: Option<WatchlistInstrument>,
    ) -> Option<EntryId> {
        let mut list = Watchlist::new(name);
        if let Some(item) = with {
            let _ = list.add(item, None);
        }
        let id = list.id;
        self.lists.push(list);
        self.persist();
        Some(id)
    }

    pub fn rename_list(&mut self, id: EntryId, name: &str) {
        if let Some(list) = self.list_mut(id) {
            list.name = name.to_owned();
            self.persist();
        }
    }

    pub fn duplicate_list(&mut self, id: EntryId, name: &str) -> Option<EntryId> {
        let copy = self
            .lists
            .iter()
            .find(|list| list.id == id)
            .map(|list| list.duplicated(name))?;
        let new_id = copy.id;
        self.lists.push(copy);
        self.persist();
        Some(new_id)
    }

    /// Deletes a list; Favorites never deletes.
    pub fn delete_list(&mut self, id: EntryId) {
        let Some(list) = self.lists.iter().find(|list| list.id == id) else {
            return;
        };
        if list.is_favorites {
            return;
        }
        self.lists.retain(|list| list.id != id);
        self.persist();
    }

    // MARK: Contents

    pub fn add_to_list(&mut self, list: EntryId, item: WatchlistInstrument) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.add(item, None);
            self.persist();
        }
    }

    pub fn remove_from_list(&mut self, list: EntryId, instrument: &InstrumentId) {
        if let Some(list) = self.list_mut(list)
            && list.remove_instrument(instrument)
        {
            self.persist();
        }
    }

    pub fn remove_entry(&mut self, list: EntryId, entry: EntryId) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.remove_entry(entry);
            self.persist();
        }
    }

    pub fn move_entries(&mut self, list: EntryId, ids: &[EntryId], before: Option<EntryId>) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.move_entries(ids, before);
            self.persist();
        }
    }

    pub fn move_instruments(&mut self, list: EntryId, ids: &[EntryId], section: Option<EntryId>) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.move_instruments(ids, section);
            self.persist();
        }
    }

    // MARK: Sections

    pub fn add_section(&mut self, list: EntryId, title: &str) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.add_section(title, None);
            self.persist();
        }
    }

    pub fn rename_section(&mut self, list: EntryId, section: EntryId, title: &str) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.rename_section(section, title);
            self.persist();
        }
    }

    pub fn delete_section(&mut self, list: EntryId, section: EntryId) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.delete_section(section);
            self.persist();
        }
    }

    pub fn set_section_collapsed(&mut self, list: EntryId, section: EntryId, collapsed: bool) {
        if let Some(list) = self.list_mut(list) {
            let _ = list.set_section_collapsed(section, collapsed);
            self.persist();
        }
    }

    // MARK: Display

    pub fn set_sort(&mut self, list: EntryId, sort: WatchlistSort) {
        if let Some(list) = self.list_mut(list) {
            list.display.sort = sort;
            self.persist();
        }
    }

    pub fn set_column(&mut self, list: EntryId, column: WatchlistColumn, visible: bool) {
        if let Some(list) = self.list_mut(list) {
            list.display.set_column(column, visible);
            self.persist();
        }
    }

    pub fn set_shows_description(&mut self, list: EntryId, shows: bool) {
        if let Some(list) = self.list_mut(list) {
            list.display.shows_description = shows;
            self.persist();
        }
    }

    // MARK: Flags

    pub fn set_flag(&mut self, instrument: &InstrumentId, flag: Option<WatchlistFlag>) {
        let key = instrument.key();
        match flag {
            Some(flag) => {
                self.flags.insert(key, flag);
            }
            None => {
                self.flags.remove(&key);
            }
        }
        self.persist_flags();
    }

    // MARK: Import / export

    pub fn import_into(&mut self, list: EntryId, text: &str) -> Option<ImportReport> {
        let parsed = crate::model::watchlist_text::parse(text);
        let target = self.list_mut(list)?;
        match crate::model::watchlist_text::import_into(target, &parsed) {
            Ok(report) => {
                self.persist();
                Some(report)
            }
            Err(_) => None,
        }
    }

    pub fn export(&self, list: EntryId) -> Option<String> {
        self.lists
            .iter()
            .find(|target| target.id == list)
            .map(crate::model::watchlist_text::export)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::store::Store;

    fn item(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument::from_market(DataSource::Binance, symbol, symbol)
    }

    #[test]
    fn round_trips_lists_and_flags_through_the_store() {
        let shared = SharedStore::new(std::sync::Mutex::new(
            Store::open_in_memory().expect("store"),
        ));
        {
            let mut store = WatchlistStore::new(shared.clone());
            let favorites = store.favorites();
            store.add_to_list(favorites, item("BTCUSDT"));
            let id = store
                .create_list("Meme", Some(item("DOGEUSDT")))
                .expect("list");
            store.set_flag(
                &InstrumentId::new(DataSource::Binance, "BTCUSDT"),
                Some(WatchlistFlag::Blue),
            );
            let _ = id;
        }
        let reloaded = WatchlistStore::new(shared);
        assert_eq!(reloaded.lists().len(), 2);
        let favorites = reloaded.favorites();
        let list = reloaded.lists().iter().find(|l| l.id == favorites).unwrap();
        assert!(list.contains(&InstrumentId::new(DataSource::Binance, "BTCUSDT")));
        assert!(
            reloaded
                .flags()
                .contains_key(&InstrumentId::new(DataSource::Binance, "BTCUSDT").key())
        );
    }
}
