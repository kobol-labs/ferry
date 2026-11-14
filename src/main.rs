use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use ferry::affinity::SessionAffinityTracker;
use ferry::cache::ResponseCache;
use ferry::condenser::ContextCondenser;
use ferry::config::Config;
use ferry::router::Router as HybridRouter;
use ferry::stats::MetricsTracker;
use ferry::upstream::UpstreamClient;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::info;

struct AppState {
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
    router: HybridRouter,
    condenser: ContextCondenser,
    upstream: UpstreamClient,
    cache: Arc<ResponseCache>,
    request_id_counter: AtomicU64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferry=info,tower_http=info".into()),
        )
        .init();

    let config_path = std::env::var("FERRY_CONFIG").unwrap_or_else(|_| "ferry.toml".to_string());
    info!(config = %config_path, "Loading Ferry configuration");

    let config = Arc::new(Config::load_from_file_or_default(&config_path));
    let metrics = Arc::new(MetricsTracker::new());
    let affinity_tracker = Arc::new(SessionAffinityTracker::with_idle_reclaim(
        config.affinity.enabled,
        config.local.max_slots,
        config.affinity.idle_reclaim_secs,
    ));
    let router = HybridRouter::with_affinity(config.clone(), metrics.clone(), affinity_tracker.clone());
    let condenser = ContextCondenser::new(config.condensation.clone());
    let cache = Arc::new(ResponseCache::new(
        config.cache.enabled,
        config.cache.max_entries,
        config.cache.ttl_secs,
    ));
    let upstream = UpstreamClient::with_affinity(
        config.clone(),
        metrics.clone(),
        cache.clone(),
        affinity_tracker,
    );

    let state = Arc::new(AppState {
        config: config.clone(),
        metrics: metrics.clone(),
        router,
        condenser,
        upstream,
        cache,
        request_id_counter: AtomicU64::new(1),
    });

    // Spawn background health probe for local Strata hardware
    let probe_state = state.clone();
    tokio::spawn(async move {
        let probe_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(probe_state.config.local.connect_timeout_secs))
            .timeout(std::time::Duration::from_secs(probe_state.config.local.connect_timeout_secs.max(10)))
            .pool_max_idle_per_host(2)
            .build()
            .unwrap_or_default();

        let health_url = format!(
            "{}/health",
            probe_state.config.local.api_base.trim_end_matches("/v1").trim_end_matches('/')
        );

        let probe_interval = std::time::Duration::from_secs(probe_state.config.local.probe_interval_secs);
        loop {
            let resp = probe_client
                .get(&health_url)
                .header("Authorization", format!("Bearer {}", probe_state.config.local.api_key))
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    if probe_state
                        .metrics
                        .record_health_probe_success(ferry::stats::DEFAULT_CIRCUIT_COOLDOWN_SECS)
                    {
                        info!(
                            url = %health_url,
                            "Strata hardware is BACK ONLINE. Circuit breaker CLOSED - local routing resumed."
                        );
                    }
                }
                Ok(r) => {
                    probe_state.metrics.record_health_probe_failure();
                    tracing::debug!(
                        url = %health_url,
                        status = r.status().as_u16(),
                        "Strata health probe returned non-success status"
                    );
                }
                Err(err) => {
                    probe_state.metrics.record_health_probe_failure();
                    tracing::debug!(
                        url = %health_url,
                        error = %err,
                        "Strata health probe connection failed"
                    );
                }
            }
            tokio::time::sleep(probe_interval).await;
        }
    });

    // Spawn background Canary Probe for Cloud Quota / Rate-Limit Recovery
    let cloud_probe_state = state.clone();
    tokio::spawn(async move {
        let probe_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(1)
            .build()
            .unwrap_or_default();

        let cloud_url = format!(
            "{}/chat/completions",
            cloud_probe_state.config.cloud.api_base.trim_end_matches('/')
        );

        let canary_payload = serde_json::json!({
            "model": cloud_probe_state.config.cloud.model,
            "messages": [
                {"role": "user", "content": "ping"}
            ],
            "max_tokens": 1,
            "stream": false
        });

        info!(
            model = %cloud_probe_state.config.cloud.model,
            "Ferry Cloud Quota Canary Probe active (polls every 30s during 429 cooldown)"
        );

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;

            if cloud_probe_state.metrics.is_cloud_rate_limited() {
                tracing::debug!("Cloud provider is in 429 cooldown. Dispatching lightweight canary probe to check quota recovery...");
                let resp = probe_client
                    .post(&cloud_url)
                    .header("Authorization", format!("Bearer {}", cloud_probe_state.config.cloud.api_key))
                    .header("Content-Type", "application/json")
                    .json(&canary_payload)
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        info!(
                            model = %cloud_probe_state.config.cloud.model,
                            "Cloud provider quota / rate limit has CLEARED. Canary probe succeeded (HTTP 200). Restoring cloud routing immediately!"
                        );
                        cloud_probe_state.metrics.clear_cloud_rate_limit();
                    }
                    Ok(r) if r.status() == StatusCode::TOO_MANY_REQUESTS => {
                        tracing::debug!(
                            "Cloud provider canary probe still returned HTTP 429 (Quota still exhausted). Remaining on local Strata."
                        );
                        // Extend cooldown by 60s so live requests continue routing locally
                        cloud_probe_state.metrics.record_cloud_rate_limit(60);
                    }
                    Ok(r) => {
                        tracing::debug!(
                            status = r.status().as_u16(),
                            "Cloud canary probe returned non-success status"
                        );
                    }
                    Err(err) => {
                        tracing::debug!(
                            error = %err,
                            "Cloud canary probe network error"
                        );
                    }
                }
            }
        }
    });

    // Spawn background Pushgateway telemetry pusher if enabled
    if state.config.telemetry.enabled {
        let push_metrics = state.metrics.clone();
        let push_url = format!(
            "{}/metrics/job/{}",
            state.config.telemetry.pushgateway_url.trim_end_matches('/'),
            state.config.telemetry.job_name
        );
        let push_interval = std::time::Duration::from_secs(state.config.telemetry.push_interval_secs);
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(4))
                .build()
                .unwrap_or_default();

            info!(
                url = %push_url,
                interval_secs = push_interval.as_secs(),
                "Ferry Pushgateway telemetry exporter active"
            );

            loop {
                tokio::time::sleep(push_interval).await;
                let payload = push_metrics.prometheus_metrics();
                if let Err(err) = client.post(&push_url).body(payload).send().await {
                    tracing::debug!(url = %push_url, error = %err, "Failed to push metrics to Pushgateway");
                }
            }
        });
    }

    let max_body_bytes = std::env::var("FERRY_MAX_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(32 * 1024 * 1024);

    let app = Router::new()
        // Universal endpoints
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/metrics", get(metrics_handler))
        // OpenAI standard specification routes
        .route("/v1/models", get(models_handler).options(cors_options_handler))
        .route("/v1/models/{model}", get(model_detail_handler).options(cors_options_handler))
        .route("/models", get(models_handler).options(cors_options_handler))
        .route("/models/{model}", get(model_detail_handler).options(cors_options_handler))
        .route("/v1/embeddings", post(embeddings_handler).options(cors_options_handler))
        .route("/embeddings", post(embeddings_handler).options(cors_options_handler))
        .route(
            "/v1/chat/completions",
            post(chat_completions_handler).options(cors_options_handler),
        )
        .route(
            "/chat/completions",
            post(chat_completions_handler).options(cors_options_handler),
        )
        .layer(axum::extract::DefaultBodyLimit::max(max_body_bytes))
        .with_state(state);

    let bind_addr = &config.server.bind;
    info!(bind = %bind_addr, "Starting Ferry server");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn cors_options_handler() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", "*".parse().unwrap());
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        "access-control-allow-headers",
        "content-type, authorization, x-ferry-condense, x-ferry-route, x-ferry-cache, x-ferry-queue-timeout-ms, x-ferry-wait-ms, x-ferry-session-id, x-session-id, x-agent-id".parse().unwrap(),
    );
    headers.insert(
        "access-control-expose-headers",
        "x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover, x-ferry-cache, x-ferry-slot, x-ferry-affinity, x-ferry-session-id".parse().unwrap(),
    );
    (StatusCode::NO_CONTENT, headers)
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let circuit_open = state.metrics.is_circuit_open();
    let failures = state
        .metrics
        .consecutive_local_failures
        .load(Ordering::Relaxed);
    let trips = state.metrics.circuit_trips_total.load(Ordering::Relaxed);
    let active_slots = state.metrics.active_local_slots.load(Ordering::Relaxed);

    Json(json!({
        "status": "ok",
        "service": "ferry",
        "version": env!("CARGO_PKG_VERSION"),
        "condensation_enabled": state.config.condensation.enabled,
        "circuit_breaker_open": circuit_open,
        "strata_online": state.metrics.strata_online.load(Ordering::Relaxed),
        "consecutive_local_failures": failures,
        "circuit_trips_total": trips,
        "active_local_slots": active_slots,
        "max_local_slots": state.config.local.max_slots,
    }))
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snapshot = state.metrics.snapshot();
    let mut val = serde_json::to_value(&snapshot).unwrap_or(json!({}));
    if let Some(obj) = val.as_object_mut() {
        obj.insert("cache_entries".to_string(), json!(state.cache.len().await));
        obj.insert("cache_enabled".to_string(), json!(state.config.cache.enabled));
    }
    Json(val)
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let metrics_text = state.metrics.prometheus_metrics();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        metrics_text,
    )
}

async fn models_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let local_model = &state.config.local.model;
    let cloud_model = &state.config.cloud.model;

    let models = vec![
        json!({
            "id": "ferry",
            "object": "model",
            "created": 1735689600,
            "owned_by": "kobol-labs",
            "permission": [],
            "root": "ferry",
            "parent": null
        }),
        json!({
            "id": local_model,
            "object": "model",
            "created": 1735689600,
            "owned_by": "local-strata",
            "permission": [],
            "root": local_model,
            "parent": null
        }),
        json!({
            "id": cloud_model,
            "object": "model",
            "created": 1735689600,
            "owned_by": "cloud-minimax",
            "permission": [],
            "root": cloud_model,
            "parent": null
        }),
        // Standard OpenAI aliases to seamlessly connect OpenWebUI / third-party tools
        json!({
            "id": "gpt-4o",
            "object": "model",
            "created": 1735689600,
            "owned_by": "ferry-alias",
            "permission": [],
            "root": "gpt-4o",
            "parent": null
        }),
        json!({
            "id": "gpt-4",
            "object": "model",
            "created": 1735689600,
            "owned_by": "ferry-alias",
            "permission": [],
            "root": "gpt-4",
            "parent": null
        }),
    ];

    Json(json!({
        "object": "list",
        "data": models
    }))
}

async fn model_detail_handler(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> Response {
    let local_model = &state.config.local.model;
    let cloud_model = &state.config.cloud.model;

    let owned_by = if model_id == *local_model {
        "local-strata"
    } else if model_id == *cloud_model {
        "cloud-minimax"
    } else {
        "kobol-labs"
    };

    Json(json!({
        "id": model_id,
        "object": "model",
        "created": 1735689600,
        "owned_by": owned_by,
        "permission": [],
        "root": model_id,
        "parent": null
    }))
    .into_response()
}

async fn embeddings_handler(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let input = body.get("input").cloned().unwrap_or(Value::String(String::new()));
    let inputs: Vec<String> = match input {
        Value::String(s) => vec![s],
        Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec!["".to_string()],
    };

    let data: Vec<Value> = inputs
        .into_iter()
        .enumerate()
        .map(|(i, _text)| {
            // Provide a fast deterministic 1536-dim vector stub for OpenWebUI vector initialization
            let stub_vector: Vec<f32> = (0..1536).map(|x| ((x % 100) as f32) / 100.0).collect();
            json!({
                "object": "embedding",
                "embedding": stub_vector,
                "index": i
            })
        })
        .collect();

    let resp = json!({
        "object": "list",
        "data": data,
        "model": "text-embedding-ada-002",
        "usage": {
            "prompt_tokens": 8,
            "total_tokens": 8
        }
    });

    (
        StatusCode::OK,
        [
            ("access-control-allow-origin", "*"),
            ("content-type", "application/json"),
        ],
        Json(resp),
    )
        .into_response()
}

fn extract_session_id(headers: &HeaderMap, body: &Value) -> String {
    if let Some(val) = headers
        .get("x-ferry-session-id")
        .or_else(|| headers.get("x-session-id"))
        .or_else(|| headers.get("x-agent-id"))
        .and_then(|h| h.to_str().ok())
    {
        if !val.trim().is_empty() {
            return val.trim().to_string();
        }
    }

    if let Some(user) = body.get("user").and_then(|u| u.as_str()) {
        if !user.trim().is_empty() {
            return user.trim().to_string();
        }
    }

    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut hashed_any = false;
        for msg in messages.iter().take(2) {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    content.hash(&mut hasher);
                    hashed_any = true;
                }
            } else if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
                for part in parts {
                    let text_opt = part
                        .get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| part.as_str());
                    if let Some(text) = text_opt {
                        if !text.is_empty() {
                            text.hash(&mut hasher);
                            hashed_any = true;
                        }
                    }
                }
            }
        }
        if hashed_any {
            return format!("session-{:016x}", hasher.finish());
        }
    }

    "session-default".to_string()
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let req_id = state.request_id_counter.fetch_add(1, Ordering::Relaxed);
    let session_id = extract_session_id(&headers, &body);

    if let Some(pillar) = headers
        .get("x-sentinel-pillar")
        .or_else(|| headers.get("x-pillar"))
        .and_then(|h| h.to_str().ok())
    {
        state.metrics.record_pillar(pillar);
    }

    // 1. Client header control: X-Ferry-Condense (defaults to config setting)
    let condense_enabled = match headers.get("x-ferry-condense").and_then(|h| h.to_str().ok()) {
        Some(v) if matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no" | "off") => false,
        Some(v) if matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on") => true,
        _ => state.config.condensation.enabled,
    };

    // Stateless 3x identical failing tool-call tail nudge:
    // Must execute BEFORE ResponseCache lookup and BEFORE condense() (which folds historical duplicates).
    let loop_nudged = if condense_enabled && state.config.condensation.nudge_repeated_failures {
        let nudged = ferry::condenser::nudge_repeated_tool_failure(&mut body);
        if nudged {
            state.metrics.record_loop_nudge();
            info!(
                session = %session_id,
                "Stateless loop guard appended tail nudge after 3x identical failing tool calls"
            );
        }
        nudged
    } else {
        false
    };

    // 2. Fast-path deterministic response cache check (for non-streaming temperature <= 0.05).
    // Bypassed completely (`cache_key = None`) when loop nudge fires so nudged turns never read
    // stale cached loops or poison clean deterministic ResponseCache entries.
    let cache_allowed = !loop_nudged
        && match headers.get("x-ferry-cache").and_then(|h| h.to_str().ok()) {
            Some("false") | Some("0") | Some("no") => false,
            _ => state.config.cache.enabled,
        };

    let cache_key = if cache_allowed {
        ResponseCache::compute_cache_key_with_route(
            &body,
            headers.get("x-ferry-route").and_then(|h| h.to_str().ok()),
        )
    } else {
        None
    };

    if let Some(ref key) = cache_key {
        if let Some(cached) = state.cache.get(key).await {
            let mut res_headers = HeaderMap::new();
            res_headers.insert("access-control-allow-origin", "*".parse().unwrap());
            res_headers.insert("content-type", "application/json".parse().unwrap());
            res_headers.insert("x-ferry-provider", "cache".parse().unwrap());
            res_headers.insert("x-ferry-cache", "HIT".parse().unwrap());
            res_headers.insert("x-ferry-duration-ms", "0".parse().unwrap());
            if let Ok(v) = axum::http::HeaderValue::from_str(&session_id) {
                res_headers.insert("x-ferry-session-id", v);
            }
            return (
                StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK),
                res_headers,
                cached.body_bytes,
            ).into_response();
        }
    }

    let cond_stats = if condense_enabled {
        state.condenser.condense(&mut body)
    } else {
        ferry::condenser::CondensationStats::default()
    };

    if cond_stats.orig_chars > 0 {
        let orig_tokens = ferry::condenser::estimate_tokens_from_chars(cond_stats.orig_chars);
        let cond_tokens = ferry::condenser::estimate_tokens_from_chars(cond_stats.cond_chars);
        state.metrics.record_condensation(
            orig_tokens,
            cond_tokens,
            cond_stats.reasoning_blocks_dropped,
            cond_stats.tool_outputs_truncated,
            cond_stats.chars_saved,
            cond_stats.ansi_stripped_count,
            cond_stats.progress_cleaned_count,
            cond_stats.lines_deduped_count,
            cond_stats.whitespace_collapsed_count,
            cond_stats.json_minified_count,
            cond_stats.ledger_checks_passed,
            cond_stats.ledger_vetoed_count,
            cond_stats.signals_extracted_count,
            cond_stats.loops_collapsed_count,
            cond_stats.lockfile_diffs_pruned_count,
            cond_stats.base64_blobs_suppressed_count,
            cond_stats.dir_traversals_shielded_count,
        );
    }

    let mut total_chars_saved = cond_stats.chars_saved;

    // 3. Determine if request is targeting Cloud or Local:
    // If targeting Cloud (cloud-only, hybrid cognitive escalation, or explicit cloud model requested),
    // we EXEMPT it from aggressive local hardware squeezing and rolling compaction.
    // This guarantees Cloud turns receive FULL high-fidelity context without being lobotomized.
    let route_override = headers.get("x-ferry-route").and_then(|h| h.to_str().ok());
    let explicit_wait = headers
        .get("x-ferry-queue-timeout-ms")
        .or_else(|| headers.get("x-ferry-wait-ms"))
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let effective_mode = route_override
        .and_then(ferry::router::RoutingMode::parse)
        .or_else(|| ferry::router::RoutingMode::parse(&state.config.routing.default_route))
        .unwrap_or(ferry::router::RoutingMode::LocalPreferred);

    let requested_cloud_model = body.get("model").and_then(|m| m.as_str())
        .map(|m| m == state.config.cloud.model || m == "cloud")
        .unwrap_or(false);

    let precomputed_hard_logic = if effective_mode == ferry::router::RoutingMode::Hybrid {
        Some(ferry::router::Router::is_hard_logic_turn(&body))
    } else {
        None
    };

    let is_cloud_target = effective_mode == ferry::router::RoutingMode::CloudOnly
        || requested_cloud_model
        || precomputed_hard_logic.as_ref().map(|(is_hard, _)| *is_hard).unwrap_or(false);

    // Rolling context compaction when context exceeds compact_threshold_tokens (local targets only when condensation is enabled):
    if condense_enabled && !is_cloud_target && state.config.condensation.rolling_compaction {
        if let Some(compact_stats) = state.condenser.rolling_compact(&mut body) {
            total_chars_saved += compact_stats.chars_saved;
            state.metrics.record_rolling_compaction(compact_stats.chars_saved);
            tracing::info!(
                session = %session_id,
                turns_compacted = compact_stats.turns_compacted,
                chars_saved = compact_stats.chars_saved,
                target = "local",
                "Rolling context compaction pruned historical turns"
            );
        }
    }

    let mut precomputed_tokens: Option<u64> = None;

    // Apply hardware-specific adaptive squeezing ONLY for local targets when condensation is enabled:
    if !is_cloud_target {

        // Adaptive squeezing to fit local context limit:
        let max_context = state.config.local.max_context_tokens as u64;
        let squeeze_ratio = state.config.local.adaptive_squeeze_ratio;
        if condense_enabled && state.config.local.adaptive_squeezing {
            let est_tokens = ferry::router::Router::estimate_tokens(&body);
            precomputed_tokens = Some(est_tokens);
            if est_tokens > max_context && est_tokens <= (max_context as f64 * squeeze_ratio) as u64 {
                if let Some(extra_saved) = state.condenser.adaptive_squeeze(&mut body, max_context) {
                    total_chars_saved += extra_saved;
                    let new_tokens = ferry::router::Router::estimate_tokens(&body);
                    precomputed_tokens = Some(new_tokens);
                    if new_tokens <= max_context {
                        state.metrics.adaptive_squeezed_count.fetch_add(1, Ordering::Relaxed);
                        tracing::info!(
                            before = est_tokens,
                            after = new_tokens,
                            max = max_context,
                            extra_saved,
                            "Adaptive squeezing successfully fit prompt into local context"
                        );
                    }
                }
            }
        }
    }

    let savings_pct = if cond_stats.orig_chars > 0 && total_chars_saved > 0 {
        (total_chars_saved as f64 / cond_stats.orig_chars as f64) * 100.0
    } else {
        0.0
    };

    let decision = state
        .router
        .decide_for_session_with_hints(
            &body,
            route_override,
            explicit_wait,
            &session_id,
            precomputed_tokens,
            precomputed_hard_logic,
        )
        .await;

    // If caller requested local-only and local provider is unavailable, fail fast with 503
    if decision.target == ferry::router::TargetProvider::Cloud && decision.local_only {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [
                ("access-control-allow-origin", "*"),
                ("content-type", "application/json"),
            ],
            json!({
                "error": {
                    "message": format!("Local provider unavailable and local-only routing requested: {}", decision.reason),
                    "type": "ferry_routing_error",
                    "code": "local_unavailable"
                }
            }).to_string(),
        ).into_response();
    }

    state.upstream.forward_chat(body, decision, req_id, total_chars_saved, savings_pct, cache_key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_handler_loop_nudge_bypasses_cache_and_runs_before_fold() {
        let captured_bodies: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = captured_bodies.clone();

        let upstream_app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(payload): Json<Value>| {
                let cap = captured_clone.clone();
                async move {
                    cap.lock().await.push(payload);
                    Json(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "pivoting approach" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, upstream_app).await;
        });

        let mut cfg = Config::default();
        cfg.local.api_base = format!("http://{}/v1", addr);
        cfg.routing.default_route = "local-only".to_string();
        cfg.cache.enabled = true;
        cfg.condensation.enabled = true;
        cfg.condensation.fold_repeated_tool_failures = true;
        cfg.condensation.nudge_repeated_failures = true;

        let config = Arc::new(cfg);
        let metrics = Arc::new(MetricsTracker::new());
        let affinity = Arc::new(SessionAffinityTracker::with_idle_reclaim(true, 4, 60));
        let router = HybridRouter::with_affinity(config.clone(), metrics.clone(), affinity);
        let condenser = ContextCondenser::new(config.condensation.clone());
        let cache = Arc::new(ResponseCache::new(true, 64, 60));
        let upstream = UpstreamClient::new(config.clone(), metrics.clone(), cache.clone());

        let state = Arc::new(AppState {
            config,
            metrics: metrics.clone(),
            router,
            condenser,
            upstream,
            cache: cache.clone(),
            request_id_counter: AtomicU64::new(1),
        });

        // 3 identical non-diagnostic >=25 char outputs: if condense() ran BEFORE loop guard,
        // fold_repeated_tool_failures would rewrite attempt #2 and mask byte-equality!
        let repeated_out = "Exit status 1: command produced identical stuck output";
        let loop_body = json!({
            "model": "ferry",
            "temperature": 0.0,
            "stream": false,
            "messages": [
                { "role": "user", "content": "Check service" },
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"check_svc\"}" } }] },
                { "role": "tool", "content": repeated_out },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"check_svc\"}" } }] },
                { "role": "tool", "content": repeated_out },
                { "role": "assistant", "tool_calls": [{ "id": "c3", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"check_svc\"}" } }] },
                { "role": "tool", "content": repeated_out }
            ]
        });

        // Seed ResponseCache with a stale loop response for the unmutated key
        let clean_key = ResponseCache::compute_cache_key(&loop_body).unwrap();
        cache
            .set(
                clean_key.clone(),
                bytes::Bytes::from_static(b"{\"stale_cached_loop\":true}"),
                200,
                vec![],
            )
            .await;
        assert_eq!(cache.len().await, 1);

        // Execute handler: must nudge, bypass early cache hit, and NOT write nudged key to cache
        let resp = chat_completions_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(loop_body.clone()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_ne!(
            resp.headers().get("x-ferry-cache").and_then(|v| v.to_str().ok()),
            Some("HIT")
        );
        let resp_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let resp_json: Value = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp_json["choices"][0]["message"]["content"], "pivoting approach");

        // Verify metric incremented and Prometheus renders it
        assert_eq!(metrics.loop_nudges_total.load(Ordering::Relaxed), 1);
        assert!(metrics.prometheus_metrics().contains("ferry_loop_nudges_total 1"));

        // Verify ResponseCache was NOT poisoned with a second entry
        assert_eq!(cache.len().await, 1);

        // Verify upstream received folded attempt #2 AND tail nudge on attempt #3
        let sent = captured_bodies.lock().await;
        assert_eq!(sent.len(), 1);
        let sent_msgs = sent[0]["messages"].as_array().unwrap();
        assert!(sent_msgs[4]["content"]
            .as_str()
            .unwrap()
            .contains("Repeated identical tool failure suppressed"));
        assert!(sent_msgs[6]["content"]
            .as_str()
            .unwrap()
            .ends_with(ferry::condenser::LOOP_GUARD_SUFFIX));
        drop(sent);

        // When X-Ferry-Condense: false header is passed, loop guard is disabled and cache HIT works
        let mut no_condense_headers = HeaderMap::new();
        no_condense_headers.insert("x-ferry-condense", "false".parse().unwrap());
        let cached_resp = chat_completions_handler(
            State(state.clone()),
            no_condense_headers,
            Json(loop_body.clone()),
        )
        .await;
        assert_eq!(
            cached_resp.headers().get("x-ferry-cache").and_then(|v| v.to_str().ok()),
            Some("HIT")
        );
        assert_eq!(metrics.loop_nudges_total.load(Ordering::Relaxed), 1);

        // When config `condensation.nudge_repeated_failures = false`, loop guard is also disabled
        let mut cfg_disabled = (*state.config).clone();
        cfg_disabled.condensation.nudge_repeated_failures = false;
        let disabled_state = Arc::new(AppState {
            config: Arc::new(cfg_disabled.clone()),
            metrics: metrics.clone(),
            router: HybridRouter::with_affinity(
                Arc::new(cfg_disabled.clone()),
                metrics.clone(),
                Arc::new(SessionAffinityTracker::with_idle_reclaim(true, 4, 60)),
            ),
            condenser: ContextCondenser::new(cfg_disabled.condensation.clone()),
            upstream: UpstreamClient::new(Arc::new(cfg_disabled), metrics.clone(), cache.clone()),
            cache,
            request_id_counter: AtomicU64::new(2),
        });
        let disabled_resp = chat_completions_handler(
            State(disabled_state),
            HeaderMap::new(),
            Json(loop_body),
        )
        .await;
        assert_eq!(
            disabled_resp.headers().get("x-ferry-cache").and_then(|v| v.to_str().ok()),
            Some("HIT")
        );
        assert_eq!(metrics.loop_nudges_total.load(Ordering::Relaxed), 1);
    }
}
