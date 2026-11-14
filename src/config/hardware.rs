use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
}

fn default_bind() -> String {
    "0.0.0.0:4000".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default = "default_local_name")]
    pub name: String,
    #[serde(default = "default_local_base")]
    pub api_base: String,
    #[serde(default = "default_local_key")]
    pub api_key: String,
    #[serde(default = "default_local_model")]
    pub model: String,
    #[serde(default = "default_max_slots")]
    pub max_slots: usize,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "default_total_kv_token_budget")]
    pub total_kv_token_budget: usize,
    #[serde(default = "default_adaptive_squeezing")]
    pub adaptive_squeezing: bool,
    #[serde(default = "default_adaptive_squeeze_ratio")]
    pub adaptive_squeeze_ratio: f64,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,
    #[serde(default = "default_probe_interval_secs")]
    pub probe_interval_secs: u64,
    #[serde(default = "default_queue_timeout_ms")]
    pub queue_timeout_ms: u64,
}

fn default_local_name() -> String { "strata".to_string() }
fn default_local_base() -> String { "http://127.0.0.1:8080/v1".to_string() }
fn default_local_key() -> String { String::new() }
fn default_local_model() -> String { "qwen3.8-flash-next-unsloth-ud-iq4_xs".to_string() }
pub fn default_max_slots() -> usize { 4 }
fn default_max_context_tokens() -> usize { 256000 }
fn default_total_kv_token_budget() -> usize { 1024000 }
fn default_adaptive_squeezing() -> bool { true }
fn default_adaptive_squeeze_ratio() -> f64 { 1.35 }
fn default_timeout_secs() -> u64 { 90 }
fn default_connect_timeout_secs() -> u64 { 10 }
fn default_circuit_breaker_threshold() -> u32 { 3 }
fn default_probe_interval_secs() -> u64 { 5 }
fn default_queue_timeout_ms() -> u64 { 15000 }

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            name: default_local_name(),
            api_base: default_local_base(),
            api_key: default_local_key(),
            model: default_local_model(),
            max_slots: default_max_slots(),
            max_context_tokens: default_max_context_tokens(),
            total_kv_token_budget: default_total_kv_token_budget(),
            adaptive_squeezing: default_adaptive_squeezing(),
            adaptive_squeeze_ratio: default_adaptive_squeeze_ratio(),
            timeout_secs: default_timeout_secs(),
            connect_timeout_secs: default_connect_timeout_secs(),
            circuit_breaker_threshold: default_circuit_breaker_threshold(),
            probe_interval_secs: default_probe_interval_secs(),
            queue_timeout_ms: default_queue_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    #[serde(default = "default_cloud_name")]
    pub name: String,
    #[serde(default = "default_cloud_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_cloud_model")]
    pub model: String,
    #[serde(default = "default_cloud_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_cloud_name() -> String { "minimax".to_string() }
fn default_cloud_base() -> String { "https://api.minimax.io/v1".to_string() }
fn default_cloud_model() -> String { "MiniMax-M3.1-Flash".to_string() }
fn default_cloud_timeout_secs() -> u64 { 300 }

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            name: default_cloud_name(),
            api_base: default_cloud_base(),
            api_key: String::new(),
            model: default_cloud_model(),
            timeout_secs: default_cloud_timeout_secs(),
        }
    }
}
