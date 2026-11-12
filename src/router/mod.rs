pub mod classifier;
pub mod circuit_breaker;
pub mod decision;

pub use classifier::*;
pub use circuit_breaker::*;
pub use decision::*;

use std::sync::Arc;
use serde_json::Value;

use crate::affinity::{AffinityType, SessionAffinityTracker, compute_prefix_hash};
use crate::config::Config;
use crate::stats::MetricsTracker;

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

    // Forwarding methods to preserve backward-compatible Router API
    pub fn estimate_tokens(body: &Value) -> u64 {
        classifier::estimate_tokens(body)
    }

    pub fn detect_orchestrator(body: &Value, session_id: &str, priority_override: Option<&str>) -> bool {
        classifier::detect_orchestrator(body, session_id, priority_override)
    }

    pub fn extract_message_text(msg: &Value) -> String {
        classifier::extract_message_text(msg)
    }

    pub fn get_last_tool_name(messages: &[Value]) -> Option<String> {
        classifier::get_last_tool_name(messages)
    }

    pub fn get_last_tool_args(messages: &[Value]) -> Option<String> {
        classifier::get_last_tool_args(messages)
    }

    pub fn is_passive_read_tool_call(messages: &[Value], tool_name: &str) -> bool {
        classifier::is_passive_read_tool_call(messages, tool_name)
    }

    pub fn is_mechanical_user_command(lower: &str) -> bool {
        classifier::is_mechanical_user_command(lower)
    }

    pub fn is_source_code_or_comment_line(line_lower: &str) -> bool {
        classifier::is_source_code_or_comment_line(line_lower)
    }

    pub fn has_real_diagnostic_signal(text: &str) -> Option<&'static str> {
        classifier::has_real_diagnostic_signal(text)
    }

    pub fn is_hard_logic_turn(body: &Value) -> (bool, String) {
        classifier::is_hard_logic_turn(body)
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
        self.decide_for_session_with_hints(body, route_override, explicit_timeout_ms, session_id, None, None).await
    }

    pub async fn decide_for_session_with_hints(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
        precomputed_tokens: Option<u64>,
        precomputed_hard_logic: Option<(bool, String)>,
    ) -> RouteDecision {
        let est_tokens = precomputed_tokens.unwrap_or_else(|| Self::estimate_tokens(body));
        let effective_mode = route_override
            .and_then(RoutingMode::parse)
            .or_else(|| RoutingMode::parse(&self.config.routing.default_route))
            .unwrap_or(RoutingMode::LocalPreferred);

        let local_only = effective_mode == RoutingMode::LocalOnly;
        let local_prefer = effective_mode == RoutingMode::LocalPreferred;
        let cloud_only = effective_mode == RoutingMode::CloudOnly;
        let fast_mode = matches!(route_override, Some("fast") | Some("low-latency"));

        // Extract system prompt prefix hash to enable warm KV cache reuse across subagents
        let prefix_hash = body
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| compute_prefix_hash(arr))
            .unwrap_or(0);

        // 1. Force Cloud if client header or configuration requested cloud-only
        if cloud_only {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: "routing mode cloud-only".to_string(),
                local_only: false,
                slot_guard: None,
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
            };
        }

        // 2. Check if circuit breaker is open (Strata is offline / failing)
        if let Some(reason) = circuit_open_reason(&self.metrics) {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason,
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

        // 4. Hybrid cognitive turn routing (escalate hard logic / diagnostics to Cloud; revert to Local for mechanical)
        if effective_mode == RoutingMode::Hybrid {
            let (is_hard, reason) = precomputed_hard_logic.unwrap_or_else(|| Self::is_hard_logic_turn(body));
            if is_hard {
                if self.metrics.is_cloud_rate_limited() && self.metrics.is_strata_online() {
                    tracing::warn!(
                        session = %session_id,
                        reason = %reason,
                        "Cognitive escalation flagged ({reason}), but Cloud provider is currently rate-limited (429 backoff). Retaining request on local Strata."
                    );
                } else {
                    return RouteDecision {
                        target: TargetProvider::Cloud,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("hybrid cognitive escalation: {reason}"),
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

        // 5. Context window threshold check
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

        // 6. Atomic local slot & token capacity acquisition with session & prefix affinity
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
        } else if queue_timeout_ms == 0 {
            0
        } else if is_orchestrator {
            30000 // 30s queue wait for orchestrators: balances swarm responsiveness with local GPU utilization
        } else if local_prefer || effective_mode == RoutingMode::Hybrid {
            queue_timeout_ms
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

        if self.metrics.is_cloud_rate_limited() && self.metrics.is_strata_online() {
            tracing::warn!(
                session = %session_id,
                "Queue wait expired ({effective_wait_ms}ms), but Cloud is currently rate-limited (429 backoff). Attempting emergency slot eviction..."
            );
            if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, true) {
                if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                    self.metrics.record_affinity(aff_type.is_hit());
                    let slot_guard = LocalSlotGuard::new(
                        self.metrics.clone(),
                        Some(self.affinity_tracker.clone()),
                        Some(slot_id),
                        est_tokens,
                    );
                    return RouteDecision {
                        target: TargetProvider::Local,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("rescued from cloud 429: acquired slot {slot_id} via emergency eviction"),
                        local_only: false,
                        slot_guard: Some(slot_guard),
                        slot_id: Some(slot_id),
                        is_affinity_hit: aff_type.is_hit(),
                        affinity_type: aff_type,
                        session_id: session_id.to_string(),
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
            slot_id: None,
            is_affinity_hit: false,
            affinity_type: AffinityType::NewSlot,
            session_id: session_id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_routing_mode_parse() {
        assert_eq!(RoutingMode::parse("local-only"), Some(RoutingMode::LocalOnly));
        assert_eq!(RoutingMode::parse("local"), Some(RoutingMode::LocalOnly));
        assert_eq!(RoutingMode::parse("LOCAL-ONLY"), Some(RoutingMode::LocalOnly));
        assert_eq!(RoutingMode::parse("local-preferred"), Some(RoutingMode::LocalPreferred));
        assert_eq!(RoutingMode::parse("prefer-local"), Some(RoutingMode::LocalPreferred));
        assert_eq!(RoutingMode::parse("hybrid"), Some(RoutingMode::Hybrid));
        assert_eq!(RoutingMode::parse("hybrid-smart"), Some(RoutingMode::Hybrid));
        assert_eq!(RoutingMode::parse("cognitive"), Some(RoutingMode::Hybrid));
        assert_eq!(RoutingMode::parse("cloud-only"), Some(RoutingMode::CloudOnly));
        assert_eq!(RoutingMode::parse("cloud"), Some(RoutingMode::CloudOnly));
        assert_eq!(RoutingMode::parse("unknown-mode"), None);
    }

    #[test]
    fn test_hard_logic_diagnostic_signals_in_tool() {
        // True diagnostic signal in active bash tool output -> hard logic escalation
        let body_err = json!({
            "messages": [
                {"role": "user", "content": "Run the test suite"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "run_command", "arguments": "{\"CommandLine\": \"cargo test\"}"}
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": "test result: FAILED. 2 passed; 1 failed;\nfailures:\n    test_circuit_breaker\n\nstatus: 500 internal server error"
                }
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_err);
        assert!(is_hard, "Failed tests with status: 500 must escalate to Cloud");
        assert!(reason.contains("diagnostic signal detected"));

        // Source code lines returning errors in passive file read -> MUST NOT escalate (stays local $0)
        let body_code = json!({
            "messages": [
                {"role": "user", "content": "Read the error handler file"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_2",
                            "type": "function",
                            "function": {"name": "read_file", "arguments": "{\"Path\": \"error.rs\"}"}
                        }
                    ]
                },
                {
                    "role": "tool",
                    "name": "read_file",
                    "content": "pub fn handle_error() {\n    panic!(\"something failed\");\n}"
                }
            ]
        });
        let (is_hard_code, reason_code) = Router::is_hard_logic_turn(&body_code);
        assert!(!is_hard_code, "Passive source code read must NOT escalate: got {}", reason_code);
    }

    #[test]
    fn test_hard_logic_user_prompts() {
        // High-cognitive prompt with keywords -> escalate to Cloud
        let body_kw = json!({
            "messages": [
                {"role": "user", "content": "Perform a threat model analysis of our auth handler"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_kw);
        assert!(is_hard);
        assert!(reason.contains("cognitive prompt keyword") || reason.contains("reasoning intent verb"));

        // Mid-session security specialist testing command -> stays local ($0)
        let body_specialist = json!({
            "messages": [
                {"role": "user", "content": "Initial audit task"},
                {"role": "assistant", "content": "Scanning routes"},
                {"role": "user", "content": "Check the cards API for IDOR vulnerability and test /api/cards endpoint"}
            ]
        });
        let (is_hard_spec, reason_spec) = Router::is_hard_logic_turn(&body_specialist);
        assert!(!is_hard_spec, "Mid-session specialist testing must stay local, got: {}", reason_spec);

        // Initial task prompt (> 60 chars) -> hard logic escalation
        let body_initial = json!({
            "messages": [
                {"role": "system", "content": "You are an AI coding assistant."},
                {"role": "user", "content": "Please implement cognitive task segregation and multi-tier routing in Ferry core keeping mechanical CLI commands on local hardware."}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_initial);
        assert!(is_hard);
        assert!(reason.contains("initial task specification"));
    }

    #[test]
    fn test_detect_orchestrator_disambiguation() {
        // 1. Specialist subagent with "root agent" mentioned in prompt -> MUST NOT be an orchestrator
        let specialist_body = json!({
            "metadata": {
                "agent_name": "Auth & JWT Specialist",
                "role": "specialist"
            },
            "messages": [
                {
                    "role": "system",
                    "content": "You are an automated security hunter specializing in JWT analysis. Report all findings back to the Root Agent once completed."
                },
                {"role": "user", "content": "Analyze /api/login.js"}
            ]
        });
        assert!(
            !Router::detect_orchestrator(&specialist_body, "session-specialist-1", None),
            "Specialist mentioning Root Agent must NOT be tagged as orchestrator"
        );

        // 2. True Root Agent coordinator -> MUST be tagged as orchestrator
        let root_body = json!({
            "metadata": {
                "agent_name": "Root Agent",
                "role": "coordinator"
            },
            "messages": [
                {
                    "role": "system",
                    "content": "You are the Root Agent coordinating the security assessment. Dispatch specialists as needed."
                },
                {"role": "user", "content": "Begin audit"}
            ]
        });
        assert!(
            Router::detect_orchestrator(&root_body, "session-root-1", None),
            "True Root Agent coordinator must be tagged as orchestrator"
        );

        // 3. Coordinator tool call (wait_for_agents / dispatch_agent) -> orchestrator
        let dispatch_body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "wait_for_agents", "arguments": "{}"}
                        }
                    ]
                }
            ]
        });
        assert!(
            Router::detect_orchestrator(&dispatch_body, "session-generic-1", None),
            "Coordinator tool calls must be tagged as orchestrator"
        );
    }

    #[tokio::test]
    async fn test_cloud_rate_limit_backoff_keeps_turns_local() {
        let mut config = Config::default();
        config.routing.default_route = "hybrid".to_string();
        config.local.max_slots = 4;
        let config = Arc::new(config);
        let metrics = Arc::new(MetricsTracker::default());
        let router = Router::new(config, metrics.clone());

        let hard_body = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Verify if the SQL injection flaw can be bypassed using hex encoding."
                }
            ]
        });

        // 1. Without cloud rate limit, hard logic escalates to Cloud
        let dec1 = router.decide_with_session(&hard_body, None, None, "test-sess-1").await;
        assert_eq!(dec1.target, TargetProvider::Cloud);

        // 2. When Cloud is marked 429 rate limited, turn stays local on Strata
        metrics.record_cloud_rate_limit(60);
        assert!(metrics.is_cloud_rate_limited());
        let dec2 = router.decide_with_session(&hard_body, None, None, "test-sess-2").await;
        assert_eq!(dec2.target, TargetProvider::Local);
        assert!(dec2.slot_guard.is_some());
    }
}
