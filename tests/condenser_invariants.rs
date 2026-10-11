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
