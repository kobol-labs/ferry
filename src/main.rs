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
    let affinity_tracker = Arc::new(SessionAffinityTracker::new(
        config.affinity.enabled,
        config.local.max_slots,
    ));
    let router = HybridRouter::with_affinity(config.clone(), metrics.clone(), affinity_tracker.clone());
    let condenser = ContextCondenser::new(config.condensation.clone());
    let cache = Arc::new(ResponseCache::new(
        config.cache.enabled,
        config.cache.max_entries,
        config.cache.ttl_secs,
    ));
    let upstream = UpstreamClient::new(config.clone(), metrics.clone(), cache.clone());

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
            .timeout(std::time::Duration::from_secs(5))
            .pool_max_idle_per_host(2)
            .build()
            .unwrap_or_default();

        let health_url = format!(
            "{}/health",
            probe_state.config.local.api_base.trim_end_matches("/v1").trim_end_matches('/')
        );

        let probe_interval = std::time::Duration::from_secs(probe_state.config.local.probe_interval_secs);
        loop {
            tokio::time::sleep(probe_interval).await;
            let resp = probe_client
                .get(&health_url)
                .header("Authorization", format!("Bearer {}", probe_state.config.local.api_key))
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    let was_open = probe_state.metrics.is_circuit_open();
                    let was_offline = !probe_state.metrics.strata_online.load(Ordering::Relaxed);
                    probe_state.metrics.record_local_success();
                    if was_open || was_offline {
                        info!(
                            url = %health_url,
                            "Strata hardware is BACK ONLINE. Circuit breaker CLOSED - local routing resumed."
                        );
                    }
                }
                Ok(r) => {
                    tracing::debug!(
                        url = %health_url,
                        status = r.status().as_u16(),
                        "Strata health probe returned non-success status"
                    );
                }
                Err(err) => {
                    tracing::debug!(
                        url = %health_url,
                        error = %err,
                        "Strata health probe connection failed"
                    );
                }
            }
        }
    });

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
        for msg in messages.iter().take(2) {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                content.hash(&mut hasher);
            }
        }
        let hash = hasher.finish();
        if hash != 0 {
            return format!("session-{:016x}", hash);
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

    // 1. Fast-path deterministic response cache check (for non-streaming temperature <= 0.05)
    let cache_allowed = match headers.get("x-ferry-cache").and_then(|h| h.to_str().ok()) {
        Some("false") | Some("0") | Some("no") => false,
        _ => state.config.cache.enabled,
    };

    let cache_key = if cache_allowed {
        ResponseCache::compute_cache_key(&body)
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

    // 2. Client header control: X-Ferry-Condense (defaults to config setting)
    let condense_enabled = match headers.get("x-ferry-condense").and_then(|h| h.to_str().ok()) {
        Some("false") | Some("0") | Some("no") => false,
        _ => state.config.condensation.enabled,
    };

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
        );
    }

    let mut total_chars_saved = cond_stats.chars_saved;

    // 3. Rolling context compaction: compacts long conversation histories into an executive checkpoint
    if let Some(compact_stats) = state.condenser.rolling_compact(&mut body) {
        total_chars_saved += compact_stats.chars_saved;
        state.metrics.record_rolling_compaction(compact_stats.chars_saved);
        tracing::info!(
            session = %session_id,
            turns_compacted = compact_stats.turns_compacted,
            chars_saved = compact_stats.chars_saved,
            "Rolling context compaction pruned historical turns to fit within context threshold"
        );
    }

    let max_context = state.config.local.max_context_tokens as u64;
    let squeeze_ratio = state.config.local.adaptive_squeeze_ratio;

    if state.config.local.adaptive_squeezing {
        let est_tokens = ferry::router::Router::estimate_tokens(&body);
        if est_tokens > max_context && est_tokens <= (max_context as f64 * squeeze_ratio) as u64 {
            if let Some(extra_saved) = state.condenser.adaptive_squeeze(&mut body, max_context) {
                total_chars_saved += extra_saved;
                let new_tokens = ferry::router::Router::estimate_tokens(&body);
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

    let savings_pct = if cond_stats.orig_chars > 0 && total_chars_saved > 0 {
        (total_chars_saved as f64 / cond_stats.orig_chars as f64) * 100.0
    } else {
        0.0
    };

    // 4. Client header controls: X-Ferry-Route override and explicit queue wait
    let route_override = headers.get("x-ferry-route").and_then(|h| h.to_str().ok());
    let explicit_wait = headers
        .get("x-ferry-queue-timeout-ms")
        .or_else(|| headers.get("x-ferry-wait-ms"))
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let decision = state.router.decide_for_session(&body, route_override, explicit_wait, &session_id).await;

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
