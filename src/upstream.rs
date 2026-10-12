use crate::cache::ResponseCache;
use crate::config::Config;
use crate::router::{RouteDecision, TargetProvider};
use crate::stats::MetricsTracker;
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::stream::Stream;
use reqwest::Client;
use serde_json::Value;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tracing::{error, info, warn};

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
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(e))) => {
                warn!(error = %e, "Upstream stream closed or errored mid-transfer; cleanly terminating SSE stream with [DONE]");
                *this.ended = true;
                if *this.is_sse {
                    Poll::Ready(Some(Ok(bytes::Bytes::from("data: [DONE]\n\n"))))
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
                            Poll::Ready(Some(Ok(bytes::Bytes::from(": keepalive\n\n"))))
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
}

impl UpstreamClient {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>, cache: Arc<ResponseCache>) -> Self {
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
        }
    }

    pub async fn forward_chat(
        &self,
        mut body: Value,
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
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            error = %error_text,
                            "Local provider returned server error. Failing over to cloud..."
                        );
                        let tripped = self.metrics.record_local_failure(self.config.local.circuit_breaker_threshold);
                        if tripped {
                            warn!(
                                req_id = req_id,
                                threshold = self.config.local.circuit_breaker_threshold,
                                "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                            );
                        }
                        drop(slot_guard); // Release local slot immediately before failing over to Cloud
                        self.metrics.inc_failover();
                        self.metrics.inc_cloud(decision.est_tokens, false);
                        self.dispatch_cloud(body, decision, req_id, true, chars_saved, savings_pct, cache_key).await
                    }
                    Err(err) => {
                        warn!(
                            req_id = req_id,
                            error = %err,
                            "Local provider connection failed. Failing over to cloud..."
                        );
                        let tripped = self.metrics.record_local_failure(self.config.local.circuit_breaker_threshold);
                        if tripped {
                            warn!(
                                req_id = req_id,
                                threshold = self.config.local.circuit_breaker_threshold,
                                "Circuit breaker TRIPPED! Strata marked offline after consecutive failures. Future requests will route directly to Cloud."
                            );
                        }
                        drop(slot_guard); // Release local slot immediately before failing over to Cloud
                        self.metrics.inc_failover();
                        self.metrics.inc_cloud(decision.est_tokens, false);
                        self.dispatch_cloud(body, decision, req_id, true, chars_saved, savings_pct, cache_key).await
                    }
                }
            }
            TargetProvider::Cloud => {
                self.metrics.inc_cloud(decision.est_tokens, decision.is_spillover);
                self.dispatch_cloud(body, decision, req_id, false, chars_saved, savings_pct, cache_key).await
            }
        }
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
                self.handle_upstream_response(resp, is_stream, None, meta, cache_key).await
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

    async fn handle_upstream_response(
        &self,
        resp: reqwest::Response,
        _is_stream: bool,
        guard: Option<crate::router::LocalSlotGuard>,
        meta: ResponseMeta,
        cache_key: Option<String>,
    ) -> Response {
        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
        let mut headers = HeaderMap::new();

        for (k, v) in resp.headers().iter() {
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
            let body_bytes = resp.bytes().await.unwrap_or_default();
            if let Some(key) = cache_key {
                self.cache.set(key, body_bytes.clone(), status.as_u16(), vec![]).await;
            }
            return (status, headers, body_bytes).into_response();
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
}

pub fn normalize_wire_format(body: &mut Value, target_model: &str) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("model".to_string(), Value::String(target_model.to_string()));
    }
}
