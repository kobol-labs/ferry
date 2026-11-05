use std::sync::Arc;
use serde_json::Value;

use crate::affinity::{AffinityType, SessionAffinityTracker, compute_prefix_hash};
use crate::config::Config;
use crate::stats::MetricsTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProvider {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    LocalOnly,
    LocalPreferred,
    Hybrid,
    CloudOnly,
}

impl RoutingMode {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "local-only" | "local" => Some(Self::LocalOnly),
            "local-preferred" | "local-prefer" | "prefer-local" => Some(Self::LocalPreferred),
            "hybrid" | "hybrid-smart" | "cognitive" => Some(Self::Hybrid),
            "cloud-only" | "cloud" => Some(Self::CloudOnly),
            _ => None,
        }
    }
}

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

pub struct RouteOverride {
    pub route: Option<String>,
    pub queue_timeout_ms: Option<u64>,
    pub session_id: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug)]
pub struct LocalSlotGuard {
    metrics: Option<Arc<MetricsTracker>>,
    affinity_tracker: Option<Arc<SessionAffinityTracker>>,
    slot_id: Option<usize>,
    tokens: u64,
}

impl LocalSlotGuard {
    pub fn new(
        metrics: Arc<MetricsTracker>,
        affinity_tracker: Option<Arc<SessionAffinityTracker>>,
        slot_id: Option<usize>,
        tokens: u64,
    ) -> Self {
        Self {
            metrics: Some(metrics),
            affinity_tracker,
            slot_id,
            tokens,
        }
    }
}

impl Drop for LocalSlotGuard {
    fn drop(&mut self) {
        if let (Some(ref tracker), Some(slot)) = (&self.affinity_tracker, self.slot_id) {
            tracker.release_slot(slot);
        }
        if let Some(m) = self.metrics.take() {
            m.release_local_capacity(self.tokens);
        }
    }
}

#[derive(Debug)]
pub struct RouteDecision {
    pub target: TargetProvider,
    pub est_tokens: u64,
    pub is_spillover: bool,
    pub reason: String,
    pub local_only: bool,
    pub slot_guard: Option<LocalSlotGuard>,
    pub slot_id: Option<usize>,
    pub is_affinity_hit: bool,
    pub affinity_type: AffinityType,
    pub session_id: String,
}

pub struct Router {
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
    affinity_tracker: Arc<SessionAffinityTracker>,
}

impl Router {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>) -> Self {
        let affinity_tracker = Arc::new(SessionAffinityTracker::with_idle_reclaim(
            config.affinity.enabled,
            config.local.max_slots,
            config.affinity.idle_reclaim_secs,
        ));
        Self::with_affinity(config, metrics, affinity_tracker)
    }

    pub fn with_affinity(
        config: Arc<Config>,
        metrics: Arc<MetricsTracker>,
        affinity_tracker: Arc<SessionAffinityTracker>,
    ) -> Self {
        Self {
            config,
            metrics,
            affinity_tracker,
        }
    }

    pub fn affinity_tracker(&self) -> &Arc<SessionAffinityTracker> {
        &self.affinity_tracker
    }

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
                        total_chars += params.to_string().len();
                    }
                }
            }
        }

        crate::condenser::estimate_tokens_from_chars(total_chars)
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
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
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
            if let Some(args_str) = Self::get_last_tool_args(messages) {
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
            if Self::is_source_code_or_comment_line(&line_lower) {
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
        let text = Self::extract_message_text(last_msg);

        match role {
            "tool" => {
                let tool_name = Self::get_last_tool_name(messages).unwrap_or_default();
                let is_read_only = Self::is_passive_read_tool_call(messages, &tool_name);

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

                    // Contextual continuity: only escalate passive file reads if the IMMEDIATE
                    // preceding message was an explicit interactive reasoning/debugging prompt from the user.
                    // Routine agent loops running in an audit/benchmark session must keep file inspection local ($0).
                    if messages.len() >= 2 {
                        let prev_msg = &messages[messages.len() - 2];
                        if prev_msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                            let user_text = Self::extract_message_text(prev_msg);
                            let user_lower = user_text.to_lowercase();
                            let reasoning_verbs = [
                                "why", "diagnose", "debug", "architect", "design", "explain",
                                "solve", "analyze", "investigate", "fix the bug", "find the bug"
                            ];
                            for verb in reasoning_verbs {
                                if user_lower.starts_with(verb) || user_lower.contains(&format!(" {verb} ")) {
                                    return (true, format!("info gathering for immediate user reasoning task: '{verb}'"));
                                }
                            }
                        }
                    }

                    return (false, format!("passive file/dir inspection via '{tool_name}'"));
                }

                // For active execution tools (run_command, test runners, etc.):
                if let Some(sig) = Self::has_real_diagnostic_signal(&text) {
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
                if Self::is_mechanical_user_command(&trimmed_lower) {
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
                        let prev_text = Self::extract_message_text(prev_msg);
                        if let Some(sig) = Self::has_real_diagnostic_signal(&prev_text) {
                            return (true, format!("diagnostic signal in prior tool output: '{sig}'"));
                        }
                    }
                }
                (false, "default mechanical turn".to_string())
            }
        }
    }

    pub async fn decide(&self, body: &Value, route_override: Option<&str>) -> RouteDecision {
        self.decide_for_session(body, route_override, None, "default").await
    }

    pub async fn decide_with_timeout(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
    ) -> RouteDecision {
        self.decide_for_session(body, route_override, explicit_timeout_ms, "default").await
    }

    pub async fn decide_for_session(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
    ) -> RouteDecision {
        self.decide_with_session(body, route_override, explicit_timeout_ms, session_id).await
    }

    pub async fn decide_with_session(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
    ) -> RouteDecision {
        self.decide_for_session_with_hints(body, route_override, explicit_timeout_ms, session_id, None, None).await
    }

    pub async fn decide_for_session_with_hints(
        &self,
        body: &Value,
        route_override: Option<&str>,
        explicit_timeout_ms: Option<u64>,
        session_id: &str,
        precomputed_tokens: Option<u64>,
        precomputed_hard_logic: Option<(bool, String)>,
    ) -> RouteDecision {
        let est_tokens = precomputed_tokens.unwrap_or_else(|| Self::estimate_tokens(body));
        let effective_mode = route_override
            .and_then(RoutingMode::parse)
            .or_else(|| RoutingMode::parse(&self.config.routing.default_route))
            .unwrap_or(RoutingMode::LocalPreferred);

        let local_only = effective_mode == RoutingMode::LocalOnly;
        let local_prefer = effective_mode == RoutingMode::LocalPreferred;
        let cloud_only = effective_mode == RoutingMode::CloudOnly;
        let fast_mode = matches!(route_override, Some("fast") | Some("low-latency"));

        // Extract system prompt prefix hash to enable warm KV cache reuse across subagents
        let prefix_hash = body
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|arr| compute_prefix_hash(arr))
            .unwrap_or(0);

        // 1. Force Cloud if client header or configuration requested cloud-only
        if cloud_only {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: "routing mode cloud-only".to_string(),
                local_only: false,
                slot_guard: None,
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
            };
        }

        // 2. Check if circuit breaker is open (Strata is offline / failing)
        if self.metrics.is_circuit_open() {
            let failures = self.metrics.consecutive_local_failures.load(std::sync::atomic::Ordering::Relaxed);
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: format!("circuit breaker open (Strata offline, {failures} failures)"),
                local_only,
                slot_guard: None,
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
            };
        }

        // 3. Check if the client explicitly requested the cloud model (when not local_only)
        if !local_only {
            if let Some(requested_model) = body.get("model").and_then(|m| m.as_str()) {
                if requested_model == self.config.cloud.model || requested_model == "cloud" {
                    return RouteDecision {
                        target: TargetProvider::Cloud,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("explicitly requested cloud model ({requested_model})"),
                        local_only: false,
                        slot_guard: None,
                        slot_id: None,
                        is_affinity_hit: false,
                        affinity_type: AffinityType::NewSlot,
                        session_id: session_id.to_string(),
                    };
                }
            }
        }

        // 4. Hybrid cognitive turn routing (escalate hard logic / diagnostics to Cloud; revert to Local for mechanical)
        if effective_mode == RoutingMode::Hybrid {
            let (is_hard, reason) = precomputed_hard_logic.unwrap_or_else(|| Self::is_hard_logic_turn(body));
            if is_hard {
                if self.metrics.is_cloud_rate_limited() && self.metrics.is_strata_online() {
                    tracing::warn!(
                        session = %session_id,
                        reason = %reason,
                        "Cognitive escalation flagged ({reason}), but Cloud provider is currently rate-limited (429 backoff). Retaining request on local Strata."
                    );
                } else {
                    return RouteDecision {
                        target: TargetProvider::Cloud,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("hybrid cognitive escalation: {reason}"),
                        local_only: false,
                        slot_guard: None,
                        slot_id: None,
                        is_affinity_hit: false,
                        affinity_type: AffinityType::NewSlot,
                        session_id: session_id.to_string(),
                    };
                }
            }
        }

        let max_context = self.config.local.max_context_tokens as u64;
        let max_slots = self.config.local.max_slots;
        let total_kv_budget = self.config.local.total_kv_token_budget as u64;

        // 5. Context window threshold check
        if est_tokens > max_context {
            return RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: format!("context tokens ({est_tokens}) > local max ({max_context})"),
                local_only,
                slot_guard: None,
                slot_id: None,
                is_affinity_hit: false,
                affinity_type: AffinityType::NewSlot,
                session_id: session_id.to_string(),
            };
        }

        // 6. Atomic local slot & token capacity acquisition with session & prefix affinity
        let is_orchestrator = Self::detect_orchestrator(body, session_id, route_override);
        if is_orchestrator {
            self.metrics.orchestrator_requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, is_orchestrator) {
            let is_hit = aff_type.is_hit();
            if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                self.metrics.record_affinity(is_hit);
                let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
                let aff_desc = match aff_type {
                    AffinityType::SessionHit => "session affinity hit",
                    AffinityType::PrefixHit => "prefix cache affinity hit",
                    AffinityType::NewSlot => "new slot assigned",
                };
                return RouteDecision {
                    target: TargetProvider::Local,
                    est_tokens,
                    is_spillover: false,
                    reason: format!("local slot {slot_id} acquired ({aff_desc}, {active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                    local_only,
                    slot_guard: Some(LocalSlotGuard::new(
                        self.metrics.clone(),
                        Some(self.affinity_tracker.clone()),
                        Some(slot_id),
                        est_tokens,
                    )),
                    slot_id: Some(slot_id),
                    is_affinity_hit: is_hit,
                    affinity_type: aff_type,
                    session_id: session_id.to_string(),
                };
            } else {
                self.affinity_tracker.release_slot(slot_id);
            }
        }

        // Adaptive Queueing: Determine wait limit based on client hints and token size
        let queue_timeout_ms = self.config.local.queue_timeout_ms;
        let effective_wait_ms = if let Some(ms) = explicit_timeout_ms {
            ms
        } else if fast_mode {
            500
        } else if local_only {
            60000
        } else if queue_timeout_ms == 0 {
            0
        } else if is_orchestrator {
            30000 // 30s queue wait for orchestrators: balances swarm responsiveness with local GPU utilization
        } else if local_prefer || effective_mode == RoutingMode::Hybrid {
            queue_timeout_ms
        } else if est_tokens <= max_context {
            queue_timeout_ms.clamp(5000, 120000)
        } else {
            queue_timeout_ms
        };

        if effective_wait_ms > 0 {
            let start = std::time::Instant::now();
            let wait_limit = std::time::Duration::from_millis(effective_wait_ms);
            let poll_interval = if is_orchestrator || fast_mode { 5 } else { 25 };
            let mut logged_queue_entry = false;
            let mut last_queue_heartbeat = start;
            while start.elapsed() < wait_limit {
                if !logged_queue_entry {
                    let prio_label = if is_orchestrator { "orchestrator" } else { "standard" };
                    tracing::info!(
                        session = %session_id,
                        est_tokens,
                        timeout_ms = effective_wait_ms,
                        priority = prio_label,
                        "Local slots saturated; request entered contention queue waiting for available slot"
                    );
                    logged_queue_entry = true;
                } else if last_queue_heartbeat.elapsed() >= std::time::Duration::from_secs(10) {
                    tracing::info!(
                        session = %session_id,
                        elapsed_ms = start.elapsed().as_millis(),
                        timeout_ms = effective_wait_ms,
                        "Request waiting in contention queue"
                    );
                    last_queue_heartbeat = std::time::Instant::now();
                }
                tokio::time::sleep(std::time::Duration::from_millis(poll_interval)).await;
                // Allow eviction for orchestrators immediately, or for workers after waiting 70% of timeout (capped at 15s)
                let allow_eviction = is_orchestrator || (start.elapsed().as_millis() >= (effective_wait_ms as u128 * 7 / 10).min(15000));
                if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, allow_eviction) {
                    let is_hit = aff_type.is_hit();
                    if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                        let wait_ms = start.elapsed().as_millis() as u64;
                        self.metrics.record_queue_wait(wait_ms);
                        self.metrics.record_affinity(is_hit);
                        let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
                        let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
                        let aff_desc = match aff_type {
                            AffinityType::SessionHit => "session affinity hit",
                            AffinityType::PrefixHit => "prefix cache affinity hit",
                            AffinityType::NewSlot => "new slot assigned",
                        };
                        return RouteDecision {
                            target: TargetProvider::Local,
                            est_tokens,
                            is_spillover: false,
                            reason: format!("local slot {slot_id} acquired from contention queue after {wait_ms}ms ({aff_desc}, {active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens)"),
                            local_only,
                            slot_guard: Some(LocalSlotGuard::new(
                                self.metrics.clone(),
                                Some(self.affinity_tracker.clone()),
                                Some(slot_id),
                                est_tokens,
                            )),
                            slot_id: Some(slot_id),
                            is_affinity_hit: is_hit,
                            affinity_type: aff_type,
                            session_id: session_id.to_string(),
                        };
                    } else {
                        self.affinity_tracker.release_slot(slot_id);
                    }
                }
            }
            if logged_queue_entry {
                let wait_ms = start.elapsed().as_millis() as u64;
                self.metrics.record_queue_wait(wait_ms);
            }
        }

        if is_orchestrator {
            self.metrics.orchestrator_spillover.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        if self.metrics.is_cloud_rate_limited() && self.metrics.is_strata_online() {
            tracing::warn!(
                session = %session_id,
                "Queue wait expired ({effective_wait_ms}ms), but Cloud is currently rate-limited (429 backoff). Attempting emergency slot eviction..."
            );
            if let Some((slot_id, aff_type)) = self.affinity_tracker.try_acquire_slot_opt(session_id, prefix_hash, true) {
                if self.metrics.try_acquire_local_capacity(est_tokens, max_slots, total_kv_budget) {
                    self.metrics.record_affinity(aff_type.is_hit());
                    let slot_guard = LocalSlotGuard::new(
                        self.metrics.clone(),
                        Some(self.affinity_tracker.clone()),
                        Some(slot_id),
                        est_tokens,
                    );
                    return RouteDecision {
                        target: TargetProvider::Local,
                        est_tokens,
                        is_spillover: false,
                        reason: format!("rescued from cloud 429: acquired slot {slot_id} via emergency eviction"),
                        local_only: false,
                        slot_guard: Some(slot_guard),
                        slot_id: Some(slot_id),
                        is_affinity_hit: aff_type.is_hit(),
                        affinity_type: aff_type,
                        session_id: session_id.to_string(),
                    };
                }
            }
        }

        let active = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);
        let active_tokens = self.metrics.active_local_tokens.load(std::sync::atomic::Ordering::Relaxed);
        RouteDecision {
            target: TargetProvider::Cloud,
            est_tokens,
            is_spillover: true,
            reason: format!("local capacity saturated ({active}/{max_slots} slots, {active_tokens}/{total_kv_budget} tokens) -> cloud spillover"),
            local_only,
            slot_guard: None,
            slot_id: None,
            is_affinity_hit: false,
            affinity_type: AffinityType::NewSlot,
            session_id: session_id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_routing_mode_parse() {
        assert_eq!(RoutingMode::parse("local-only"), Some(RoutingMode::LocalOnly));
        assert_eq!(RoutingMode::parse("local"), Some(RoutingMode::LocalOnly));
        assert_eq!(RoutingMode::parse("LOCAL-ONLY"), Some(RoutingMode::LocalOnly));

        assert_eq!(RoutingMode::parse("local-preferred"), Some(RoutingMode::LocalPreferred));
        assert_eq!(RoutingMode::parse("local-prefer"), Some(RoutingMode::LocalPreferred));
        assert_eq!(RoutingMode::parse("prefer-local"), Some(RoutingMode::LocalPreferred));

        assert_eq!(RoutingMode::parse("hybrid"), Some(RoutingMode::Hybrid));
        assert_eq!(RoutingMode::parse("hybrid-smart"), Some(RoutingMode::Hybrid));
        assert_eq!(RoutingMode::parse("cognitive"), Some(RoutingMode::Hybrid));

        assert_eq!(RoutingMode::parse("cloud-only"), Some(RoutingMode::CloudOnly));
        assert_eq!(RoutingMode::parse("cloud"), Some(RoutingMode::CloudOnly));

        assert_eq!(RoutingMode::parse("unknown"), None);
        assert_eq!(RoutingMode::parse("fast"), None);
    }

    #[test]
    fn test_hard_logic_diagnostic_signals_in_tool() {
        // Panic in tool output -> hard logic escalation
        let body = json!({
            "messages": [
                {"role": "user", "content": "run the integration tests"},
                {"role": "assistant", "content": "running tests"},
                {"role": "tool", "content": "thread 'main' panicked at 'panic: connection refused to 192.168.9.99:8080'"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body);
        assert!(is_hard);
        assert!(reason.contains("panic:"));

        // Status 500 error in tool output -> hard logic escalation
        let body_500 = json!({
            "messages": [
                {"role": "tool", "content": "HTTP/1.1 500 Internal Server Error\nstatus 500\nDatabase down"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_500);
        assert!(is_hard);
        assert!(reason.contains("status 500"));

        // Vulnerability alert in tool output -> hard logic escalation
        let body_cwe = json!({
            "messages": [
                {"role": "tool", "content": "Semgrep found CWE-79 reflected cross-site scripting vulnerability"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_cwe);
        assert!(is_hard);
        assert!(reason.contains("cwe-") || reason.contains("vulnerability"));

        // Clean tool output -> mechanical (local GPU)
        let body_clean = json!({
            "messages": [
                {"role": "user", "content": "check container status"},
                {"role": "assistant", "content": "running docker ps"},
                {"role": "tool", "content": "CONTAINER ID   IMAGE      COMMAND                  CREATED         STATUS\n1a2b3c4d       nginx      \"/docker-entrypoint.…\"   2 minutes ago   Up 2 minutes"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_clean);
        assert!(!is_hard);
        assert!(reason.contains("mechanical"));

        // JSON tool response with "error": null -> MUST NOT escalate (false alarm filter)
        let body_json = json!({
            "messages": [
                {"role": "tool", "content": "{\"status\": \"ok\", \"error\": null, \"data\": {\"count\": 42}}"}
            ]
        });
        let (is_hard, _) = Router::is_hard_logic_turn(&body_json);
        assert!(!is_hard, "JSON with error: null should not be classified as hard logic");

        // Passive read_file tool inspecting code that contains 'panic!' or 'error' -> MUST NOT escalate
        let body_read = json!({
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_read_1",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\": \"src/condenser.rs\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_read_1",
                    "content": "pub fn handle_error() {\n    panic!(\"something failed\");\n}"
                }
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_read);
        assert!(!is_hard, "read_file output containing code should stay local, got: {}", reason);
        assert!(reason.contains("passive file/dir inspection"));

        // In a security audit session, ambient 'audit' keywords in the initial user message must NOT force read_file to cloud
        let body_audit_read = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Perform a comprehensive security audit and vulnerability assessment on the target."
                },
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_inspect_1",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\": \"public/login.html\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_inspect_1",
                    "content": "<html><body><form id=\"login-form\">...</form></body></html>"
                }
            ]
        });
        let (is_hard_audit, reason_audit) = Router::is_hard_logic_turn(&body_audit_read);
        assert!(!is_hard_audit, "read_file in security audit must stay local ($0), got reason: {}", reason_audit);
        assert!(reason_audit.contains("passive file/dir inspection"));

        // Next.js route source code containing `NextResponse.json({ error: ... }, { status: 500 })` -> MUST NOT escalate
        let body_nextjs_src = json!({
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_bash_1",
                            "type": "function",
                            "function": {
                                "name": "execute_bash",
                                "arguments": "{\"command\": \"cat src/app/api/cards/route.ts\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_bash_1",
                    "content": "export async function POST(req: Request) {\n  try {\n    return NextResponse.json({ ok: true });\n  } catch (error: unknown) {\n    console.error('Error:', error);\n    return NextResponse.json({ error: 'Internal server error' }, { status: 500 });\n  }\n}"
                }
            ]
        });
        let (is_hard_nextjs, reason_nextjs) = Router::is_hard_logic_turn(&body_nextjs_src);
        assert!(!is_hard_nextjs, "Next.js source code inspection must stay local, got: {}", reason_nextjs);
        assert!(Router::has_real_diagnostic_signal("return NextResponse.json({ error: 'Failed' }, { status: 500 });").is_none());
        assert!(Router::has_real_diagnostic_signal("{\"error\":\"Unauthorized\"}").is_none());
        assert!(Router::has_real_diagnostic_signal("stdout: ok\nstderr: % Total % Received").is_none());
        assert_eq!(Router::has_real_diagnostic_signal("err: connection reset"), Some("error signal"));
    }

    #[test]
    fn test_hard_logic_user_prompts() {
        // Procedural confirmation -> mechanical (stays local)
        let body_ok = json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "yes"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_ok);
        assert!(!is_hard);
        assert!(reason.contains("procedural"));

        let body_proceed = json!({
            "messages": [
                {"role": "user", "content": "proceed"}
            ]
        });
        let (is_hard, _) = Router::is_hard_logic_turn(&body_proceed);
        assert!(!is_hard);

        // Mechanical user command (even if > 120 chars) -> MUST STAY LOCAL
        let body_git = json!({
            "messages": [
                {"role": "system", "content": "Assistant"},
                {"role": "user", "content": "setup"},
                {"role": "assistant", "content": "ready"},
                {"role": "user", "content": "Please run git status and git diff to see all the modified files in the repository before making any new commits to main"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_git);
        assert!(!is_hard, "Mechanical git prompt should stay local, got: {}", reason);
        assert!(reason.contains("mechanical action request"));

        // User prompt with cognitive keyword in ongoing conversation -> hard logic escalation
        let body_kw = json!({
            "messages": [
                {"role": "user", "content": "Initial setup"},
                {"role": "assistant", "content": "Ready"},
                {"role": "user", "content": "please diagnose the root cause of the memory leak in this auth module"}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_kw);
        assert!(is_hard);
        assert!(reason.contains("cognitive prompt keyword") || reason.contains("reasoning intent verb"));

        // Mid-session security specialist testing command -> stays local ($0)
        let body_specialist = json!({
            "messages": [
                {"role": "user", "content": "Initial audit task"},
                {"role": "assistant", "content": "Scanning routes"},
                {"role": "user", "content": "Check the cards API for IDOR vulnerability and test /api/cards endpoint"}
            ]
        });
        let (is_hard_spec, reason_spec) = Router::is_hard_logic_turn(&body_specialist);
        assert!(!is_hard_spec, "Mid-session specialist testing must stay local, got: {}", reason_spec);

        // Initial task prompt (> 60 chars) -> hard logic escalation
        let body_initial = json!({
            "messages": [
                {"role": "system", "content": "You are an AI coding assistant."},
                {"role": "user", "content": "Please implement cognitive task segregation and multi-tier routing in Ferry core keeping mechanical CLI commands on local hardware."}
            ]
        });
        let (is_hard, reason) = Router::is_hard_logic_turn(&body_initial);
        assert!(is_hard);
        assert!(reason.contains("initial task specification"));
    }

    #[test]
    fn test_detect_orchestrator_disambiguation() {
        // 1. Specialist subagent with "root agent" mentioned in prompt -> MUST NOT be an orchestrator
        let specialist_body = json!({
            "metadata": {
                "agent_name": "Auth & JWT Specialist",
                "role": "specialist"
            },
            "messages": [
                {
                    "role": "system",
                    "content": "You are an automated security hunter specializing in JWT analysis. Report all findings back to the Root Agent once completed."
                },
                {"role": "user", "content": "Analyze /api/login.js"}
            ]
        });
        assert!(
            !Router::detect_orchestrator(&specialist_body, "session-specialist-1", None),
            "Specialist mentioning Root Agent must NOT be tagged as orchestrator"
        );

        // 2. True Root Agent coordinator -> MUST be tagged as orchestrator
        let root_body = json!({
            "metadata": {
                "agent_name": "Root Agent",
                "role": "coordinator"
            },
            "messages": [
                {
                    "role": "system",
                    "content": "You are the Root Agent coordinating the security assessment. Dispatch specialists as needed."
                },
                {"role": "user", "content": "Begin audit"}
            ]
        });
        assert!(
            Router::detect_orchestrator(&root_body, "session-root-1", None),
            "True Root Agent coordinator must be tagged as orchestrator"
        );

        // 3. Coordinator tool call (wait_for_agents / dispatch_agent) -> orchestrator
        let dispatch_body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "wait_for_agents", "arguments": "{}"}
                        }
                    ]
                }
            ]
        });
        assert!(
            Router::detect_orchestrator(&dispatch_body, "session-generic-1", None),
            "Coordinator tool calls must be tagged as orchestrator"
        );
    }

    #[tokio::test]
    async fn test_cloud_rate_limit_backoff_keeps_turns_local() {
        let mut config = Config::default();
        config.routing.default_route = "hybrid".to_string();
        config.local.max_slots = 4;
        let config = Arc::new(config);
        let metrics = Arc::new(MetricsTracker::default());
        let router = Router::new(config, metrics.clone());

        let hard_body = json!({
            "messages": [
                {
                    "role": "user",
                    "content": "Verify if the SQL injection flaw can be bypassed using hex encoding."
                }
            ]
        });

        // 1. Without cloud rate limit, hard logic escalates to Cloud
        let dec1 = router.decide_with_session(&hard_body, None, None, "test-sess-1").await;
        assert_eq!(dec1.target, TargetProvider::Cloud);

        // 2. When Cloud is marked 429 rate limited, turn stays local on Strata
        metrics.record_cloud_rate_limit(60);
        assert!(metrics.is_cloud_rate_limited());
        let dec2 = router.decide_with_session(&hard_body, None, None, "test-sess-2").await;
        assert_eq!(dec2.target, TargetProvider::Local);
        assert!(dec2.slot_guard.is_some());
    }
}
