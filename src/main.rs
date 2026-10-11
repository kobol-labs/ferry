use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
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
    let router = HybridRouter::new(config.clone(), metrics.clone());
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

        let probe_url = format!("{}/models", probe_state.config.local.api_base.trim_end_matches('/'));
        let interval_secs = probe_state.config.local.probe_interval_secs.max(1);

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;

            let res = probe_client
                .get(&probe_url)
                .header("Authorization", format!("Bearer {}", probe_state.config.local.api_key))
                .send()
                .await;

            match res {
                Ok(resp) if resp.status().is_success() => {
                    let _ = resp.bytes().await;
                    let was_open = probe_state.metrics.is_circuit_open();
                    probe_state.metrics.record_local_success();
                    if was_open {
                        info!("Circuit breaker RESET: Strata is back online and healthy. Resuming local GPU routing.");
                    }
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let _ = resp.bytes().await;
                    tracing::debug!(status = status, "Strata probe returned non-200");
                    let tripped = probe_state.metrics.record_local_failure(probe_state.config.local.circuit_breaker_threshold);
                    if tripped {
                        tracing::warn!("Circuit breaker TRIPPED by background probe (non-200 response). Strata marked offline.");
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "Strata probe connection failed");
                    let tripped = probe_state.metrics.record_local_failure(probe_state.config.local.circuit_breaker_threshold);
                    if tripped {
                        tracing::warn!("Circuit breaker TRIPPED by background probe (connection failure). Strata marked offline.");
                    }
                }
            }
        }
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/metrics", get(metrics_handler))
        .route("/v1/models", get(models_handler).options(cors_options_handler))
        .route("/v1/models/{id}", get(model_detail_handler).options(cors_options_handler))
        .route("/v1/embeddings", post(embeddings_handler).options(cors_options_handler))
        .route(
            "/v1/chat/completions",
            post(chat_completions_handler).options(cors_options_handler),
        )
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind).await?;
    info!(
        bind = %config.server.bind,
        local_model = %config.local.model,
        local_slots = config.local.max_slots,
        cloud_model = %config.cloud.model,
        condensation_enabled = config.condensation.enabled,
        cache_enabled = config.cache.enabled,
        "Ferry hybrid proxy started"
    );

    axum::serve(listener, app).await?;
    Ok(())
}

async fn cors_options_handler() -> impl IntoResponse {
    (
        [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "GET, POST, OPTIONS"),
            (
                "access-control-allow-headers",
                "content-type, authorization, x-ferry-condense, x-ferry-route, x-ferry-cache, x-ferry-queue-timeout-ms, x-ferry-wait-ms",
            ),
            (
                "access-control-expose-headers",
                "x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover, x-ferry-cache",
            ),
        ],
        "",
    )
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snap = state.metrics.snapshot();
    (
        [("access-control-allow-origin", "*")],
        Json(json!({
            "status": if snap.circuit_open { "degraded" } else { "ok" },
            "service": "ferry",
            "version": env!("CARGO_PKG_VERSION"),
            "strata_online": snap.strata_online,
            "circuit_breaker_open": snap.circuit_open,
            "consecutive_local_failures": snap.consecutive_local_failures,
            "circuit_trips_total": snap.circuit_trips_total,
            "active_local_slots": snap.active_local_slots,
            "max_local_slots": state.config.local.max_slots,
            "condensation_enabled": state.config.condensation.enabled,
            "cache_enabled": state.config.cache.enabled,
            "cache_hits": state.cache.hits.load(Ordering::Relaxed),
            "cache_misses": state.cache.misses.load(Ordering::Relaxed),
        })),
    )
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [("access-control-allow-origin", "*")],
        Json(state.metrics.snapshot()),
    )
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [
            ("content-type", "text/plain; version=0.0.4; charset=utf-8"),
            ("access-control-allow-origin", "*"),
        ],
        state.metrics.prometheus_metrics(),
    )
}

async fn models_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1700000000);

    let models = vec![
        json!({ "id": "ferry", "object": "model", "created": now, "owned_by": "kobol-labs" }),
        json!({ "id": "hybrid-pentest", "object": "model", "created": now, "owned_by": "kobol-labs" }),
        json!({ "id": &state.config.local.model, "object": "model", "created": now, "owned_by": "local" }),
        json!({ "id": &state.config.cloud.model, "object": "model", "created": now, "owned_by": "cloud" }),
        json!({ "id": "gpt-4o", "object": "model", "created": now, "owned_by": "openai-alias" }),
        json!({ "id": "gpt-4o-mini", "object": "model", "created": now, "owned_by": "openai-alias" }),
        json!({ "id": "gpt-4", "object": "model", "created": now, "owned_by": "openai-alias" }),
        json!({ "id": "gpt-3.5-turbo", "object": "model", "created": now, "owned_by": "openai-alias" }),
        json!({ "id": "claude-3-5-sonnet", "object": "model", "created": now, "owned_by": "anthropic-alias" }),
        json!({ "id": "deepseek-chat", "object": "model", "created": now, "owned_by": "deepseek-alias" }),
    ];

    (
        [
            ("access-control-allow-origin", "*"),
            ("content-type", "application/json"),
        ],
        Json(json!({
            "object": "list",
            "data": models,
        })),
    )
}

async fn model_detail_handler(
    Path(id): Path<String>,
) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1700000000);

    (
        [
            ("access-control-allow-origin", "*"),
            ("content-type", "application/json"),
        ],
        Json(json!({
            "id": id,
            "object": "model",
            "created": now,
            "owned_by": "kobol-labs",
            "permission": [],
            "root": id,
            "parent": null
        })),
    )
}

async fn embeddings_handler(
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("ferry-embeddings");
    let input_len = match body.get("input") {
        Some(Value::Array(arr)) => arr.len(),
        Some(Value::String(_)) => 1,
        _ => 1,
    };

    let dummy_dim = 1536;
    let mut data = Vec::with_capacity(input_len);
    for idx in 0..input_len {
        data.push(json!({
            "object": "embedding",
            "embedding": vec![0.0f32; dummy_dim],
            "index": idx
        }));
    }

    (
        [
            ("access-control-allow-origin", "*"),
            ("content-type", "application/json"),
        ],
        Json(json!({
            "object": "list",
            "data": data,
            "model": model,
            "usage": {
                "prompt_tokens": input_len * 4,
                "total_tokens": input_len * 4
            }
        })),
    )
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let req_id = state.request_id_counter.fetch_add(1, Ordering::Relaxed);

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

    // 3. Client header controls: X-Ferry-Route override and explicit queue wait
    let route_override = headers.get("x-ferry-route").and_then(|h| h.to_str().ok());
    let explicit_wait = headers
        .get("x-ferry-queue-timeout-ms")
        .or_else(|| headers.get("x-ferry-wait-ms"))
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let decision = state.router.decide_with_timeout(&body, route_override, explicit_wait).await;

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
