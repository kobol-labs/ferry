use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct CachedResponse {
    pub body_bytes: bytes::Bytes,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub created_at: Instant,
    pub ttl: Duration,
}

pub struct ResponseCache {
    store: RwLock<HashMap<String, CachedResponse>>,
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    max_entries: usize,
    default_ttl: Duration,
    enabled: bool,
}

impl ResponseCache {
    pub fn new(enabled: bool, max_entries: usize, default_ttl_secs: u64) -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            max_entries,
            default_ttl: Duration::from_secs(default_ttl_secs),
            enabled,
        }
    }

    pub fn compute_cache_key(body: &Value) -> Option<String> {
        let is_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
        if is_stream {
            return None; // Don't cache streaming responses
        }

        let temp = body.get("temperature").and_then(|t| t.as_f64()).unwrap_or(0.0);
        if temp > 0.05 {
            return None; // Non-deterministic requests are not cached
        }

        let messages = body.get("messages")?;
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("default");
        let tools = body.get("tools");

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        model.hash(&mut hasher);
        messages.to_string().hash(&mut hasher);
        if let Some(t) = tools {
            t.to_string().hash(&mut hasher);
        }

        Some(format!("{:016x}", hasher.finish()))
    }

    pub async fn get(&self, key: &str) -> Option<CachedResponse> {
        if !self.enabled {
            return None;
        }

        let map = self.store.read().await;
        if let Some(entry) = map.get(key) {
            if entry.created_at.elapsed() < entry.ttl {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Some(entry.clone());
            }
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub async fn set(&self, key: String, body_bytes: bytes::Bytes, status: u16, headers: Vec<(String, String)>) {
        if !self.enabled {
            return;
        }

        let mut map = self.store.write().await;
        if map.len() >= self.max_entries {
            let now = Instant::now();
            map.retain(|_, v| now.duration_since(v.created_at) < v.ttl);
            if map.len() >= self.max_entries {
                if let Some(first_key) = map.keys().next().cloned() {
                    map.remove(&first_key);
                }
            }
        }

        map.insert(
            key,
            CachedResponse {
                body_bytes,
                status,
                headers,
                created_at: Instant::now(),
                ttl: self.default_ttl,
            },
        );
    }
}
