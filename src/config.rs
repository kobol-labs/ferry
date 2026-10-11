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
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_local_name() -> String { "strata".to_string() }
fn default_local_base() -> String { "http://192.168.9.99:8080/v1".to_string() }
fn default_local_key() -> String { "sk-strata-stevegaming".to_string() }
fn default_local_model() -> String { "qwen3.8-flash-next-iq2_xs".to_string() }
fn default_max_slots() -> usize { 3 }
fn default_max_context_tokens() -> usize { 65000 }
fn default_timeout_secs() -> u64 { 90 }

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            name: default_local_name(),
            api_base: default_local_base(),
            api_key: default_local_key(),
            model: default_local_model(),
            max_slots: default_max_slots(),
            max_context_tokens: default_max_context_tokens(),
            timeout_secs: default_timeout_secs(),
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
fn default_cloud_model() -> String { "MiniMax-M3".to_string() }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondenserConfig {
    #[serde(default = "default_condensation_enabled")]
    pub enabled: bool,
    #[serde(default = "default_drop_historical_reasoning")]
    pub drop_historical_reasoning: bool,
    #[serde(default = "default_truncate_tool_outputs")]
    pub truncate_tool_outputs: bool,
    #[serde(default = "default_keep_recent_turns")]
    pub keep_recent_turns: usize,
    #[serde(default = "default_max_tool_chars")]
    pub max_tool_chars: usize,
    #[serde(default = "default_head_lines")]
    pub head_lines: usize,
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
}

fn default_condensation_enabled() -> bool { true }
fn default_drop_historical_reasoning() -> bool { true }
fn default_truncate_tool_outputs() -> bool { true }
fn default_keep_recent_turns() -> usize { 2 }
fn default_max_tool_chars() -> usize { 1500 }
fn default_head_lines() -> usize { 15 }
fn default_tail_lines() -> usize { 15 }

impl Default for CondenserConfig {
    fn default() -> Self {
        Self {
            enabled: default_condensation_enabled(),
            drop_historical_reasoning: default_drop_historical_reasoning(),
            truncate_tool_outputs: default_truncate_tool_outputs(),
            keep_recent_turns: default_keep_recent_turns(),
            max_tool_chars: default_max_tool_chars(),
            head_lines: default_head_lines(),
            tail_lines: default_tail_lines(),
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
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_BASE") { if !v.trim().is_empty() { self.cloud.api_base = v; } }
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_KEY") { if !v.trim().is_empty() { self.cloud.api_key = v; } }
        if let Ok(v) = std::env::var("FERRY_CLOUD_MODEL") { if !v.trim().is_empty() { self.cloud.model = v; } }
        if let Ok(v) = std::env::var("FERRY_CONDENSATION_ENABLED") {
            if let Ok(b) = v.parse() { self.condensation.enabled = b; }
        }
        if let Ok(v) = std::env::var("FERRY_DROP_HISTORICAL_REASONING") {
            if let Ok(b) = v.parse() { self.condensation.drop_historical_reasoning = b; }
        }
        if let Ok(v) = std::env::var("FERRY_TRUNCATE_TOOL_OUTPUTS") {
            if let Ok(b) = v.parse() { self.condensation.truncate_tool_outputs = b; }
        }
        if let Ok(v) = std::env::var("FERRY_KEEP_RECENT_TURNS") {
            if let Ok(n) = v.parse() { self.condensation.keep_recent_turns = n; }
        }
        if let Ok(v) = std::env::var("FERRY_MAX_TOOL_CHARS") {
            if let Ok(n) = v.parse() { self.condensation.max_tool_chars = n; }
        }
    }
}
