use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub local: LocalConfig,
    #[serde(default)]
    pub cloud: CloudConfig,
    #[serde(default)]
    pub condensation: CondenserConfig,
}

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

fn default_local_base() -> String { "http://192.168.9.99:8080/v1".to_string() }
fn default_local_key() -> String { "sk-strata-stevegaming".to_string() }
fn default_local_model() -> String { "qwen3.8-flash-next-iq2_xs".to_string() }
fn default_max_slots() -> usize { 2 }
fn default_max_context_tokens() -> usize { 128000 }
fn default_total_kv_token_budget() -> usize { 260000 }
fn default_adaptive_squeezing() -> bool { true }
fn default_adaptive_squeeze_ratio() -> f64 { 1.35 }
fn default_timeout_secs() -> u64 { 90 }
fn default_connect_timeout_secs() -> u64 { 3 }
fn default_circuit_breaker_threshold() -> u32 { 3 }
fn default_probe_interval_secs() -> u64 { 5 }
fn default_queue_timeout_ms() -> u64 { 3500 }

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
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
    #[serde(default = "default_cloud_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_cloud_model")]
    pub model: String,
    #[serde(default = "default_cloud_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_cloud_base() -> String { "https://api.minimax.io/v1".to_string() }
fn default_cloud_model() -> String { "MiniMax-M3".to_string() }
fn default_cloud_timeout_secs() -> u64 { 300 }

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            api_base: default_cloud_base(),
            api_key: String::new(),
            model: default_cloud_model(),
            timeout_secs: default_cloud_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondenserConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub drop_historical_reasoning: bool,
    #[serde(default = "default_true")]
    pub truncate_tool_outputs: bool,
    #[serde(default = "default_keep_recent_turns")]
    pub keep_recent_turns: usize,
    #[serde(default = "default_max_tool_chars")]
    pub max_tool_chars: usize,
    #[serde(default = "default_recent_max_tool_chars")]
    pub recent_max_tool_chars: usize,
    #[serde(default = "default_head_lines")]
    pub head_lines: usize,
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
    #[serde(default = "default_true")]
    pub strip_terminal_noise: bool,
    #[serde(default = "default_true")]
    pub deduplicate_lines: bool,
    #[serde(default = "default_true")]
    pub minify_json_outputs: bool,
    #[serde(default = "default_true")]
    pub fold_repeated_tool_failures: bool,
}

fn default_true() -> bool { true }
fn default_keep_recent_turns() -> usize { 1 }
fn default_max_tool_chars() -> usize { 2000 }
fn default_recent_max_tool_chars() -> usize { 60000 }
fn default_head_lines() -> usize { 15 }
fn default_tail_lines() -> usize { 10 }

impl Default for CondenserConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            drop_historical_reasoning: default_true(),
            truncate_tool_outputs: default_true(),
            keep_recent_turns: default_keep_recent_turns(),
            max_tool_chars: default_max_tool_chars(),
            recent_max_tool_chars: default_recent_max_tool_chars(),
            head_lines: default_head_lines(),
            tail_lines: default_tail_lines(),
            strip_terminal_noise: default_true(),
            deduplicate_lines: default_true(),
            minify_json_outputs: default_true(),
            fold_repeated_tool_failures: default_true(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            local: LocalConfig::default(),
            cloud: CloudConfig::default(),
            condensation: CondenserConfig::default(),
        }
    }
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let mut config: Config = if content.trim().is_empty() {
            Config::default()
        } else {
            toml::from_str(&content)?
        };

        config.apply_env_overrides();
        Ok(config)
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("FERRY_BIND") { if !v.trim().is_empty() { self.server.bind = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_BASE") { if !v.trim().is_empty() { self.local.api_base = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_KEY") { if !v.trim().is_empty() { self.local.api_key = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MODEL") { if !v.trim().is_empty() { self.local.model = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_SLOTS") {
            if let Ok(n) = v.parse() { self.local.max_slots = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_CONTEXT") {
            if let Ok(n) = v.parse() { self.local.max_context_tokens = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_TIMEOUT_SECS") {
            if let Ok(n) = v.parse() { self.local.timeout_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_CONNECT_TIMEOUT_SECS") {
            if let Ok(n) = v.parse() { self.local.connect_timeout_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_CIRCUIT_BREAKER_THRESHOLD") {
            if let Ok(n) = v.parse() { self.local.circuit_breaker_threshold = n; }
        }
        if let Ok(v) = std::env::var("FERRY_PROBE_INTERVAL_SECS") {
            if let Ok(n) = v.parse() { self.local.probe_interval_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_QUEUE_TIMEOUT_MS") {
            if let Ok(n) = v.parse() { self.local.queue_timeout_ms = n; }
        }
        if let Ok(v) = std::env::var("FERRY_TOTAL_KV_TOKEN_BUDGET") {
            if let Ok(n) = v.parse() { self.local.total_kv_token_budget = n; }
        }
        if let Ok(v) = std::env::var("FERRY_ADAPTIVE_SQUEEZING") {
            if let Some(b) = parse_bool(&v) { self.local.adaptive_squeezing = b; }
        }
        if let Ok(v) = std::env::var("FERRY_ADAPTIVE_SQUEEZE_RATIO") {
            if let Ok(f) = v.parse() { self.local.adaptive_squeeze_ratio = f; }
        }
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_BASE") { if !v.trim().is_empty() { self.cloud.api_base = v; } }
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_KEY") { if !v.trim().is_empty() { self.cloud.api_key = v; } }
        if let Ok(v) = std::env::var("FERRY_CLOUD_MODEL") { if !v.trim().is_empty() { self.cloud.model = v; } }
        if let Ok(v) = std::env::var("FERRY_CLOUD_TIMEOUT_SECS") {
            if let Ok(n) = v.parse() { self.cloud.timeout_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_CONDENSATION_ENABLED") {
            if let Some(b) = parse_bool(&v) { self.condensation.enabled = b; }
        }
        if let Ok(v) = std::env::var("FERRY_DROP_HISTORICAL_REASONING") {
            if let Some(b) = parse_bool(&v) { self.condensation.drop_historical_reasoning = b; }
        }
        if let Ok(v) = std::env::var("FERRY_TRUNCATE_TOOL_OUTPUTS") {
            if let Some(b) = parse_bool(&v) { self.condensation.truncate_tool_outputs = b; }
        }
        if let Ok(v) = std::env::var("FERRY_KEEP_RECENT_TURNS") {
            if let Ok(n) = v.parse() { self.condensation.keep_recent_turns = n; }
        }
        if let Ok(v) = std::env::var("FERRY_MAX_TOOL_CHARS") {
            if let Ok(n) = v.parse() { self.condensation.max_tool_chars = n; }
        }
        if let Ok(v) = std::env::var("FERRY_RECENT_MAX_TOOL_CHARS") {
            if let Ok(n) = v.parse() { self.condensation.recent_max_tool_chars = n; }
        }
        if let Ok(v) = std::env::var("FERRY_HEAD_LINES") {
            if let Ok(n) = v.parse() { self.condensation.head_lines = n; }
        }
        if let Ok(v) = std::env::var("FERRY_TAIL_LINES") {
            if let Ok(n) = v.parse() { self.condensation.tail_lines = n; }
        }
        if let Ok(v) = std::env::var("FERRY_STRIP_TERMINAL_NOISE").or_else(|_| std::env::var("FERRY_STRIP_ANSI")) {
            if let Some(b) = parse_bool(&v) { self.condensation.strip_terminal_noise = b; }
        }
        if let Ok(v) = std::env::var("FERRY_DEDUPLICATE_LINES") {
            if let Some(b) = parse_bool(&v) { self.condensation.deduplicate_lines = b; }
        }
        if let Ok(v) = std::env::var("FERRY_MINIFY_JSON") {
            if let Some(b) = parse_bool(&v) { self.condensation.minify_json_outputs = b; }
        }
        if let Ok(v) = std::env::var("FERRY_FOLD_REPEATED_TOOL_FAILURES") {
            if let Some(b) = parse_bool(&v) { self.condensation.fold_repeated_tool_failures = b; }
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
