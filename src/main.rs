
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
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

    let config = Arc::new(Config::from_file(&config_path)?);
    let metrics = Arc::new(MetricsTracker::new());
    let router = HybridRouter::new(config.clone(), metrics.clone());
    let condenser = ContextCondenser::new(config.condensation.clone());
    let upstream = UpstreamClient::new(config.clone(), metrics.clone());

    let state = Arc::new(AppState {
        config: config.clone(),
        metrics: metrics.clone(),
        router,
        condenser,
        upstream,
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
                "content-type, authorization, x-ferry-condense, x-ferry-route",
            ),
            (
                "access-control-expose-headers",
                "x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover",
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
    (
        [("access-control-allow-origin", "*")],
        Json(json!({
            "object": "list",
            "data": [
                { "id": "ferry", "object": "model", "owned_by": "kobol-labs" },
                { "id": "hybrid-pentest", "object": "model", "owned_by": "kobol-labs" },
                { "id": &state.config.local.model, "object": "model", "owned_by": "local" },
                { "id": &state.config.cloud.model, "object": "model", "owned_by": "cloud" },
            ]
        })),
    )
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    let req_id = state.request_id_counter.fetch_add(1, Ordering::Relaxed);

    // 1. Client header control: X-Ferry-Condense (defaults to config setting)
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
        let orig_tokens = (cond_stats.orig_chars / 4) as u64;
        let cond_tokens = (cond_stats.cond_chars / 4) as u64;
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

    let savings_pct = if cond_stats.orig_chars > 0 && cond_stats.chars_saved > 0 {
        (cond_stats.chars_saved as f64 / cond_stats.orig_chars as f64) * 100.0
    } else {
        0.0
    };

    // 2. Client header control: X-Ferry-Route override (e.g. "local-only", "cloud-only")
    let route_override = headers.get("x-ferry-route").and_then(|h| h.to_str().ok());
    let decision = state.router.decide(&body, route_override).await;

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

    state.upstream.forward_chat(body, decision, req_id, cond_stats.chars_saved, savings_pct).await
}
