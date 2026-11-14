use axum::{
    extract::Json,
    http::StatusCode,
    routing::{get, post},
    Router as AxumRouter,
};
use ferry::affinity::{SessionAffinityTracker, compute_prefix_hash};
use ferry::cache::ResponseCache;
use ferry::config::Config;
use ferry::router::{Router, TargetProvider};
use ferry::stats::MetricsTracker;
use ferry::upstream::UpstreamClient;
use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::Mutex;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ============================================================================
// AC 1: Cloud-429 Emergency Eviction Slot Leak & Atomic Token Budget CAS
// ============================================================================

#[tokio::test]
async fn test_cloud_429_emergency_eviction_releases_slot_on_kv_budget_failure() {
    let mut cfg = Config::default();
    cfg.local.max_slots = 2;
    cfg.local.max_context_tokens = 2000;
    cfg.local.total_kv_token_budget = 500;
    cfg.local.queue_timeout_ms = 10;

    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics.clone());

    // Occupy ~400 tokens on slot 0 so slot 1 is idle, but remaining KV budget is only 100 tokens
    let first_prompt = "a".repeat(1400); // ~400 tokens
    let body1 = json!({
        "messages": [{ "role": "user", "content": first_prompt }]
    });
    let dec1 = router
        .decide_for_session_with_hints(&body1, Some("local-preferred"), Some(0), "sess-1", Some(400), None)
        .await;
    assert_eq!(dec1.target, TargetProvider::Local);
    assert_eq!(dec1.slot_id, Some(0));

    // Mark Cloud rate-limited (429 cooldown active) while Strata is online
    metrics.record_cloud_rate_limit(60);
    assert!(metrics.is_cloud_rate_limited());
    assert!(metrics.is_strata_online());

    // Request 2 needs 200 tokens -> 400 + 200 = 600 > 500 total_kv_token_budget.
    // Queue wait expires -> enters Cloud-429 emergency eviction -> acquires slot 1 -> fails KV budget!
    let body2 = json!({
        "messages": [{ "role": "user", "content": "b".repeat(700) }]
    });
    let dec2 = router
        .decide_for_session_with_hints(&body2, Some("local-only"), Some(15), "sess-2", Some(200), None)
        .await;

    assert_eq!(dec2.target, TargetProvider::Cloud);
    assert!(dec2.local_only, "local_only flag must be preserved on spillover");
    assert!(
        !router.affinity_tracker().is_slot_busy(1),
        "Slot 1 MUST be released when try_acquire_local_capacity fails on Cloud-429 emergency path"
    );

    // Drop dec1's guard so KV budget frees up; now emergency rescue with local-only must succeed and keep local_only = true
    drop(dec1);
    let _g0 = router.affinity_tracker().try_acquire_slot_opt("occupy-0", 111, true);
    let _g1 = router.affinity_tracker().try_acquire_slot_opt("occupy-1", 222, true);
    router.affinity_tracker().release_slot(0);
    router.affinity_tracker().release_slot(1);

    // Occupy both slots briefly, release during/after or test emergency rescue directly by holding active_local_slots = 2 then releasing right as queue ends:
    // Or simpler: test local_only propagation when emergency rescue succeeds (occupy lease with allow_eviction=false, explicit_timeout=1ms -> wait, initial try uses is_orchestrator=false so allow_eviction=false!)
    let mut cfg_rescue = Config::default();
    cfg_rescue.local.max_slots = 1;
    cfg_rescue.local.max_context_tokens = 2000;
    cfg_rescue.local.total_kv_token_budget = 2000;
    cfg_rescue.affinity.idle_reclaim_secs = 60;
    let metrics_rescue = Arc::new(MetricsTracker::new());
    metrics_rescue.record_cloud_rate_limit(60);
    let router_rescue = Router::new(Arc::new(cfg_rescue), metrics_rescue);

    // Warm lease on slot 0 held by sess-owner (released, idle, within 60s lease)
    let (s0, _) = router_rescue
        .affinity_tracker()
        .try_acquire_slot_opt("sess-owner", 999, true)
        .unwrap();
    router_rescue.affinity_tracker().release_slot(s0);

    // Non-orchestrator with explicit_timeout_ms = 0 skips queue and hits Cloud-429 emergency eviction!
    let rescue_dec = router_rescue
        .decide_for_session_with_hints(&body2, Some("local-only"), Some(0), "worker-sess", Some(200), None)
        .await;
    assert_eq!(rescue_dec.target, TargetProvider::Local);
    assert!(
        rescue_dec.local_only,
        "Emergency Cloud-429 rescue must propagate local_only instead of hardcoding false"
    );
}

#[test]
fn test_try_acquire_local_capacity_concurrent_no_token_budget_oversubscription() {
    for _ in 0..50 {
        let metrics = Arc::new(MetricsTracker::new());
        let max_slots = 16;
        let max_budget = 900u64;
        let per_req_tokens = 300u64; // At most 3 callers (900 / 300) may hold capacity simultaneously
        let threads = 24;
        let barrier = Arc::new(Barrier::new(threads));
        let peak_tokens = Arc::new(AtomicU64::new(0));
        let acquired_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(threads);
        for _ in 0..threads {
            let m = metrics.clone();
            let b = barrier.clone();
            let peak = peak_tokens.clone();
            let count = acquired_count.clone();
            handles.push(std::thread::spawn(move || {
                b.wait();
                if m.try_acquire_local_capacity(per_req_tokens, max_slots, max_budget) {
                    let cur = m.active_local_tokens.load(Ordering::SeqCst);
                    peak.fetch_max(cur, Ordering::SeqCst);
                    count.fetch_add(1, Ordering::SeqCst);
                    std::thread::yield_now();
                    m.release_local_capacity(per_req_tokens);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert!(
            peak_tokens.load(Ordering::SeqCst) <= max_budget,
            "Concurrent callers oversubscribed max_token_budget: peak={} > {}",
            peak_tokens.load(Ordering::SeqCst),
            max_budget
        );
        assert_eq!(metrics.active_local_tokens.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.active_local_slots.load(Ordering::SeqCst), 0);
        assert!(acquired_count.load(Ordering::SeqCst) >= 1);
    }
}

// ============================================================================
// AC 2: Multi-byte UTF-8 Boundary at 8192 & Multi-part Array Prefix/Orchestrator
// ============================================================================

#[test]
fn test_compute_prefix_hash_multibyte_utf8_at_8192_boundary_no_panic() {
    // '—' is 3 bytes (0xE2 0x80 0x94). Placing it at byte 8190..8193 straddles index 8192!
    let mut s_emdash = "a".repeat(8190);
    s_emdash.push('—');
    s_emdash.push_str("trailing text after emdash");
    assert!(s_emdash.len() > 8192);
    assert!(!s_emdash.is_char_boundary(8192));

    let msgs_emdash = vec![json!({ "role": "system", "content": s_emdash })];
    let h1 = compute_prefix_hash(&msgs_emdash);
    assert_ne!(h1, 0);

    // '🦀' is 4 bytes (0xF0 0x9F 0xA6 0x80). Placing it at byte 8191..8195 straddles index 8192!
    let mut s_crab = "b".repeat(8191);
    s_crab.push('🦀');
    s_crab.push_str("trailing crab");
    assert!(s_crab.len() > 8192);
    assert!(!s_crab.is_char_boundary(8192));

    let msgs_crab = vec![json!({ "role": "system", "content": s_crab })];
    let h2 = compute_prefix_hash(&msgs_crab);
    assert_ne!(h2, 0);
    assert_ne!(h1, h2);
}

#[test]
fn test_compute_prefix_hash_and_orchestrator_multipart_array_content() {
    let msgs_a = vec![json!({
        "role": "system",
        "content": [
            { "type": "text", "text": "You are Security Auditor Alpha inspecting JWT tokens." }
        ]
    })];
    let msgs_b = vec![json!({
        "role": "system",
        "content": [
            { "type": "text", "text": "You are Performance Profiler Beta analyzing SQL plans." }
        ]
    })];

    let hash_a = compute_prefix_hash(&msgs_a);
    let hash_b = compute_prefix_hash(&msgs_b);
    assert_ne!(hash_a, 0);
    assert_ne!(hash_b, 0);
    assert_ne!(
        hash_a, hash_b,
        "Distinct multi-part array system prompts must produce distinct prefix hashes"
    );

    // Multi-part array content with multi-byte UTF-8 straddling byte 8192 across parts
    let mut part1 = "x".repeat(8190);
    part1.push('🦀');
    let msgs_multi_utf8 = vec![json!({
        "role": "system",
        "content": [
            { "type": "text", "text": part1 },
            { "type": "text", "text": "second part" }
        ]
    })];
    let hash_utf8 = compute_prefix_hash(&msgs_multi_utf8);
    assert_ne!(hash_utf8, 0);

    // detect_orchestrator with multi-part array content
    let root_array_body = json!({
        "messages": [
            {
                "role": "system",
                "content": [
                    { "type": "text", "text": "You are the Root Agent coordinating all security sub-tasks." }
                ]
            },
            { "role": "user", "content": [{ "type": "text", "text": "Start scan" }] }
        ]
    });
    assert!(
        Router::detect_orchestrator(&root_array_body, "sess-1", None),
        "Root Agent with multi-part array system prompt must be detected as orchestrator"
    );

    let specialist_array_body = json!({
        "messages": [
            {
                "role": "system",
                "content": [
                    { "type": "text", "text": "You are a specialist subagent. Report findings to the Root Agent." }
                ]
            }
        ]
    });
    assert!(
        !Router::detect_orchestrator(&specialist_array_body, "sess-2", None),
        "Specialist with multi-part array system prompt must NOT be detected as orchestrator"
    );
}

// ============================================================================
// AC 3: SessionAffinityTracker PrefixHit Lease Protection
// ============================================================================

#[test]
fn test_prefix_hit_honors_allow_eviction_false_within_active_lease() {
    // 1-slot tracker with 60s lease: allow_eviction=false must NOT steal warm slot from active orchestrator
    let tracker_single = SessionAffinityTracker::with_idle_reclaim(true, 1, 60);
    let shared_prefix = 0xCAFE_BABE_1234;

    let (slot_orch, _) = tracker_single
        .try_acquire_slot_opt("orchestrator", shared_prefix, true)
        .unwrap();
    assert_eq!(slot_orch, 0);
    tracker_single.release_slot(slot_orch);

    let steal_attempt = tracker_single.try_acquire_slot_opt("worker-1", shared_prefix, false);
    assert!(
        steal_attempt.is_none(),
        "allow_eviction=false must NOT steal an actively leased slot via PrefixHit"
    );
    assert_eq!(tracker_single.get_affinity("orchestrator"), Some(0));

    // 2-slot tracker: slot 0 leased by orchestrator (warm), slot 1 unleased.
    // Worker with allow_eviction=false and matching prefix must get slot 1 (NewSlot), leaving slot 0 for orchestrator!
    let tracker_dual = SessionAffinityTracker::with_idle_reclaim(true, 2, 60);
    let (s0, _) = tracker_dual
        .try_acquire_slot_opt("orchestrator", shared_prefix, true)
        .unwrap();
    assert_eq!(s0, 0);
    tracker_dual.release_slot(s0);

    let (worker_slot, worker_aff) = tracker_dual
        .try_acquire_slot_opt("worker-1", shared_prefix, false)
        .expect("Unassigned slot 1 should be allocated");
    assert_eq!(worker_slot, 1, "Worker with !allow_eviction must take unassigned slot 1 instead of stealing slot 0");
    assert_eq!(worker_aff, ferry::affinity::AffinityType::NewSlot);
    tracker_dual.release_slot(worker_slot);

    // Orchestrator still owns slot 0 and gets SessionHit
    let (orch_turn2, orch_aff2) = tracker_dual
        .try_acquire_slot_opt("orchestrator", shared_prefix, true)
        .unwrap();
    assert_eq!(orch_turn2, 0);
    assert_eq!(orch_aff2, ferry::affinity::AffinityType::SessionHit);
    tracker_dual.release_slot(orch_turn2);
}

// ============================================================================
// AC 4: local-only Never Calls Cloud + Cloud-429 Never Bypasses Slots / Ping-Pongs
// ============================================================================

#[tokio::test]
async fn test_local_only_never_calls_cloud_on_500_or_connection_refused() {
    let cloud_hits = Arc::new(AtomicUsize::new(0));
    let cloud_hits_clone = cloud_hits.clone();

    let cloud_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move || {
            let hits = cloud_hits_clone.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                (StatusCode::OK, Json(json!({ "choices": [{ "message": { "content": "LEAKED TO CLOUD" } }] })))
            }
        }),
    );
    let cloud_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(cloud_listener, cloud_app).await;
    });

    // 1. Local upstream returns HTTP 500
    let local_500_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "GPU OOM error") }),
    );
    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(local_listener, local_500_app).await;
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}/v1", local_addr);
    cfg.cloud.api_base = format!("http://{}/v1", cloud_addr);
    cfg.routing.default_route = "local-only".to_string();

    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let cache = Arc::new(ResponseCache::new(false, 16, 60));
    let router = Router::new(config.clone(), metrics.clone());
    let upstream = UpstreamClient::new(config.clone(), metrics.clone(), cache.clone());

    let body = json!({
        "model": "ferry",
        "stream": false,
        "messages": [{ "role": "user", "content": "sensitive local-only secret" }]
    });

    // Send 4 consecutive local-only 500 requests:
    // Requests 1..=3 hit local 500 and trip circuit_open = true at threshold 3;
    // Request #4 receives RouteDecision { target: Cloud, local_only: true } and MUST return 503 with cloud_hits == 0!
    for req_num in 1..=4 {
        let dec = router.decide(&body, Some("local-only")).await;
        assert!(dec.local_only);
        if req_num <= 3 {
            assert_eq!(dec.target, TargetProvider::Local);
        } else {
            assert!(metrics.is_circuit_open(), "Circuit breaker should be open by request #4");
            assert_eq!(dec.target, TargetProvider::Cloud);
        }

        let resp = upstream.forward_chat(body.clone(), dec, req_num, 0, 0.0, None).await;
        if req_num < 4 {
            assert!(
                matches!(resp.status(), StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE),
                "Request #{req_num}: expected 502 or 503 when local returns 500 under local-only, got {}",
                resp.status()
            );
        } else {
            assert_eq!(
                resp.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "Request #4 (circuit open + local-only) MUST return 503 Service Unavailable"
            );
            assert_eq!(
                resp.headers().get("x-ferry-provider").and_then(|v| v.to_str().ok()),
                Some("local")
            );
            assert_eq!(
                resp.headers().get("x-ferry-failover").and_then(|v| v.to_str().ok()),
                Some("false")
            );
            let err_bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
            let err_json: Value = serde_json::from_slice(&err_bytes).unwrap();
            assert_eq!(err_json["error"]["code"], "local_unavailable");
        }
        assert_eq!(
            cloud_hits.load(Ordering::SeqCst),
            0,
            "CRITICAL: local-only leaked request #{req_num} to Cloud!"
        );
        assert_eq!(metrics.cloud_requests.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.active_local_slots.load(Ordering::SeqCst), 0);
    }

    // 2. Local upstream refuses connection (unbound port)
    let dead_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener); // Close immediately so connect fails with ECONNREFUSED

    let mut cfg_refused = (*config).clone();
    cfg_refused.local.api_base = format!("http://{}/v1", dead_addr);
    let cfg_refused = Arc::new(cfg_refused);
    let metrics2 = Arc::new(MetricsTracker::new());
    let router2 = Router::new(cfg_refused.clone(), metrics2.clone());
    let upstream2 = UpstreamClient::new(cfg_refused, metrics2.clone(), cache);

    let dec2 = router2.decide(&body, Some("local-only")).await;
    assert_eq!(dec2.target, TargetProvider::Local);
    assert!(dec2.local_only);

    let resp2 = upstream2.forward_chat(body, dec2, 2, 0, 0.0, None).await;
    assert!(
        matches!(resp2.status(), StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT),
        "Expected 502/503/504 on local connection refused under local-only, got {}",
        resp2.status()
    );
    assert_eq!(
        cloud_hits.load(Ordering::SeqCst),
        0,
        "CRITICAL: local-only leaked request to Cloud on local connection refused!"
    );
    assert_eq!(metrics2.active_local_slots.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_cloud_429_respects_slot_limits_and_never_ping_pongs_back_to_cloud() {
    let cloud_hits = Arc::new(AtomicUsize::new(0));
    let local_hits = Arc::new(AtomicUsize::new(0));

    let ch = cloud_hits.clone();
    let cloud_429_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move || {
            let ch = ch.clone();
            async move {
                ch.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({ "error": { "message": "rate limit exceeded" } })),
                )
            }
        }),
    );
    let cloud_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(cloud_listener, cloud_429_app).await;
    });

    // Local returns 500 to test that even when fallback enters Local, a Local failure NEVER ping-pongs back to Cloud!
    let lh = local_hits.clone();
    let local_500_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move || {
            let lh = lh.clone();
            async move {
                lh.fetch_add(1, Ordering::SeqCst);
                (StatusCode::INTERNAL_SERVER_ERROR, "local error during 429 fallback")
            }
        }),
    );
    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(local_listener, local_500_app).await;
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}/v1", local_addr);
    cfg.cloud.api_base = format!("http://{}/v1", cloud_addr);
    cfg.local.max_slots = 1;
    cfg.local.queue_timeout_ms = 0;

    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let cache = Arc::new(ResponseCache::new(false, 16, 60));
    let router = Router::new(config.clone(), metrics.clone());
    let upstream = UpstreamClient::with_affinity(
        config.clone(),
        metrics.clone(),
        cache,
        router.affinity_tracker().clone(),
    );

    let body = json!({
        "model": "cloud",
        "stream": false,
        "messages": [{ "role": "user", "content": "escalated query" }]
    });

    // Case A: Occupy the ONLY local slot (1/1 active slots in metrics & affinity)
    let hold_body = json!({ "messages": [{ "role": "user", "content": "hi" }] });
    let held_local = router.decide(&hold_body, Some("local-preferred")).await;
    assert_eq!(held_local.target, TargetProvider::Local);
    assert_eq!(metrics.active_local_slots.load(Ordering::SeqCst), 1);

    let cloud_dec = router.decide(&body, None).await;
    assert_eq!(cloud_dec.target, TargetProvider::Cloud);

    let resp_saturated = upstream.forward_chat(body.clone(), cloud_dec, 10, 0, 0.0, None).await;
    assert_eq!(
        resp_saturated.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "When all local slots are busy, Cloud 429 must return 429 cleanly without bypassing slot limits"
    );
    assert_eq!(
        local_hits.load(Ordering::SeqCst),
        0,
        "Cloud 429 bypassed saturated local slot limit!"
    );
    assert_eq!(cloud_hits.load(Ordering::SeqCst), 1);

    // Case B: Release local slot so 1 slot is free; Cloud 429 falls back to Local (1 hit), Local returns 500 -> MUST NOT ping-pong back to Cloud!
    drop(held_local);
    assert_eq!(metrics.active_local_slots.load(Ordering::SeqCst), 0);
    metrics.clear_cloud_rate_limit();

    let cloud_dec2 = router.decide(&body, None).await;
    assert_eq!(cloud_dec2.target, TargetProvider::Cloud);

    let resp_fallback_err = upstream.forward_chat(body.clone(), cloud_dec2, 11, 0, 0.0, None).await;
    assert!(
        matches!(resp_fallback_err.status(), StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE),
        "Expected 502/503 when local fallback fails, got {}",
        resp_fallback_err.status()
    );
    assert_eq!(local_hits.load(Ordering::SeqCst), 1, "Local should be tried once when slot is free");
    assert_eq!(
        cloud_hits.load(Ordering::SeqCst),
        2,
        "CRITICAL: Cloud-429 fallback ping-ponged back to Cloud a second time after Local failed!"
    );
    assert_eq!(metrics.active_local_slots.load(Ordering::SeqCst), 0, "Fallback slot must be released");
    assert!(!router.affinity_tracker().is_slot_busy(0), "Affinity slot 0 must be released");

    // Case C: When circuit breaker is open, Cloud 429 must NOT dispatch locally
    for _ in 0..5 {
        metrics.record_local_failure(3);
    }
    assert!(metrics.is_circuit_open());
    let cloud_dec3 = router.decide(&body, None).await;
    assert_eq!(cloud_dec3.target, TargetProvider::Cloud);
    let resp_cb_open = upstream.forward_chat(body.clone(), cloud_dec3, 12, 0, 0.0, None).await;
    assert_eq!(resp_cb_open.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(local_hits.load(Ordering::SeqCst), 1, "Open circuit breaker must block Cloud-429 local fallback");

    // Case D: Restore healthy circuit, but prompt exceeds local max_context_tokens -> must return 429 without hitting local
    metrics.record_local_success();
    let mut over_ctx_dec = router.decide(&body, None).await;
    over_ctx_dec.est_tokens = (config.local.max_context_tokens as u64) * 3;
    let resp_over_ctx = upstream.forward_chat(body.clone(), over_ctx_dec, 13, 0, 0.0, None).await;
    assert_eq!(resp_over_ctx.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(local_hits.load(Ordering::SeqCst), 1, "Oversized prompt must not fall back to local on Cloud 429");

    // Case E: 2-slot setup where slot 0 is busy (450 tokens) and slot 1 is IDLE, while total_kv_token_budget (500) is exhausted:
    // Cloud 429 acquires idle slot 1 in affinity_tracker, fails try_acquire_local_capacity(200, 2, 500),
    // and MUST release slot 1 (`!is_slot_busy(1)`) while returning 429 cleanly!
    let mut cfg_kv = (*config).clone();
    cfg_kv.local.max_slots = 2;
    cfg_kv.local.total_kv_token_budget = 500;
    let cfg_kv = Arc::new(cfg_kv);
    let metrics_kv = Arc::new(MetricsTracker::new());
    let cache_kv = Arc::new(ResponseCache::new(false, 16, 60));
    let router_kv = Router::new(cfg_kv.clone(), metrics_kv.clone());
    let upstream_kv = UpstreamClient::with_affinity(
        cfg_kv,
        metrics_kv.clone(),
        cache_kv,
        router_kv.affinity_tracker().clone(),
    );

    let hold_kv_dec = router_kv
        .decide_for_session_with_hints(&hold_body, Some("local-preferred"), Some(0), "kv-holder", Some(450), None)
        .await;
    assert_eq!(hold_kv_dec.target, TargetProvider::Local);
    assert_eq!(hold_kv_dec.slot_id, Some(0));
    assert!(router_kv.affinity_tracker().is_slot_busy(0));
    assert!(!router_kv.affinity_tracker().is_slot_busy(1));

    let mut cloud_kv_dec = router_kv.decide(&body, None).await;
    assert_eq!(cloud_kv_dec.target, TargetProvider::Cloud);
    cloud_kv_dec.est_tokens = 200; // 450 + 200 = 650 > 500 total_kv_token_budget

    let resp_kv_exhausted = upstream_kv.forward_chat(body, cloud_kv_dec, 14, 0, 0.0, None).await;
    assert_eq!(
        resp_kv_exhausted.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "Exhausted KV budget on Cloud-429 fallback must return 429 cleanly"
    );
    assert!(
        !router_kv.affinity_tracker().is_slot_busy(1),
        "Idle slot 1 MUST be released when try_acquire_local_capacity fails during Cloud-429 fallback"
    );
    assert_eq!(local_hits.load(Ordering::SeqCst), 1);
    drop(hold_kv_dec);
}

// ============================================================================
// AC 5: > 2 MiB Payload (4 MiB Tool Output) Accepted by Axum Router & Condensed
// ============================================================================

#[tokio::test]
async fn test_4mib_payload_accepted_and_condensed_by_ferry_router() {
    let received_bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received_bodies.clone();

    let mock_local = AxumRouter::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route(
            "/v1/chat/completions",
            post(move |Json(payload): Json<Value>| {
                let store = received_clone.clone();
                async move {
                    store.lock().await.push(payload);
                    Json(json!({
                        "id": "chatcmpl-large",
                        "object": "chat.completion",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "condensed ok" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            }),
        )
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024));

    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(local_listener, mock_local).await;
    });

    // Reserve an ephemeral port for the ferry binary
    let port_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let ferry_addr = port_probe.local_addr().unwrap();
    drop(port_probe);

    let child = Command::new(env!("CARGO_BIN_EXE_ferry"))
        .env("FERRY_CONFIG", "__nonexistent_test_ferry.toml")
        .env("FERRY_BIND", ferry_addr.to_string())
        .env("FERRY_LOCAL_API_BASE", format!("http://{}/v1", local_addr))
        .env("FERRY_DEFAULT_ROUTE", "local-only")
        .env("FERRY_CACHE_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ferry binary");
    let _guard = ChildGuard(child);

    let client = reqwest::Client::new();
    let health_url = format!("http://{}/health", ferry_addr);
    let mut ready = false;
    for _ in 0..50 {
        if let Ok(r) = client.get(&health_url).send().await {
            if r.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(ready, "Ferry server failed to become healthy");

    // Build a > 4 MiB historical tool output (~4.2 MiB JSON payload)
    let line = "Compiling dependency_crate v0.1.0 (/workspace/crates/dep) -- verbose output line padding 1234567890\n";
    let big_tool_output = line.repeat(43_000); // ~4.3 MB
    assert!(big_tool_output.len() > 4 * 1024 * 1024);

    let payload = json!({
        "model": "ferry",
        "stream": false,
        "messages": [
            { "role": "system", "content": "You are a build assistant." },
            { "role": "user", "content": "Run cargo build" },
            {
                "role": "assistant",
                "tool_calls": [{ "id": "call_big", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"cargo build\"}" } }]
            },
            { "role": "tool", "tool_call_id": "call_big", "content": big_tool_output },
            { "role": "assistant", "content": "Build ran." },
            { "role": "user", "content": "Summarize status" },
            { "role": "assistant", "content": "Checking." },
            { "role": "user", "content": "Next step" }
        ]
    });

    let chat_url = format!("http://{}/v1/chat/completions", ferry_addr);
    let resp = client.post(&chat_url).json(&payload).send().await.unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "4 MiB request should be accepted (200 OK), NOT rejected with 413 Payload Too Large"
    );

    let chars_saved: usize = resp
        .headers()
        .get("x-ferry-chars-saved")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert!(
        chars_saved > 3_000_000,
        "4 MiB tool output must be condensed cleanly, chars_saved={chars_saved}"
    );

    let forwarded = received_bodies.lock().await;
    assert_eq!(forwarded.len(), 1);
    let forwarded_len = forwarded[0].to_string().len();
    assert!(
        forwarded_len < 200_000,
        "Condensed upstream body should be drastically smaller than 4 MiB, got {forwarded_len}"
    );
    drop(forwarded);

    // Also verify extract_session_id via HTTP: distinct multi-part array prompts get distinct session IDs,
    // and empty messages array falls back to "session-default" instead of dead-code nonzero DefaultHasher!
    let arr_req_1 = json!({
        "model": "ferry",
        "stream": false,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Session Alpha Prompt" }] }]
    });
    let arr_req_2 = json!({
        "model": "ferry",
        "stream": false,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Session Beta Prompt" }] }]
    });
    let empty_req = json!({
        "model": "ferry",
        "stream": false,
        "messages": []
    });

    let r1 = client.post(&chat_url).json(&arr_req_1).send().await.unwrap();
    let r2 = client.post(&chat_url).json(&arr_req_2).send().await.unwrap();
    let r3 = client.post(&chat_url).json(&empty_req).send().await.unwrap();

    let sid1 = r1.headers().get("x-ferry-session-id").unwrap().to_str().unwrap().to_string();
    let sid2 = r2.headers().get("x-ferry-session-id").unwrap().to_str().unwrap().to_string();
    let sid3 = r3.headers().get("x-ferry-session-id").unwrap().to_str().unwrap().to_string();

    assert_ne!(sid1, sid2, "Distinct multi-part array prompts must produce distinct session IDs");
    assert_ne!(sid1, "session-default");
    assert_eq!(sid3, "session-default", "Empty messages array must fall back to session-default");

    // Upper-bound check 1: Payload exceeding default 32 MiB limit returns HTTP 413 Payload Too Large.
    // Use TcpStream so early 413 response + close (before server drains 33 MiB) never races client BodyWrite on macOS loopback.
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(ferry_addr).await.unwrap();
        let oversized_len = 33 * 1024 * 1024 + 128;
        let req_head = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{{\"messages\":[{{\"role\":\"user\",\"content\":\"",
            ferry_addr, oversized_len
        );
        let _ = stream.write_all(req_head.as_bytes()).await;
        let chunk = vec![b'x'; 64 * 1024];
        let mut remaining = oversized_len - (req_head.len() - req_head.find("\r\n\r\n").unwrap() - 4);
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            if stream.write_all(&chunk[..n]).await.is_err() {
                break;
            }
            remaining -= n;
        }
        let mut resp_buf = Vec::new();
        let _ = stream.read_to_end(&mut resp_buf).await;
        let resp_head = String::from_utf8_lossy(&resp_buf);
        assert!(
            resp_head.starts_with("HTTP/1.1 413"),
            "Payload > default 32 MiB limit must return 413 Payload Too Large, got: {}",
            resp_head.lines().next().unwrap_or("")
        );
    }

    // Upper-bound check 2: Custom FERRY_MAX_BODY_BYTES (32 KiB) rejects a 48 KiB JSON payload with StatusCode::PAYLOAD_TOO_LARGE (413)
    let port_probe2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let capped_addr = port_probe2.local_addr().unwrap();
    drop(port_probe2);

    let capped_child = Command::new(env!("CARGO_BIN_EXE_ferry"))
        .env("FERRY_CONFIG", "__nonexistent_test_ferry.toml")
        .env("FERRY_BIND", capped_addr.to_string())
        .env("FERRY_LOCAL_API_BASE", format!("http://{}/v1", local_addr))
        .env("FERRY_DEFAULT_ROUTE", "local-only")
        .env("FERRY_MAX_BODY_BYTES", (32 * 1024).to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn capped ferry binary");
    let _capped_guard = ChildGuard(capped_child);

    let capped_health = format!("http://{}/health", capped_addr);
    for _ in 0..50 {
        if let Ok(r) = client.get(&capped_health).send().await {
            if r.status().is_success() {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let capped_chat_url = format!("http://{}/v1/chat/completions", capped_addr);
    let over_capped_payload = json!({
        "model": "ferry",
        "stream": false,
        "messages": [{ "role": "user", "content": "x".repeat(48 * 1024) }]
    });
    let resp_over_env_cap = client.post(&capped_chat_url).json(&over_capped_payload).send().await.unwrap();
    assert_eq!(
        resp_over_env_cap.status(),
        reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "Payload exceeding FERRY_MAX_BODY_BYTES must return 413 Payload Too Large"
    );
}
