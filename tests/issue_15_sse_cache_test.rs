use axum::{
    body::to_bytes,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router as AxumRouter,
};
use bytes::Bytes;
use ferry::{
    affinity::AffinityType,
    cache::ResponseCache,
    config::Config,
    router::{LocalSlotGuard, RouteDecision, TargetProvider},
    stats::MetricsTracker,
    upstream::{
        format_sse_error_frame, transform_strata_chunk, GuardedStream, UpstreamClient,
        KEEPALIVE_SSE_CHUNK,
    },
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn make_local_decision(local_only: bool, guard: Option<LocalSlotGuard>) -> RouteDecision {
    RouteDecision {
        target: TargetProvider::Local,
        est_tokens: 128,
        is_spillover: false,
        reason: "test local route".to_string(),
        local_only,
        slot_guard: guard,
        slot_id: Some(0),
        is_affinity_hit: false,
        affinity_type: AffinityType::NewSlot,
        session_id: "sess-test".to_string(),
    }
}

#[tokio::test]
async fn test_guarded_stream_tcp_fragmented_colon_chunk_not_replaced_with_keepalive() {
    let chunks = vec![
        Ok::<Bytes, std::io::Error>(Bytes::from("data: {\"a\"")),
        Ok(Bytes::from(": \"b\"}\n\n")),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ];
    let guarded = GuardedStream::new(
        futures_util::stream::iter(chunks),
        None,
        true,
        Duration::from_secs(10),
    );

    let collected: Vec<Bytes> = guarded.map(|r| r.unwrap()).collect().await;
    let combined = collected.concat();
    let combined_str = std::str::from_utf8(&combined).unwrap();

    assert_eq!(
        combined_str,
        "data: {\"a\": \"b\"}\n\ndata: [DONE]\n\n",
        "TCP-fragmented JSON chunk starting with ':' must NOT be replaced with KEEPALIVE_SSE_CHUNK"
    );
    assert!(!combined_str.contains("chatcmpl-keepalive"));
}

#[tokio::test]
async fn test_guarded_stream_keepalive_never_spliced_mid_line_or_after_single_newline() {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    let inner = ferry::upstream::ReceiverStream::new(rx);
    let guarded = GuardedStream::new(inner, None, true, Duration::from_millis(15));

    let producer = tokio::spawn(async move {
        // Send incomplete JSON line
        tx.send(Ok(Bytes::from("data: {\"a\""))).await.unwrap();
        // Pause longer than keepalive_duration while mid-line!
        tokio::time::sleep(Duration::from_millis(45)).await;
        // Finish line with a SINGLE newline (still NOT an SSE \n\n event boundary!)
        tx.send(Ok(Bytes::from(": \"b\"}\n"))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(35)).await;
        // Complete the SSE event boundary (\n\n)
        tx.send(Ok(Bytes::from("\n"))).await.unwrap();
        drop(tx);
    });

    let collected: Vec<Bytes> = guarded.map(|r| r.unwrap()).collect().await;
    producer.await.unwrap();

    let combined = String::from_utf8(collected.concat()).unwrap();
    let events: Vec<&str> = combined
        .split("\n\n")
        .filter(|e| !e.trim().is_empty())
        .collect();

    assert!(
        !events.is_empty(),
        "Expected at least the reassembled SSE data event"
    );
    for event in &events {
        let json_part = event
            .strip_prefix("data: ")
            .expect("Every SSE event must start with 'data: ' without mid-line splicing");
        let parsed: Value = serde_json::from_str(json_part)
            .expect("Every SSE event payload must be valid single-line JSON");
        assert!(parsed.is_object());
    }
    let first_json: Value = serde_json::from_str(events[0].strip_prefix("data: ").unwrap()).unwrap();
    assert_eq!(first_json, json!({"a": "b"}));
}

#[tokio::test]
async fn test_guarded_stream_mid_line_error_prepends_event_boundary_before_done() {
    let chunks = vec![
        Ok(Bytes::from("data: {\"partial\":")),
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset mid-line",
        )),
    ];
    let guarded = GuardedStream::new(
        futures_util::stream::iter(chunks),
        None,
        true,
        Duration::from_secs(10),
    );

    let collected: Vec<Bytes> = guarded.map(|r| r.unwrap()).collect().await;
    let combined = String::from_utf8(collected.concat()).unwrap();
    assert!(
        combined.ends_with("\n\ndata: [DONE]\n\n"),
        "Mid-line upstream error must prepend \\n\\n before data: [DONE]\\n\\n, got: {:?}",
        combined
    );
}

#[tokio::test]
async fn test_local_streaming_tcp_split_chunks_over_real_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut req_buf = vec![0u8; 4096];
        let _ = socket.read(&mut req_buf).await;

        let headers = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "Transfer-Encoding: chunked\r\n",
            "\r\n"
        );
        socket.write_all(headers.as_bytes()).await.unwrap();

        // Chunk 1: `data: {"a"` (10 bytes -> 0xA)
        socket.write_all(b"a\r\ndata: {\"a\"\r\n").await.unwrap();
        socket.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Chunk 2: `: "b"}\n\n` (8 bytes -> 0x8) — starts with `:`!
        socket.write_all(b"8\r\n: \"b\"}\n\n\r\n").await.unwrap();
        socket.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Chunk 3: `data: [DONE]\n\n` (14 bytes -> 0xE) + terminating chunk
        socket
            .write_all(b"e\r\ndata: [DONE]\n\n\r\n0\r\n\r\n")
            .await
            .unwrap();
        socket.flush().await.unwrap();
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}", addr);
    cfg.cloud.api_base = format!("http://{}", addr);
    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let cache = Arc::new(ResponseCache::new(true, 32, 60));
    let client = UpstreamClient::new(config, metrics, cache);

    let body = json!({
        "model": "ferry",
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}]
    });
    let resp = client
        .forward_chat(body, make_local_decision(false, None), 1, 0, 0.0, None)
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let raw = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = std::str::from_utf8(&raw).unwrap();
    assert_eq!(text, "data: {\"a\": \"b\"}\n\ndata: [DONE]\n\n");
    assert!(!text.contains(std::str::from_utf8(KEEPALIVE_SSE_CHUNK).unwrap()));
}

#[tokio::test]
async fn test_streaming_local_500_fails_over_to_cloud_when_not_local_only() {
    let local_hits = Arc::new(AtomicUsize::new(0));
    let cloud_hits = Arc::new(AtomicUsize::new(0));

    let lh = local_hits.clone();
    let local_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move || {
            let lh = lh.clone();
            async move {
                lh.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "CUDA OOM\n\"fatal\""})),
                )
            }
        }),
    );
    let local_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(local_listener, local_app).await.unwrap();
    });

    let ch = cloud_hits.clone();
    let cloud_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move |Json(payload): Json<Value>| {
            let ch = ch.clone();
            async move {
                ch.fetch_add(1, Ordering::SeqCst);
                assert!(
                    payload.get("id_slot").is_none(),
                    "id_slot must be stripped before Cloud failover"
                );
                (
                    StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    "data: {\"choices\":[{\"delta\":{\"content\":\"cloud-ok\"}}]}\n\ndata: [DONE]\n\n",
                )
            }
        }),
    );
    let cloud_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(cloud_listener, cloud_app).await.unwrap();
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}/v1", local_addr);
    cfg.cloud.api_base = format!("http://{}/v1", cloud_addr);
    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let cache = Arc::new(ResponseCache::new(true, 32, 60));
    let client = UpstreamClient::new(config, metrics.clone(), cache);

    assert!(metrics.try_acquire_local_capacity(128, 4, 100_000));
    let guard = LocalSlotGuard::new(metrics.clone(), None, Some(0), 128);

    let body = json!({
        "model": "ferry",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let resp = client
        .forward_chat(body, make_local_decision(false, Some(guard)), 42, 0, 0.0, None)
        .await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-ferry-provider").unwrap(),
        "cloud",
        "Streaming local 500 must fail over to Cloud when !local_only"
    );
    assert_eq!(resp.headers().get("x-ferry-failover").unwrap(), "true");
    let raw = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = std::str::from_utf8(&raw).unwrap();
    assert!(text.contains("cloud-ok"));
    assert_eq!(local_hits.load(Ordering::SeqCst), 1);
    assert_eq!(cloud_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        metrics.active_local_slots.load(Ordering::SeqCst),
        0,
        "Local slot guard must be released on failover"
    );
}

#[tokio::test]
async fn test_streaming_local_500_and_connect_error_respect_local_only_and_400_passthrough() {
    let cloud_hits = Arc::new(AtomicUsize::new(0));
    let ch = cloud_hits.clone();
    let cloud_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move || {
            let ch = ch.clone();
            async move {
                ch.fetch_add(1, Ordering::SeqCst);
                (StatusCode::OK, "should not be called")
            }
        }),
    );
    let cloud_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(cloud_listener, cloud_app).await.unwrap();
    });

    // Local server returning status determined by `mode`
    #[derive(Clone)]
    struct MockMode(Arc<AtomicUsize>);
    let mode = MockMode(Arc::new(AtomicUsize::new(500)));
    let mode_clone = mode.clone();
    let local_app = AxumRouter::new()
        .route(
            "/v1/chat/completions",
            post(|State(m): State<MockMode>| async move {
                let code = m.0.load(Ordering::SeqCst);
                if code == 400 {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": {"message": "bad schema"}})),
                    )
                        .into_response()
                } else {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "local crash"})),
                    )
                        .into_response()
                }
            }),
        )
        .with_state(mode_clone);

    let local_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(local_listener, local_app).await.unwrap();
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}/v1", local_addr);
    cfg.cloud.api_base = format!("http://{}/v1", cloud_addr);
    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let cache = Arc::new(ResponseCache::new(true, 32, 60));
    let client = UpstreamClient::new(config, metrics.clone(), cache.clone());

    let stream_body = json!({
        "model": "ferry",
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}]
    });

    // 1. local_only = true + Local 500 => must return 502 or 503 and NOT call Cloud
    let resp_500 = client
        .forward_chat(
            stream_body.clone(),
            make_local_decision(true, None),
            1,
            0,
            0.0,
            None,
        )
        .await;
    assert!(
        matches!(
            resp_500.status(),
            StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE
        ),
        "Expected 502 or 503 for local_only 500 failure, got {}",
        resp_500.status()
    );
    assert_eq!(cloud_hits.load(Ordering::SeqCst), 0);

    // 2. local_only = false + Local 400 => must forward true 400 Bad Request and NOT call Cloud
    mode.0.store(400, Ordering::SeqCst);
    let resp_400 = client
        .forward_chat(
            stream_body.clone(),
            make_local_decision(false, None),
            2,
            0,
            0.0,
            None,
        )
        .await;
    assert_eq!(resp_400.status(), StatusCode::BAD_REQUEST);
    let body_400 = to_bytes(resp_400.into_body(), 64 * 1024).await.unwrap();
    assert!(std::str::from_utf8(&body_400).unwrap().contains("bad schema"));
    assert_eq!(cloud_hits.load(Ordering::SeqCst), 0);

    // 3. local_only = true + closed port (connect error) => must return 502 or 503
    let dead_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead_listener.local_addr().unwrap();
    drop(dead_listener);

    let mut dead_cfg = Config::default();
    dead_cfg.local.api_base = format!("http://{}/v1", dead_addr);
    dead_cfg.local.connect_timeout_secs = 1;
    dead_cfg.cloud.api_base = format!("http://{}/v1", cloud_addr);
    let dead_client = UpstreamClient::new(
        Arc::new(dead_cfg),
        Arc::new(MetricsTracker::new()),
        cache,
    );

    let resp_conn = dead_client
        .forward_chat(stream_body, make_local_decision(true, None), 3, 0, 0.0, None)
        .await;
    assert!(
        matches!(
            resp_conn.status(),
            StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE
        ),
        "Expected 502 or 503 for local_only connect failure, got {}",
        resp_conn.status()
    );
    assert_eq!(cloud_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_response_cache_temperature_errors_keys_and_oldest_eviction() {
    // 1. Omitted temperature must NOT be cached (OpenAI defaults omitted temperature to 1.0)
    let omitted_temp = json!({
        "model": "ferry",
        "messages": [{"role": "user", "content": "hello"}],
        "stream": false
    });
    assert!(
        ResponseCache::compute_cache_key(&omitted_temp).is_none(),
        "Requests omitting temperature must not be cached"
    );

    let explicit_zero = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "hello"}],
        "stream": false
    });
    let base_key = ResponseCache::compute_cache_key(&explicit_zero)
        .expect("Explicit temperature 0.0 must be cacheable");

    // 2. Parameter sensitivity (no collisions across distinct generation constraints)
    let with_json_format = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "hello"}],
        "response_format": {"type": "json_object"}
    });
    let with_tool_choice = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "hello"}],
        "tool_choice": "none"
    });
    let with_max_tokens = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "hello"}],
        "max_tokens": 64
    });
    assert_ne!(
        base_key,
        ResponseCache::compute_cache_key(&with_json_format).unwrap()
    );
    assert_ne!(
        base_key,
        ResponseCache::compute_cache_key(&with_tool_choice).unwrap()
    );
    assert_ne!(
        base_key,
        ResponseCache::compute_cache_key(&with_max_tokens).unwrap()
    );

    // Distinguish i64 -1 vs u64::MAX
    let msg_neg_one = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "x"}],
        "seed": -1i64
    });
    let msg_u64_max = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "x"}],
        "seed": u64::MAX
    });
    assert_ne!(
        ResponseCache::compute_cache_key(&msg_neg_one).unwrap(),
        ResponseCache::compute_cache_key(&msg_u64_max).unwrap()
    );

    // 3. Reject HTTP 4xx/5xx responses in ResponseCache
    let cache = ResponseCache::new(true, 3, 60);
    cache
        .set("err429".to_string(), Bytes::from_static(b"rate limit"), 429, vec![])
        .await;
    cache
        .set("err500".to_string(), Bytes::from_static(b"boom"), 500, vec![])
        .await;
    assert_eq!(cache.len().await, 0, "4xx/5xx responses must not be cached");

    // 4. Evict oldest created_at entry when full (not arbitrary HashMap bucket)
    for round in 0..10 {
        let c = ResponseCache::new(true, 3, 60);
        let k1 = format!("oldest-{}-alpha", round);
        let k2 = format!("middle-{}-bravo", round);
        let k3 = format!("newest-{}-charlie", round);
        let k4 = format!("incoming-{}-delta", round);

        c.set(k1.clone(), Bytes::from_static(b"1"), 200, vec![]).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        c.set(k2.clone(), Bytes::from_static(b"2"), 200, vec![]).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        c.set(k3.clone(), Bytes::from_static(b"3"), 200, vec![]).await;
        tokio::time::sleep(Duration::from_millis(2)).await;
        c.set(k4.clone(), Bytes::from_static(b"4"), 200, vec![]).await;

        assert_eq!(c.len().await, 3);
        assert!(
            c.get(&k1).await.is_none(),
            "Oldest entry {} must be evicted on round {}",
            k1,
            round
        );
        assert!(c.get(&k2).await.is_some());
        assert!(c.get(&k3).await.is_some());
        assert!(c.get(&k4).await.is_some());
    }
}

#[test]
fn test_sse_error_frame_escapes_multiline_quotes_and_backslashes() {
    let raw_err = "{\n  \"error\": \"CUDA error at C:\\\\llm\\\\runner.cu:42\\n  detail: \\\"out of memory\\\"\"\r\n}";
    let frame = format_sse_error_frame(raw_err);
    let frame_str = std::str::from_utf8(&frame).unwrap();

    // Must be two SSE events: single-line `data: <json>` and `data: [DONE]`
    let parts: Vec<&str> = frame_str.split("\n\n").filter(|s| !s.is_empty()).collect();
    assert_eq!(parts.len(), 2, "Expected 2 SSE events, got: {:?}", parts);
    assert!(
        !parts[0].contains('\n') && !parts[0].contains('\r'),
        "Error data event must be strictly single-line SSE"
    );
    let json_str = parts[0].strip_prefix("data: ").unwrap();
    let parsed: Value = serde_json::from_str(json_str).expect("Must parse as valid JSON");
    assert_eq!(parsed["error"]["type"], "upstream_error");
    assert_eq!(parsed["error"]["message"], raw_err);
    assert_eq!(parts[1], "data: [DONE]");

    // Stateless transform_strata_chunk must also preserve `: "b"}\n\n` while rewriting real comments
    assert_eq!(
        transform_strata_chunk(Bytes::from(": \"b\"}\n\n")),
        Bytes::from(": \"b\"}\n\n")
    );
    assert_eq!(
        transform_strata_chunk(Bytes::from(": ping keepalive\n\n")),
        Bytes::from_static(KEEPALIVE_SSE_CHUNK)
    );
}

#[tokio::test]
async fn test_response_cache_route_constraints_all_fields_and_upstream_error_truncation_bypass() {
    let base = json!({
        "model": "ferry",
        "temperature": 0.0,
        "messages": [{"role": "user", "content": "deterministic prompt"}]
    });

    let k_default = ResponseCache::compute_cache_key_with_route(&base, None).unwrap();
    let k_local = ResponseCache::compute_cache_key_with_route(&base, Some("local-only")).unwrap();
    let k_local_alias = ResponseCache::compute_cache_key_with_route(&base, Some(" local ")).unwrap();
    let k_cloud = ResponseCache::compute_cache_key_with_route(&base, Some("cloud-only")).unwrap();

    assert_eq!(k_local, k_local_alias, "Canonical local-only aliases share route key");
    assert_ne!(k_default, k_local);
    assert_ne!(k_local, k_cloud);
    assert_ne!(k_default, k_cloud);

    // Verify all remaining generation fields alter the cache key
    for extra in [
        json!({"max_completion_tokens": 128}),
        json!({"stop": ["\n\n"]}),
        json!({"seed": 42}),
        json!({"reasoning_effort": "high"}),
        json!({"functions": [{"name": "fn1"}]}),
        json!({"function_call": "none"}),
    ] {
        let mut modified = base.clone();
        for (k, v) in extra.as_object().unwrap() {
            modified[k] = v.clone();
        }
        let k_mod = ResponseCache::compute_cache_key(&modified).unwrap();
        assert_ne!(k_default, k_mod, "Field {:?} must affect cache key", extra);
    }

    // Verify UpstreamClient::handle_upstream_response never caches 4xx/5xx or truncated bodies
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        // Request 1: 400 Bad Request (with Connection: close so reqwest opens a fresh socket next)
        let (mut s1, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = s1.read(&mut buf).await;
        s1.write_all(
            b"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 19\r\nConnection: close\r\n\r\n{\"error\":\"invalid\"}",
        )
        .await
        .unwrap();
        s1.flush().await.unwrap();
        drop(s1);

        // Request 2: 200 OK headers flushed first, then truncated body (Content-Length: 200, closes after 5 bytes!)
        let (mut s2, _) = listener.accept().await.unwrap();
        let _ = s2.read(&mut buf).await;
        s2.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{\"par",
        )
        .await
        .unwrap();
        s2.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(25)).await;
        drop(s2);
    });

    let mut cfg = Config::default();
    cfg.local.api_base = format!("http://{}", addr);
    cfg.cloud.api_base = format!("http://{}", addr);
    let cache = Arc::new(ResponseCache::new(true, 32, 60));
    let client = UpstreamClient::new(
        Arc::new(cfg),
        Arc::new(MetricsTracker::new()),
        cache.clone(),
    );

    // 1. 400 response must not populate cache
    let r1 = client
        .forward_chat(
            base.clone(),
            make_local_decision(true, None),
            10,
            0,
            0.0,
            Some(k_default.clone()),
        )
        .await;
    assert_eq!(r1.status(), StatusCode::BAD_REQUEST);
    assert_eq!(cache.len().await, 0);

    // 2. Truncated 200 body must return 502 and NOT populate cache with partial/empty bytes
    let r2 = client
        .forward_chat(
            base,
            make_local_decision(true, None),
            11,
            0,
            0.0,
            Some(k_default),
        )
        .await;
    assert_eq!(r2.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(cache.len().await, 0);
}

