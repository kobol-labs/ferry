use ferry::condenser::ContextCondenser;
use ferry::config::CondenserConfig;
use ferry::stats::MetricsTracker;
use ferry::router::{Router, TargetProvider};
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_system_prompt_never_mutated() {
    let config = CondenserConfig {
        enabled: true,
        drop_historical_reasoning: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 100,
        recent_max_tool_chars: 500,
        head_lines: 2,
        tail_lines: 2,
        strip_terminal_noise: true,
        deduplicate_lines: true,
        minify_json_outputs: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let system_content = "You are a specialized security agent.\n\x1b[31mCRITICAL:\x1b[0m Do not execute destructive actions.\n{\n  \"rule\": true\n}\n<think>system thoughts</think>";
    let mut body = json!({
        "messages": [
            { "role": "system", "content": system_content },
            { "role": "user", "content": "Scan the application." },
            { "role": "assistant", "content": "Starting scan.", "reasoning": "some thought" },
            { "role": "tool", "content": "Port 80 open\nPort 443 open" },
            { "role": "assistant", "content": "Now reviewing code." }
        ]
    });

    let _stats = condenser.condense(&mut body);

    let msgs = body["messages"].as_array().unwrap();
    // Invariant: System prompt is completely untouched
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[0]["content"], system_content);
}

#[test]
fn test_user_messages_never_mutated() {
    let config = CondenserConfig {
        enabled: true,
        drop_historical_reasoning: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 50,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let user_prompt_1 = "Please audit /pkg/docker/docker.go for container escapes.\n\n\n\nMake sure to check symlinks.";
    let user_prompt_2 = "Can you check the database queries as well?\n\n";

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "System prompt" },
            { "role": "user", "content": user_prompt_1 },
            { "role": "assistant", "content": "Checked docker.go" },
            { "role": "user", "content": user_prompt_2 },
            { "role": "assistant", "content": "Checking db queries" }
        ]
    });

    let _stats = condenser.condense(&mut body);

    let msgs = body["messages"].as_array().unwrap();
    // Invariant: User messages are completely untouched
    assert_eq!(msgs[1]["content"], user_prompt_1);
    assert_eq!(msgs[3]["content"], user_prompt_2);
}

#[test]
fn test_assistant_tool_calls_never_mutated() {
    let config = CondenserConfig {
        enabled: true,
        drop_historical_reasoning: true,
        keep_recent_turns: 1,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let tool_calls = json!([
        {
            "id": "call_12345",
            "type": "function",
            "function": {
                "name": "read_file",
                "arguments": "{\"path\": \"/pkg/docker/docker.go\", \"offset\": 0, \"limit\": 500}"
            }
        }
    ]);

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "System prompt" },
            { "role": "user", "content": "Read file" },
            {
                "role": "assistant",
                "content": "Reading file now.",
                "reasoning": "I need to inspect the Snapshot function.",
                "reasoning_content": "Deep reasoning here.",
                "tool_calls": tool_calls
            },
            { "role": "tool", "content": "package docker\nimport \"os\"" },
            { "role": "assistant", "content": "Found vulnerability." }
        ]
    });

    let stats = condenser.condense(&mut body);

    assert_eq!(stats.reasoning_blocks_dropped, 2);
    let msgs = body["messages"].as_array().unwrap();
    let asst_msg = &msgs[2];

    // Invariant: reasoning and reasoning_content are stripped
    assert!(asst_msg.get("reasoning").is_none());
    assert!(asst_msg.get("reasoning_content").is_none());

    // Invariant: tool_calls array is 100% preserved
    assert_eq!(asst_msg["tool_calls"], tool_calls);
}

#[test]
fn test_source_code_braces_never_mangled_by_deduplication() {
    let config = CondenserConfig {
        enabled: true,
        deduplicate_lines: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let go_code = "func Snapshot() error {\n\tif err != nil {\n\t\treturn err\n\t}\n}\n";
    let cleaned = condenser.clean_tool_text(go_code);

    // Invariant: closing braces `\t}` and `}` must NOT be deduplicated
    assert_eq!(cleaned, go_code);
    assert!(!cleaned.contains("repeated"));
}

#[test]
fn test_repetitive_terminal_logs_collapsed() {
    let config = CondenserConfig {
        enabled: true,
        deduplicate_lines: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let log = "Scanning target port 80...\n[WARN] Connection refused to port 8080\n[WARN] Connection refused to port 8080\n[WARN] Connection refused to port 8080\n[WARN] Connection refused to port 8080\nScan completed.";
    let cleaned = condenser.clean_tool_text(log);

    assert!(cleaned.contains("[WARN] Connection refused to port 8080"));
    assert!(cleaned.contains("[... repeated 3 times ...]"));
    assert!(cleaned.contains("Scan completed."));
}

#[test]
fn test_recent_turn_preserved_vs_historical_turn_truncated() {
    let config = CondenserConfig {
        enabled: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 100,        // Historical threshold
        recent_max_tool_chars: 10000, // Recent active threshold
        head_lines: 2,
        tail_lines: 2,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let historical_large_dump = (1..=50).map(|i| format!("Historical line {}: data", i)).collect::<Vec<_>>().join("\n");
    let recent_source_code = (1..=50).map(|i| format!("Active source code line {}: func()", i)).collect::<Vec<_>>().join("\n");

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a scanner." },
            { "role": "user", "content": "Begin audit." },
            // Historical Turn (Turn 1)
            { "role": "assistant", "content": "Running find." },
            { "role": "tool", "content": historical_large_dump },
            // Recent Active Turn (Turn 2)
            { "role": "assistant", "content": "Reading active source file." },
            { "role": "tool", "content": recent_source_code }
        ]
    });

    let stats = condenser.condense(&mut body);

    assert_eq!(stats.tool_outputs_truncated, 1);
    let msgs = body["messages"].as_array().unwrap();

    // Turn 1 tool output (historical) MUST be truncated
    let hist_content = msgs[3]["content"].as_str().unwrap();
    assert!(hist_content.contains("Ferry Proxy: truncated"));

    // Turn 2 tool output (recent active) MUST NOT be truncated because len < 10000
    let recent_content = msgs[5]["content"].as_str().unwrap();
    assert!(!recent_content.contains("Ferry Proxy: truncated"));
    assert!(recent_content.contains("Active source code line 50"));
}

#[test]
fn test_dual_key_openai_agents_format() {
    let config = CondenserConfig {
        enabled: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 50,
        recent_max_tool_chars: 5000,
        head_lines: 2,
        tail_lines: 2,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let historical_dump = (1..=30).map(|i| format!("Output item {}", i)).collect::<Vec<_>>().join("\n");

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "System" },
            { "role": "user", "content": "Start" },
            // Agents API schema: type: function_call_output, output: "..."
            { "type": "function_call_output", "call_id": "c1", "output": historical_dump },
            { "role": "assistant", "content": "Done" }
        ]
    });

    let stats = condenser.condense(&mut body);
    assert_eq!(stats.tool_outputs_truncated, 1);

    let msgs = body["messages"].as_array().unwrap();
    let out = msgs[2]["output"].as_str().unwrap();
    assert!(out.contains("Ferry Proxy: truncated"));
}

#[test]
fn test_cleaner_counters_and_metrics() {
    let config = CondenserConfig {
        enabled: true,
        strip_terminal_noise: true,
        deduplicate_lines: true,
        minify_json_outputs: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let mut stats = ferry::condenser::CondensationStats::default();

    let raw = "\x1b[32mOK\x1b[0m\nDownload: [======>] 100%\nRepeated log message here\nRepeated log message here\nRepeated log message here\n\n\n\nFinal text";
    let cleaned = condenser.clean_tool_text_with_stats(raw, &mut stats);

    let json_raw = "{\n  \"foo\": \"bar\",\n  \"count\": 42\n}";
    let cleaned_json = condenser.clean_tool_text_with_stats(json_raw, &mut stats);

    assert!(stats.ansi_stripped_count >= 1);
    assert!(stats.progress_cleaned_count >= 1);
    assert!(stats.lines_deduped_count >= 1);
    assert!(stats.whitespace_collapsed_count >= 1);
    assert!(stats.json_minified_count >= 1);
    assert!(!cleaned.contains("\x1b[32m"));
    assert!(!cleaned.contains("100%"));
    assert_eq!(cleaned_json, r#"{"count":42,"foo":"bar"}"#);
}

#[test]
fn test_code_fence_protection_in_tool_output() {
    let config = CondenserConfig {
        enabled: true,
        strip_terminal_noise: true,
        deduplicate_lines: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let raw_tool_body = "Progress log line 1\nProgress log line 1\nProgress log line 1\n```rust\nfn calculate() {\n    let a = 1;\n    let a = 1;\n    let a = 1;\n}\n```\nDone.";
    let cleaned = condenser.clean_tool_text(raw_tool_body);

    // Repetitive logs outside code block are collapsed
    assert!(cleaned.contains("[... repeated 2 times ...]"));
    // Code block contents are completely preserved without deduplication
    assert!(cleaned.contains("```rust\nfn calculate() {\n    let a = 1;\n    let a = 1;\n    let a = 1;\n}\n```"));
}

#[test]
fn test_constraint_ledger_verifies_anchors() {
    let config = CondenserConfig {
        enabled: true,
        deduplicate_lines: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let raw = "Auditing /var/log/detainer.log at https://github.com:443\nDuplicate notice\nDuplicate notice\nDuplicate notice\nDO NOT delete /etc/shadow";
    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a scanner." },
            { "role": "user", "content": "Scan start" },
            { "role": "assistant", "content": "Running tool." },
            { "role": "tool", "content": raw }
        ]
    });

    let stats = condenser.condense(&mut body);

    // Ledger check passed because path, URL, port, and negation are preserved
    assert_eq!(stats.ledger_checks_passed, 1);
    assert_eq!(stats.ledger_vetoed_count, 0);

    let msgs = body["messages"].as_array().unwrap();
    let cleaned = msgs[3]["content"].as_str().unwrap();
    assert!(cleaned.contains("/var/log/detainer.log"));
    assert!(cleaned.contains("https://github.com:443"));
    assert!(cleaned.contains("DO NOT delete /etc/shadow"));
}

#[test]
fn test_constraint_ledger_veto_restores_verbatim() {
    let config = CondenserConfig {
        enabled: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    // Test verify_ledger directly
    let original = "Audit /var/run/secrets/token and do NOT bypass https://internal.corp:8443";
    let anchors = condenser.extract_critical_anchors(original);
    assert!(anchors.iter().any(|a| a == "/var/run/secrets/token"));
    assert!(anchors.iter().any(|a| a == "https://internal.corp:8443"));
    assert!(anchors.iter().any(|a| a.eq_ignore_ascii_case("not")));

    // If candidate text lost the anchor "/var/run/secrets/token", verify_ledger MUST return false
    let bad_condensed = "Audit token and do NOT bypass https://internal.corp:8443";
    assert!(!condenser.verify_ledger(&anchors, bad_condensed));
    assert_eq!(condenser.find_missing_anchor(&anchors, bad_condensed), Some("/var/run/secrets/token".to_string()));

    // If candidate text preserves all anchors, verify_ledger returns true
    let good_condensed = "Audit /var/run/secrets/token and do not bypass https://internal.corp:8443";
    assert!(condenser.verify_ledger(&anchors, good_condensed));
}

#[test]
fn test_constraint_ledger_veto_in_condense() {
    let config = CondenserConfig {
        enabled: true,
        truncate_tool_outputs: true,
        recent_max_tool_chars: 50, // Truncation triggers on recent active turn
        head_lines: 1,
        tail_lines: 1,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let mut lines = Vec::new();
    for i in 1..=30 { lines.push(format!("Header line {}", i)); }
    lines.push("CRITICAL /var/run/secrets/token".to_string());
    for i in 1..=25 { lines.push(format!("Footer line {}", i)); }
    let raw = lines.join("\n");

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a scanner." },
            { "role": "user", "content": "Scan start" },
            { "role": "assistant", "content": "Running tool." },
            { "role": "tool", "content": raw }
        ]
    });

    let stats = condenser.condense(&mut body);

    // Because truncation would lose the anchor in the middle, ledger vetoes and restores verbatim
    assert_eq!(stats.ledger_vetoed_count, 1);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[3]["content"].as_str().unwrap(), raw);
}

#[test]
fn test_prefix_cache_alignment_preserves_initial_turn_bytes() {
    let config = CondenserConfig {
        enabled: true,
        drop_historical_reasoning: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 50,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let system_prompt = "You are a vulnerability research agent with strict operating parameters.";
    let initial_user_prompt = "Please audit detainer repo commit 42e3f1.";

    let mut body = json!({
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": initial_user_prompt },
            { "role": "assistant", "content": "Running git log.", "reasoning": "checking commit" },
            { "role": "tool", "content": "Commit details:\n".to_string() + &"detail line\n".repeat(50) }
        ]
    });

    let _stats = condenser.condense(&mut body);
    let msgs = body["messages"].as_array().unwrap();

    // Verify system message is 100% byte-for-byte identical
    assert_eq!(msgs[0]["content"].as_str().unwrap(), system_prompt);
    // Verify first user prompt is 100% byte-for-byte identical
    assert_eq!(msgs[1]["content"].as_str().unwrap(), initial_user_prompt);
}

#[test]
fn test_multibyte_utf8_truncation_no_panic() {
    let config = CondenserConfig {
        enabled: true,
        truncate_tool_outputs: true,
        max_tool_chars: 100,
        recent_max_tool_chars: 100,
        head_lines: 5,
        tail_lines: 5,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    // Multi-byte Chinese chars on a single line: byte offset 300 will land inside a 3-byte char
    let raw = "a".to_string() + &"测试中文日志数据".repeat(100);
    let mut body = json!({
        "messages": [
            { "role": "system", "content": "System" },
            { "role": "user", "content": "Start" },
            { "role": "assistant", "content": "Run" },
            { "role": "tool", "content": raw }
        ]
    });

    // Invariant: this MUST NOT panic on non-char-boundary slicing
    let stats = condenser.condense(&mut body);
    assert!(stats.tool_outputs_truncated >= 1);
    let msgs = body["messages"].as_array().unwrap();
    let content = msgs[3]["content"].as_str().unwrap();
    assert!(content.contains("Ferry Proxy: truncated"));
}

#[test]
fn test_strip_multiple_think_tags() {
    let raw = "<think>first thought</think>Middle text <think>second thought</think>Final answer.";
    let stripped = ferry::condenser::strip_think_tags(raw).unwrap();
    assert_eq!(stripped, "Middle text Final answer.");
}

#[test]
fn test_strip_unclosed_think_tag() {
    let raw = "Intro text <think>Unfinished thoughts cut off by token limit";
    let stripped = ferry::condenser::strip_think_tags(raw).unwrap();
    assert_eq!(stripped, "Intro text");
}

#[test]
fn test_developer_role_prefix_protected() {
    let config = CondenserConfig {
        enabled: true,
        drop_historical_reasoning: true,
        truncate_tool_outputs: true,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let dev_content = "You are a developer system prompt.\n<think>do not remove</think>\n{\n  \"rule\": 1\n}";
    let mut body = json!({
        "messages": [
            { "role": "developer", "content": dev_content },
            { "role": "user", "content": "Hello" }
        ]
    });

    let _stats = condenser.condense(&mut body);
    let msgs = body["messages"].as_array().unwrap();
    // Invariant: developer role is 100% byte-for-byte protected
    assert_eq!(msgs[0]["content"].as_str().unwrap(), dev_content);
}

#[test]
fn test_security_anchors_cve_ip_port_and_configs() {
    let config = CondenserConfig::default();
    let condenser = ContextCondenser::new(config);

    let text = "Discovered CVE-2024-38063 and GHSA-42x-99 on 192.168.1.50:8080. Open port 443/tcp in Dockerfile and Cargo.toml.";
    let anchors = condenser.extract_critical_anchors(text);

    assert!(anchors.iter().any(|a| a == "CVE-2024-38063"));
    assert!(anchors.iter().any(|a| a == "GHSA-42x-99"));
    assert!(anchors.iter().any(|a| a == "192.168.1.50:8080"));
    assert!(anchors.iter().any(|a| a == "443/tcp"));
    assert!(anchors.iter().any(|a| a == "Dockerfile"));
    assert!(anchors.iter().any(|a| a == "Cargo.toml"));
}

#[test]
fn test_tool_output_array_content_condensed() {
    let config = CondenserConfig {
        enabled: true,
        truncate_tool_outputs: true,
        keep_recent_turns: 1,
        max_tool_chars: 50,
        recent_max_tool_chars: 5000,
        head_lines: 2,
        tail_lines: 2,
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let historical_dump = (1..=30).map(|i| format!("Array output line {}", i)).collect::<Vec<_>>().join("\n");

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "System" },
            { "role": "user", "content": "Start" },
            {
                "role": "tool",
                "content": [
                    { "type": "text", "text": historical_dump }
                ]
            },
            { "role": "assistant", "content": "Done" }
        ]
    });

    let stats = condenser.condense(&mut body);
    assert_eq!(stats.tool_outputs_truncated, 1);

    let msgs = body["messages"].as_array().unwrap();
    let text = msgs[2]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Ferry Proxy: truncated"));
}

#[test]
fn test_windows_path_anchors_preserved() {
    let config = CondenserConfig::default();
    let condenser = ContextCondenser::new(config);

    let text = r"Check C:\Users\app\main.py and src\pkg\docker.go for vulnerabilities.";
    let anchors = condenser.extract_critical_anchors(text);

    assert!(anchors.iter().any(|a| a.contains("main.py")));
    assert!(anchors.iter().any(|a| a.contains("docker.go")));
}

#[tokio::test]
async fn test_explicit_cloud_model_selection_router() {
    use ferry::config::Config;
    use ferry::router::{Router, TargetProvider};
    use ferry::stats::MetricsTracker;
    use std::sync::Arc;

    let config = Arc::new(Config::default());
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics);

    let body = json!({
        "model": "MiniMax-M3",
        "messages": [
            { "role": "user", "content": "Hello" }
        ]
    });

    let decision = router.decide(&body, None).await;
    assert_eq!(decision.target, TargetProvider::Cloud);
    assert!(decision.reason.contains("explicitly requested"));
}

#[tokio::test]
async fn test_circuit_breaker_trip_and_recovery() {
    use ferry::config::Config;
    use ferry::router::{Router, TargetProvider};
    use ferry::stats::MetricsTracker;
    use std::sync::Arc;

    let config = Arc::new(Config::default());
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics.clone());

    let body = json!({
        "messages": [
            { "role": "user", "content": "Query" }
        ]
    });

    // 1. Initial healthy state: routes to local
    let d1 = router.decide(&body, None).await;
    assert_eq!(d1.target, ferry::router::TargetProvider::Local);
    assert!(!metrics.is_circuit_open());
    assert!(metrics.strata_online.load(std::sync::atomic::Ordering::Relaxed));

    // 2. Simulate 2 consecutive failures (threshold is 3)
    assert!(!metrics.record_local_failure(3));
    assert!(!metrics.record_local_failure(3));
    assert!(!metrics.is_circuit_open());

    // 3. 3rd consecutive failure: trips the circuit breaker!
    let tripped = metrics.record_local_failure(3);
    assert!(tripped);
    assert!(metrics.is_circuit_open());
    assert!(!metrics.strata_online.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(metrics.circuit_trips_total.load(std::sync::atomic::Ordering::Relaxed), 1);

    // 4. Now router MUST immediately route to Cloud with 0ms delay!
    let d2 = router.decide(&body, None).await;
    assert_eq!(d2.target, TargetProvider::Cloud);
    assert!(d2.reason.contains("circuit breaker open"));

    // 5. Simulate Strata recovery (probe succeeds or turn succeeds)
    metrics.record_local_success();
    assert!(!metrics.is_circuit_open());
    assert!(metrics.strata_online.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(metrics.consecutive_local_failures.load(std::sync::atomic::Ordering::Relaxed), 0);

    // 6. Router immediately resumes local GPU routing!
    let d3 = router.decide(&body, None).await;
    assert_eq!(d3.target, ferry::router::TargetProvider::Local);
}

#[tokio::test]
async fn test_route_override_client_headers() {
    use ferry::config::Config;
    use ferry::router::{Router, TargetProvider};
    use ferry::stats::MetricsTracker;
    use std::sync::Arc;

    let config = Arc::new(Config::default());
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics.clone());

    let body = json!({
        "messages": [
            { "role": "user", "content": "Query" }
        ]
    });

    // Cloud-only override
    let d_cloud = router.decide(&body, Some("cloud-only")).await;
    assert_eq!(d_cloud.target, TargetProvider::Cloud);
    assert!(!d_cloud.local_only);

    // Local-only override when healthy
    let d_local = router.decide(&body, Some("local-only")).await;
    assert_eq!(d_local.target, ferry::router::TargetProvider::Local);
    assert!(d_local.local_only);

    // Local-only override when circuit is open
    metrics.record_local_failure(1);
    let d_fail = router.decide(&body, Some("local-only")).await;
    assert!(d_fail.local_only);
    assert_eq!(d_fail.target, TargetProvider::Cloud);
}

#[tokio::test]
async fn test_queue_contention_slot_release() {
    use ferry::config::Config;
    use ferry::router::{Router, TargetProvider};
    use ferry::stats::MetricsTracker;
    use std::sync::Arc;

    let mut cfg = Config::default();
    cfg.local.max_slots = 1;
    cfg.local.queue_timeout_ms = 100; // wait up to 100ms
    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics.clone());

    let body = json!({
        "messages": [
            { "role": "user", "content": "Query" }
        ]
    });

    // Saturate the only slot
    metrics.active_local_slots.store(1, std::sync::atomic::Ordering::Relaxed);

    // Spawn a worker to free the slot in 30ms
    let m = metrics.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
        m.active_local_slots.store(0, std::sync::atomic::Ordering::Relaxed);
    });

    // Router should wait in queue, notice slot free, and route to Local!
    let d = router.decide(&body, None).await;
    assert_eq!(d.target, ferry::router::TargetProvider::Local);
    assert!(d.slot_guard.is_some());
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 1);

    // Dropping decision releases slot immediately
    drop(d);
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn test_atomic_slot_concurrency_no_oversubscription() {
    use ferry::config::Config;
    use ferry::router::{Router, TargetProvider};
    use ferry::stats::MetricsTracker;
    use std::sync::Arc;

    let mut cfg = Config::default();
    cfg.local.max_slots = 3;
    cfg.local.queue_timeout_ms = 0; // immediate spillover on contention
    let config = Arc::new(cfg);
    let metrics = Arc::new(MetricsTracker::new());
    let router = Arc::new(Router::new(config, metrics.clone()));

    let mut handles = Vec::new();

    // Spawn 10 concurrent requests simultaneously
    for _ in 0..10 {
        let r = router.clone();
        handles.push(tokio::spawn(async move {
            let body = json!({
                "messages": [
                    { "role": "user", "content": "Concurrent test query" }
                ]
            });
            r.decide(&body, None).await
        }));
    }

    let mut local_count = 0;
    let mut cloud_count = 0;
    let mut retained_decisions = Vec::new();

    for handle in handles {
        let decision = handle.await.unwrap();
        match decision.target {
            ferry::router::TargetProvider::Local => {
                local_count += 1;
                retained_decisions.push(decision);
            }
            TargetProvider::Cloud => {
                cloud_count += 1;
            }
        }
    }

    // Invariant: EXACTLY 3 requests acquired local slots; 7 spilled to cloud
    assert_eq!(local_count, 3);
    assert_eq!(cloud_count, 7);
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 3);

    // Releasing one decision frees up exactly one slot
    retained_decisions.pop();
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 2);

    // Releasing all remaining decisions drops active slots back to zero
    retained_decisions.clear();
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn test_latency_and_duration_prometheus_metrics() {
    use ferry::stats::MetricsTracker;

    let metrics = MetricsTracker::new();
    metrics.record_local_completion(450);
    metrics.record_local_completion(550);
    metrics.record_cloud_completion(1200);

    let snap = metrics.snapshot();
    assert_eq!(snap.local_duration_ms_total, 1000);
    assert_eq!(snap.cloud_duration_ms_total, 1200);
    assert_eq!(snap.last_duration_ms, 1200);

    let prom = metrics.prometheus_metrics();
    assert!(prom.contains("ferry_request_duration_ms_total{provider=\"local\"} 1000"));
    assert!(prom.contains("ferry_request_duration_ms_total{provider=\"cloud\"} 1200"));
    assert!(prom.contains("ferry_last_request_duration_ms 1200"));
}

#[test]
fn test_zero_allocation_char_counting_parity() {
    use ferry::condenser::count_message_chars;

    let messages = json!([
        { "role": "system", "content": "System prompt" },
        { "role": "user", "content": [{"type": "text", "text": "Part 1 "}, {"type": "text", "text": "Part 2"}] },
        {
            "role": "assistant",
            "content": "Assistant msg",
            "reasoning": "internal thought",
            "tool_calls": [
                {
                    "type": "function",
                    "function": { "name": "foo", "arguments": "{\"x\": 1}" }
                }
            ]
        },
        { "type": "function_call_output", "output": "command output" }
    ]);

    let count = count_message_chars(messages.as_array().unwrap());
    // "System prompt" (13) + "Part 1 " (7) + "Part 2" (6) + "Assistant msg" (13) +
    // "internal thought" (16) + "{\"x\": 1}" (8) + "command output" (14) = 77
    assert_eq!(count, 77);
}

#[test]
fn test_signal_aware_truncation_preserves_middle_error() {
    use ferry::condenser::truncate_tool_content_smart;

    let mut lines = Vec::new();
    for i in 1..=100 {
        if i == 50 {
            lines.push("fatal error: connection to postgres failed: password authentication failed".to_string());
        } else {
            lines.push(format!("info: log event {i} passed successfully"));
        }
    }
    let full_text = lines.join("\n");

    let mut signals_extracted = 0;
    // With signal-aware smart truncation, head 5 lines, tail 5 lines
    let truncated = truncate_tool_content_smart(&full_text, 5, 5, &mut signals_extracted);

    assert_eq!(signals_extracted, 1);
    assert!(truncated.contains("fatal error: connection to postgres failed"));
    assert!(truncated.contains("info: log event 1"));
    assert!(truncated.contains("info: log event 100"));
    assert!(truncated.contains("Ferry Proxy: truncated"));

    // When no signals exist in the middle, truncation runs standard head/tail omission
    let clean_text: String = (1..=100)
        .map(|i| format!("info: standard log event {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut no_signals = 0;
    let clean_truncated = truncate_tool_content_smart(&clean_text, 5, 5, &mut no_signals);
    assert_eq!(no_signals, 0);
    assert!(clean_truncated.contains("info: standard log event 1"));
    assert!(clean_truncated.contains("info: standard log event 100"));
    assert!(!clean_truncated.contains("info: standard log event 50"));
}

#[test]
fn test_runaway_repetition_folding_collapses_duplicate_historical_failures() {
    let config = CondenserConfig {
        enabled: true,
        fold_repeated_tool_failures: true,
        keep_recent_turns: 1,
        truncate_tool_outputs: false, // focus purely on loop folding
        ..Default::default()
    };
    let condenser = ContextCondenser::new(config);

    let repeated_error = "curl: (7) Failed to connect to localhost port 8000: Connection refused (repeated attempt)";

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a scanner." },
            { "role": "user", "content": "Probe endpoint" },
            { "role": "assistant", "content": "Attempt 1" },
            { "role": "tool", "content": repeated_error },
            { "role": "assistant", "content": "Attempt 2" },
            { "role": "tool", "content": repeated_error },
            { "role": "assistant", "content": "Attempt 3" },
            { "role": "tool", "content": repeated_error },
            { "role": "assistant", "content": "Recent active turn" },
            { "role": "tool", "content": repeated_error } // recent turn: must be untouched
        ]
    });

    let stats = condenser.condense(&mut body);
    assert!(stats.loops_collapsed_count >= 1);

    let msgs = body["messages"].as_array().unwrap();
    // Attempt 1: preserved verbatim
    assert_eq!(msgs[3]["content"], repeated_error);
    // Attempt 2: folded
    assert!(msgs[5]["content"].as_str().unwrap().contains("Repeated identical tool failure suppressed"));
    // Attempt 3: folded
    assert!(msgs[7]["content"].as_str().unwrap().contains("Repeated identical tool failure suppressed"));
    // Recent turn (Attempt 4): untouched
    assert_eq!(msgs[9]["content"], repeated_error);
}

#[tokio::test]
async fn test_elastic_token_budget_allocation_and_release() {
    let metrics = Arc::new(MetricsTracker::default());
    let max_slots = 2;
    let total_budget = 200_000u64;

    // 1. Allocate 120,000 tokens for Slot 1 -> Succeeds
    assert!(metrics.try_acquire_local_capacity(120_000, max_slots, total_budget));
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed), 120_000);

    // 2. Try to allocate 100,000 tokens for Slot 2 -> Fails (120k + 100k = 220k > 200k budget)
    assert!(!metrics.try_acquire_local_capacity(100_000, max_slots, total_budget));
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 1);

    // 3. Allocate 60,000 tokens for Slot 2 -> Succeeds (120k + 60k = 180k <= 200k budget)
    assert!(metrics.try_acquire_local_capacity(60_000, max_slots, total_budget));
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed), 180_000);

    // 4. Try to allocate for Slot 3 -> Fails (exceeds max_slots = 2)
    assert!(!metrics.try_acquire_local_capacity(10_000, max_slots, total_budget));

    // 5. Release Slot 1 (120k tokens)
    metrics.release_local_capacity(120_000);
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed), 60_000);

    // 6. Now Slot 2 can allocate up to 140,000 tokens
    assert!(metrics.try_acquire_local_capacity(130_000, max_slots, total_budget));
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed), 190_000);

    // Clean up
    metrics.release_local_capacity(60_000);
    metrics.release_local_capacity(130_000);
    assert_eq!(metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn test_adaptive_squeeze_pass() {
    let mut config = CondenserConfig::default();
    config.keep_recent_turns = 1;
    let condenser = ContextCondenser::new(config);

    let large_historical_tool = "INFO: Testing vulnerability target\n".repeat(80); // ~2800 chars
    let recent_tool = "RECENT: Fuzzing response status 200 OK\n".repeat(40);

    let mut body = json!({
        "messages": [
            { "role": "system", "content": "You are a scanner." },
            { "role": "user", "content": "Run scan" },
            { "role": "assistant", "content": "Checking historical target" },
            { "role": "tool", "content": large_historical_tool },
            { "role": "assistant", "content": "Checking recent target" },
            { "role": "tool", "content": recent_tool }
        ]
    });

    // Target a token budget smaller than current
    let orig_tokens = Router::estimate_tokens(&body);
    let target = orig_tokens - 100;

    let extra_saved = condenser.adaptive_squeeze(&mut body, target);
    assert!(extra_saved.is_some());
    assert!(extra_saved.unwrap() > 0);

    let new_tokens = Router::estimate_tokens(&body);
    assert!(new_tokens < orig_tokens);

    // Verify recent turn was NOT touched by adaptive squeeze
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[5]["content"], recent_tool);
}





#[tokio::test]
async fn test_guarded_stream_sse_graceful_done_on_upstream_error() {
    use futures_util::StreamExt;
    let err_stream = futures_util::stream::iter(vec![
        Ok(bytes::Bytes::from("data: {\"chunk\": 1}\n\n")),
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset by peer")),
    ]);

    let guarded = ferry::upstream::GuardedStream::new(
        err_stream,
        None,
        true,
        std::time::Duration::from_secs(10),
    );

    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = guarded.collect().await;
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].as_ref().unwrap(), &bytes::Bytes::from("data: {\"chunk\": 1}\n\n"));
    assert_eq!(chunks[1].as_ref().unwrap(), &bytes::Bytes::from("\n\ndata: [DONE]\n\n"));
}

#[tokio::test]
async fn test_short_context_priority_queueing_wait_limit() {
    let mut config = ferry::config::Config::default();
    config.local.queue_timeout_ms = 200; // base wait
    config.local.max_slots = 1;
    let metrics = Arc::new(MetricsTracker::default());
    let router = Router::new(Arc::new(config), metrics.clone());

    // Acquire the only slot
    assert!(metrics.try_acquire_local_capacity(1000, 1, 390000));

    // A short request (<= 35000 tokens) should have extended patience (12000ms max, tested with quick release)
    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": "quick tool check" }]
    });

    let est = Router::estimate_tokens(&body);
    assert!(est <= 35000);

    // Release slot after 80ms in background
    let m = metrics.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        m.release_local_capacity(1000);
    });

    let decision = router.decide(&body, None).await;
    assert_eq!(decision.target, ferry::router::TargetProvider::Local);
    assert!(decision.reason.contains("contention queue"));
}

#[tokio::test]
async fn test_response_cache_deterministic_hit_and_miss() {
    use ferry::cache::ResponseCache;
    use serde_json::json;

    let cache = ResponseCache::new(true, 10, 60);

    let body_deterministic = json!({
        "model": "ferry",
        "messages": [
            { "role": "system", "content": "You are a test" },
            { "role": "user", "content": "test question" }
        ],
        "temperature": 0.0,
        "stream": false
    });

    let body_streaming = json!({
        "model": "ferry",
        "messages": [
            { "role": "user", "content": "test question" }
        ],
        "stream": true
    });

    // 1. Streaming should never be cached
    assert!(ResponseCache::compute_cache_key(&body_streaming).is_none());

    // 2. Deterministic non-streaming should generate a key
    let key = ResponseCache::compute_cache_key(&body_deterministic);
    assert!(key.is_some());
    let key = key.unwrap();

    // 3. Cache miss initially
    assert!(cache.get(&key).await.is_none());

    // 4. Populate cache
    cache.set(key.clone(), bytes::Bytes::from("{\"result\": \"4\"}"), 200, vec![]).await;

    // 5. Cache hit
    let hit = cache.get(&key).await;
    assert!(hit.is_some());
    assert_eq!(hit.unwrap().body_bytes, bytes::Bytes::from("{\"result\": \"4\"}"));
    assert_eq!(cache.hits.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_adaptive_queueing_wait_timeout_override() {
    use ferry::config::Config;
    use ferry::router::Router;
    use ferry::stats::MetricsTracker;
    use serde_json::json;
    use std::sync::Arc;

    let mut config = Config::default();
    config.local.max_slots = 1;
    config.local.queue_timeout_ms = 100;
    let config = Arc::new(config);
    let metrics = Arc::new(MetricsTracker::new());
    let router = Router::new(config, metrics.clone());

    let body = json!({
        "messages": [{ "role": "user", "content": "hello" }]
    });

    // Acquire the only slot
    let d1 = router.decide(&body, None).await;
    assert_eq!(d1.target, ferry::router::TargetProvider::Local);

    // Client requests fast mode -> should spill almost immediately (500ms max)
    let d_fast = router.decide_with_timeout(&body, Some("fast"), None).await;
    assert_eq!(d_fast.target, ferry::router::TargetProvider::Cloud);
    assert!(d_fast.is_spillover);
}
