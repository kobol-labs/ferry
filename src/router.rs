use std::sync::Arc;
use serde_json::Value;

use crate::affinity::{AffinityType, SessionAffinityTracker, compute_prefix_hash};
use crate::config::Config;
use crate::stats::MetricsTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProvider {
    Local,
    Cloud,
}

pub struct RouteOverride {
    pub route: Option<String>,
    pub queue_timeout_ms: Option<u64>,
    pub session_id: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug)]
pub struct LocalSlotGuard {
    metrics: Option<Arc<MetricsTracker>>,
    affinity_tracker: Option<Arc<SessionAffinityTracker>>,
    slot_id: Option<usize>,
    tokens: u64,
}

impl LocalSlotGuard {
    pub fn new(
        metrics: Arc<MetricsTracker>,
        affinity_tracker: Option<Arc<SessionAffinityTracker>>,
        slot_id: Option<usize>,
        tokens: u64,
    ) -> Self {
        Self {
            metrics: Some(metrics),
            affinity_tracker,
            slot_id,
            tokens,
        }
    }
}

impl Drop for LocalSlotGuard {
    fn drop(&mut self) {
        if let (Some(ref tracker), Some(slot)) = (&self.affinity_tracker, self.slot_id) {
            tracker.release_slot(slot);
        }
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
    pub slot_id: Option<usize>,
    pub is_affinity_hit: bool,
    pub affinity_type: AffinityType,
    pub session_id: String,
}

pub struct Router {
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
    affinity_tracker: Arc<SessionAffinityTracker>,
}

impl Router {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>) -> Self {
        let affinity_tracker = Arc::new(SessionAffinityTracker::with_idle_reclaim(
            config.affinity.enabled,
            config.local.max_slots,
            config.affinity.idle_reclaim_secs,
        ));
        Self::with_affinity(config, metrics, affinity_tracker)
    }

    pub fn with_affinity(
        config: Arc<Config>,
        metrics: Arc<MetricsTracker>,
        affinity_tracker: Arc<SessionAffinityTracker>,
    ) -> Self {
        Self {
            config,
            metrics,
            affinity_tracker,
        }
    }

    pub fn affinity_tracker(&self) -> &Arc<SessionAffinityTracker> {
        &self.affinity_tracker
    }

    pub fn estimate_tokens(body: &Value) -> u64 {
        let mut total_chars: usize = 0;

        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    total_chars += content.len();
                } else if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            total_chars += text.len();
                        }
                    }
                }
                if let Some(calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                    for call in calls {
                        if let Some(func) = call.get("function") {
                            if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                total_chars += args.len();
                            }
                        }
                    }
                }
            }
        }

        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            for tool in tools {
                if let Some(func) = tool.get("function") {
                    if let Some(desc) = func.get("description").and_then(|d| d.as_str()) {
                        total_chars += desc.len();
                    }
                    if let Some(params) = func.get("parameters") {
                        total_chars += params.to_string().len();
                    }
                }
            }
        }

        crate::condenser::estimate_tokens_from_chars(total_chars)
    }

    pub fn detect_orchestrator(body: &Value, session_id: &str, priority_override: Option<&str>) -> bool {
        if let Some(prio) = priority_override {
            let p_lower = prio.to_lowercase();
            if matches!(p_lower.as_str(), "high" | "orchestrator" | "root" | "prio") {
                return true;
            }
        }

        if session_id.to_lowercase().contains("root") || session_id.to_lowercase().contains("orchestrat") {
            return true;
        }

        if let Some(user) = body.get("user").and_then(|u| u.as_str()) {
            let u_lower = user.to_lowercase();
            if u_lower.contains("root") || u_lower.contains("orchestrat") {
                return true;
            }
        }

        if let Some(meta) = body.get("metadata").and_then(|m| m.as_object()) {
            for key in &["agent_id", "agent_name", "role", "agent"] {
                if let Some(val) = meta.get(*key).and_then(|v| v.as_str()) {
                    let v_lower = val.to_lowercase();
                    if v_lower.contains("root") || v_lower.contains("orchestrat") {
                        return true;
                    }
                }
            }
        }

        // Inspect messages for orchestrator signatures (Root Agent / coordination skills)
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                    if role == "system" {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                            let c_lower = content.to_lowercase();
                            if c_lower.contains("root agent")
                                || c_lower.contains("root=true")
                                || c_lower.contains("coordination/root_agent")
                            {
                                return true;
                            }
                        }
                    }
                }
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    let c_lower = content.to_lowercase();
                    if c_lower.contains("as the root agent")
                        || c_lower.contains("wait_for_agents")
                        || c_lower.contains("list_coverage")
                    {
                        return true;
                    }
                }
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                    for tc in tool_calls {
                        if let Some(func) = tc.get("function") {
                            if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                if matches!(name, "wait_for_agents" | "list_coverage" | "list_reports" | "assign_task") {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        false
    }

    pub async fn decide(&self, body: &Value, route_override: Option<&str>) -> RouteDecision {
        self.decide_for_session(body, route_override, None, "default").await
    }

    pub async fn decide_with_timeout(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
    ) -> RouteDecision {
        self.decide_for_session(body, route_override, explicit_timeout_ms, "default").await
    }

    pub async fn decide_for_session(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
    ) -> RouteDecision {
        self.decide_with_session(body, route_override, explicit_timeout_ms, session_id).await
    }

    pub async fn decide_with_session(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
    ) -> RouteDecision {
        let est_tokens = Self::estimate_tokens(body);
        let local_only = matches!(route_override, Some("local-only") | Some("local"));
        let local_prefer = matches!(route_override, Some("local-prefer") | Some("prefer-local"));
        let cloud_only = matches!(route_override, Some("cloud-only") | Some("cloud"));
        let fast_mode = matches!(route_override, Some("fast") | Some("low-latency"));

        // Extract system prompt prefix hash to enable warm KV cache reuse across subagents
        let prefix_hash = body
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| compute_prefix_hash(arr))
            .unwrap_or(0);

        // 1. Force Cloud if client header requested cloud-only
        if cloud_only {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: "client header X-Ferry-Route requested cloud-only".to_string(),
                local_only: false,
                slot_guard: None,
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
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
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
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
                        slot_id: None,
                        is_affinity_hit: false,
                        affinity_type: AffinityType::NewSlot,
                        session_id: session_id.to_string(),
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
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
            };
        }

        // 5. Atomic local slot & token capacity acquisition with session & prefix affinity
        let is_orchestrator = Self::detect_orchestrator(body, session_id, route_override);
        if is_orchestrator {
            self.metrics.orchestrator_requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, is_orchestrator) {
            let is_hit = aff_type.is_hit();
            if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                self.metrics.record_affinity(is_hit);
                let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
                let aff_desc = match aff_type {
                    AffinityType::SessionHit => "session affinity hit",
                    AffinityType::PrefixHit => "prefix cache affinity hit",
                    AffinityType::NewSlot => "new slot assigned",
                };
                return RouteDecision {
                    target: TargetProvider::Local,
                    est_tokens,
                    is_spillover: false,
                    reason: format!("local slot {slot_id} acquired ({aff_desc}, {active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                    local_only,
                    slot_guard: Some(LocalSlotGuard::new(
                        self.metrics.clone(),
                        Some(self.affinity_tracker.clone()),
                        Some(slot_id),
                        est_tokens,
                    )),
                    slot_id: Some(slot_id),
                    is_affinity_hit: is_hit,
                    affinity_type: aff_type,
                    session_id: session_id.to_string(),
                };
            } else {
                self.affinity_tracker.release_slot(slot_id);
            }
        }

        // Adaptive Queueing: Determine wait limit based on client hints and token size
        let queue_timeout_ms = self.config.local.queue_timeout_ms;
        let effective_wait_ms = if let Some(ms) = explicit_timeout_ms {
            ms
        } else if fast_mode {
            500
        } else if local_only {
            60000
        } else if is_orchestrator {
            15000 // 15s fast-spillover for orchestrators so swarm is never blocked
        } else if local_prefer {
            45000
        } else if queue_timeout_ms == 0 {
            0
        } else if est_tokens <= max_context {
            queue_timeout_ms.clamp(5000, 120000)
        } else {
            queue_timeout_ms
        };

        if effective_wait_ms > 0 {
            let start = std::time::Instant::now();
            let wait_limit = std::time::Duration::from_millis(effective_wait_ms);
            let poll_interval = if is_orchestrator || fast_mode { 5 } else { 25 };
            let mut logged_queue_entry = false;
            let mut last_queue_heartbeat = start;
            while start.elapsed() < wait_limit {
                if !logged_queue_entry {
                    let prio_label = if is_orchestrator { "orchestrator" } else { "standard" };
                    tracing::info!(
                        session = %session_id,
                        est_tokens,
                        timeout_ms = effective_wait_ms,
                        priority = prio_label,
                        "Local slots saturated; request entered contention queue waiting for available slot"
                    );
                    logged_queue_entry = true;
                } else if last_queue_heartbeat.elapsed() >= std::time::Duration::from_secs(10) {
                    tracing::info!(
                        session = %session_id,
                        elapsed_ms = start.elapsed().as_millis(),
                        timeout_ms = effective_wait_ms,
                        "Request waiting in contention queue"
                    );
                    last_queue_heartbeat = std::time::Instant::now();
                }
                tokio::time::sleep(std::time::Duration::from_millis(poll_interval)).await;
                // Allow eviction for orchestrators immediately, or for workers after waiting 70% of timeout (capped at 15s)
                let allow_eviction = is_orchestrator || (start.elapsed().as_millis() >= (effective_wait_ms as u128 * 7 / 10).min(15000));
                if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, allow_eviction) {
                    let is_hit = aff_type.is_hit();
                    if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                        let wait_ms = start.elapsed().as_millis() as u64;
                        self.metrics.record_queue_wait(wait_ms);
                        self.metrics.record_affinity(is_hit);
                        let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                        let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
                        let aff_desc = match aff_type {
                            AffinityType::SessionHit => "session affinity hit",
                            AffinityType::PrefixHit => "prefix cache affinity hit",
                            AffinityType::NewSlot => "new slot assigned",
                        };
                        return RouteDecision {
                            target: TargetProvider::Local,
                            est_tokens,
                            is_spillover: false,
                            reason: format!("local slot {slot_id} acquired from contention queue after {wait_ms}ms ({aff_desc}, {active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                            local_only,
                            slot_guard: Some(LocalSlotGuard::new(
                                self.metrics.clone(),
                                Some(self.affinity_tracker.clone()),
                                Some(slot_id),
                                est_tokens,
                            )),
                            slot_id: Some(slot_id),
                            is_affinity_hit: is_hit,
                            affinity_type: aff_type,
                            session_id: session_id.to_string(),
                        };
                    } else {
                        self.affinity_tracker.release_slot(slot_id);
                    }
                }
            }
            if logged_queue_entry {
                let wait_ms = start.elapsed().as_millis() as u64;
                self.metrics.record_queue_wait(wait_ms);
            }
        }

        if is_orchestrator {
            self.metrics.orchestrator_spillover.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            slot_id: None,
            is_affinity_hit: false,
            affinity_type: AffinityType::NewSlot,
            session_id: session_id.to_string(),
        }
    }
}
