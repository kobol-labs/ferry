pub mod condenser;
pub mod hardware;
pub mod routing;
pub mod telemetry;

pub use condenser::*;
pub use hardware::*;
pub use routing::*;
pub use telemetry::*;

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
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub affinity: AffinityConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

impl Config {
    pub fn load_from_file_or_default<P: AsRef<Path>>(path: P) -> Self {
        let mut cfg = if path.as_ref().exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[warn] failed to parse {}: {}, using defaults", path.as_ref().display(), e);
                        Self::default()
                    }
                },
                Err(e) => {
                    eprintln!("[warn] failed to read {}: {}, using defaults", path.as_ref().display(), e);
                    Self::default()
                }
            }
        } else {
            Self::default()
        };

        cfg.apply_env_overrides();
        cfg
    }

    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            local: LocalConfig::default(),
            cloud: CloudConfig::default(),
            condensation: CondenserConfig::default(),
            cache: CacheConfig::default(),
            affinity: AffinityConfig::default(),
            routing: RoutingConfig::default(),
            telemetry: TelemetryConfig::default(),
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("FERRY_BIND") { self.server.bind = v; }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_BASE") { if !v.trim().is_empty() { self.local.api_base = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_API_KEY") { if !v.trim().is_empty() { self.local.api_key = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MODEL") { if !v.trim().is_empty() { self.local.model = v; } }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_SLOTS") {
            if let Ok(n) = v.parse() { self.local.max_slots = n; }
        }
        if let Ok(v) = std::env::var("FERRY_LOCAL_MAX_CONTEXT_TOKENS") {
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
        if let Ok(v) = std::env::var("FERRY_CLOUD_API_KEY").or_else(|_| std::env::var("MINIMAX_API_KEY")) { if !v.trim().is_empty() { self.cloud.api_key = v; } }
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
        if let Ok(v) = std::env::var("FERRY_NUDGE_REPEATED_FAILURES") {
            if let Some(b) = parse_bool(&v) { self.condensation.nudge_repeated_failures = b; }
        }
        if let Ok(v) = std::env::var("FERRY_ROLLING_COMPACTION") {
            if let Some(b) = parse_bool(&v) { self.condensation.rolling_compaction = b; }
        }
        if let Ok(v) = std::env::var("FERRY_COMPACT_THRESHOLD_TOKENS") {
            if let Ok(n) = v.parse() { self.condensation.compact_threshold_tokens = n; }
        }
        if let Ok(v) = std::env::var("FERRY_COMPACT_KEEP_RECENT_TURNS") {
            if let Ok(n) = v.parse() { self.condensation.compact_keep_recent_turns = n; }
        }
        if let Ok(v) = std::env::var("FERRY_CACHE_ENABLED") {
            if let Some(b) = parse_bool(&v) { self.cache.enabled = b; }
        }
        if let Ok(v) = std::env::var("FERRY_CACHE_MAX_ENTRIES") {
            if let Ok(n) = v.parse() { self.cache.max_entries = n; }
        }
        if let Ok(v) = std::env::var("FERRY_CACHE_TTL_SECS") {
            if let Ok(n) = v.parse() { self.cache.ttl_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_DEFAULT_ROUTE") {
            if !v.trim().is_empty() { self.routing.default_route = v; }
        }
        if let Ok(v) = std::env::var("FERRY_AFFINITY_ENABLED") {
            if let Some(b) = parse_bool(&v) { self.affinity.enabled = b; }
        }
        if let Ok(v) = std::env::var("FERRY_IDLE_RECLAIM_SECS").or_else(|_| std::env::var("FERRY_AFFINITY_IDLE_RECLAIM_SECS")) {
            if let Ok(n) = v.parse() { self.affinity.idle_reclaim_secs = n; }
        }
        if let Ok(v) = std::env::var("FERRY_TELEMETRY_ENABLED") {
            if let Some(b) = parse_bool(&v) { self.telemetry.enabled = b; }
        }
        if let Ok(v) = std::env::var("FERRY_PUSHGATEWAY_URL") {
            self.telemetry.pushgateway_url = v;
            self.telemetry.enabled = true;
        }
        if let Ok(v) = std::env::var("FERRY_TELEMETRY_JOB") {
            self.telemetry.job_name = v;
        }
        if let Ok(v) = std::env::var("FERRY_TELEMETRY_INTERVAL_SECS") {
            if let Ok(n) = v.parse() { self.telemetry.push_interval_secs = n; }
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default()
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
