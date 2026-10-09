use crate::model::Kline;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Entry {
    fetched: Instant,
    data: Vec<Kline>,
}

struct Cache(Mutex<HashMap<String, Entry>>);

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Cache(Mutex::new(HashMap::new())))
}

/// Fresh cached candles for a source/symbol/interval, or `None` on miss/staleness.
pub fn get(source: &str, symbol: &str, interval: &str, ttl: Duration) -> Option<Vec<Kline>> {
    let key = format!("{source}:{}:{interval}", symbol.to_lowercase());
    let guard = cache().0.lock().ok()?;
    let entry = guard.get(&key)?;
    (entry.fetched.elapsed() < ttl).then(|| entry.data.clone())
}

pub fn set(source: &str, symbol: &str, interval: &str, data: Vec<Kline>) {
    let key = format!("{source}:{}:{interval}", symbol.to_lowercase());
    if let Ok(mut guard) = cache().0.lock() {
        guard.insert(
            key,
            Entry {
                fetched: Instant::now(),
                data,
            },
        );
    }
}
