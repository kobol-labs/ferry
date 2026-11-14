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
        Self::compute_cache_key_with_route(body, None)
    }

    pub fn compute_cache_key_with_route(body: &Value, route_override: Option<&str>) -> Option<String> {
        let is_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
        if is_stream {
            return None; // Don't cache streaming responses
        }

        // Require explicit numeric temperature <= 0.05 (OpenAI omits temperature => default 1.0)
        let temp = body.get("temperature").and_then(|t| t.as_f64())?;
        if !temp.is_finite() || !(0.0..=0.05).contains(&temp) {
            return None; // Non-deterministic requests are not cached
        }

        let messages = body.get("messages")?;
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("default");

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        model.hash(&mut hasher);
        hash_json_value(messages, &mut hasher);

        const KEY_FIELDS: &[&str] = &[
            "tools",
            "tool_choice",
            "response_format",
            "max_tokens",
            "max_completion_tokens",
            "stop",
            "seed",
            "reasoning_effort",
            "functions",
            "function_call",
        ];
        for field in KEY_FIELDS {
            field.hash(&mut hasher);
            match body.get(*field) {
                Some(val) => {
                    1u8.hash(&mut hasher);
                    hash_json_value(val, &mut hasher);
                }
                None => {
                    0u8.hash(&mut hasher);
                }
            }
        }

        let normalized_route = route_override
            .map(|r| r.trim().to_ascii_lowercase())
            .filter(|r| !r.is_empty())
            .map(|r| match crate::router::RoutingMode::parse(&r) {
                Some(crate::router::RoutingMode::LocalOnly) => "local-only".to_string(),
                Some(crate::router::RoutingMode::LocalPreferred) => "local-preferred".to_string(),
                Some(crate::router::RoutingMode::Hybrid) => "hybrid".to_string(),
                Some(crate::router::RoutingMode::CloudOnly) => "cloud-only".to_string(),
                None => r,
            });
        "route".hash(&mut hasher);
        normalized_route.hash(&mut hasher);

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
        if !self.enabled || self.max_entries == 0 || !(200..300).contains(&status) {
            return;
        }

        let mut map = self.store.write().await;
        if !map.contains_key(&key) && map.len() >= self.max_entries {
            let now = Instant::now();
            map.retain(|_, v| now.duration_since(v.created_at) < v.ttl);
            if map.len() >= self.max_entries {
                if let Some(oldest_key) = map
                    .iter()
                    .min_by_key(|(_, v)| v.created_at)
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_key);
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

    pub async fn len(&self) -> usize {
        self.store.read().await.len()
    }
}

/// Zero-allocation recursive hash of a `serde_json::Value` tree.
fn hash_json_value<H: Hasher>(val: &Value, hasher: &mut H) {
    match val {
        Value::Null => 0u8.hash(hasher),
        Value::Bool(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        Value::Number(n) => {
            2u8.hash(hasher);
            if let Some(i) = n.as_i64() {
                0u8.hash(hasher);
                i.hash(hasher);
            } else if let Some(u) = n.as_u64() {
                1u8.hash(hasher);
                u.hash(hasher);
            } else if let Some(f) = n.as_f64() {
                2u8.hash(hasher);
                f.to_bits().hash(hasher);
            }
        }
        Value::String(s) => {
            3u8.hash(hasher);
            s.hash(hasher);
        }
        Value::Array(arr) => {
            4u8.hash(hasher);
            arr.len().hash(hasher);
            for item in arr {
                hash_json_value(item, hasher);
            }
        }
        Value::Object(map) => {
            5u8.hash(hasher);
            map.len().hash(hasher);
            for (k, v) in map {
                k.hash(hasher);
                hash_json_value(v, hasher);
            }
        }
    }
}
