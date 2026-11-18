use crate::cache::ResponseCache;
use crate::config::Config;
use crate::router::{RouteDecision, TargetProvider};
use crate::stats::MetricsTracker;
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::stream::{Stream, StreamExt};
use reqwest::Client;
use serde_json::Value;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tracing::{error, info, warn};

pub static KEEPALIVE_SSE_CHUNK: &[u8] = b"data: {\"id\":\"chatcmpl-keepalive\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"ferry\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"},\"finish_reason\":null}]}\n\n";
pub static DONE_SSE_CHUNK: &[u8] = b"data: [DONE]\n\n";

pin_project! {
    pub struct ReceiverStream<T> {
        #[pin]
        rx: tokio::sync::mpsc::Receiver<T>,
    }
}

impl<T> ReceiverStream<T> {
    pub fn new(rx: tokio::sync::mpsc::Receiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().rx.poll_recv(cx)
    }
}

pin_project! {
    pub struct GuardedStream<S> {
        #[pin]
        inner: S,
        #[pin]
        sleep: tokio::time::Sleep,
        _guard: Option<crate::router::LocalSlotGuard>,
        is_sse: bool,
        keepalive_duration: std::time::Duration,
        ended: bool,
        sse_tracker: SseFrameTracker,
    }
}

impl<S> GuardedStream<S> {
    pub fn new(
        inner: S,
        guard: Option<crate::router::LocalSlotGuard>,
        is_sse: bool,
        keepalive_duration: std::time::Duration,
    ) -> Self {
        Self {
            inner,
            sleep: tokio::time::sleep(keepalive_duration),
            _guard: guard,
            is_sse,
            keepalive_duration,
            ended: false,
            sse_tracker: SseFrameTracker::new(),
        }
    }
}

impl<S, E> Stream for GuardedStream<S>
where
    S: Stream<Item = Result<bytes::Bytes, E>>,
    E: std::fmt::Display,
{
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if *this.ended {
            return Poll::Ready(None);
        }

        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.sleep.as_mut().reset(tokio::time::Instant::now() + *this.keepalive_duration);
                let bytes = if *this.is_sse {
                    this.sse_tracker.process_chunk(bytes)
                } else {
                    bytes
                };
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                warn!(error = %e, "Upstream stream closed or errored mid-transfer; cleanly terminating SSE stream with [DONE]");
                *this.ended = true;
                if *this.is_sse {
                    Poll::Ready(Some(Ok(this.sse_tracker.on_stream_error())))
                } else {
                    Poll::Ready(None)
                }
            }
            Poll::Ready(None) => {
                *this.ended = true;
                Poll::Ready(None)
            }
            Poll::Pending => {
                if *this.is_sse {
                    match Future::poll(this.sleep.as_mut(), cx) {
                        Poll::Ready(()) => {
                            this.sleep.as_mut().reset(tokio::time::Instant::now() + *this.keepalive_duration);
                            if let Some(chunk) = this.sse_tracker.on_keepalive_tick() {
                                Poll::Ready(Some(Ok(chunk)))
                            } else {
                                let _ = Future::poll(this.sleep.as_mut(), cx);
                                Poll::Pending
                            }
                        }
                        Poll::Pending => Poll::Pending,
                    }
                } else {
                    Poll::Pending
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponseMeta {
    pub provider: &'static str,
    pub model: String,
    pub est_tokens: u64,
    pub chars_saved: usize,
    pub savings_pct: f64,
    pub duration_ms: u128,
    pub is_failover: bool,
    pub slot_id: Option<usize>,
    pub is_affinity_hit: bool,
    pub affinity_type: crate::affinity::AffinityType,
    pub session_id: String,
}

pub struct UpstreamClient {
    client: Client,
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
    cache: Arc<ResponseCache>,
    affinity_tracker: Arc<crate::affinity::SessionAffinityTracker>,
}

impl UpstreamClient {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>, cache: Arc<ResponseCache>) -> Self {
        let affinity_tracker = Arc::new(crate::affinity::SessionAffinityTracker::with_idle_reclaim(
            config.affinity.enabled,
            config.local.max_slots,
            config.affinity.idle_reclaim_secs,
        ));
        Self::with_affinity(config, metrics, cache, affinity_tracker)
    }

    pub fn with_affinity(
        config: Arc<Config>,
        metrics: Arc<MetricsTracker>,
        cache: Arc<ResponseCache>,
        affinity_tracker: Arc<crate::affinity::SessionAffinityTracker>,
    ) -> Self {
        let client = Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(10)
            .tcp_nodelay(true)
            .connect_timeout(std::time::Duration::from_secs(config.local.connect_timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            config,
            metrics,
            cache,
            affinity_tracker,
        }
    }

    pub async fn forward_chat(
        &self,
        body: Value,
        decision: RouteDecision,
        req_id: u64,
        chars_saved: usize,
        savings_pct: f64,
        cache_key: Option<String>,
    ) -> Response {
        self.forward_chat_with_cloud_fallback(
            body,
            None,
            decision,
            req_id,
            chars_saved,
            savings_pct,
            cache_key,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn forward_chat_with_cloud_fallback(
        &self,
        mut body: Value,
        cloud_fallback: Option<(Value, usize, f64)>,
        mut decision: RouteDecision,
        req_id: u64,
        chars_saved: usize,
        savings_pct: f64,
        cache_key: Option<String>,
    ) -> Response {
        let start = Instant::now();
        let is_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);

        match decision.target {
            TargetProvider::Local => {
                let slot_guard = decision.slot_guard.take();
                self.metrics.record_local_request_stats(decision.est_tokens);
                info!(
                    req_id = req_id,
                    target = "local",
                    model = %self.config.local.model,
                    tokens = decision.est_tokens,
                    slot = decision.slot_id.unwrap_or(0),
                    affinity_hit = decision.is_affinity_hit,
                    session = %decision.session_id,
                    reason = %decision.reason,
                    "Routing to Local Provider"
                );

                let local_url = format!("{}/chat/completions", self.config.local.api_base.trim_end_matches('/'));
                normalize_wire_format(&mut body, &self.config.local.model);

                // Inject slot affinity into local llama-server / Strata payload
                if let Some(slot) = decision.slot_id {
                    body["id_slot"] = serde_json::json!(slot);
                }

                let req = self
                    .client
                    .post(&local_url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", self.config.local.api_key))
                    .header(header::CONTENT_TYPE, "application/json")
                    .json(&body)
                    .timeout(std::time::Duration::from_secs(self.config.local.timeout_secs));

                if is_stream {
                    let meta = ResponseMeta {
                        provider: "local",
                        model: self.config.local.model.clone(),
                        est_tokens: decision.est_tokens,
                        chars_saved,
                        savings_pct,
                        duration_ms: 0,
                        is_failover: false,
                        slot_id: decision.slot_id,
                        is_affinity_hit: decision.is_affinity_hit,
                        affinity_type: decision.affinity_type,
                        session_id: decision.session_id.clone(),
                    };
                    return self
                        .stream_local_response_with_prefill_keepalive(
                            req,
                            body,
                            cloud_fallback,
                            decision,
                            slot_guard,
                            meta,
                            req_id,
                            chars_saved,
                            savings_pct,
                            cache_key,
                            start,
                        )
                        .await;
                }

                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        self.metrics.record_local_success();
                        let duration = start.elapsed();
                        self.metrics.record_local_completion(duration.as_millis());
                        info!(
                            req_id = req_id,
                            target = "local",
                            status = resp.status().as_u16(),
                            duration_ms = duration.as_millis(),
                            "Local turn completed successfully"
                        );
                        let meta = ResponseMeta {
                            provider: "local",
                            model: self.config.local.model.clone(),
                            est_tokens: decision.est_tokens,
                            chars_saved,
                            savings_pct,
                            duration_ms: duration.as_millis(),
                            is_failover: false,
                            slot_id: decision.slot_id,
                            is_affinity_hit: decision.is_affinity_hit,
                            affinity_type: decision.affinity_type,
                            session_id: decision.session_id.clone(),
                        };

                        if !is_stream && !extract_declared_tools(&body).is_empty() {
                            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
                            let headers = Self::build_response_headers(resp.headers(), &meta);
                            drop(slot_guard);
                            match resp.bytes().await {
                                Ok(body_bytes) => {
                                    match inspect_and_repair_local_tool_response(&body, &body_bytes) {
                                        LocalToolInspection::ValidUnchanged => {
                                            if let Some(key) = cache_key {
                                                self.cache.set(key, body_bytes.clone(), status.as_u16(), vec![]).await;
                                            }
                                            return (status, headers, body_bytes).into_response();
                                        }
                                        LocalToolInspection::Repaired(repaired_bytes) => {
                                            info!(
                                                req_id = req_id,
                                                "Recovered structured OpenAI tool_calls from local XML <tool_call> output"
                                            );
                                            if let Some(key) = cache_key {
                                                self.cache.set(key, repaired_bytes.clone(), status.as_u16(), vec![]).await;
                                            }
                                            return (status, headers, repaired_bytes).into_response();
                                        }
                                        LocalToolInspection::FailoverRequired { reason } => {
                                            if !decision.local_only {
                                                warn!(
                                                    req_id = req_id,
                                                    reason = %reason,
                                                    "Local tool-enabled turn returned invalid/missing tool_calls; failing over to Cloud"
                                                );
                                                return self
                                                    .failover_local_to_cloud(
                                                        body,
                                                        cloud_fallback,
                                                        decision,
                                                        req_id,
                                                        chars_saved,
                                                        savings_pct,
                                                        cache_key,
                                                    )
                                                    .await;
                                            }
                                            warn!(
                                                req_id = req_id,
                                                reason = %reason,
                                                "Local tool-enabled turn returned invalid/missing tool_calls under local-only routing; returning uncached local response"
                                            );
                                            return (status, headers, body_bytes).into_response();
                                        }
                                    }
                                }
                                Err(err) => {
                                    warn!(req_id = req_id, error = %err, "Failed reading local response body");
                                    return (
                                        StatusCode::BAD_GATEWAY,
                                        [
                                            ("access-control-allow-origin", "*"),
                                            ("content-type", "application/json"),
                                        ],
                                        serde_json::json!({
                                            "error": {
                                                "message": format!("Failed reading upstream response body: {}", err),
                                                "type": "upstream_error"
                                            }
                                        })
                                        .to_string(),
                                    )
                                        .into_response();
                                }
                            }
                        }

                        self.handle_upstream_response(resp, is_stream, slot_guard, meta, cache_key).await
                    }
                    Ok(resp) if resp.status().is_client_error() && resp.status() != StatusCode::TOO_MANY_REQUESTS => {
                        self.metrics.record_local_success();
                        let status = resp.status();
                        let duration = start.elapsed();
                        self.metrics.record_local_completion(duration.as_millis());
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            duration_ms = duration.as_millis(),
                            "Local provider returned client error (forwarding to client, not failing over)"
                        );
                        let meta = ResponseMeta {
                            provider: "local",
                            model: self.config.local.model.clone(),
                            est_tokens: decision.est_tokens,
                            chars_saved,
                            savings_pct,
                            duration_ms: duration.as_millis(),
                            is_failover: false,
                            slot_id: decision.slot_id,
                            is_affinity_hit: decision.is_affinity_hit,
                            affinity_type: decision.affinity_type,
                            session_id: decision.session_id.clone(),
                        };
                        self.handle_upstream_response(resp, is_stream, slot_guard, meta, None).await
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let error_text = resp.text().await.unwrap_or_default();
                        let tripped = self.metrics.record_local_failure(self.config.local.circuit_breaker_threshold);
                        if tripped {
                            warn!(
                                req_id = req_id,
                                threshold = self.config.local.circuit_breaker_threshold,
                                "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                            );
                        }
                        drop(slot_guard); // Release local slot immediately
                        if decision.local_only {
                            warn!(
                                req_id = req_id,
                                status = status.as_u16(),
                                error = %error_text,
                                "Local provider returned server error under local-only routing; refusing cloud failover"
                            );
                            let err_status = if status == StatusCode::GATEWAY_TIMEOUT {
                                StatusCode::GATEWAY_TIMEOUT
                            } else if status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::TOO_MANY_REQUESTS {
                                StatusCode::SERVICE_UNAVAILABLE
                            } else {
                                StatusCode::BAD_GATEWAY
                            };
                            return (
                                err_status,
                                [
                                    ("access-control-allow-origin", "*"),
                                    ("content-type", "application/json"),
                                    ("x-ferry-provider", "local"),
                                    ("x-ferry-failover", "false"),
                                ],
                                serde_json::json!({
                                    "error": {
                                        "message": format!(
                                            "Local upstream returned HTTP {} and local-only routing prevents cloud failover: {}",
                                            status.as_u16(),
                                            error_text
                                        ),
                                        "type": "ferry_local_upstream_error",
                                        "code": "local_upstream_failed"
                                    }
                                })
                                .to_string(),
                            )
                                .into_response();
                        }
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            error = %error_text,
                            "Local provider returned server error. Failing over to cloud..."
                        );
                        self.failover_local_to_cloud(
                            body,
                            cloud_fallback,
                            decision,
                            req_id,
                            chars_saved,
                            savings_pct,
                            cache_key,
                        )
                        .await
                    }
                    Err(err) => {
                        let tripped = self.metrics.record_local_failure(self.config.local.circuit_breaker_threshold);
                        if tripped {
                            warn!(
                                req_id = req_id,
                                threshold = self.config.local.circuit_breaker_threshold,
                                "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                            );
                        }
                        drop(slot_guard); // Release local slot immediately
                        if decision.local_only {
                            warn!(
                                req_id = req_id,
                                error = %err,
                                "Local provider request failed under local-only routing; refusing cloud failover"
                            );
                            let err_status = if err.is_timeout() {
                                StatusCode::GATEWAY_TIMEOUT
                            } else if err.is_connect() {
                                StatusCode::SERVICE_UNAVAILABLE
                            } else {
                                StatusCode::BAD_GATEWAY
                            };
                            return (
                                err_status,
                                [
                                    ("access-control-allow-origin", "*"),
                                    ("content-type", "application/json"),
                                    ("x-ferry-provider", "local"),
                                    ("x-ferry-failover", "false"),
                                ],
                                serde_json::json!({
                                    "error": {
                                        "message": format!(
                                            "Local upstream request failed and local-only routing prevents cloud failover: {}",
                                            err
                                        ),
                                        "type": "ferry_local_upstream_error",
                                        "code": "local_upstream_unreachable"
                                    }
                                })
                                .to_string(),
                            )
                                .into_response();
                        }
                        warn!(
                            req_id = req_id,
                            error = %err,
                            "Local provider connection failed. Failing over to cloud..."
                        );
                        self.failover_local_to_cloud(
                            body,
                            cloud_fallback,
                            decision,
                            req_id,
                            chars_saved,
                            savings_pct,
                            cache_key,
                        )
                        .await
                    }
                }
            }
            TargetProvider::Cloud => {
                if decision.local_only {
                    drop(decision.slot_guard.take());
                    warn!(
                        req_id = req_id,
                        reason = %decision.reason,
                        "Local provider unavailable under local-only routing; refusing cloud dispatch"
                    );
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [
                            ("access-control-allow-origin", "*"),
                            ("content-type", "application/json"),
                            ("x-ferry-provider", "local"),
                            ("x-ferry-failover", "false"),
                        ],
                        serde_json::json!({
                            "error": {
                                "message": format!(
                                    "Local provider unavailable and local-only routing requested: {}",
                                    decision.reason
                                ),
                                "type": "ferry_routing_error",
                                "code": "local_unavailable"
                            }
                        })
                        .to_string(),
                    )
                        .into_response();
                }
                self.metrics.inc_cloud(decision.est_tokens, decision.is_spillover);
                self.dispatch_cloud(body, decision, req_id, false, chars_saved, savings_pct, cache_key).await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn failover_local_to_cloud(
        &self,
        local_body: Value,
        cloud_fallback: Option<(Value, usize, f64)>,
        mut decision: RouteDecision,
        req_id: u64,
        local_chars_saved: usize,
        local_savings_pct: f64,
        cache_key: Option<String>,
    ) -> Response {
        let local_tokens = decision.est_tokens;
        let (cloud_body, eff_chars_saved, eff_savings_pct) = match cloud_fallback {
            Some((uncompacted, cs, sp)) => {
                let cloud_tokens = crate::router::Router::estimate_tokens(&uncompacted);
                decision.est_tokens = cloud_tokens;
                (uncompacted, cs, sp)
            }
            None => (local_body, local_chars_saved, local_savings_pct),
        };
        self.metrics
            .record_local_to_cloud_failover(local_tokens, decision.est_tokens);
        self.dispatch_cloud(
            cloud_body,
            decision,
            req_id,
            true,
            eff_chars_saved,
            eff_savings_pct,
            cache_key,
        )
        .await
    }

    async fn dispatch_cloud(
        &self,
        mut body: Value,
        decision: RouteDecision,
        req_id: u64,
        is_failover: bool,
        chars_saved: usize,
        savings_pct: f64,
        cache_key: Option<String>,
    ) -> Response {
        if decision.local_only {
            warn!(
                req_id = req_id,
                reason = %decision.reason,
                "Defense-in-depth: blocking cloud request because decision.local_only is true"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [
                    ("access-control-allow-origin", "*"),
                    ("content-type", "application/json"),
                    ("x-ferry-provider", "local"),
                    ("x-ferry-failover", "false"),
                ],
                serde_json::json!({
                    "error": {
                        "message": format!(
                            "Local provider unavailable and local-only routing requested: {}",
                            decision.reason
                        ),
                        "type": "ferry_routing_error",
                        "code": "local_unavailable"
                    }
                })
                .to_string(),
            )
                .into_response();
        }

        let start = Instant::now();
        let is_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);

        info!(
            req_id = req_id,
            target = "cloud",
            model = %self.config.cloud.model,
            tokens = decision.est_tokens,
            failover = is_failover,
            reason = %decision.reason,
            "Routing to Cloud Provider"
        );

        // Strip local-specific id_slot parameter before sending to Cloud
        if let Some(obj) = body.as_object_mut() {
            obj.remove("id_slot");
        }

        let cloud_url = format!("{}/chat/completions", self.config.cloud.api_base.trim_end_matches('/'));
        normalize_wire_format(&mut body, &self.config.cloud.model);

        let req = self
            .client
            .post(&cloud_url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.config.cloud.api_key))
            .header(header::CONTENT_TYPE, "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(self.config.cloud.timeout_secs));

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let duration = start.elapsed();
                self.metrics.record_cloud_completion(duration.as_millis());
                let is_cloud_overload_or_unavailable = status == StatusCode::TOO_MANY_REQUESTS
                    || status.as_u16() == 529
                    || status == StatusCode::BAD_GATEWAY
                    || status == StatusCode::SERVICE_UNAVAILABLE
                    || status == StatusCode::GATEWAY_TIMEOUT;
                if is_cloud_overload_or_unavailable {
                    if status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529 {
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            "Cloud provider returned HTTP {} (Rate Limit / Overloaded). Activating 60-second local fallback guard.",
                            status.as_u16()
                        );
                        self.metrics.record_cloud_rate_limit(60);
                    } else {
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            "Cloud provider returned transient HTTP {} error; attempting in-flight local fallback.",
                            status.as_u16()
                        );
                    }

                    // Zero-drop in-flight failover: Attempt local execution ONLY if Strata is healthy,
                    // prompt fits local context (or after rolling compaction / adaptive squeeze), and an actual LocalSlotGuard is acquired.
                    if !is_failover
                        && decision.reason != "routing mode cloud-only"
                        && !self.metrics.is_circuit_open()
                        && self.metrics.is_strata_online()
                    {
                        let max_context = self.config.local.max_context_tokens as u64;
                        let max_slots = self.config.local.max_slots;
                        let total_kv_budget = self.config.local.total_kv_token_budget as u64;

                        let mut est_tokens = if decision.est_tokens > 0 {
                            decision.est_tokens
                        } else {
                            crate::router::Router::estimate_tokens(&body)
                        };
                        let mut fallback_chars_saved = chars_saved;

                        if est_tokens > max_context {
                            let condenser = crate::condenser::ContextCondenser::new(self.config.condensation.clone());
                            if self.config.condensation.enabled && self.config.condensation.rolling_compaction {
                                if let Some(compact_stats) = condenser.rolling_compact(&mut body) {
                                    fallback_chars_saved += compact_stats.chars_saved;
                                    self.metrics.record_rolling_compaction(compact_stats.chars_saved);
                                    est_tokens = crate::router::Router::estimate_tokens(&body);
                                }
                            }

                            if est_tokens > max_context && self.config.local.adaptive_squeezing {
                                let squeeze_limit = (max_context as f64 * self.config.local.adaptive_squeeze_ratio) as u64;
                                if est_tokens <= squeeze_limit {
                                    if let Some(extra_saved) = condenser.adaptive_squeeze(&mut body, max_context) {
                                        fallback_chars_saved += extra_saved;
                                        est_tokens = crate::router::Router::estimate_tokens(&body);
                                        if est_tokens <= max_context {
                                            self.metrics
                                                .adaptive_squeezed_count
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                    }
                                }
                            }

                            if est_tokens > max_context {
                                if let Some(emergency_saved) = condenser.emergency_compact_to_fit(&mut body, max_context) {
                                    fallback_chars_saved += emergency_saved;
                                    est_tokens = crate::router::Router::estimate_tokens(&body);
                                }
                            }
                        }

                        if est_tokens <= max_context {
                            let prefix_hash = body
                                .get("messages")
                                .and_then(|m| m.as_array())
                                .map(|arr| crate::affinity::compute_prefix_hash(arr))
                                .unwrap_or(0);
                            let wait_limit_ms = self.config.local.queue_timeout_ms.min(30_000);

                            if let Some((slot_id, aff_type, slot_guard)) = crate::router::acquire_local_slot_with_wait(
                                &self.metrics,
                                &self.affinity_tracker,
                                max_slots,
                                total_kv_budget,
                                &decision.session_id,
                                prefix_hash,
                                est_tokens,
                                true,
                                false,
                                wait_limit_ms,
                            )
                            .await
                            {
                                warn!(
                                    req_id = req_id,
                                    slot = slot_id,
                                    tokens = est_tokens,
                                    status = status.as_u16(),
                                    "Failing over in-flight request to local Strata engine after Cloud HTTP {}",
                                    status.as_u16()
                                );
                                self.metrics.revert_cloud_request_for_local_fallback(
                                    decision.est_tokens,
                                    decision.is_spillover,
                                );
                                let local_decision = RouteDecision {
                                    target: TargetProvider::Local,
                                    est_tokens,
                                    is_spillover: false,
                                    reason: format!("Failover from Cloud HTTP {} (slot {slot_id})", status.as_u16()),
                                    local_only: true,
                                    slot_guard: Some(slot_guard),
                                    slot_id: Some(slot_id),
                                    is_affinity_hit: aff_type.is_hit(),
                                    affinity_type: aff_type,
                                    session_id: decision.session_id.clone(),
                                };
                                return Box::pin(self.forward_chat(
                                    body,
                                    local_decision,
                                    req_id,
                                    fallback_chars_saved,
                                    savings_pct,
                                    cache_key,
                                ))
                                .await;
                            }
                        }
                    }
                }
                info!(
                    req_id = req_id,
                    target = "cloud",
                    status = status.as_u16(),
                    duration_ms = duration.as_millis(),
                    "Cloud turn completed"
                );
                let meta = ResponseMeta {
                    provider: "cloud",
                    model: self.config.cloud.model.clone(),
                    est_tokens: decision.est_tokens,
                    chars_saved,
                    savings_pct,
                    duration_ms: duration.as_millis(),
                    is_failover,
                    slot_id: None,
                    is_affinity_hit: false,
                    affinity_type: crate::affinity::AffinityType::NewSlot,
                    session_id: decision.session_id,
                };
                let effective_cache_key = if status.is_success() { cache_key } else { None };
                self.handle_upstream_response(resp, is_stream, None, meta, effective_cache_key).await
            }
            Err(err) => {
                error!(req_id = req_id, error = %err, "Cloud request failed");
                (
                    StatusCode::BAD_GATEWAY,
                    [
                        ("access-control-allow-origin", "*"),
                        ("content-type", "application/json"),
                    ],
                    serde_json::json!({ "error": format!("Cloud upstream failed: {}", err) }).to_string(),
                )
                    .into_response()
            }
        }
    }

    fn build_response_headers(
        upstream_headers: &reqwest::header::HeaderMap,
        meta: &ResponseMeta,
    ) -> HeaderMap {
        let mut headers = HeaderMap::new();

        for (k, v) in upstream_headers.iter() {
            let key = k.as_str().to_lowercase();
            if key == "content-type" || key == "cache-control" {
                if let (Ok(hname), Ok(hval)) = (
                    HeaderName::from_bytes(k.as_ref()),
                    HeaderValue::from_bytes(v.as_ref()),
                ) {
                    headers.insert(hname, hval);
                }
            }
        }

        // Always ensure universal CORS headers so browser-based clients (OpenWebUI, dashboards) are never blocked
        headers.insert(HeaderName::from_static("access-control-allow-origin"), HeaderValue::from_static("*"));
        headers.insert(HeaderName::from_static("access-control-allow-methods"), HeaderValue::from_static("GET, POST, OPTIONS"));
        headers.insert(HeaderName::from_static("access-control-allow-headers"), HeaderValue::from_static("content-type, authorization, x-ferry-condense, x-ferry-route, x-ferry-cache, x-ferry-queue-timeout-ms, x-ferry-wait-ms, x-ferry-session-id, x-session-id, x-agent-id"));

        // Observability headers
        headers.insert(HeaderName::from_static("x-ferry-provider"), HeaderValue::from_static(meta.provider));
        if let Ok(val) = HeaderValue::from_str(&meta.model) {
            headers.insert(HeaderName::from_static("x-ferry-model"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.est_tokens.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-est-tokens"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.chars_saved.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-chars-saved"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&format!("{:.2}%", meta.savings_pct)) {
            headers.insert(HeaderName::from_static("x-ferry-savings-percent"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.duration_ms.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-duration-ms"), val);
        }
        headers.insert(
            HeaderName::from_static("x-ferry-failover"),
            HeaderValue::from_static(if meta.is_failover { "true" } else { "false" }),
        );

        // Session & Slot Affinity headers
        if let Some(slot) = meta.slot_id {
            if let Ok(val) = HeaderValue::from_str(&slot.to_string()) {
                headers.insert(HeaderName::from_static("x-ferry-slot"), val);
            }
            headers.insert(
                HeaderName::from_static("x-ferry-affinity"),
                HeaderValue::from_static(meta.affinity_type.as_str()),
            );
        }
        if !meta.session_id.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&meta.session_id) {
                headers.insert(HeaderName::from_static("x-ferry-session-id"), val);
            }
        }

        headers.insert(
            HeaderName::from_static("access-control-expose-headers"),
            HeaderValue::from_static("x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover, x-ferry-cache, x-ferry-slot, x-ferry-affinity, x-ferry-session-id"),
        );

        headers
    }

    async fn handle_upstream_response(
        &self,
        resp: reqwest::Response,
        _is_stream: bool,
        guard: Option<crate::router::LocalSlotGuard>,
        meta: ResponseMeta,
        cache_key: Option<String>,
    ) -> Response {
        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
        let headers = Self::build_response_headers(resp.headers(), &meta);

        let is_sse = _is_stream
            || resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|ct| ct.contains("text/event-stream"))
                .unwrap_or(false);

        // Fast path for non-streaming responses: read bytes directly, release slot immediately, populate cache
        if !_is_stream && !is_sse {
            drop(guard); // Free local slot immediately!
            match resp.bytes().await {
                Ok(body_bytes) => {
                    if status.is_success() {
                        if let Some(key) = cache_key {
                            self.cache.set(key, body_bytes.clone(), status.as_u16(), vec![]).await;
                        }
                    }
                    return (status, headers, body_bytes).into_response();
                }
                Err(err) => {
                    warn!(error = %err, "Failed reading upstream response body; skipping cache");
                    return (
                        StatusCode::BAD_GATEWAY,
                        [
                            ("access-control-allow-origin", "*"),
                            ("content-type", "application/json"),
                        ],
                        serde_json::json!({
                            "error": {
                                "message": format!("Failed reading upstream response body: {}", err),
                                "type": "upstream_error"
                            }
                        })
                        .to_string(),
                    )
                        .into_response();
                }
            }
        }

        // Streaming SSE path with background keepalive heartbeat (5s heartbeat to prevent client socket drops)
        let byte_stream = resp.bytes_stream();
        let guarded_stream = GuardedStream::new(
            byte_stream,
            guard,
            is_sse,
            std::time::Duration::from_secs(5),
        );

        (status, headers, Body::from_stream(guarded_stream)).into_response()
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_local_response_with_prefill_keepalive(
        &self,
        req: reqwest::RequestBuilder,
        body: Value,
        cloud_fallback: Option<(Value, usize, f64)>,
        decision: RouteDecision,
        guard: Option<crate::router::LocalSlotGuard>,
        mut meta: ResponseMeta,
        req_id: u64,
        chars_saved: usize,
        savings_pct: f64,
        cache_key: Option<String>,
        start: Instant,
    ) -> Response {
        let cb_threshold = self.config.local.circuit_breaker_threshold;
        let mut req_fut = Box::pin(req.send());
        let first_keepalive = tokio::time::sleep(std::time::Duration::from_secs(4));
        tokio::pin!(first_keepalive);

        // Await upstream headers up to the initial 4s prefill keepalive tick so fast connect/5xx/429
        // errors fail over cleanly (or return 502/503 if local_only) and fast 4xx errors forward status.
        let initial_outcome = tokio::select! {
            res = &mut req_fut => Some(res),
            _ = &mut first_keepalive => None,
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(64);
        let metrics = self.metrics.clone();

        match initial_outcome {
            Some(Ok(resp)) if resp.status().is_success() => {
                metrics.record_local_success();
                let duration = start.elapsed();
                metrics.record_local_completion(duration.as_millis());
                meta.duration_ms = duration.as_millis();
                info!(
                    req_id = req_id,
                    target = "local",
                    status = resp.status().as_u16(),
                    duration_ms = duration.as_millis(),
                    "Local prefill finished; streaming chunks to client"
                );
                tokio::spawn(async move {
                    let mut stream = resp.bytes_stream();
                    let mut sse_tracker = SseFrameTracker::new();
                    let mut stream_interval = tokio::time::interval(std::time::Duration::from_secs(4));
                    stream_interval.tick().await;

                    loop {
                        tokio::select! {
                            _ = stream_interval.tick() => {
                                if let Some(chunk) = sse_tracker.on_keepalive_tick() {
                                    if tx.send(Ok(chunk)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            item = stream.next() => {
                                match item {
                                    Some(Ok(bytes)) => {
                                        stream_interval.reset();
                                        let bytes = sse_tracker.process_chunk(bytes);
                                        if tx.send(Ok(bytes)).await.is_err() {
                                            break;
                                        }
                                    }
                                    Some(Err(e)) => {
                                        warn!(req_id = req_id, error = %e, "Upstream stream error; closing client stream");
                                        let _ = tx.send(Ok(sse_tracker.on_stream_error())).await;
                                        break;
                                    }
                                    None => {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    drop(guard);
                });
            }
            Some(Ok(resp)) if resp.status().is_client_error() && resp.status() != StatusCode::TOO_MANY_REQUESTS => {
                metrics.record_local_success();
                let status = resp.status();
                let duration = start.elapsed();
                metrics.record_local_completion(duration.as_millis());
                meta.duration_ms = duration.as_millis();
                warn!(
                    req_id = req_id,
                    status = status.as_u16(),
                    duration_ms = duration.as_millis(),
                    "Local provider returned client error on stream request (forwarding status to client)"
                );
                return self.handle_upstream_response(resp, false, guard, meta, None).await;
            }
            Some(Ok(resp)) => {
                let upstream_status = resp.status();
                let err_text = resp.text().await.unwrap_or_default();
                warn!(
                    req_id = req_id,
                    status = upstream_status.as_u16(),
                    error = %err_text,
                    local_only = decision.local_only,
                    "Local provider returned server/rate-limit error on stream prefill"
                );
                let tripped = metrics.record_local_failure(cb_threshold);
                if tripped {
                    warn!(
                        req_id = req_id,
                        threshold = cb_threshold,
                        "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                    );
                }
                drop(guard);
                if decision.local_only {
                    let out_status = if upstream_status == StatusCode::SERVICE_UNAVAILABLE
                        || upstream_status == StatusCode::TOO_MANY_REQUESTS
                    {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::BAD_GATEWAY
                    };
                    return (
                        out_status,
                        [
                            ("access-control-allow-origin", "*"),
                            ("content-type", "application/json"),
                            ("x-ferry-provider", "local"),
                            ("x-ferry-failover", "false"),
                        ],
                        serde_json::json!({
                            "error": {
                                "message": format!("Local upstream error ({}): {}", upstream_status.as_u16(), err_text),
                                "type": "upstream_error"
                            }
                        })
                        .to_string(),
                    )
                        .into_response();
                }
                return self
                    .failover_local_to_cloud(
                        body,
                        cloud_fallback,
                        decision,
                        req_id,
                        chars_saved,
                        savings_pct,
                        cache_key,
                    )
                    .await;
            }
            Some(Err(err)) => {
                warn!(
                    req_id = req_id,
                    error = %err,
                    local_only = decision.local_only,
                    "Local provider stream connection failed"
                );
                let tripped = metrics.record_local_failure(cb_threshold);
                if tripped {
                    warn!(
                        req_id = req_id,
                        threshold = cb_threshold,
                        "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                    );
                }
                drop(guard);
                if decision.local_only {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [
                            ("access-control-allow-origin", "*"),
                            ("content-type", "application/json"),
                            ("x-ferry-provider", "local"),
                            ("x-ferry-failover", "false"),
                        ],
                        serde_json::json!({
                            "error": {
                                "message": format!("Local upstream unreachable: {}", err),
                                "type": "upstream_error"
                            }
                        })
                        .to_string(),
                    )
                        .into_response();
                }
                return self
                    .failover_local_to_cloud(
                        body,
                        cloud_fallback,
                        decision,
                        req_id,
                        chars_saved,
                        savings_pct,
                        cache_key,
                    )
                    .await;
            }
            None => {
                // Prefill exceeded initial 4s keepalive window: commit 200 OK SSE headers and emit keepalive heartbeat
                let keepalive_chunk = bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK);
                let _ = tx.try_send(Ok(keepalive_chunk.clone()));

                tokio::spawn(async move {
                    let mut prefill_interval = tokio::time::interval(std::time::Duration::from_secs(4));
                    prefill_interval.tick().await; // consume immediate first tick

                    let resp = loop {
                        tokio::select! {
                            _ = prefill_interval.tick() => {
                                if tx.send(Ok(keepalive_chunk.clone())).await.is_err() {
                                    warn!(req_id = req_id, "Client disconnected during upstream prefill; dropping turn");
                                    drop(guard);
                                    return;
                                }
                            }
                            res = &mut req_fut => {
                                break res;
                            }
                        }
                    };

                    match resp {
                        Ok(resp) if resp.status().is_success() => {
                            metrics.record_local_success();
                            let duration = start.elapsed();
                            metrics.record_local_completion(duration.as_millis());
                            info!(
                                req_id = req_id,
                                target = "local",
                                status = resp.status().as_u16(),
                                duration_ms = duration.as_millis(),
                                "Local prefill finished; streaming chunks to client"
                            );

                            let mut stream = resp.bytes_stream();
                            let mut sse_tracker = SseFrameTracker::new();
                            let mut stream_interval = tokio::time::interval(std::time::Duration::from_secs(4));
                            stream_interval.tick().await;

                            loop {
                                tokio::select! {
                                    _ = stream_interval.tick() => {
                                        if let Some(chunk) = sse_tracker.on_keepalive_tick() {
                                            if tx.send(Ok(chunk)).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    item = stream.next() => {
                                        match item {
                                            Some(Ok(bytes)) => {
                                                stream_interval.reset();
                                                let bytes = sse_tracker.process_chunk(bytes);
                                                if tx.send(Ok(bytes)).await.is_err() {
                                                    break;
                                                }
                                            }
                                            Some(Err(e)) => {
                                                warn!(req_id = req_id, error = %e, "Upstream stream error; closing client stream");
                                                let _ = tx.send(Ok(sse_tracker.on_stream_error())).await;
                                                break;
                                            }
                                            None => {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            drop(guard);
                        }
                        Ok(resp) if resp.status().is_client_error() && resp.status() != StatusCode::TOO_MANY_REQUESTS => {
                            metrics.record_local_success();
                            let status = resp.status();
                            let err_text = resp.text().await.unwrap_or_default();
                            warn!(req_id = req_id, status = status.as_u16(), error = %err_text, "Local provider returned client error after prefill commit");
                            let _ = tx.send(Ok(format_sse_error_frame(&err_text))).await;
                            drop(guard);
                        }
                        Ok(resp) => {
                            let status = resp.status();
                            let err_text = resp.text().await.unwrap_or_default();
                            warn!(req_id = req_id, status = status.as_u16(), error = %err_text, "Local provider returned server error after prefill commit");
                            metrics.record_local_failure(cb_threshold);
                            let _ = tx.send(Ok(format_sse_error_frame(&format!("Local upstream error: {}", err_text)))).await;
                            drop(guard);
                        }
                        Err(e) => {
                            error!(req_id = req_id, error = %e, "Local stream request failed after prefill commit");
                            metrics.record_local_failure(cb_threshold);
                            let _ = tx.send(Ok(format_sse_error_frame(&format!("Upstream request failed: {}", e)))).await;
                            drop(guard);
                        }
                    }
                });
            }
        }

        // Assemble 200 OK SSE headers
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(HeaderName::from_static("access-control-allow-origin"), HeaderValue::from_static("*"));
        headers.insert(HeaderName::from_static("access-control-allow-methods"), HeaderValue::from_static("GET, POST, OPTIONS"));
        headers.insert(HeaderName::from_static("access-control-allow-headers"), HeaderValue::from_static("content-type, authorization, x-ferry-condense, x-ferry-route, x-ferry-cache, x-ferry-queue-timeout-ms, x-ferry-wait-ms, x-ferry-session-id, x-session-id, x-agent-id"));
        headers.insert(HeaderName::from_static("x-ferry-provider"), HeaderValue::from_static(meta.provider));
        if let Ok(val) = HeaderValue::from_str(&meta.model) {
            headers.insert(HeaderName::from_static("x-ferry-model"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.est_tokens.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-est-tokens"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.chars_saved.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-chars-saved"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&format!("{:.2}%", meta.savings_pct)) {
            headers.insert(HeaderName::from_static("x-ferry-savings-percent"), val);
        }
        if let Ok(val) = HeaderValue::from_str(&meta.duration_ms.to_string()) {
            headers.insert(HeaderName::from_static("x-ferry-duration-ms"), val);
        }
        headers.insert(HeaderName::from_static("x-ferry-failover"), HeaderValue::from_static("false"));
        if let Some(slot) = meta.slot_id {
            if let Ok(val) = HeaderValue::from_str(&slot.to_string()) {
                headers.insert(HeaderName::from_static("x-ferry-slot"), val);
            }
            headers.insert(HeaderName::from_static("x-ferry-affinity"), HeaderValue::from_static(meta.affinity_type.as_str()));
        }
        if !meta.session_id.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&meta.session_id) {
                headers.insert(HeaderName::from_static("x-ferry-session-id"), val);
            }
        }
        headers.insert(
            HeaderName::from_static("access-control-expose-headers"),
            HeaderValue::from_static("x-ferry-provider, x-ferry-model, x-ferry-est-tokens, x-ferry-chars-saved, x-ferry-savings-percent, x-ferry-duration-ms, x-ferry-failover, x-ferry-cache, x-ferry-slot, x-ferry-affinity, x-ferry-session-id"),
        );

        let receiver_stream = ReceiverStream::new(rx);
        (StatusCode::OK, headers, Body::from_stream(receiver_stream)).into_response()
    }
}

pub fn normalize_wire_format(body: &mut Value, target_model: &str) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".to_string(), Value::String(target_model.to_string()));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SseScanState {
    EventBoundary,
    AfterSingleNewline,
    AfterFirstCr,
    AfterNewlineCr,
    MidLine,
}

#[derive(Debug, Clone)]
struct SseFrameTracker {
    state: SseScanState,
    pending_keepalive: bool,
}

impl SseFrameTracker {
    fn new() -> Self {
        Self {
            state: SseScanState::EventBoundary,
            pending_keepalive: false,
        }
    }

    fn at_event_boundary(&self) -> bool {
        matches!(self.state, SseScanState::EventBoundary)
    }

    fn advance(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state = match (self.state, b) {
                (SseScanState::EventBoundary, b'\n') => SseScanState::EventBoundary,
                (SseScanState::EventBoundary, b'\r') => SseScanState::AfterNewlineCr,
                (SseScanState::EventBoundary, _) => SseScanState::MidLine,

                (SseScanState::MidLine, b'\n') => SseScanState::AfterSingleNewline,
                (SseScanState::MidLine, b'\r') => SseScanState::AfterFirstCr,
                (SseScanState::MidLine, _) => SseScanState::MidLine,

                (SseScanState::AfterFirstCr, b'\n') => SseScanState::AfterSingleNewline,
                (SseScanState::AfterFirstCr, b'\r') => SseScanState::AfterNewlineCr,
                (SseScanState::AfterFirstCr, _) => SseScanState::MidLine,

                (SseScanState::AfterSingleNewline, b'\n') => SseScanState::EventBoundary,
                (SseScanState::AfterSingleNewline, b'\r') => SseScanState::AfterNewlineCr,
                (SseScanState::AfterSingleNewline, _) => SseScanState::MidLine,

                (SseScanState::AfterNewlineCr, b'\n') => SseScanState::EventBoundary,
                (SseScanState::AfterNewlineCr, b'\r') => SseScanState::EventBoundary,
                (SseScanState::AfterNewlineCr, _) => SseScanState::MidLine,
            };
        }
    }

    fn process_chunk(&mut self, bytes: bytes::Bytes) -> bytes::Bytes {
        if bytes.is_empty() {
            return bytes;
        }
        let was_at_event_boundary = self.at_event_boundary();
        self.advance(bytes.as_ref());

        if was_at_event_boundary && is_pure_sse_comment_chunk(bytes.as_ref(), false) {
            self.state = SseScanState::EventBoundary;
            self.pending_keepalive = false;
            return bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK);
        }

        if self.pending_keepalive && self.at_event_boundary() {
            self.pending_keepalive = false;
            let mut out = bytes::BytesMut::with_capacity(bytes.len() + KEEPALIVE_SSE_CHUNK.len());
            out.extend_from_slice(bytes.as_ref());
            out.extend_from_slice(KEEPALIVE_SSE_CHUNK);
            return out.freeze();
        }

        bytes
    }

    fn on_keepalive_tick(&mut self) -> Option<bytes::Bytes> {
        if self.at_event_boundary() {
            self.pending_keepalive = false;
            Some(bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK))
        } else {
            self.pending_keepalive = true;
            None
        }
    }

    fn on_stream_error(&mut self) -> bytes::Bytes {
        let was_boundary = self.at_event_boundary();
        self.state = SseScanState::EventBoundary;
        self.pending_keepalive = false;
        if was_boundary {
            bytes::Bytes::from_static(DONE_SSE_CHUNK)
        } else {
            bytes::Bytes::from_static(b"\n\ndata: [DONE]\n\n")
        }
    }
}

fn is_pure_sse_comment_chunk(s: &[u8], reject_json_tail: bool) -> bool {
    if !s.ends_with(b"\n") {
        return false;
    }
    let mut saw_comment_line = false;
    for raw_line in s.split(|&b| b == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        // SSE comment lines MUST begin with ':' at column 0 (never preceded by space/tab).
        if !line.starts_with(b":") {
            return false;
        }
        if reject_json_tail && looks_like_json_value_tail(&line[1..]) {
            return false;
        }
        saw_comment_line = true;
    }
    saw_comment_line
}

fn looks_like_json_value_tail(after_colon: &[u8]) -> bool {
    let mut trimmed = after_colon;
    while let Some(rest) = trimmed.strip_prefix(b" ").or_else(|| trimmed.strip_prefix(b"\t")) {
        trimmed = rest;
    }
    trimmed.ends_with(b"}")
        || trimmed.ends_with(b"]")
        || trimmed.starts_with(b"\"")
}

pub fn format_sse_error_frame(err_msg: &str) -> bytes::Bytes {
    let payload = serde_json::json!({
        "error": {
            "message": err_msg,
            "type": "upstream_error"
        }
    })
    .to_string();
    bytes::Bytes::from(format!("data: {}\n\ndata: [DONE]\n\n", payload))
}

pub fn transform_strata_chunk(bytes: bytes::Bytes) -> bytes::Bytes {
    if is_pure_sse_comment_chunk(bytes.as_ref(), true) {
        bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK)
    } else {
        bytes
    }
}

pub const STRIX_LIFECYCLE_TOOLS: &[&str] = &[
    "finish_scan",
    "finish_agent",
    "wait_for_message",
    "create_vulnerability_report",
    "record_coverage",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalToolInspection {
    ValidUnchanged,
    Repaired(bytes::Bytes),
    FailoverRequired { reason: String },
}

pub fn extract_declared_tools(req_body: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(tools) = req_body.get("tools").and_then(|t| t.as_array()) {
        for tool in tools {
            if let Some(name) = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| tool.get("name"))
                .and_then(|n| n.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn is_valid_tool_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn declared_param_type<'a>(req_body: &'a Value, func_name: &str, param_name: &str) -> Option<&'a str> {
    let tools = req_body.get("tools")?.as_array()?;
    for tool in tools {
        let f = tool.get("function").unwrap_or(tool);
        if f.get("name").and_then(|n| n.as_str()) == Some(func_name) {
            return f
                .get("parameters")
                .and_then(|p| p.get("properties"))
                .and_then(|props| props.get(param_name))
                .and_then(|prop| prop.get("type"))
                .and_then(|t| t.as_str());
        }
    }
    None
}

fn coerce_xml_param_value(raw_val: &str, expected_type: Option<&str>) -> Value {
    let clean = raw_val
        .strip_prefix("\r\n")
        .or_else(|| raw_val.strip_prefix('\n'))
        .unwrap_or(raw_val);
    let clean = clean
        .strip_suffix("\r\n")
        .or_else(|| clean.strip_suffix('\n'))
        .unwrap_or(clean);

    if expected_type == Some("string") {
        return Value::String(clean.to_string());
    }

    let trimmed = clean.trim();
    match expected_type {
        Some("boolean") => match trimmed {
            "true" => return Value::Bool(true),
            "false" => return Value::Bool(false),
            _ => {}
        },
        Some("integer") => {
            if let Ok(i) = trimmed.parse::<i64>() {
                return Value::Number(i.into());
            }
        }
        Some("number") => {
            if let Ok(f) = trimmed.parse::<f64>() {
                if let Some(num) = serde_json::Number::from_f64(f) {
                    return Value::Number(num);
                }
            }
        }
        Some("array" | "object") => {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                return v;
            }
        }
        _ => {
            if trimmed == "true" {
                return Value::Bool(true);
            }
            if trimmed == "false" {
                return Value::Bool(false);
            }
            if let Ok(i) = trimmed.parse::<i64>() {
                return Value::Number(i.into());
            }
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                    return v;
                }
            }
        }
    }

    Value::String(clean.to_string())
}

fn parse_single_xml_tool_call_block(
    inner: &str,
    req_body: &Value,
    declared_tools: &[String],
) -> Option<(String, String)> {
    let trimmed = inner.trim();
    if trimmed.is_empty() || trimmed.contains("<tool_call") || trimmed.contains("</tool_call>") {
        return None;
    }

    // Format 1: JSON-in-XML: `<tool_call>{"name": "...", "arguments": ...}</tool_call>`
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let val = serde_json::from_str::<Value>(trimmed).ok()?;
        let name = val
            .get("name")
            .or_else(|| val.get("function"))
            .and_then(|v| v.as_str())?
            .trim();
        if !is_valid_tool_identifier(name) {
            return None;
        }
        if !declared_tools.is_empty() && !declared_tools.iter().any(|d| d == name) {
            return None;
        }
        let args_str = match val.get("arguments").or_else(|| val.get("parameters")) {
            Some(Value::String(s)) => {
                serde_json::from_str::<Value>(s).ok()?;
                s.clone()
            }
            Some(v @ Value::Object(_)) => serde_json::to_string(v).ok()?,
            None => "{}".to_string(),
            _ => return None,
        };
        return Some((name.to_string(), args_str));
    }

    // Format 2: Qwen3 XML: `<function=NAME> ... (<parameter=K>V</parameter>)* ... </function>`
    let after_fn = trimmed.strip_prefix("<function=")?;
    let gt_idx = after_fn.find('>')?;
    let func_name = after_fn[..gt_idx].trim();
    if !is_valid_tool_identifier(func_name) {
        return None;
    }
    if !declared_tools.is_empty() && !declared_tools.iter().any(|d| d == func_name) {
        return None;
    }

    let mut body_part = after_fn[gt_idx + 1..].trim();
    if let Some(stripped) = body_part.strip_suffix("</function>") {
        body_part = stripped.trim();
    }
    if body_part.contains("<function=") || body_part.contains("</function>") {
        return None;
    }

    let mut args_map = serde_json::Map::new();
    let mut p_pos = 0;
    while p_pos < body_part.len() {
        let rem = &body_part[p_pos..];
        let trimmed_rem = rem.trim_start();
        p_pos += rem.len() - trimmed_rem.len();
        if p_pos >= body_part.len() {
            break;
        }

        let after_param = body_part[p_pos..].strip_prefix("<parameter=")?;
        let p_gt = after_param.find('>')?;
        let param_name = after_param[..p_gt].trim();
        if !is_valid_tool_identifier(param_name) {
            return None;
        }

        let val_start = p_pos + "<parameter=".len() + p_gt + 1;
        let val_end_rel = body_part[val_start..].find("</parameter>")?;
        let raw_val = &body_part[val_start..val_start + val_end_rel];
        if raw_val.contains("<parameter=") {
            return None;
        }

        let expected_type = declared_param_type(req_body, func_name, param_name);
        let coerced = coerce_xml_param_value(raw_val, expected_type);
        args_map.insert(param_name.to_string(), coerced);

        p_pos = val_start + val_end_rel + "</parameter>".len();
    }

    let args_str = serde_json::to_string(&Value::Object(args_map)).ok()?;
    Some((func_name.to_string(), args_str))
}

fn try_recover_xml_tool_calls(
    content: &str,
    req_body: &Value,
    declared_tools: &[String],
) -> Option<(Vec<Value>, String)> {
    if !content.contains("<tool_call>") {
        return None;
    }

    let mut cleaned_content = String::new();
    let mut recovered_calls = Vec::new();
    let mut pos = 0;

    while let Some(rel_start) = content[pos..].find("<tool_call>") {
        let block_start = pos + rel_start;
        cleaned_content.push_str(&content[pos..block_start]);

        let inner_start = block_start + "<tool_call>".len();
        let rel_end = content[inner_start..].find("</tool_call>")?;
        let block_end = inner_start + rel_end;
        let inner = &content[inner_start..block_end];

        let (func_name, args_json_str) =
            parse_single_xml_tool_call_block(inner, req_body, declared_tools)?;
        let idx = recovered_calls.len();
        recovered_calls.push(serde_json::json!({
            "id": format!("call_ferry_{idx}"),
            "type": "function",
            "function": {
                "name": func_name,
                "arguments": args_json_str
            }
        }));

        pos = block_end + "</tool_call>".len();
    }

    cleaned_content.push_str(&content[pos..]);

    if recovered_calls.is_empty()
        || cleaned_content.contains("<tool_call")
        || cleaned_content.contains("</tool_call>")
        || cleaned_content.contains("<function=")
        || cleaned_content.contains("</function>")
    {
        return None;
    }

    Some((recovered_calls, cleaned_content.trim().to_string()))
}

fn tool_choice_is_none(req_body: &Value) -> bool {
    matches!(
        req_body
            .get("tool_choice")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref(),
        Some("none")
    )
}

fn tool_choice_requires_tool(req_body: &Value) -> bool {
    match req_body.get("tool_choice") {
        Some(Value::String(s)) => matches!(s.trim().to_ascii_lowercase().as_str(), "required" | "any"),
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

pub fn strip_think_blocks(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(start) = out.find("<think>") {
        if let Some(end_rel) = out[start..].find("</think>") {
            let end = start + end_rel + "</think>".len();
            out.replace_range(start..end, "");
        } else {
            out.truncate(start);
            break;
        }
    }
    while let Some(close_idx) = out.find("</think>") {
        out.replace_range(..close_idx + "</think>".len(), "");
    }
    out
}

fn extract_assistant_content_str(message: &Value) -> String {
    if let Some(s) = message.get("content").and_then(|c| c.as_str()) {
        return s.to_string();
    }
    if let Some(parts) = message.get("content").and_then(|c| c.as_array()) {
        let mut buf = String::new();
        for part in parts {
            if let Some(t) = part
                .get("text")
                .and_then(|v| v.as_str())
                .or_else(|| part.as_str())
            {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(t);
            }
        }
        return buf;
    }
    String::new()
}

pub fn inspect_and_repair_local_tool_response(
    req_body: &Value,
    resp_bytes: &[u8],
) -> LocalToolInspection {
    if tool_choice_is_none(req_body) {
        return LocalToolInspection::ValidUnchanged;
    }

    let declared_tools = extract_declared_tools(req_body);
    if declared_tools.is_empty() {
        return LocalToolInspection::ValidUnchanged;
    }

    let Ok(mut resp_json) = serde_json::from_slice::<Value>(resp_bytes) else {
        return LocalToolInspection::ValidUnchanged;
    };

    let Some(choice0) = resp_json
        .get_mut("choices")
        .and_then(|c| c.as_array_mut())
        .and_then(|arr| arr.first_mut())
    else {
        return LocalToolInspection::ValidUnchanged;
    };

    let Some(message) = choice0.get_mut("message") else {
        return LocalToolInspection::ValidUnchanged;
    };

    let has_existing_tool_calls = message
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);

    let existing_tool_calls_valid = message
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|arr| {
            !arr.is_empty()
                && arr.iter().all(|call| {
                    let name = call
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .or_else(|| call.get("name"))
                        .and_then(|n| n.as_str())
                        .map(str::trim)
                        .unwrap_or("");
                    let args_valid = match call
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .or_else(|| call.get("arguments"))
                    {
                        Some(Value::Object(_)) => true,
                        Some(Value::String(s)) => serde_json::from_str::<Value>(s)
                            .ok()
                            .map(|v| v.is_object())
                            .unwrap_or(false),
                        _ => false,
                    };
                    is_valid_tool_identifier(name)
                        && declared_tools.iter().any(|d| d == name)
                        && args_valid
                })
        })
        .unwrap_or(false);

    if existing_tool_calls_valid {
        return LocalToolInspection::ValidUnchanged;
    }

    let raw_content_str = extract_assistant_content_str(message);
    let visible_content = strip_think_blocks(&raw_content_str);
    let reasoning_str = message
        .get("reasoning_content")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();

    // Step 1: In-place Qwen XML / JSON <tool_call> recovery when structured tool_calls are missing/empty
    if !has_existing_tool_calls {
        if visible_content.contains("<tool_call>") {
            if let Some((recovered_calls, cleaned_content)) =
                try_recover_xml_tool_calls(&visible_content, req_body, &declared_tools)
            {
                if let Some(msg_obj) = message.as_object_mut() {
                    msg_obj.insert("tool_calls".to_string(), Value::Array(recovered_calls));
                    msg_obj.insert("content".to_string(), Value::String(cleaned_content));
                }
                if let Some(choice_obj) = choice0.as_object_mut() {
                    choice_obj.insert(
                        "finish_reason".to_string(),
                        Value::String("tool_calls".to_string()),
                    );
                }
                if let Ok(repaired_vec) = serde_json::to_vec(&resp_json) {
                    return LocalToolInspection::Repaired(bytes::Bytes::from(repaired_vec));
                }
            }
        } else if let Some(tc_idx) = reasoning_str.find("<tool_call>") {
            let reasoning_prefix = reasoning_str[..tc_idx].trim_end().to_string();
            let reasoning_tc_slice = &reasoning_str[tc_idx..];
            if let Some((recovered_calls, cleaned_tail)) =
                try_recover_xml_tool_calls(reasoning_tc_slice, req_body, &declared_tools)
            {
                let updated_reasoning = if cleaned_tail.is_empty() {
                    reasoning_prefix
                } else if reasoning_prefix.is_empty() {
                    cleaned_tail
                } else {
                    format!("{reasoning_prefix}\n{cleaned_tail}")
                };
                if let Some(msg_obj) = message.as_object_mut() {
                    msg_obj.insert("tool_calls".to_string(), Value::Array(recovered_calls));
                    msg_obj.insert(
                        "reasoning_content".to_string(),
                        Value::String(updated_reasoning),
                    );
                    msg_obj.insert(
                        "content".to_string(),
                        Value::String(visible_content.trim().to_string()),
                    );
                }
                if let Some(choice_obj) = choice0.as_object_mut() {
                    choice_obj.insert(
                        "finish_reason".to_string(),
                        Value::String("tool_calls".to_string()),
                    );
                }
                if let Ok(repaired_vec) = serde_json::to_vec(&resp_json) {
                    return LocalToolInspection::Repaired(bytes::Bytes::from(repaired_vec));
                }
            }
        }
    }

    // Step 2: Failover to Cloud on invalid/missing tool_calls when a tool call is mandatory
    if has_existing_tool_calls && !existing_tool_calls_valid {
        return LocalToolInspection::FailoverRequired {
            reason: "local response contains undeclared/malformed tool_calls function name or invalid JSON arguments"
                .to_string(),
        };
    }

    if visible_content.contains("<tool_call")
        || visible_content.contains("</tool_call>")
        || visible_content.contains("<function=")
        || reasoning_str.contains("<tool_call")
    {
        return LocalToolInspection::FailoverRequired {
            reason: "local response contains unparsed or malformed <tool_call>/<function=> tag in content"
                .to_string(),
        };
    }

    if visible_content.trim().is_empty() && !has_existing_tool_calls {
        return LocalToolInspection::FailoverRequired {
            reason: "local response returned empty content and empty tool_calls on tool-enabled turn"
                .to_string(),
        };
    }

    if tool_choice_requires_tool(req_body) {
        return LocalToolInspection::FailoverRequired {
            reason: "local response omitted tool_calls when tool_choice requires a tool".to_string(),
        };
    }

    if let Some(lifecycle_tool) = declared_tools
        .iter()
        .find(|d| STRIX_LIFECYCLE_TOOLS.contains(&d.as_str()))
    {
        return LocalToolInspection::FailoverRequired {
            reason: format!(
                "local response emitted text-only completion without tool_calls on Strix lifecycle turn ('{lifecycle_tool}')"
            ),
        };
    }

    LocalToolInspection::ValidUnchanged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_zero_allocation_keepalive_and_done() {
        assert_eq!(bytes::Bytes::from_static(DONE_SSE_CHUNK), bytes::Bytes::from("data: [DONE]\n\n"));
        let keepalive = bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK);
        assert!(keepalive.starts_with(b"data: {"));
        assert!(keepalive.ends_with(b"\n\n"));
    }

    #[test]
    fn test_transform_strata_chunk_comment() {
        let comment_chunk = bytes::Bytes::from(": ping keepalive\n\n");
        let transformed = transform_strata_chunk(comment_chunk);
        assert_eq!(transformed, bytes::Bytes::from_static(KEEPALIVE_SSE_CHUNK));

        let data_chunk = bytes::Bytes::from("data: {\"choices\":[]}\n\n");
        let untransformed = transform_strata_chunk(data_chunk.clone());
        assert_eq!(untransformed, data_chunk);
    }

    #[test]
    fn test_recover_qwen_xml_tool_call_single_and_multi_parameter() {
        let req_body = json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "execute_bash",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "command": { "type": "string" },
                                "timeout": { "type": "integer" }
                            }
                        }
                    }
                },
                {
                    "type": "function",
                    "function": {
                        "name": "finish_scan"
                    }
                }
            ]
        });

        let resp_body = json!({
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Let me run a quick check first.\n<tool_call>\n<function=execute_bash>\n<parameter=command>\ncurl -s http://localhost:3000/api/health\n</parameter>\n<parameter=timeout>\n30\n</parameter>\n</function>\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });

        let outcome = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&resp_body).unwrap(),
        );
        let LocalToolInspection::Repaired(repaired_bytes) = outcome else {
            panic!("Expected Repaired outcome, got: {outcome:?}");
        };

        let repaired: Value = serde_json::from_slice(&repaired_bytes).unwrap();
        assert_eq!(repaired["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            repaired["choices"][0]["message"]["content"],
            "Let me run a quick check first."
        );
        let tcs = repaired["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["function"]["name"], "execute_bash");
        let args: Value =
            serde_json::from_str(tcs[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["command"], "curl -s http://localhost:3000/api/health");
        assert_eq!(args["timeout"], 30);
    }

    #[test]
    fn test_recover_json_in_xml_tool_call() {
        let req_body = json!({
            "tools": [
                { "type": "function", "function": { "name": "finish_scan" } }
            ]
        });

        let resp_body = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<tool_call>\n{\"name\": \"finish_scan\", \"arguments\": {\"summary\": \"All checks complete\"}}\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });

        let outcome = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&resp_body).unwrap(),
        );
        let LocalToolInspection::Repaired(repaired_bytes) = outcome else {
            panic!("Expected Repaired outcome, got: {outcome:?}");
        };

        let repaired: Value = serde_json::from_slice(&repaired_bytes).unwrap();
        assert_eq!(repaired["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(repaired["choices"][0]["message"]["content"], "");
        assert_eq!(
            repaired["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "finish_scan"
        );
    }

    #[test]
    fn test_malformed_or_truncated_tool_call_triggers_cloud_failover() {
        let req_body = json!({
            "tools": [
                { "type": "function", "function": { "name": "send_message" } }
            ]
        });

        // 1. Malformed `<function=send_message_to=message>`
        let malformed_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<tool_call>\n<function=send_message_to=message>\n<parameter=text>hello</parameter>\n</function>\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });
        assert!(matches!(
            inspect_and_repair_local_tool_response(
                &req_body,
                &serde_json::to_vec(&malformed_resp).unwrap()
            ),
            LocalToolInspection::FailoverRequired { .. }
        ));

        // 2. Truncated `<tool_call>` without closing `</tool_call>`
        let truncated_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<tool_call>\n<function=send_message>\n<parameter=text>cut off mid stream"
                },
                "finish_reason": "length"
            }]
        });
        assert!(matches!(
            inspect_and_repair_local_tool_response(
                &req_body,
                &serde_json::to_vec(&truncated_resp).unwrap()
            ),
            LocalToolInspection::FailoverRequired { .. }
        ));

        // 3. Empty `content` + empty `tool_calls`
        let empty_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "   \n  ",
                    "reasoning_content": "Thinking about calling a tool..."
                },
                "finish_reason": "stop"
            }]
        });
        assert!(matches!(
            inspect_and_repair_local_tool_response(
                &req_body,
                &serde_json::to_vec(&empty_resp).unwrap()
            ),
            LocalToolInspection::FailoverRequired { .. }
        ));
    }

    #[test]
    fn test_text_only_response_with_finish_scan_triggers_failover_but_normal_tools_do_not() {
        // When `finish_scan` is in `tools`, a text-only response without `tool_calls` would cause
        // `strix.core.runner` to abort with `ended without calling finish_scan` -> must failover!
        let strix_req = json!({
            "tools": [
                { "type": "function", "function": { "name": "execute_bash" } },
                { "type": "function", "function": { "name": "finish_scan" } }
            ]
        });
        let text_only_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "I have completed the scan and verified all endpoints."
                },
                "finish_reason": "stop"
            }]
        });
        let strix_outcome = inspect_and_repair_local_tool_response(
            &strix_req,
            &serde_json::to_vec(&text_only_resp).unwrap(),
        );
        assert!(
            matches!(strix_outcome, LocalToolInspection::FailoverRequired { .. }),
            "Text-only response when finish_scan is declared must trigger Cloud failover, got: {strix_outcome:?}"
        );

        // When no lifecycle tools are present and `tool_choice` is not required, a normal text response stays local.
        let standard_req = json!({
            "tools": [
                { "type": "function", "function": { "name": "read_file" } },
                { "type": "function", "function": { "name": "execute_bash" } }
            ]
        });
        let standard_outcome = inspect_and_repair_local_tool_response(
            &standard_req,
            &serde_json::to_vec(&text_only_resp).unwrap(),
        );
        assert_eq!(standard_outcome, LocalToolInspection::ValidUnchanged);

        // When `tool_choice` is explicitly `"none"`, text-only responses must NOT failover even if lifecycle tools are in `tools`.
        let strix_tool_choice_none_req = json!({
            "tool_choice": "none",
            "tools": [
                { "type": "function", "function": { "name": "execute_bash" } },
                { "type": "function", "function": { "name": "finish_scan" } }
            ]
        });
        let none_outcome = inspect_and_repair_local_tool_response(
            &strix_tool_choice_none_req,
            &serde_json::to_vec(&text_only_resp).unwrap(),
        );
        assert_eq!(none_outcome, LocalToolInspection::ValidUnchanged);
    }

    #[test]
    fn test_native_tool_calls_with_truncated_or_malformed_json_arguments_triggers_failover() {
        let req_body = json!({
            "tools": [
                { "type": "function", "function": { "name": "execute_bash" } }
            ]
        });

        let truncated_args_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "execute_bash",
                            "arguments": "{\"command\": \"curl -s http://localhost:3000/api/items"
                        }
                    }]
                },
                "finish_reason": "length"
            }]
        });

        let outcome = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&truncated_args_resp).unwrap(),
        );
        assert!(
            matches!(outcome, LocalToolInspection::FailoverRequired { .. }),
            "Native tool_calls with truncated JSON arguments must trigger Cloud failover, got: {outcome:?}"
        );
    }

    #[test]
    fn test_think_tags_reasoning_content_and_array_content_tool_recovery() {
        let req_body = json!({
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "execute_bash",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "command": { "type": "string" }
                            }
                        }
                    }
                }
            ]
        });

        // 1. `<think>` block mentioning `<function=undeclared_tool>` before a real `<tool_call>`
        let think_plus_tool_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<think>I should not use <function=undeclared_tool> here, I will use execute_bash.</think>\nRunning check:\n<tool_call>\n<function=execute_bash>\n<parameter=command>id</parameter>\n</function>\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });
        let outcome1 = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&think_plus_tool_resp).unwrap(),
        );
        let LocalToolInspection::Repaired(bytes1) = outcome1 else {
            panic!("Expected Repaired when <think> mentions <function=...>, got: {outcome1:?}");
        };
        let repaired1: Value = serde_json::from_slice(&bytes1).unwrap();
        assert_eq!(
            repaired1["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "execute_bash"
        );
        assert_eq!(repaired1["choices"][0]["message"]["content"], "Running check:");

        // 2. `<think>...</think>`-only response with no visible content and no tool_calls triggers FailoverRequired
        let think_only_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "<think>Thinking about the next step but ran out of budget...</think>"
                },
                "finish_reason": "length"
            }]
        });
        assert!(matches!(
            inspect_and_repair_local_tool_response(
                &req_body,
                &serde_json::to_vec(&think_only_resp).unwrap()
            ),
            LocalToolInspection::FailoverRequired { .. }
        ));

        // 3. Array-of-parts `message.content` containing `<tool_call>`
        let array_parts_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "Checking files:" },
                        { "type": "text", "text": "<tool_call>\n<function=execute_bash>\n<parameter=command>ls</parameter>\n</function>\n</tool_call>" }
                    ]
                },
                "finish_reason": "stop"
            }]
        });
        let outcome3 = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&array_parts_resp).unwrap(),
        );
        assert!(matches!(outcome3, LocalToolInspection::Repaired(_)));

        // 4. `<tool_call>` emitted inside `reasoning_content` when `content` is empty/null
        let reasoning_tc_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "I need to run whoami.\n<tool_call>\n<function=execute_bash>\n<parameter=command>whoami</parameter>\n</function>\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });
        let outcome4 = inspect_and_repair_local_tool_response(
            &req_body,
            &serde_json::to_vec(&reasoning_tc_resp).unwrap(),
        );
        let LocalToolInspection::Repaired(bytes4) = outcome4 else {
            panic!("Expected Repaired from reasoning_content <tool_call>, got: {outcome4:?}");
        };
        let repaired4: Value = serde_json::from_slice(&bytes4).unwrap();
        assert_eq!(
            repaired4["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "execute_bash"
        );
        assert_eq!(
            repaired4["choices"][0]["message"]["reasoning_content"],
            "I need to run whoami."
        );
        assert_eq!(repaired4["choices"][0]["message"]["content"], "");

        // 5. Orphaned `</think>` prefix (when chat template injects opening `<think>` in prompt)
        let send_msg_req = json!({
            "tools": [
                { "type": "function", "function": { "name": "send_message" } }
            ]
        });
        let orphan_close_think_resp = json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Thinking about calling <function=foo>...</think>\n<tool_call>\n<function=send_message>\n<parameter=target_agent_id>a1</parameter>\n<parameter=message>hi</parameter>\n</function>\n</tool_call>"
                },
                "finish_reason": "stop"
            }]
        });
        let outcome5 = inspect_and_repair_local_tool_response(
            &send_msg_req,
            &serde_json::to_vec(&orphan_close_think_resp).unwrap(),
        );
        let LocalToolInspection::Repaired(bytes5) = outcome5 else {
            panic!("Expected Repaired on orphaned </think> prefix, got: {outcome5:?}");
        };
        let repaired5: Value = serde_json::from_slice(&bytes5).unwrap();
        assert_eq!(
            repaired5["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "send_message"
        );
        assert_eq!(repaired5["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn test_failover_metrics_do_not_double_count_requests_or_tokens() {
        let tracker = crate::stats::MetricsTracker::new();

        // 1. Local -> Cloud failover: initial local 500 tokens, fails over to Cloud with 800 tokens (uncompacted)
        tracker.record_local_request_stats(500);
        assert_eq!(tracker.total_requests.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(tracker.local_requests.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(tracker.total_tokens_est.load(std::sync::atomic::Ordering::Relaxed), 500);

        tracker.record_local_to_cloud_failover(500, 800);
        let snap1 = tracker.snapshot();
        assert_eq!(snap1.total_requests, 1, "total_requests must not be double-counted on Local->Cloud failover");
        assert_eq!(snap1.local_requests, 0);
        assert_eq!(snap1.cloud_requests, 1);
        assert_eq!(snap1.failover_requests, 1);
        assert_eq!(snap1.local_tokens_est, 0);
        assert_eq!(snap1.cloud_tokens_est, 800);
        assert_eq!(snap1.total_tokens_est, 800);

        // 2. Cloud -> Local fallback: initial cloud 1000 tokens (spillover), reverts and dispatches local 400 tokens
        tracker.inc_cloud(1000, true);
        assert_eq!(tracker.total_requests.load(std::sync::atomic::Ordering::Relaxed), 2);
        tracker.revert_cloud_request_for_local_fallback(1000, true);
        tracker.record_local_request_stats(400);

        let snap2 = tracker.snapshot();
        assert_eq!(snap2.total_requests, 2, "total_requests must not be double-counted on Cloud->Local fallback");
        assert_eq!(snap2.local_requests, 1);
        assert_eq!(snap2.cloud_requests, 1);
        assert_eq!(snap2.spillover_requests, 0);
        assert_eq!(snap2.failover_requests, 2);
        assert_eq!(snap2.local_tokens_est, 400);
        assert_eq!(snap2.cloud_tokens_est, 800);
        assert_eq!(snap2.total_tokens_est, 1200);
    }
}


