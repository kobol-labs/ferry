use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub local: LocalConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_max_slots")]
    pub max_slots: usize,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_max_slots() -> usize {
    3
}

fn default_max_context_tokens() -> usize {
    65000
}

fn default_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_cloud_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_cloud_timeout_secs() -> u64 {
    300
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
