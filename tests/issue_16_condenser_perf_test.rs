use axum::{routing::post, Json, Router as AxumRouter};
use ferry::condenser::{
    estimate_tokens_from_chars, suppress_base64_and_hex_blobs, ContextCondenser, LOOP_GUARD_SUFFIX,
};
use ferry::config::CondenserConfig;
use ferry::router::Router;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::TcpListener as StdTcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

struct CountingAlloc;

thread_local! {
    static COUNTING_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTING_ACTIVE.try_with(|active| {
            if active.get() {
                let _ = THREAD_ALLOCS.try_with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;

fn count_allocations<F: FnOnce() -> R, R>(f: F) -> (R, usize) {
    THREAD_ALLOCS.with(|c| c.set(0));
    COUNTING_ACTIVE.with(|a| a.set(true));
    let out = f();
    COUNTING_ACTIVE.with(|a| a.set(false));
    let allocs = THREAD_ALLOCS.with(|c| c.get());
    (out, allocs)
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ============================================================================
// AC 1: `fold_repeated_tool_failures` only folds genuine diagnostic failures
// ============================================================================

#[test]
fn test_ac1_successful_identical_tool_runs_never_folded_as_failures() {
    let config = CondenserConfig {
        enabled: true,
        fold_repeated_tool_failures: true,
        keep_recent_turns: 1,
        truncate_tool_outputs: false,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let clean_pytest = "============================= test session starts ==============================\ncollected 24 items\ntests/test_suite.py ........................                             [100%]\n============================== 24 passed in 0.31s ==============================";
    let clean_git_status =
        "On branch main\nYour branch is up to date with 'origin/main'.\n\nnothing to commit, working tree clean";
    let clean_cargo_test =
        "running 12 tests\ntest result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a developer assistant." },
            { "role": "user", "content": "Run test suite and check repo status." },
            { "role": "assistant", "content": "Running pytest #1" },
            { "role": "tool", "tool_call_id": "t1", "content": clean_pytest },
            { "role": "assistant", "content": "Running pytest #2" },
            { "role": "tool", "tool_call_id": "t2", "content": clean_pytest },
            { "role": "assistant", "content": "Checking git status #1" },
            { "type": "function_call_output", "call_id": "g1", "output": clean_git_status },
            { "role": "assistant", "content": "Checking git status #2" },
            { "type": "function_call_output", "call_id": "g2", "output": clean_git_status },
            { "role": "assistant", "content": "Running cargo test #1" },
            { "role": "tool", "tool_call_id": "c1", "content": [{ "type": "text", "text": clean_cargo_test }] },
            { "role": "assistant", "content": "Running cargo test #2" },
            { "role": "tool", "tool_call_id": "c2", "content": [{ "type": "text", "text": clean_cargo_test }] },
            { "role": "assistant", "content": "Recent active turn" }
        ]
    });

    let stats = condenser.condense(&mut body);
    assert_eq!(
        stats.loops_collapsed_count, 0,
        "Successful identical tool outputs must NEVER be collapsed as failures"
    );

    let serialized = body.to_string();
    assert!(
        !serialized.contains("Repeated identical tool failure suppressed"),
        "Successful tool runs were falsely rewritten as failures: {serialized}"
    );
}

#[test]
fn test_ac1_failing_identical_tool_runs_fold_across_all_formats_and_strip_loop_guard() {
    let config = CondenserConfig {
        enabled: true,
        fold_repeated_tool_failures: true,
        keep_recent_turns: 1,
        truncate_tool_outputs: false,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let err_content = "error: compilation failed due to unresolved import `crate::missing_mod`";
    let err_output = "File not found: /opt/app/config/missing_production_secrets.yaml";
    let err_multipart = "Traceback (most recent call last):\n  File \"app.py\", line 10, in <module>\nAssertionError: broken";
    let err_with_nudge = format!("{err_content}{LOOP_GUARD_SUFFIX}");

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "Agent" },
            { "role": "assistant", "content": "Try 1" },
            { "role": "tool", "content": err_content },
            { "role": "assistant", "content": "Try 2" },
            { "role": "tool", "content": err_with_nudge },
            { "role": "assistant", "content": "Out 1" },
            { "type": "function_call_output", "call_id": "o1", "output": err_output },
            { "role": "assistant", "content": "Out 2" },
            { "type": "function_call_output", "call_id": "o2", "output": err_output },
            { "role": "assistant", "content": "Multi 1" },
            { "role": "tool", "content": [{ "type": "text", "text": err_multipart }] },
            { "role": "assistant", "content": "Multi 2" },
            { "role": "tool", "content": [{ "type": "text", "text": err_multipart }] },
            { "role": "assistant", "content": "Recent active assistant turn" }
        ]
    });

    let stats = condenser.condense(&mut body);
    assert_eq!(stats.loops_collapsed_count, 3);

    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[2]["content"].as_str().unwrap(), err_content);
    assert!(msgs[4]["content"]
        .as_str()
        .unwrap()
        .contains("Repeated identical tool failure suppressed (attempt #2)"));

    assert_eq!(msgs[6]["output"].as_str().unwrap(), err_output);
    assert!(msgs[8]["output"]
        .as_str()
        .unwrap()
        .contains("Repeated identical tool failure suppressed (attempt #2)"));

    assert_eq!(msgs[10]["content"][0]["text"].as_str().unwrap(), err_multipart);
    assert!(msgs[12]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Repeated identical tool failure suppressed (attempt #2)"));
}

// ============================================================================
// AC 2: `rolling_compact` protocol invariants & multi-part/output extraction
// ============================================================================

#[test]
fn test_ac2_rolling_compact_preserves_developer_and_multi_header_prefixes() {
    let config = CondenserConfig {
        rolling_compaction: true,
        compact_threshold_tokens: 40,
        compact_keep_recent_turns: 1,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let dev_prompt = "CRITICAL DEVELOPER POLICY: Never expose internal tokens.";
    let sys_prompt = "System environment bootstrap context.";

    let mut body = json!({
        "messages": [
            { "role": "developer", "content": dev_prompt },
            { "role": "system", "content": sys_prompt },
            { "role": "user", "content": "Step 1 user query" },
            { "role": "assistant", "content": "Step 1 assistant response" },
            { "role": "tool", "name": "scan", "content": "Detailed scan output line for compaction threshold\n".repeat(25) },
            { "role": "user", "content": "Step 2 user query" },
            { "role": "assistant", "content": "Step 2 assistant response" },
            { "role": "user", "content": "Current user turn" },
            { "role": "assistant", "content": "Current assistant turn" }
        ]
    });

    let stats = condenser.rolling_compact(&mut body);
    assert!(stats.is_some(), "Expected rolling_compact to trigger");

    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "developer");
    assert_eq!(msgs[0]["content"], dev_prompt);
    assert_eq!(msgs[1]["role"], "system");
    assert_eq!(msgs[1]["content"], sys_prompt);
}

#[test]
fn test_ac2_rolling_compact_never_orphans_function_call_outputs_or_merges_user_turns() {
    let config = CondenserConfig {
        rolling_compaction: true,
        compact_threshold_tokens: 40,
        compact_keep_recent_turns: 1,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    // 1. Responses API `function_call` + parallel `function_call_output` right across split_idx
    let mut body_fc = json!({
        "messages": [
            { "role": "developer", "content": "Developer instructions" },
            { "role": "user", "content": "Initial investigation request" },
            { "type": "function_call", "call_id": "fc_old", "name": "bash", "arguments": "{\"cmd\":\"ls\"}" },
            { "type": "function_call_output", "call_id": "fc_old", "output": "historical directory output line\n".repeat(30) },
            { "type": "function_call", "call_id": "fc_keep_1", "name": "read_file", "arguments": "{\"path\":\"a.rs\"}" },
            { "type": "function_call", "call_id": "fc_keep_2", "name": "read_file", "arguments": "{\"path\":\"b.rs\"}" },
            { "type": "function_call_output", "call_id": "fc_keep_1", "output": "fn a() {}" },
            { "type": "function_call_output", "call_id": "fc_keep_2", "output": "fn b() {}" }
        ]
    });
    // total_msgs = 8, keep_recent = 2 -> raw split_idx = 6 (lands right on fc_keep_1's output!)
    let stats_fc = condenser.rolling_compact(&mut body_fc);
    assert!(stats_fc.is_some(), "Expected rolling_compact to compact old turns");

    let msgs_fc = body_fc["messages"].as_array().unwrap();
    let mut seen_call_ids = std::collections::HashSet::new();
    for m in msgs_fc {
        if m.get("type").and_then(|t| t.as_str()) == Some("function_call") {
            seen_call_ids.insert(m["call_id"].as_str().unwrap().to_string());
        }
        if m.get("type").and_then(|t| t.as_str()) == Some("function_call_output") {
            let cid = m["call_id"].as_str().unwrap();
            assert!(
                seen_call_ids.contains(cid),
                "Orphaned function_call_output with call_id={cid} in compacted messages: {msgs_fc:?}"
            );
        }
    }
    assert!(seen_call_ids.contains("fc_keep_1"));
    assert!(seen_call_ids.contains("fc_keep_2"));

    // 2. Adjacent `user -> user` prevention on both String and multi-part Array user messages
    let mut body_adj = json!({
        "messages": [
            { "role": "system", "content": "System prompt" },
            { "role": "user", "content": [{ "type": "text", "text": "Audit /api/v1/admin endpoint with sps_session=tok123" }] },
            { "type": "function_call", "call_id": "c1", "name": "probe", "arguments": "{\"url\":\"http://target.local:8080/api/v1/admin\"}" },
            { "type": "function_call_output", "call_id": "c1", "output": "status 500: CVE-2026-4242 SQL syntax error\npadding line\n".repeat(25) },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": "Now inspect the second screenshot and verify." },
                    { "type": "image_url", "image_url": { "url": "https://example.com/screen.png" } }
                ]
            },
            { "role": "assistant", "content": "Reviewing screenshot now." }
        ]
    });

    let stats_adj = condenser.rolling_compact(&mut body_adj);
    assert!(stats_adj.is_some(), "Expected rolling_compact on multi-part/output history");

    let msgs_adj = body_adj["messages"].as_array().unwrap();
    for pair in msgs_adj.windows(2) {
        let r0 = pair[0].get("role").and_then(|r| r.as_str()).unwrap_or("");
        let r1 = pair[1].get("role").and_then(|r| r.as_str()).unwrap_or("");
        assert!(
            !(r0 == "user" && r1 == "user"),
            "Found illegal adjacent user -> user messages: {msgs_adj:?}"
        );
    }

    // Verify multi-part user text, `output` signals, and security entities were all extracted!
    let merged_user = &msgs_adj[1];
    assert_eq!(merged_user["role"], "user");
    let parts = merged_user["content"].as_array().expect("Multi-part array structure must be preserved");
    assert_eq!(parts.len(), 2);
    let merged_text = parts[0]["text"].as_str().unwrap();
    assert!(merged_text.contains("[Historical Context Checkpoint:"));
    assert!(merged_text.contains("Audit /api/v1/admin endpoint"));
    assert!(merged_text.contains("CVE-2026-4242"));
    assert!(merged_text.contains("sps_session"));
    assert!(merged_text.contains("/api/v1/admin"));
    assert!(merged_text.ends_with("Now inspect the second screenshot and verify."));
    assert_eq!(parts[1]["type"], "image_url");
}

// ============================================================================
// AC 3: `X-Ferry-Condense: false` & `is_cloud_target` exemption in `src/main.rs`
// ============================================================================

#[tokio::test]
async fn test_ac3_x_ferry_condense_false_and_cloud_target_exemptions() {
    let local_captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cloud_captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    let local_cap = local_captured.clone();
    let local_app = AxumRouter::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/v1/chat/completions",
            post(move |Json(payload): Json<Value>| {
                let cap = local_cap.clone();
                async move {
                    cap.lock().await.push(payload);
                    Json(json!({
                        "id": "chatcmpl-local",
                        "object": "chat.completion",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "local ok" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            }),
        );

    let cloud_cap = cloud_captured.clone();
    let cloud_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move |Json(payload): Json<Value>| {
            let cap = cloud_cap.clone();
            async move {
                cap.lock().await.push(payload);
                Json(json!({
                    "id": "chatcmpl-cloud",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "cloud ok" },
                        "finish_reason": "stop"
                    }]
                }))
            }
        }),
    );

    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(local_listener, local_app).await;
    });

    let cloud_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(cloud_listener, cloud_app).await;
    });

    // Reserve a free port for the Ferry proxy binary
    let free_port = {
        let sock = StdTcpListener::bind("127.0.0.1:0").unwrap();
        sock.local_addr().unwrap().port()
    };
    let ferry_bind = format!("127.0.0.1:{free_port}");

    let child = Command::new(env!("CARGO_BIN_EXE_ferry"))
        .env("FERRY_CONFIG", "/nonexistent_ferry_test_config.toml")
        .env("FERRY_BIND", &ferry_bind)
        .env("FERRY_LOCAL_API_BASE", format!("http://{local_addr}/v1"))
        .env("FERRY_CLOUD_API_BASE", format!("http://{cloud_addr}/v1"))
        .env("FERRY_CLOUD_API_KEY", "test-cloud-key")
        .env("FERRY_CACHE_ENABLED", "false")
        .env("FERRY_CONDENSATION_ENABLED", "true")
        .env("FERRY_ROLLING_COMPACTION", "true")
        .env("FERRY_COMPACT_THRESHOLD_TOKENS", "80")
        .env("FERRY_COMPACT_KEEP_RECENT_TURNS", "1")
        .env("FERRY_LOCAL_MAX_CONTEXT_TOKENS", "8000")
        .env("FERRY_ADAPTIVE_SQUEEZING", "true")
        .env("FERRY_ADAPTIVE_SQUEEZE_RATIO", "5.0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ferry binary");
    let _guard = ChildGuard(child);

    let client = reqwest::Client::new();
    let health_url = format!("http://{ferry_bind}/health");
    let mut ready = false;
    for _ in 0..50 {
        if let Ok(r) = client.get(&health_url).send().await {
            if r.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready, "Ferry test server did not become ready in time");

    let large_tool_out = (1..=35)
        .map(|i| format!("Historical scan log line #{i}: normal port check completed"))
        .collect::<Vec<_>>()
        .join("\n");

    let payload = json!({
        "model": "ferry",
        "stream": false,
        "messages": [
            { "role": "system", "content": "System prompt" },
            { "role": "user", "content": "Turn 1 request" },
            { "role": "assistant", "content": "Turn 1 reply" },
            { "role": "tool", "name": "nmap", "content": large_tool_out },
            { "role": "user", "content": "Turn 2 request" },
            { "role": "assistant", "content": "Turn 2 reply" },
            { "role": "tool", "name": "curl", "content": large_tool_out },
            { "role": "user", "content": "Turn 3 active request" },
            { "role": "assistant", "content": "Turn 3 active reply" }
        ]
    });
    let orig_messages = payload["messages"].clone();

    let chat_url = format!("http://{ferry_bind}/v1/chat/completions");

    // 1. X-Ferry-Condense: false on local-only -> MUST bypass condense, rolling_compact, AND adaptive_squeeze!
    let resp_bypass = client
        .post(&chat_url)
        .header("x-ferry-route", "local-only")
        .header("x-ferry-condense", "false")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert!(resp_bypass.status().is_success());

    {
        let cap = local_captured.lock().await;
        assert_eq!(cap.len(), 1);
        assert_eq!(
            cap[0]["messages"], orig_messages,
            "X-Ferry-Condense: false must bypass rolling_compact and adaptive_squeeze completely"
        );
    }

    // 2. Cloud route (x-ferry-route: cloud-only) with condensation enabled -> MUST exempt is_cloud_target from rolling_compact!
    let resp_cloud = client
        .post(&chat_url)
        .header("x-ferry-route", "cloud-only")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert!(resp_cloud.status().is_success());

    {
        let cap = cloud_captured.lock().await;
        assert_eq!(cap.len(), 1);
        assert_eq!(
            cap[0]["messages"].as_array().unwrap().len(),
            orig_messages.as_array().unwrap().len(),
            "Cloud target must be exempted from rolling_compact turn pruning"
        );
        let cloud_str = cap[0].to_string();
        assert!(
            !cloud_str.contains("Historical Context Checkpoint"),
            "Cloud target must never have turns replaced with Historical Context Checkpoint"
        );
    }

    // 3. Positive control: local-only with condensation enabled -> DOES run rolling_compact!
    let resp_local = client
        .post(&chat_url)
        .header("x-ferry-route", "local-only")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert!(resp_local.status().is_success());

    {
        let cap = local_captured.lock().await;
        assert_eq!(cap.len(), 2);
        let local_str = cap[1].to_string();
        assert!(
            local_str.contains("Historical Context Checkpoint"),
            "Local target with condensation enabled should still apply rolling_compact"
        );
    }
}

#[tokio::test]
async fn test_ac3_x_ferry_condense_false_bypasses_adaptive_squeeze() {
    let local_captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cloud_captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    let local_cap = local_captured.clone();
    let local_app = AxumRouter::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/v1/chat/completions",
            post(move |Json(payload): Json<Value>| {
                let cap = local_cap.clone();
                async move {
                    cap.lock().await.push(payload);
                    Json(json!({
                        "id": "chatcmpl-local-sq",
                        "object": "chat.completion",
                        "choices": [{
                            "index": 0,
                            "message": { "role": "assistant", "content": "local squeezed ok" },
                            "finish_reason": "stop"
                        }]
                    }))
                }
            }),
        );

    let cloud_cap = cloud_captured.clone();
    let cloud_app = AxumRouter::new().route(
        "/v1/chat/completions",
        post(move |Json(payload): Json<Value>| {
            let cap = cloud_cap.clone();
            async move {
                cap.lock().await.push(payload);
                Json(json!({
                    "id": "chatcmpl-cloud-sq",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": "cloud spillover ok" },
                        "finish_reason": "stop"
                    }]
                }))
            }
        }),
    );

    let local_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = local_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(local_listener, local_app).await;
    });

    let cloud_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cloud_addr = cloud_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(cloud_listener, cloud_app).await;
    });

    let free_port = {
        let sock = StdTcpListener::bind("127.0.0.1:0").unwrap();
        sock.local_addr().unwrap().port()
    };
    let ferry_bind = format!("127.0.0.1:{free_port}");

    // Disable rolling_compaction and truncate_tool_outputs so ONLY adaptive_squeeze can shrink historical tool outputs!
    let child = Command::new(env!("CARGO_BIN_EXE_ferry"))
        .env("FERRY_CONFIG", "/nonexistent_ferry_test_config.toml")
        .env("FERRY_BIND", &ferry_bind)
        .env("FERRY_LOCAL_API_BASE", format!("http://{local_addr}/v1"))
        .env("FERRY_CLOUD_API_BASE", format!("http://{cloud_addr}/v1"))
        .env("FERRY_CLOUD_API_KEY", "test-cloud-key")
        .env("FERRY_CACHE_ENABLED", "false")
        .env("FERRY_CONDENSATION_ENABLED", "true")
        .env("FERRY_ROLLING_COMPACTION", "false")
        .env("FERRY_TRUNCATE_TOOL_OUTPUTS", "false")
        .env("FERRY_DEDUPLICATE_LINES", "false")
        .env("FERRY_STRIP_TERMINAL_NOISE", "false")
        .env("FERRY_KEEP_RECENT_TURNS", "1")
        .env("FERRY_LOCAL_MAX_CONTEXT_TOKENS", "500")
        .env("FERRY_ADAPTIVE_SQUEEZING", "true")
        .env("FERRY_ADAPTIVE_SQUEEZE_RATIO", "5.0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ferry binary");
    let _guard = ChildGuard(child);

    let client = reqwest::Client::new();
    let health_url = format!("http://{ferry_bind}/health");
    let mut ready = false;
    for _ in 0..50 {
        if let Ok(r) = client.get(&health_url).send().await {
            if r.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready, "Ferry test server did not become ready in time");

    // ~2400 chars (~686 tokens): > 500 max_context, <= 2500 squeeze window!
    let hist_tool = (1..=45)
        .map(|i| format!("Historical log entry #{i}: service check status nominal"))
        .collect::<Vec<_>>()
        .join("\n");

    let payload = json!({
        "model": "ferry",
        "stream": false,
        "messages": [
            { "role": "user", "content": "Run check" },
            { "role": "assistant", "content": "Checking historical" },
            { "role": "tool", "content": hist_tool },
            { "role": "assistant", "content": "Checking recent" },
            { "role": "user", "content": "ok" }
        ]
    });
    let orig_messages = payload["messages"].clone();
    let chat_url = format!("http://{ferry_bind}/v1/chat/completions");

    // 1. With X-Ferry-Condense: FALSE -> adaptive_squeeze MUST be bypassed (spills unmutated to cloud)!
    let resp_no_squeeze = client
        .post(&chat_url)
        .header("x-ferry-route", "local-preferred")
        .header("x-ferry-condense", "FALSE")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert!(resp_no_squeeze.status().is_success());
    {
        let ccap = cloud_captured.lock().await;
        assert_eq!(
            ccap.len(),
            1,
            "Without adaptive_squeeze, >500 token prompt should spill to cloud"
        );
        assert_eq!(
            ccap[0]["messages"], orig_messages,
            "X-Ferry-Condense: FALSE must never mutate messages via adaptive_squeeze"
        );
    }

    // 2. Positive control: without X-Ferry-Condense: false -> adaptive_squeeze shrinks historical tool output to <=500 tokens & fits on local!
    let resp_squeezed = client
        .post(&chat_url)
        .header("x-ferry-route", "local-preferred")
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert!(resp_squeezed.status().is_success());
    {
        let lcap = local_captured.lock().await;
        assert_eq!(
            lcap.len(),
            1,
            "With adaptive_squeeze enabled, prompt should fit into local 500-token context"
        );
        assert!(lcap[0]["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("Ferry Proxy: truncated"));
    }
}

// ============================================================================
// AC 4: Zero-allocation `suppress_base64_and_hex_blobs` & `estimated_json_len`
// ============================================================================

#[test]
fn test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations() {
    let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiYWRtaW4iOnRydWUsImlhdCI6MTUxNjIzOTAyMiwiZXhwIjoxNTE2MjQyNjIyLCJyb2xlIjoic3VwZXJfYWRtaW4ifQ.signature_part_long";
    let api_key = "sk-proj-a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0";
    let long_path = "/opt/enterprise/services/authentication/middleware/validators/token_signature_verifier_module.rs";
    let hex_blob = "deadbeef01234567".repeat(12); // 192 chars
    let data_uri_b64 = "Q".repeat(140);

    let input = format!(
        "Auth: Bearer {jwt}\nKey: {api_key}\nPath: {long_path}\nInline: {{\"img\":\"data:image/png;base64,{data_uri_b64}\",\"blob\":\"{hex_blob}\",\"emoji\":\"🚀✅\"}}"
    );

    let (cleaned, count) = suppress_base64_and_hex_blobs(&input);
    assert_eq!(count, 2);
    assert!(cleaned.contains(jwt), "JWT must never be suppressed");
    assert!(cleaned.contains(api_key), "API key must never be suppressed");
    assert!(cleaned.contains(long_path), "File paths must never be suppressed");
    assert!(cleaned.contains("🚀✅"), "Unicode characters must be preserved verbatim");
    assert!(cleaned.contains("[... Ferry Proxy: base64 payload (140 chars) suppressed ...]"));
    assert!(cleaned.contains("[... Ferry Proxy: base64/hex blob (192 chars) suppressed ...]"));
    assert!(!cleaned.ends_with('\n'));

    // Large minified JSON line (10,000+ chars, hundreds of punctuation tokens):
    // Previously caused > 10,000 per-character Vec<String> heap allocations!
    let large_minified_json = format!(
        "[{}]",
        (0..200)
            .map(|i| format!("{{\"id\":{i},\"status\":\"healthy\",\"latency_ms\":12}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(large_minified_json.len() > 8_000);

    let ((out_large, sup_large), allocs) =
        count_allocations(|| suppress_base64_and_hex_blobs(&large_minified_json));
    assert_eq!(sup_large, 0);
    assert_eq!(out_large, large_minified_json);
    assert!(
        allocs <= 8,
        "Expected near-zero buffer allocations on 8KB minified JSON line, but got {allocs} heap allocations!"
    );
}

#[test]
fn test_ac4_estimated_json_len_zero_allocation_and_exact_parity() {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["command", "timeout_ms", "env"],
        "properties": {
            "command": {
                "type": "string",
                "description": "Shell command with \"quotes\", backslashes \\, newlines\nand control \u{0001} chars + UTF-8 π"
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 0,
                "maximum": 600000,
                "default": 30000
            },
            "dry_run": {
                "type": "boolean",
                "default": false
            },
            "fallback": null,
            "ratios": [0.25, 1.5, 100],
            "env": {
                "type": "object",
                "properties": {}
            },
            "empty_arr": []
        }
    });

    let desc = "Execute a shell command in workspace";
    let msg_text = "Run diagnostics";
    let tools_arr: Vec<Value> = (0..25)
        .map(|i| {
            json!({
                "type": "function",
                "function": {
                    "name": format!("tool_{i}"),
                    "description": desc,
                    "parameters": schema
                }
            })
        })
        .collect();

    let body = json!({
        "messages": [
            { "role": "user", "content": msg_text }
        ],
        "tools": tools_arr
    });

    let exact_expected_chars = msg_text.len() + 25 * (desc.len() + schema.to_string().len());
    let expected_tokens = estimate_tokens_from_chars(exact_expected_chars);

    let (actual_tokens, allocs) = count_allocations(|| Router::estimate_tokens(&body));
    assert_eq!(
        actual_tokens, expected_tokens,
        "estimated_json_len must match serde_json serialized byte length parity"
    );
    assert_eq!(
        allocs, 0,
        "Router::estimate_tokens must perform ZERO heap allocations, got {allocs}"
    );
}
