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

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            local: LocalConfig::default(),
            cloud: CloudConfig::default(),
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
        if let Ok(v) = std::env::var("FERRY_BIND") { self.server.bind = v; }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_BASE") { self.local.api_base = v; }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_KEY") { self.local.api_key = v; }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MODEL") { self.local.model = v; }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_SLOTS") {
            if let Ok(n) = v.parse() { self.local.max_slots = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_CONTEXT") {
            if let Ok(n) = v.parse() { self.local.max_context_tokens = n; }
        }
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_BASE") { self.cloud.api_base = v; }
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_KEY") { self.cloud.api_key = v; }
        if let Ok(v) = std::env::var("FERRY_CLOUD_MODEL") { self.cloud.model = v; }
    }
}
