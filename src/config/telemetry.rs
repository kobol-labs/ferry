use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default = "default_telemetry_enabled")]
    pub enabled: bool,
    #[serde(default = "default_pushgateway_url")]
    pub pushgateway_url: String,
    #[serde(default = "default_push_interval_secs")]
    pub push_interval_secs: u64,
    #[serde(default = "default_job_name")]
    pub job_name: String,
}

fn default_telemetry_enabled() -> bool { false }
fn default_pushgateway_url() -> String { "http://10.10.10.101:9091".to_string() }
fn default_push_interval_secs() -> u64 { 5 }
fn default_job_name() -> String { "sentry_ferry".to_string() }

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: default_telemetry_enabled(),
            pushgateway_url: default_pushgateway_url(),
            push_interval_secs: default_push_interval_secs(),
            job_name: default_job_name(),
        }
    }
}
