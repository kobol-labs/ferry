use crate::config::Config;
use crate::router::{RouteDecision, TargetProvider};
use crate::stats::MetricsTracker;
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::stream::Stream;
use futures_util::TryStreamExt;
use reqwest::Client;
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tracing::{error, info, warn};

pub struct GuardedStream<S> {
    inner: S,
    metrics: Option<Arc<MetricsTracker>>,
}

impl<S> Drop for GuardedStream<S> {
    fn drop(&mut self) {
        if let Some(m) = self.metrics.take() {
            m.dec_local();
        }
    }
}

impl<S: Stream + Unpin> Stream for GuardedStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

pub struct UpstreamClient {
    client: Client,
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
}

impl UpstreamClient {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>) -> Self {
        let client = Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            config,
            metrics,
        }
    }

    pub async fn forward_chat(
        &self,
        body: Value,
        decision: RouteDecision,
        req_id: u64,
    ) -> Response {
        let start = Instant::now();
        let is_stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);

        match decision.target {
            TargetProvider::Local => {
                self.metrics.inc_local(decision.est_tokens);
                info!(
                    req_id = req_id,
                    target = "local",
                    model = %self.config.local.model,
                    tokens = decision.est_tokens,
                    reason = %decision.reason,
                    "Routing to Local Provider"
                );

                let local_url = format!("{}/chat/completions", self.config.local.api_base.trim_end_matches('/'));
                let mut target_body = body.clone();
                if let Some(obj) = target_body.as_object_mut() {
                    obj.insert("model".to_string(), Value::String(self.config.local.model.clone()));
                }

                let req = self
                    .client
                    .post(&local_url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", self.config.local.api_key))
                    .header(header::CONTENT_TYPE, "application/json")
                    .json(&target_body)
                    .timeout(std::time::Duration::from_secs(self.config.local.timeout_secs));

                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let duration = start.elapsed();
                        info!(
                            req_id = req_id,
                            target = "local",
                            status = resp.status().as_u16(),
                            duration_ms = duration.as_millis(),
                            "Local turn completed successfully"
                        );
                        self.handle_upstream_response(resp, is_stream, true)
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let error_text = resp.text().await.unwrap_or_default();
                        warn!(
                            req_id = req_id,
                            status = status.as_u16(),
                            error = %error_text,
                            "Local provider returned error. Failing over to cloud..."
                        );
                        self.metrics.dec_local();
                        self.metrics.inc_failover();
                        self.dispatch_cloud(body, decision, req_id, true).await
                    }
                    Err(err) => {
                        warn!(
                            req_id = req_id,
                            error = %err,
                            "Local provider connection failed. Failing over to cloud..."
                        );
                        self.metrics.dec_local();
                        self.metrics.inc_failover();
                        self.dispatch_cloud(body, decision, req_id, true).await
                    }
                }
            }
            TargetProvider::Cloud => {
                self.metrics.inc_cloud(decision.est_tokens, decision.is_spillover);
                self.dispatch_cloud(body, decision, req_id, false).await
            }
        }
    }

    async fn dispatch_cloud(
        &self,
        body: Value,
        decision: RouteDecision,
        req_id: u64,
        is_failover: bool,
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

        let cloud_url = format!("{}/chat/completions", self.config.cloud.api_base.trim_end_matches('/'));
        let mut target_body = body;
        if let Some(obj) = target_body.as_object_mut() {
            obj.insert("model".to_string(), Value::String(self.config.cloud.model.clone()));
        }

        let req = self
            .client
            .post(&cloud_url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.config.cloud.api_key))
            .header(header::CONTENT_TYPE, "application/json")
            .json(&target_body)
            .timeout(std::time::Duration::from_secs(self.config.cloud.timeout_secs));

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let duration = start.elapsed();
                info!(
                    req_id = req_id,
                    target = "cloud",
                    status = status.as_u16(),
                    duration_ms = duration.as_millis(),
                    "Cloud turn completed"
                );
                self.handle_upstream_response(resp, is_stream, false)
            }
            Err(err) => {
                error!(req_id = req_id, error = %err, "Cloud request failed");
                (
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!({ "error": format!("Cloud upstream failed: {}", err) }).to_string(),
                )
                    .into_response()
            }
        }
    }

    fn handle_upstream_response(
        &self,
        resp: reqwest::Response,
        _is_stream: bool,
        is_local: bool,
    ) -> Response {
        let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
        let mut headers = HeaderMap::new();

        for (k, v) in resp.headers().iter() {
            let key = k.as_str().to_lowercase();
            if key == "content-type" || key == "cache-control" || key == "access-control-allow-origin" {
                if let (Ok(hname), Ok(hval)) = (
                    HeaderName::from_bytes(k.as_ref()),
                    HeaderValue::from_bytes(v.as_ref()),
                ) {
                    headers.insert(hname, hval);
                }
            }
        }

        let metrics = if is_local {
            Some(self.metrics.clone())
        } else {
            None
        };

        let raw_stream = Box::pin(
            resp.bytes_stream()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
        );

        let guarded = GuardedStream {
            inner: raw_stream,
            metrics,
        };

        let body = Body::from_stream(guarded);
        (status, headers, body).into_response()
    }
}
