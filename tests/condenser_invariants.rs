use ferry::condenser::ContextCondenser;
use ferry::config::CondenserConfig;
use serde_json::json;

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
        strip_ansi_escapes: true,
        strip_progress_bars: true,
        collapse_whitespace: true,
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
        strip_ansi_escapes: true,
        strip_progress_bars: true,
        deduplicate_lines: true,
        collapse_whitespace: true,
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
        protect_code_blocks: true,
        deduplicate_lines: true,
        collapse_whitespace: true,
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
        enable_constraint_ledger: true,
        protect_code_blocks: true,
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
        enable_constraint_ledger: true,
        protect_code_blocks: true,
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
        enable_constraint_ledger: true,
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
        protect_static_prefix: true,
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
        protect_static_prefix: true,
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

#[test]
fn test_explicit_cloud_model_selection_router() {
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

    let decision = router.decide(&body);
    assert_eq!(decision.target, TargetProvider::Cloud);
    assert!(decision.reason.contains("explicitly requested"));
}


