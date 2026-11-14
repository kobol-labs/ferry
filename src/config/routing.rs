use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    #[serde(default = "default_default_route")]
    pub default_route: String,
}

fn default_default_route() -> String {
    "local-preferred".to_string()
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            default_route: default_default_route(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinityConfig {
    #[serde(default = "default_affinity_enabled")]
    pub enabled: bool,
    #[serde(default = "default_idle_reclaim_secs")]
    pub idle_reclaim_secs: u64,
}

fn default_affinity_enabled() -> bool { true }
fn default_idle_reclaim_secs() -> u64 { 10 }

impl Default for AffinityConfig {
    fn default() -> Self {
        Self {
            enabled: default_affinity_enabled(),
            idle_reclaim_secs: default_idle_reclaim_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cache_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_cache_ttl_secs")]
    pub ttl_secs: u64,
}

fn default_cache_enabled() -> bool { true }
fn default_cache_max_entries() -> usize { 500 }
fn default_cache_ttl_secs() -> u64 { 600 }

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: default_cache_enabled(),
            max_entries: default_cache_max_entries(),
            ttl_secs: default_cache_ttl_secs(),
        }
    }
}
