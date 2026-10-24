use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};

const ENTRY_OVERHEAD_BYTES: usize = 128;

struct Entry {
    body: Arc<Value>,
    stored_at: Instant,
    bytes: usize,
}

pub struct ResponseCache {
    state: Mutex<CacheState>,
    ttl: Duration,
    max_bytes: usize,
}

struct CacheState {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    bytes: usize,
}

impl ResponseCache {
    pub fn new(ttl_secs: u64, max_bytes: usize) -> Self {
        Self {
            state: Mutex::new(CacheState {
                entries: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            }),
            ttl: Duration::from_secs(ttl_secs),
            max_bytes,
        }
    }

    pub fn enabled(&self) -> bool {
        self.ttl > Duration::ZERO
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<(Arc<Value>, u64)>> {
        if !self.enabled() {
            return Ok(None);
        }
        let mut state = self.lock();
        let Some(entry) = state.entries.get(key) else {
            return Ok(None);
        };
        let age = entry.stored_at.elapsed();
        if age > self.ttl {
            let removed = state.entries.remove(key);
            if let Some(removed) = removed {
                state.bytes = state.bytes.saturating_sub(removed.bytes);
            }
            state.order.retain(|item| item != key);
            return Ok(None);
        }
        Ok(Some((Arc::clone(&entry.body), age.as_secs())))
    }

    #[cfg(test)]
    pub(crate) fn usage(&self) -> (usize, usize) {
        match self.state.lock() {
            Ok(state) => (state.entries.len(), state.bytes),
            Err(_) => (0, 0),
        }
    }

    pub fn put(&self, key: String, body: Arc<Value>) -> Result<()> {
        if !self.enabled() {
            return Ok(());
        }
        let bytes = serde_json::to_vec(body.as_ref())
            .map_err(|error| Error::Json {
                context: "measuring a cached response",
                source: error,
            })?
            .len()
            .saturating_add(key.len().saturating_mul(2))
            .saturating_add(ENTRY_OVERHEAD_BYTES);
        if bytes > self.max_bytes {
            return Ok(());
        }
        let mut state = self.lock();
        if let Some(replaced) = state.entries.remove(&key) {
            state.bytes = state.bytes.saturating_sub(replaced.bytes);
            state.order.retain(|item| item != &key);
        }
        state.bytes = state.bytes.saturating_add(bytes);
        state.order.push_back(key.clone());
        state.entries.insert(
            key,
            Entry {
                body,
                stored_at: Instant::now(),
                bytes,
            },
        );
        while state.bytes > self.max_bytes {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if let Some(removed) = state.entries.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(removed.bytes);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_disabled_cache_never_stores() {
        let cache = ResponseCache::new(0, 1024);
        cache
            .put("k".to_owned(), Arc::new(json!({"a": 1})))
            .expect("put");
        assert!(cache.get("k").expect("get").is_none());
    }

    #[test]
    fn the_byte_cap_evicts_the_oldest_entry() {
        let cache = ResponseCache::new(60, 500);
        let body = Arc::new(json!({"x": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}));
        cache
            .put("first".to_owned(), Arc::clone(&body))
            .expect("put");
        cache
            .put("second".to_owned(), Arc::clone(&body))
            .expect("put");
        cache.put("third".to_owned(), body).expect("put");
        assert!(cache.get("first").expect("get").is_none());
        assert!(cache.get("third").expect("get").is_some());
    }

    #[test]
    fn a_long_run_of_inserts_stays_under_the_byte_cap() {
        let cap = 4 * 1024;
        let cache = ResponseCache::new(600, cap);
        let body = Arc::new(json!({"x": "a".repeat(200)}));
        for index in 0..10_000 {
            cache
                .put(format!("key-{index}"), Arc::clone(&body))
                .expect("put");
        }
        let (entries, bytes) = cache.usage();
        assert!(
            bytes <= cap,
            "cache holds {bytes} bytes against a {cap} cap"
        );
        assert!(entries <= 32, "cache holds {entries} entries");
        assert!(cache.get("key-0").expect("get").is_none());
        assert!(cache.get("key-9999").expect("get").is_some());
    }

    #[test]
    fn an_entry_larger_than_the_cap_is_not_stored() {
        let cache = ResponseCache::new(60, 300);
        cache
            .put("small".to_owned(), Arc::new(json!({"a": 1})))
            .expect("put");
        let oversized = Arc::new(json!({"big": "a".repeat(400)}));
        cache.put("big".to_owned(), oversized).expect("put");
        assert!(
            cache.get("small").expect("get").is_some(),
            "an oversized entry must not empty the cache"
        );
        assert!(cache.get("big").expect("get").is_none());
    }

    #[test]
    fn a_poisoned_cache_lock_still_serves() {
        let cache = Arc::new(ResponseCache::new(60, 4096));
        cache
            .put("kept".to_owned(), Arc::new(json!({"a": 1})))
            .expect("put");
        let poisoner = Arc::clone(&cache);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.state.lock().expect("lock");
            panic!("poison the cache lock on purpose");
        })
        .join();
        assert!(
            cache.get("kept").expect("get").is_some(),
            "a poisoned lock must not lose the cache"
        );
        cache
            .put("after".to_owned(), Arc::new(json!({"b": 2})))
            .expect("put");
        assert!(cache.get("after").expect("get").is_some());
    }

    #[test]
    fn a_hit_reports_its_age() {
        let cache = ResponseCache::new(60, 1024);
        cache
            .put("k".to_owned(), Arc::new(json!({"a": 1})))
            .expect("put");
        let (body, age) = cache.get("k").expect("get").expect("hit");
        assert_eq!(*body, json!({"a": 1}));
        assert_eq!(age, 0);
    }
}
