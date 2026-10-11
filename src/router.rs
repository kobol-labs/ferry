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
    tokens: u64,
}

impl LocalSlotGuard {
    pub fn new(metrics: Arc<MetricsTracker>, tokens: u64) -> Self {
        Self {
            metrics: Some(metrics),
            tokens,
        }
    }

    pub fn disarm(&mut self) {
        self.metrics = None;
    }
}

impl Drop for LocalSlotGuard {
    fn drop(&mut self) {
        if let Some(m) = self.metrics.take() {
            m.release_local_capacity(self.tokens);
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
            crate::condenser::estimate_message_tokens(messages)
        } else {
            0
        }
    }

    pub async fn decide(&self, body: &Value, route_override: Option<&str>) -> RouteDecision {
        self.decide_with_timeout(body, route_override, None).await
    }

    pub async fn decide_with_timeout(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
    ) -> RouteDecision {
        let est_tokens = Self::estimate_tokens(body);
        let local_only = matches!(route_override, Some("local-only") | Some("local"));
        let local_prefer = matches!(route_override, Some("local-prefer") | Some("prefer-local"));
        let cloud_only = matches!(route_override, Some("cloud-only") | Some("cloud"));
        let fast_mode = matches!(route_override, Some("fast") | Some("low-latency"));

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
        let total_kv_budget = self.config.local.total_kv_token_budget as u64;

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

        // 5. Atomic local slot & token capacity acquisition with contention queue support
        let queue_timeout_ms = self.config.local.queue_timeout_ms;

        if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
            let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
            let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
            return RouteDecision {
                target: TargetProvider::Local,
                est_tokens,
                is_spillover: false,
                reason: format!("local capacity acquired ({active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                local_only,
                slot_guard: Some(LocalSlotGuard::new(self.metrics.clone(), est_tokens)),
            };
        }

        // Adaptive Queueing: Determine wait limit based on client hints and token size
        let effective_wait_ms = if let Some(ms) = explicit_timeout_ms {
            ms
        } else if fast_mode {
            500
        } else if local_only {
            30000
        } else if local_prefer {
            25000
        } else if est_tokens <= max_context {
            queue_timeout_ms.max(15000)
        } else {
            queue_timeout_ms
        };

        if effective_wait_ms > 0 {
            let start = std::time::Instant::now();
            let wait_limit = std::time::Duration::from_millis(effective_wait_ms);
            while start.elapsed() < wait_limit {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                    let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                    let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
                    return RouteDecision {
                        target: TargetProvider::Local,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("local capacity acquired from contention queue ({active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                        local_only,
                        slot_guard: Some(LocalSlotGuard::new(self.metrics.clone(), est_tokens)),
                    };
                }
            }
        }

        let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
        let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
        RouteDecision {
            target: TargetProvider::Cloud,
            est_tokens,
            is_spillover: true,
            reason: format!("local capacity saturated ({active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens) -> cloud spillover"),
            local_only,
            slot_guard: None,
        }
    }
}
