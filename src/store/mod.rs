//! SQLite persistence for user-authored data, mirroring the Swift `AppDatabase`.
//!
//! Document tables hold ordered lists of JSON payloads keyed by a stable id; the
//! `setting` table holds single keyed values. Payloads stay JSON so nested models
//! can evolve through serde defaults instead of schema migrations.

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub mod import;
pub mod watchlist;

/// A store shared with the UI thread behind a mutex.
pub type SharedStore = Arc<Mutex<Store>>;

/// Opens the app's on-disk store, falling back to in-memory on any failure.
pub fn open_default() -> SharedStore {
    let store = data_dir()
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            Store::open(&dir.join("degenview.sqlite")).ok()
        })
        .or_else(|| Store::open_in_memory().ok())
        .unwrap_or_else(|| Store::open_in_memory().expect("in-memory sqlite"));
    Arc::new(Mutex::new(store))
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/com.degenview.rust"))
}

/// A row in a document table: a stable id plus an ordered JSON payload.
pub trait Document {
    fn document_id(&self) -> String;
}

const DOCUMENT_TABLES: [&str; 5] = ["favorite", "saved_view", "tab", "watchlist", "drawing"];

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        let store = Self { connection };
        store.create_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        let store = Self { connection };
        store.create_schema()?;
        Ok(store)
    }

    fn create_schema(&self) -> rusqlite::Result<()> {
        for table in DOCUMENT_TABLES {
            self.connection.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {table} (
                    id TEXT PRIMARY KEY,
                    position INTEGER NOT NULL,
                    payload TEXT NOT NULL
                );"
            ))?;
        }
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS setting (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS window_group (
                position INTEGER PRIMARY KEY,
                tab_ids TEXT NOT NULL
            );",
        )
    }

    // MARK: Documents

    pub fn documents<T: DeserializeOwned>(&self, table: &str) -> Vec<T> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT payload FROM {table} ORDER BY position"))
            .ok();
        statement
            .as_mut()
            .map(|statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| {
                        rows.filter_map(Result::ok)
                            .filter_map(|payload| serde_json::from_str(&payload).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    pub fn replace_documents<T: Serialize + Document>(
        &self,
        table: &str,
        items: &[T],
    ) -> rusqlite::Result<()> {
        self.connection
            .execute(&format!("DELETE FROM {table}"), [])?;
        let mut statement = self.connection.prepare(&format!(
            "INSERT OR REPLACE INTO {table} (id, position, payload) VALUES (?1, ?2, ?3)"
        ))?;
        for (position, item) in items.iter().enumerate() {
            let payload = serde_json::to_string(item)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            statement.execute(rusqlite::params![
                item.document_id(),
                position as i64,
                payload
            ])?;
        }
        Ok(())
    }

    #[allow(dead_code)] // watchlist/saved-view stores (Stage 6) use this
    pub fn delete_document(&self, table: &str, id: &str) -> rusqlite::Result<()> {
        self.connection
            .execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
        Ok(())
    }

    // MARK: Settings

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.connection
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> rusqlite::Result<()> {
        let payload = serde_json::to_string(value)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        self.connection.execute(
            "INSERT OR REPLACE INTO setting (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, payload],
        )?;
        Ok(())
    }

    #[allow(dead_code)] // settings UI (Stage 6) uses this to clear a key
    pub fn remove_setting(&self, key: &str) -> rusqlite::Result<()> {
        self.connection
            .execute("DELETE FROM setting WHERE key = ?1", [key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Doc {
        id: String,
        name: String,
    }

    impl Document for Doc {
        fn document_id(&self) -> String {
            self.id.clone()
        }
    }

    #[test]
    fn documents_round_trip_in_order() {
        let store = Store::open_in_memory().expect("open");
        let docs = vec![
            Doc {
                id: "a".into(),
                name: "first".into(),
            },
            Doc {
                id: "b".into(),
                name: "second".into(),
            },
        ];
        store.replace_documents("tab", &docs).expect("write");
        let loaded: Vec<Doc> = store.documents("tab");
        assert_eq!(loaded, docs);
    }

    #[test]
    fn settings_round_trip_and_remove() {
        let store = Store::open_in_memory().expect("open");
        store
            .set_setting("window_frame", &"42,42,800,600")
            .expect("write");
        assert_eq!(
            store.setting::<String>("window_frame").as_deref(),
            Some("42,42,800,600")
        );
        store.remove_setting("window_frame").expect("remove");
        assert!(store.setting::<String>("window_frame").is_none());
    }

    #[test]
    fn undecodable_rows_are_skipped() {
        let store = Store::open_in_memory().expect("open");
        store
            .connection
            .execute(
                "INSERT INTO tab (id, position, payload) VALUES ('x', 0, 'not json')",
                [],
            )
            .expect("insert bad row");
        let loaded: Vec<Doc> = store.documents("tab");
        assert!(loaded.is_empty());
    }
}
