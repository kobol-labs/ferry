use crate::config::Config;
use crate::stats::MetricsTracker;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetProvider {
    Local,
    Cloud,
}

#[derive(Debug)]
pub struct LocalSlotGuard {
    metrics: Option<Arc<MetricsTracker>>,
}

impl LocalSlotGuard {
    pub fn new(metrics: Arc<MetricsTracker>) -> Self {
        Self {
            metrics: Some(metrics),
        }
    }

    pub fn disarm(&mut self) {
        self.metrics = None;
    }
}

impl Drop for LocalSlotGuard {
    fn drop(&mut self) {
        if let Some(m) = self.metrics.take() {
            m.dec_local();
        }
    }
}

#[derive(Debug)]
pub struct RouteDecision {
    pub target: TargetProvider,
    pub est_tokens: u64,
    pub is_spillover: bool,
    pub reason: String,
    pub local_only: bool,
    pub slot_guard: Option<LocalSlotGuard>,
}

pub struct Router {
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
}

impl Router {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>) -> Self {
        Self { config, metrics }
    }

    pub fn estimate_tokens(body: &Value) -> u64 {
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let char_count = crate::condenser::count_message_chars(messages);
            (char_count as f64 / 3.0).ceil() as u64
        } else {
            0
        }
    }

    pub async fn decide(&self, body: &Value, route_override: Option<&str>) -> RouteDecision {
        let est_tokens = Self::estimate_tokens(body);
        let local_only = matches!(route_override, Some("local-only") | Some("local"));
        let cloud_only = matches!(route_override, Some("cloud-only") | Some("cloud"));

        // 1. Force Cloud if client header requested cloud-only
        if cloud_only {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: "client header X-Ferry-Route requested cloud-only".to_string(),
                local_only: false,
                slot_guard: None,
            };
        }

        // 2. Check if circuit breaker is open (Strata is offline / failing)
        if self.metrics.is_circuit_open() {
            let failures = self.metrics.consecutive_local_failures.load(std::sync::atomic::Ordering::Relaxed);
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: format!("circuit breaker open (Strata offline, {failures} failures)"),
                local_only,
                slot_guard: None,
            };
        }

        // 3. Check if the client explicitly requested the cloud model (when not local_only)
        if !local_only {
            if let Some(requested_model) = body.get("model").and_then(|m| m.as_str()) {
                if requested_model == self.config.cloud.model || requested_model == "cloud" {
                    return RouteDecision {
                        target: TargetProvider::Cloud,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("explicitly requested cloud model ({requested_model})"),
                        local_only: false,
                        slot_guard: None,
                    };
                }
            }
        }

        let max_context = self.config.local.max_context_tokens as u64;
        let max_slots = self.config.local.max_slots;

        // 4. Context window threshold check
        if est_tokens > max_context {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: format!("context tokens ({est_tokens}) > local max ({max_context})"),
                local_only,
                slot_guard: None,
            };
        }

        // 5. Atomic local slot acquisition with contention queue support
        let queue_timeout_ms = self.config.local.queue_timeout_ms;

        if self.metrics.try_acquire_local_slot(max_slots) {
            let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
            return RouteDecision {
                target: TargetProvider::Local,
                est_tokens,
                is_spillover: false,
                reason: format!("local slot acquired ({active}/{max_slots})"),
                local_only,
                slot_guard: Some(LocalSlotGuard::new(self.metrics.clone())),
            };
        }

        if queue_timeout_ms > 0 {
            let start = std::time::Instant::now();
            let wait_limit = std::time::Duration::from_millis(queue_timeout_ms);
            while start.elapsed() < wait_limit {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                if self.metrics.try_acquire_local_slot(max_slots) {
                    let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                    return RouteDecision {
                        target: TargetProvider::Local,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("local slot acquired from contention queue ({active}/{max_slots})"),
                        local_only,
                        slot_guard: Some(LocalSlotGuard::new(self.metrics.clone())),
                    };
                }
            }
        }

        let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
        RouteDecision {
            target: TargetProvider::Cloud,
            est_tokens,
            is_spillover: true,
            reason: format!("local slots saturated ({active}/{max_slots}) -> cloud spillover"),
            local_only,
            slot_guard: None,
        }
    }
}
