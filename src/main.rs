
use axum::{
    extract::State,
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

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/stats", get(stats_handler))
        .route("/metrics", get(metrics_handler))
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
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

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snap = state.metrics.snapshot();
    Json(json!({
        "status": "ok",
        "service": "ferry",
        "version": env!("CARGO_PKG_VERSION"),
        "active_local_slots": snap.active_local_slots,
        "max_local_slots": state.config.local.max_slots,
        "condensation_enabled": state.config.condensation.enabled,
    }))
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.metrics.snapshot())
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.prometheus_metrics(),
    )
}

async fn models_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [
            { "id": "ferry", "object": "model", "owned_by": "kobol-labs" },
            { "id": "hybrid-pentest", "object": "model", "owned_by": "kobol-labs" },
            { "id": &state.config.local.model, "object": "model", "owned_by": "local" },
            { "id": &state.config.cloud.model, "object": "model", "owned_by": "cloud" },
        ]
    }))
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(mut body): Json<Value>,
) -> Response {
    let req_id = state.request_id_counter.fetch_add(1, Ordering::Relaxed);

    // Context condensation: strip past reasoning & truncate stale tool dumps
    let cond_stats = state.condenser.condense(&mut body);
    if cond_stats.chars_saved > 0 {
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
        );
    }

    let decision = state.router.decide(&body);
    state.upstream.forward_chat(body, decision, req_id).await
}
