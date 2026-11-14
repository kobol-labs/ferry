use serde_json::Value;

pub const HARD_LOGIC_DIAGNOSTIC_SIGNALS: &[&str] = &[
    "panic:",
    "fatal:",
    "fatal error:",
    "traceback (most recent call last):",
    "nullpointerexception",
    "segmentation fault",
    "sigsegv",
    "sigabrt",
    "unhandled rejection",
    "uncaught exception",
    "undefined is not a function",
    "exception in thread",
    "assertion failed",
    "cwe-",
    "cve-",
    "security alert",
    "injection vulnerability",
    "buffer overflow",
    "use-after-free",
    "error[e",
    "status: 500",
    "status 500",
    "status: 502",
    "status 503",
    "test failed",
    "failing tests",
    "failures:",
    "failed (",
    "build failed",
    "compilation failed",
];

pub const HARD_LOGIC_USER_KEYWORDS: &[&str] = &[
    "threat model",
    "architect",
    "refactor",
    "diagnose",
    "root cause",
    "race condition",
    "deadlock",
    "memory leak",
    "heap dump",
    "decompile",
    "disassemble",
    "invariant",
    "formal verification",
    "algorithmic",
    "exploit synthesis",
    "proof of concept",
];

pub const COGNITIVE_TOOL_CONTEXT_KEYWORDS: &[&str] = &[
    "audit",
    "vulnerability",
    "vulnerabilities",
    "cve",
    "cwe",
    "exploit",
    "penetration",
    "security review",
];

pub fn estimate_tokens(body: &Value) -> u64 {
    let mut total_chars: usize = 0;

    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                total_chars += content.len();
            } else if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        total_chars += text.len();
                    }
                }
            }
            if let Some(calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for call in calls {
                    if let Some(func) = call.get("function") {
                        if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                            total_chars += args.len();
                        }
                    }
                }
            }
        }
    }

    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        for tool in tools {
            if let Some(func) = tool.get("function") {
                if let Some(desc) = func.get("description").and_then(|d| d.as_str()) {
                    total_chars += desc.len();
                }
                if let Some(params) = func.get("parameters") {
                    total_chars += estimated_json_len(params);
                }
            }
        }
    }

    crate::condenser::estimate_tokens_from_chars(total_chars)
}

fn json_escaped_str_len(s: &str) -> usize {
    let mut len = 2;
    for b in s.bytes() {
        len += match b {
            b'"' | b'\\' | 0x08 | 0x09 | 0x0a | 0x0c | 0x0d => 2,
            0x00..=0x1f => 6,
            _ => 1,
        };
    }
    len
}

fn estimated_json_len(val: &Value) -> usize {
    struct ByteCounter(usize);
    impl std::io::Write for ByteCounter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    match val {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => {
            let mut counter = ByteCounter(0);
            let _ = serde_json::to_writer(&mut counter, n);
            counter.0
        }
        Value::String(s) => json_escaped_str_len(s),
        Value::Array(arr) => {
            2 + arr.len().saturating_sub(1) + arr.iter().map(estimated_json_len).sum::<usize>()
        }
        Value::Object(map) => {
            2 + map.len().saturating_sub(1)
                + map
                    .iter()
                    .map(|(k, v)| json_escaped_str_len(k) + 1 + estimated_json_len(v))
                    .sum::<usize>()
        }
    }
}

pub fn detect_orchestrator(body: &Value, session_id: &str, priority_override: Option<&str>) -> bool {
    if let Some(prio) = priority_override {
        let p_lower = prio.to_lowercase();
        if matches!(p_lower.as_str(), "high" | "orchestrator" | "root" | "prio") {
            return true;
        }
    }

    // 1. Inspect metadata: if explicitly named as a specialist/worker, it is NOT an orchestrator
    if let Some(meta) = body.get("metadata").and_then(|m| m.as_object()) {
        for key in &["agent_name", "role", "agent"] {
            if let Some(val) = meta.get(*key).and_then(|v| v.as_str()) {
                let v_lower = val.to_lowercase();
                if v_lower.contains("specialist")
                    || v_lower.contains("hunter")
                    || v_lower.contains("validator")
                    || v_lower.contains("scanner")
                    || v_lower.contains("worker")
                    || v_lower.contains("triage")
                    || v_lower.contains("recon")
                {
                    return false;
                }
                if v_lower == "root agent"
                    || v_lower == "root"
                    || v_lower.contains("orchestrat")
                    || v_lower.contains("coordinator")
                {
                    return true;
                }
            }
        }
    }

    // 2. Inspect session ID: only match if it explicitly indicates root/orchestrator
    let sess_lower = session_id.to_lowercase();
    if (sess_lower.contains("root") || sess_lower.contains("orchestrat"))
        && !sess_lower.contains("specialist")
        && !sess_lower.contains("hunter")
        && !sess_lower.contains("validator")
        && !sess_lower.contains("triage")
        && !sess_lower.contains("recon")
    {
        return true;
    }

    if let Some(user) = body.get("user").and_then(|u| u.as_str()) {
        let u_lower = user.to_lowercase();
        if (u_lower.contains("root") || u_lower.contains("orchestrat"))
            && !u_lower.contains("specialist")
            && !u_lower.contains("hunter")
        {
            return true;
        }
    }

    // 3. Inspect messages for orchestrator signatures vs specialist signatures
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        // 3a. Check actual tool_calls first: any session invoking coordinator tools is an orchestrator;
        // any session invoking agent_finish is strictly a sub-agent.
        for msg in messages {
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                            if matches!(
                                name,
                                "wait_for_agents"
                                    | "create_agent"
                                    | "assign_task"
                                    | "dispatch_agent"
                                    | "spawn_agent"
                                    | "finish_scan"
                                    | "send_message_to_agent"
                            ) {
                                return true;
                            }
                            if name == "agent_finish" {
                                return false;
                            }
                        }
                    }
                }
            }
        }

        // 3b. Inspect system and initial user prompts
        for msg in messages.iter().take(2) {
            if let Some(role) = msg.get("role").and_then(|r| r.as_str()) {
                let joined;
                let content_opt = if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
                    Some(c)
                } else if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
                    let mut buf = String::new();
                    for part in parts {
                        if let Some(t) = part.get("text").and_then(|t| t.as_str()).or_else(|| part.as_str()) {
                            if !buf.is_empty() {
                                buf.push('\n');
                            }
                            buf.push_str(t);
                        }
                    }
                    joined = buf;
                    if joined.is_empty() { None } else { Some(joined.as_str()) }
                } else {
                    None
                };
                if let Some(content) = content_opt {
                    let c_lower = content.to_lowercase();
                    // If this prompt defines a specialist subagent, do NOT mark as root orchestrator
                    if (c_lower.contains("current_agent=auth")
                        || c_lower.contains("current_agent=sql")
                        || c_lower.contains("current_agent=ssrf")
                        || c_lower.contains("current_agent=tenant")
                        || c_lower.contains("current_agent=scraper")
                        || c_lower.contains("current_agent=static")
                        || c_lower.contains("current_agent=live")
                        || c_lower.contains("you are a specialist")
                        || c_lower.contains("you are an automated security hunter")
                        || c_lower.contains("sub-agent")
                        || c_lower.contains("subagent"))
                        && !c_lower.contains("mandatory for root agent")
                        && !c_lower.starts_with("you are the root agent")
                    {
                        return false;
                    }
                    if role == "system"
                        && (c_lower.starts_with("you are the root agent")
                            || c_lower.starts_with("you are a root agent")
                            || c_lower.contains("current_agent=root agent")
                            || c_lower.contains("role: root_agent")
                            || c_lower.contains("coordination/root_agent")
                            || c_lower.contains("root=true"))
                    {
                        return true;
                    }
                    if c_lower.contains("as the root agent")
                        || c_lower.contains("mandatory for root agent")
                    {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn extract_message_text(msg: &Value) -> String {
    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
        return content.to_string();
    }
    if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
        let mut text = String::new();
        for part in parts {
            if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                text.push_str(t);
                text.push('\n');
            }
        }
        return text;
    }
    String::new()
}

pub fn get_last_tool_name(messages: &[Value]) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    if let Some(name) = messages.last().and_then(|m| m.get("name")).and_then(|n| n.as_str()) {
        return Some(name.to_lowercase());
    }
    for msg in messages.iter().rev().skip(1) {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                if let Some(last_tc) = tool_calls.last() {
                    if let Some(func) = last_tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                            return Some(name.to_lowercase());
                        }
                    }
                }
            }
            break;
        }
    }
    None
}

pub fn get_last_tool_args(messages: &[Value]) -> Option<String> {
    for msg in messages.iter().rev().skip(1) {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                if let Some(last_tc) = tool_calls.last() {
                    if let Some(func) = last_tc.get("function") {
                        if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                            return Some(args.to_string());
                        }
                    }
                }
            }
            break;
        }
    }
    None
}

pub fn is_passive_read_tool_call(messages: &[Value], tool_name: &str) -> bool {
    if matches!(
        tool_name,
        "read_file"
            | "cat"
            | "view"
            | "read_resource"
            | "list_dir"
            | "list_resources"
            | "search_marketplace"
            | "search_web"
            | "grep"
            | "find_by_name"
            | "grep_search"
    ) {
        return true;
    }

    if matches!(
        tool_name,
        "execute_bash" | "bash" | "run_command" | "shell" | "terminal"
    ) {
        if let Some(args_str) = get_last_tool_args(messages) {
            let cmd_opt = serde_json::from_str::<Value>(&args_str)
                .ok()
                .and_then(|v| {
                    v.get("command")
                        .or_else(|| v.get("cmd"))
                        .or_else(|| v.get("CommandLine"))
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or(args_str);

            let mut cmd_lower = cmd_opt.trim().to_lowercase();
            // Strip leading `cd ... &&`
            if let Some(idx) = cmd_lower.find("&&") {
                if cmd_lower[..idx].trim().starts_with("cd ") {
                    cmd_lower = cmd_lower[idx + 2..].trim().to_string();
                }
            }

            let is_passive_cmd = cmd_lower.starts_with("cat ")
                || cmd_lower.starts_with("head ")
                || cmd_lower.starts_with("tail ")
                || cmd_lower.starts_with("sed ")
                || cmd_lower.starts_with("nl ")
                || cmd_lower.starts_with("grep ")
                || cmd_lower.starts_with("rg ")
                || cmd_lower.starts_with("find ")
                || cmd_lower.starts_with("ls ")
                || cmd_lower.starts_with("wc ")
                || cmd_lower.starts_with("stat ")
                || cmd_lower.starts_with("file ")
                || cmd_lower.starts_with("git status")
                || cmd_lower.starts_with("git log")
                || cmd_lower.starts_with("git diff")
                || cmd_lower.starts_with("git show");

            let has_active_exec = cmd_lower.contains("curl ")
                || cmd_lower.contains("python")
                || cmd_lower.contains("node ")
                || cmd_lower.contains("npm ")
                || cmd_lower.contains("cargo ")
                || cmd_lower.contains("pytest");

            if is_passive_cmd && !has_active_exec {
                return true;
            }
        }
    }

    false
}

pub fn is_mechanical_user_command(lower: &str) -> bool {
    let trimmed = lower.trim();
    // Git operations
    if trimmed.starts_with("git ")
        || trimmed.contains("git status")
        || trimmed.contains("git diff")
        || trimmed.contains("git log")
        || trimmed.contains("git add")
        || trimmed.contains("git commit")
        || trimmed.contains("git push")
        || trimmed.contains("git pull")
        || trimmed.contains("git branch")
        || trimmed.contains("git checkout")
    {
        return true;
    }
    // Direct shell commands
    if trimmed.starts_with("run ")
        || trimmed.starts_with("execute ")
        || trimmed.starts_with("exec ")
        || trimmed.starts_with("stop ")
        || trimmed.starts_with("start ")
        || trimmed.starts_with("restart ")
        || trimmed.starts_with("kill ")
        || trimmed.starts_with("cat ")
        || trimmed.starts_with("ls ")
        || trimmed.starts_with("curl ")
        || trimmed.starts_with("grep ")
        || trimmed.starts_with("find ")
        || trimmed.starts_with("docker ")
        || trimmed.starts_with("cargo ")
        || trimmed.starts_with("npm ")
        || trimmed.starts_with("pnpm ")
        || trimmed.starts_with("python ")
        || trimmed.starts_with("python3 ")
        || trimmed.starts_with("node ")
        || trimmed.starts_with("npx ")
    {
        return true;
    }
    // Routine inspection questions
    if trimmed.starts_with("show me ") || trimmed.starts_with("check ") || trimmed.starts_with("list ") {
        if trimmed.contains("status")
            || trimmed.contains("diff")
            || trimmed.contains("files")
            || trimmed.contains("logs")
            || trimmed.contains("dir")
            || trimmed.contains("container")
        {
            return true;
        }
    }
    false
}

pub fn is_source_code_or_comment_line(line_lower: &str) -> bool {
    let mut l = line_lower.trim_start();

    // Strip optional line-number or file:line: prefixes from `nl`, `cat -n`, or `grep -n`
    if let Some(colon_idx) = l.find(':') {
        let prefix = &l[..colon_idx];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            l = l[colon_idx + 1..].trim_start();
        } else if (prefix.ends_with(".ts")
            || prefix.ends_with(".tsx")
            || prefix.ends_with(".js")
            || prefix.ends_with(".jsx")
            || prefix.ends_with(".py")
            || prefix.ends_with(".rs")
            || prefix.ends_with(".go"))
            && !prefix.contains(' ')
        {
            let after_file = &l[colon_idx + 1..];
            if let Some(second_colon) = after_file.find(':') {
                if !after_file[..second_colon].is_empty()
                    && after_file[..second_colon].chars().all(|c| c.is_ascii_digit())
                {
                    l = after_file[second_colon + 1..].trim_start();
                } else {
                    l = after_file.trim_start();
                }
            } else {
                l = after_file.trim_start();
            }
        }
    } else if let Some(tab_idx) = l.find('\t') {
        let prefix = l[..tab_idx].trim();
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            l = l[tab_idx + 1..].trim_start();
        }
    }

    if l.starts_with("//")
        || l.starts_with("/*")
        || l.starts_with("* ")
        || l.starts_with("import ")
        || l.starts_with("export ")
        || l.starts_with("return ")
        || l.starts_with("const ")
        || l.starts_with("let ")
        || l.starts_with("var ")
        || l.starts_with("function ")
        || l.starts_with("async ")
        || l.starts_with("pub fn ")
        || l.starts_with("fn ")
        || l.starts_with("def ")
        || l.starts_with("try {")
        || l.starts_with("} catch")
        || l.starts_with("catch (")
        || l.starts_with("catch {")
    {
        return true;
    }

    if l.contains("nextresponse.json")
        || l.contains("response.json(")
        || l.contains("new response(")
        || l.contains("res.status(")
        || l.contains("res.json(")
        || l.contains("console.error(")
        || l.contains("console.warn(")
        || l.contains("logger.error(")
        || l.contains("{ status: 500")
        || l.contains("{status: 500")
        || l.contains(", status: 500")
        || l.contains("{ status: 502")
        || l.contains("{ status: 503")
        || l.contains("{ error:")
        || l.contains("{error:")
        || l.contains(", error:")
        || l.contains("error: any")
        || l.contains("error: unknown")
        || l.contains("error: string")
        || l.contains("error?:")
        || l.contains("throw new error(")
    {
        return true;
    }

    false
}

pub fn has_real_diagnostic_signal(text: &str) -> Option<&'static str> {
    for raw_line in text.lines() {
        let line_lower = raw_line.to_lowercase();
        if is_source_code_or_comment_line(&line_lower) {
            continue;
        }

        // Check each high-confidence signal on non-source-code lines
        for &signal in HARD_LOGIC_DIAGNOSTIC_SIGNALS {
            if line_lower.contains(signal) {
                return Some(signal);
            }
        }

        // Check for error markers while filtering false alarms like "error": null, 0 errors, error: none
        if let Some(pos) = line_lower.find("error:") {
            let prefix = line_lower[..pos].trim_end();
            let is_object_key = prefix.ends_with('{') || prefix.ends_with(',');
            let tail = line_lower[pos + 6..].trim_start();
            if !is_object_key
                && !tail.starts_with("null")
                && !tail.starts_with("\"\"")
                && !tail.starts_with("''")
                && !tail.starts_with('\'')
                && !tail.starts_with("false")
                && !tail.starts_with("none")
                && !tail.starts_with('0')
                && !tail.starts_with("any")
                && !tail.starts_with("unknown")
                && !tail.starts_with("string")
            {
                return Some("error:");
            }
        }

        if let Some(pos) = line_lower.find("\"error\":") {
            let tail = line_lower[pos + 8..].trim_start();
            let tail_unquoted = tail.trim_start_matches('"');
            let is_routine_4xx = tail_unquoted.starts_with("unauthorized")
                || tail_unquoted.starts_with("forbidden")
                || tail_unquoted.starts_with("not found")
                || tail_unquoted.starts_with("invalid")
                || tail_unquoted.starts_with("missing")
                || tail_unquoted.starts_with("bad request")
                || tail_unquoted.starts_with("method not allowed")
                || tail_unquoted.starts_with("unauthenticated")
                || tail_unquoted.starts_with("no token")
                || tail_unquoted.starts_with("rate limit")
                || tail_unquoted.starts_with("too many requests");

            if !tail.starts_with("null")
                && !tail.starts_with("\"\"")
                && !tail.starts_with("false")
                && !tail.starts_with("none")
                && !tail.starts_with('0')
                && !is_routine_4xx
            {
                return Some("\"error\":");
            }
        }

        if line_lower.contains("[error]")
            || (line_lower.contains("err:") && !line_lower.contains("stderr:"))
        {
            return Some("error signal");
        }
    }

    None
}

pub fn is_hard_logic_turn(body: &Value) -> (bool, String) {
    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(m) if !m.is_empty() => m,
        _ => return (false, "no messages in request body".to_string()),
    };

    let last_msg = match messages.last() {
        Some(m) => m,
        None => return (false, "empty messages array".to_string()),
    };

    let role = last_msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let text = extract_message_text(last_msg);

    match role {
        "tool" => {
            let tool_name = get_last_tool_name(messages).unwrap_or_default();
            let is_read_only = is_passive_read_tool_call(messages, &tool_name);

            // For passive file/info read tools:
            if is_read_only {
                let trimmed = text.trim();
                let trimmed_lower = trimmed.to_lowercase();
                if trimmed_lower.starts_with("error:")
                    || trimmed_lower.starts_with("file not found")
                    || trimmed_lower.starts_with("no such file")
                    || trimmed_lower.starts_with("permission denied")
                    || trimmed_lower.starts_with("notfound")
                {
                    return (true, format!("read tool failed: '{tool_name}' returned error"));
                }

                // Contextual continuity: escalate passive file reads if the initiating
                // user prompt requested an explicit cognitive or reasoning task.
                for msg in messages.iter().rev() {
                    if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                        let user_text = extract_message_text(msg);
                        let user_lower = user_text.to_lowercase();

                        for kw in HARD_LOGIC_USER_KEYWORDS {
                            if user_lower.contains(kw) {
                                return (true, format!("info gathering for cognitive task: '{kw}'"));
                            }
                        }

                        for kw in COGNITIVE_TOOL_CONTEXT_KEYWORDS {
                            if user_lower.contains(kw) {
                                return (true, format!("info gathering for cognitive task: '{kw}'"));
                            }
                        }

                        let reasoning_verbs = [
                            "why", "diagnose", "debug", "architect", "design", "explain",
                            "solve", "analyze", "investigate", "fix the bug", "find the bug"
                        ];
                        for verb in reasoning_verbs {
                            if user_lower.starts_with(verb) || user_lower.contains(&format!(" {verb} ")) {
                                return (true, format!("info gathering for user reasoning task: '{verb}'"));
                            }
                        }
                        break;
                    }
                }

                return (false, format!("passive file/dir inspection via '{tool_name}'"));
            }

            // For active execution tools (run_command, test runners, etc.):
            if let Some(sig) = has_real_diagnostic_signal(&text) {
                return (true, format!("diagnostic signal detected in tool output: '{sig}'"));
            }

            // Check for deep stack traces (> 60 lines with stack trace markers, exempting normal git diffs)
            let lines = text.lines().count();
            if lines > 60 && (text.contains("   at ") || text.contains("File \"")) {
                return (true, format!("complex stack trace ({lines} lines) in tool output"));
            }

            (false, "mechanical tool execution".to_string())
        }
        "user" => {
            let trimmed = text.trim();
            let trimmed_lower = trimmed.to_lowercase();

            // 1. Procedural affirmations stay local
            let procedural_words = [
                "yes", "no", "y", "n", "ok", "okay", "proceed", "continue",
                "go ahead", "approved", "run it", "sure", "next", "done", "cancel", "thanks", "thank you"
            ];
            if procedural_words.contains(&trimmed_lower.as_str()) {
                return (false, format!("procedural confirmation: '{trimmed}'"));
            }

            // 2. Mechanical action requests -> stay local ($0)
            if is_mechanical_user_command(&trimmed_lower) {
                return (false, format!("mechanical action request: '{trimmed}'"));
            }

            // 3. Initial prompt framing escalation (initial user task specification >= 60 chars)
            let non_system_count = messages.iter().filter(|m| {
                m.get("role").and_then(|r| r.as_str()) != Some("system")
            }).count();

            if non_system_count <= 1 && trimmed.len() >= 60 {
                return (true, "initial task specification / prompt framing".to_string());
            }

            // 4. High-cognitive keywords -> escalate to Cloud
            for kw in HARD_LOGIC_USER_KEYWORDS {
                if trimmed_lower.contains(kw) {
                    return (true, format!("cognitive prompt keyword: '{kw}'"));
                }
            }

            // 5. Reasoning / architectural / debugging intent verbs -> Cloud
            let reasoning_verbs = [
                "why", "diagnose", "debug", "architect", "design", "explain",
                "solve", "analyze", "investigate", "fix the bug", "find the bug"
            ];
            for verb in reasoning_verbs {
                if trimmed_lower.starts_with(verb) || trimmed_lower.contains(&format!(" {verb} ")) {
                    return (true, format!("reasoning intent verb: '{verb}'"));
                }
            }

            // Standard mid-session prompt stays local ($0)
            (false, "procedural or standard execution turn".to_string())
        }
        _ => {
            // For assistant or other roles, inspect preceding message if available
            if messages.len() >= 2 {
                let prev_msg = &messages[messages.len() - 2];
                let prev_role = prev_msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if prev_role == "tool" {
                    let prev_text = extract_message_text(prev_msg);
                    if let Some(sig) = has_real_diagnostic_signal(&prev_text) {
                        return (true, format!("diagnostic signal in prior tool output: '{sig}'"));
                    }
                }
            }
            (false, "default mechanical turn".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_routine_read_file_stays_local() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "check package.json and show dependencies" },
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "c1", "type": "function", "function": { "name": "read_file", "arguments": "package.json" } }
                    ]
                },
                { "role": "tool", "tool_call_id": "c1", "content": "{\"name\": \"test\", \"dependencies\": {\"express\": \"^4.0\"}}" }
            ]
        });
        let (is_hard, reason) = is_hard_logic_turn(&body);
        assert!(!is_hard, "Routine read_file must stay local, got: {reason}");
    }

    #[test]
    fn test_read_file_in_cognitive_audit_escalates() {
        let body = json!({
            "messages": [
                { "role": "user", "content": "conduct a security audit of our token authentication logic and check for cwe vulnerabilities" },
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "c_aud", "type": "function", "function": { "name": "read_file", "arguments": "src/auth.rs" } }
                    ]
                },
                { "role": "tool", "tool_call_id": "c_aud", "content": "pub fn verify_token() { /* token logic */ }" }
            ]
        });
        let (is_hard, reason) = is_hard_logic_turn(&body);
        assert!(is_hard, "Cognitive audit read_file must escalate to Cloud, got: {reason}");
        assert!(reason.contains("cognitive task"));
    }

    #[test]
    fn test_compiler_panic_escalates() {
        let body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "c_test", "type": "function", "function": { "name": "run_command", "arguments": "cargo test" } }
                    ]
                },
                {
                    "role": "tool",
                    "tool_call_id": "c_test",
                    "content": "running 1 test\ntest test_auth ... FAILED\nthread 'main' panicked at 'assertion failed: 2 == 3'"
                }
            ]
        });
        let (is_hard, reason) = is_hard_logic_turn(&body);
        assert!(is_hard, "Compiler panic must escalate, got: {reason}");
    }

    #[test]
    fn test_procedural_affirmations_stay_local() {
        for word in &["yes", "ok", "proceed", "continue", "go ahead", "approved", "run it"] {
            let body = json!({
                "messages": [
                    { "role": "assistant", "content": "Should I proceed with the build?" },
                    { "role": "user", "content": word }
                ]
            });
            let (is_hard, reason) = is_hard_logic_turn(&body);
            assert!(!is_hard, "Affirmation '{word}' must stay local, got: {reason}");
        }
    }
}
