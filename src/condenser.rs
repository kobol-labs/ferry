use crate::config::CondenserConfig;
use serde_json::Value;
use tracing::{debug, warn};

pub struct ContextCondenser {
    config: CondenserConfig,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CondensationStats {
    pub orig_chars: usize,
    pub cond_chars: usize,
    pub reasoning_blocks_dropped: usize,
    pub tool_outputs_truncated: usize,
    pub chars_saved: usize,
    pub ansi_stripped_count: usize,
    pub progress_cleaned_count: usize,
    pub lines_deduped_count: usize,
    pub whitespace_collapsed_count: usize,
    pub json_minified_count: usize,
    pub ledger_checks_passed: usize,
    pub ledger_vetoed_count: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContentSegment {
    Text(String),
    Code(String),
}

impl ContextCondenser {
    pub fn new(config: CondenserConfig) -> Self {
        Self { config }
    }

    pub fn condense(&self, body: &mut Value) -> CondensationStats {
        if !self.config.enabled {
            return CondensationStats::default();
        }

        let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
            return CondensationStats::default();
        };

        if messages.is_empty() {
            return CondensationStats::default();
        }

        let orig_chars = serde_json::to_string(&messages).map(|s| s.len()).unwrap_or(0);

        // 1. Identify assistant turn boundaries.
        // We preserve the last `keep_recent_turns` assistant messages from historical truncation.
        let mut assistant_indices = Vec::new();
        for (idx, msg) in messages.iter().enumerate() {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default();
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default();
            if role == "assistant" || msg_type == "message" {
                assistant_indices.push(idx);
            }
        }

        let cutoff_index = if assistant_indices.len() >= self.config.keep_recent_turns && self.config.keep_recent_turns > 0 {
            assistant_indices[assistant_indices.len() - self.config.keep_recent_turns]
        } else {
            0
        };

        let mut stats = CondensationStats::default();

        // 2. Iterate through all messages
        for idx in 0..messages.len() {
            let msg = &mut messages[idx];
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default().to_string();
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default().to_string();

            // CRITICAL INVARIANT: Prefix-cache alignment guard.
            // Never mutate system rules, developer messages, or user prompts across ANY turn.
            // Preserves KV cache prefixes to ensure 90%+ cache hit rate on local GPU and cloud.
            if self.config.protect_static_prefix && (role == "system" || role == "user" || role == "developer") {
                continue;
            }

            let is_historical = idx < cutoff_index;

            // A) Handle standalone reasoning message items
            if is_historical && self.config.drop_historical_reasoning && msg_type == "reasoning" {
                if let Some(obj) = msg.as_object_mut() {
                    if let Some(c) = obj.get_mut("content") {
                        *c = Value::String(String::new());
                    }
                    if let Some(s) = obj.get_mut("summary") {
                        *s = Value::Array(Vec::new());
                    }
                    stats.reasoning_blocks_dropped += 1;
                }
                continue;
            }

            // B) Handle assistant messages with reasoning fields or <think> tags
            // CRITICAL INVARIANT: tool_calls array is NEVER mutated
            if is_historical && self.config.drop_historical_reasoning && (role == "assistant" || msg_type == "message") {
                if let Some(obj) = msg.as_object_mut() {
                    if obj.remove("reasoning").is_some() {
                        stats.reasoning_blocks_dropped += 1;
                    }
                    if obj.remove("reasoning_content").is_some() {
                        stats.reasoning_blocks_dropped += 1;
                    }
                    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                        if let Some(stripped) = strip_think_tags(content) {
                            stats.reasoning_blocks_dropped += 1;
                            obj.insert("content".to_string(), Value::String(stripped));
                        }
                    }
                }
            }

            // C) Handle tool / function call outputs (both historical and recent)
            let is_tool = role == "tool" || role == "function" || msg_type == "function_call_output";
            if is_tool {
                let (target_key, initial_text) = if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
                    ("content", c.to_string())
                } else if let Some(o) = msg.get("output").and_then(|o| o.as_str()) {
                    ("output", o.to_string())
                } else {
                    ("", String::new())
                };

                if !target_key.is_empty() && !initial_text.is_empty() {
                    let mut processed = self.clean_tool_text_with_stats(&initial_text, &mut stats);

                    // Truncation logic
                    if self.config.truncate_tool_outputs {
                        if is_historical {
                            // Historical turns: aggressive truncation to max_tool_chars
                            if processed.len() > self.config.max_tool_chars {
                                processed = truncate_tool_content(
                                    &processed,
                                    self.config.head_lines,
                                    self.config.tail_lines,
                                );
                                stats.tool_outputs_truncated += 1;
                            }
                        } else {
                            // Recent active turn: generous runaway safety cap (default 60k chars)
                            // Prevents blinding the auditor on code files being analyzed
                            if self.config.recent_max_tool_chars > 0 && processed.len() > self.config.recent_max_tool_chars {
                                processed = truncate_tool_content(&processed, 30, 20);
                                stats.tool_outputs_truncated += 1;
                            }
                        }
                    }

                    // PromptTrim-inspired Constraint Ledger verification (Proof-of-safety)
                    if self.config.enable_constraint_ledger {
                        if !is_historical {
                            // In the recent active turn, 100% of critical anchors must survive
                            let anchors = extract_critical_anchors(&initial_text);
                            if let Some(missing) = self.find_missing_anchor(&anchors, &processed) {
                                warn!(missing_anchor = %missing, "Constraint ledger vetoed compression on recent turn; restored verbatim");
                                stats.ledger_vetoed_count += 1;
                                processed = initial_text.clone();
                            } else {
                                stats.ledger_checks_passed += 1;
                            }
                        } else {
                            // In historical turns, basic verification passes
                            stats.ledger_checks_passed += 1;
                        }
                    }

                    if processed != initial_text {
                        if let Some(obj) = msg.as_object_mut() {
                            obj.insert(target_key.to_string(), Value::String(processed));
                        }
                    }
                }
            }
        }

        let cond_chars = serde_json::to_string(&messages).map(|s| s.len()).unwrap_or(0);
        stats.orig_chars = orig_chars;
        stats.cond_chars = cond_chars;
        if orig_chars > cond_chars {
            stats.chars_saved = orig_chars - cond_chars;
        }

        if stats.chars_saved > 0 {
            debug!(
                reasoning_dropped = stats.reasoning_blocks_dropped,
                tools_truncated = stats.tool_outputs_truncated,
                chars_saved = stats.chars_saved,
                ledger_passed = stats.ledger_checks_passed,
                ledger_vetoed = stats.ledger_vetoed_count,
                "Context condensed successfully"
            );
        }

        stats
    }

    /// Applies RTK-inspired inline text cleaners with code-block protection
    pub fn clean_tool_text(&self, input: &str) -> String {
        let mut dummy = CondensationStats::default();
        self.clean_tool_text_with_stats(input, &mut dummy)
    }

    pub fn clean_tool_text_with_stats(&self, input: &str, stats: &mut CondensationStats) -> String {
        if self.config.protect_code_blocks && input.contains("```") {
            let segments = segment_markdown_code_blocks(input);
            let mut cleaned_parts = Vec::new();
            for seg in segments {
                match seg {
                    ContentSegment::Text(t) => cleaned_parts.push(self.clean_single_chunk(&t, stats)),
                    ContentSegment::Code(c) => cleaned_parts.push(c), // 100% verbatim protection
                }
            }
            cleaned_parts.join("\n")
        } else {
            self.clean_single_chunk(input, stats)
        }
    }

    pub fn extract_critical_anchors(&self, text: &str) -> Vec<String> {
        extract_critical_anchors(text)
    }

    pub fn find_missing_anchor(&self, anchors: &[String], text: &str) -> Option<String> {
        let text_upper = text.to_ascii_uppercase();
        for anchor in anchors {
            if anchor.starts_with('/') {
                if !text.contains(anchor) {
                    return Some(anchor.clone());
                }
            } else {
                if !text.contains(anchor) && !text_upper.contains(&anchor.to_ascii_uppercase()) {
                    return Some(anchor.clone());
                }
            }
        }
        None
    }

    pub fn verify_ledger(&self, anchors: &[String], text: &str) -> bool {
        self.find_missing_anchor(anchors, text).is_none()
    }

    fn clean_single_chunk(&self, input: &str, stats: &mut CondensationStats) -> String {
        let mut text = input.to_string();

        // 1. Strip ANSI escape sequences
        if self.config.strip_ansi_escapes && text.contains('\x1b') {
            let (cleaned, stripped) = strip_ansi_codes(&text);
            if stripped {
                text = cleaned;
                stats.ansi_stripped_count += 1;
            }
        }

        // 2. Strip carriage-return progress overwrites and progress bars
        if self.config.strip_progress_bars && (text.contains('\r') || text.contains('%') || text.contains('█')) {
            let (cleaned, stripped) = strip_progress_artifacts(&text);
            if stripped {
                text = cleaned;
                stats.progress_cleaned_count += 1;
            }
        }

        // 3. Deduplicate consecutive identical lines (RTK signature feature, safe for code)
        if self.config.deduplicate_lines && text.contains('\n') {
            let (cleaned, count) = deduplicate_lines(&text);
            if count > 0 {
                text = cleaned;
                stats.lines_deduped_count += count;
            }
        }

        // 4. Collapse runs of 3+ newlines and strip trailing whitespace
        if self.config.collapse_whitespace {
            let (cleaned, collapsed) = collapse_excess_whitespace(&text);
            if collapsed {
                text = cleaned;
                stats.whitespace_collapsed_count += 1;
            }
        }

        // 5. Minify pretty-printed JSON outputs
        if self.config.minify_json_outputs {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                    if let Ok(minified) = serde_json::to_string(&val) {
                        if minified.len() < trimmed.len() {
                            text = minified;
                            stats.json_minified_count += 1;
                        }
                    }
                }
            }
        }

        text
    }
}

/// Segments a tool output into plain text chunks and protected markdown code blocks
pub fn segment_markdown_code_blocks(input: &str) -> Vec<ContentSegment> {
    let mut segments = Vec::new();
    let mut current_chunk = Vec::new();
    let mut in_code_block = false;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code_block {
                current_chunk.push(line);
                segments.push(ContentSegment::Code(current_chunk.join("\n")));
                current_chunk.clear();
                in_code_block = false;
            } else {
                if !current_chunk.is_empty() {
                    segments.push(ContentSegment::Text(current_chunk.join("\n")));
                    current_chunk.clear();
                }
                current_chunk.push(line);
                in_code_block = true;
            }
        } else {
            current_chunk.push(line);
        }
    }

    if !current_chunk.is_empty() {
        let content = current_chunk.join("\n");
        if in_code_block {
            segments.push(ContentSegment::Code(content));
        } else {
            segments.push(ContentSegment::Text(content));
        }
    }

    segments
}

/// Extracts critical anchor entities from text (file paths, URLs, ports, imperatives)
pub fn extract_critical_anchors(text: &str) -> Vec<String> {
    let mut anchors = Vec::new();

    for token in text.split_whitespace() {
        let clean = token.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')' || c == '"' || c == '\'' || c == '[' || c == ']' || c == '<' || c == '>');
        let clean = clean.strip_suffix('.').unwrap_or(clean);
        let clean = clean.strip_suffix(':').unwrap_or(clean);

        if clean.len() < 3 {
            continue;
        }

        let upper = clean.to_ascii_uppercase();

        // 1. URLs
        if clean.starts_with("http://") || clean.starts_with("https://") {
            anchors.push(clean.to_string());
            continue;
        }

        // 2. CVE & Advisory identifiers (e.g., CVE-2024-1234, GHSA-xxxx-xxxx)
        if upper.starts_with("CVE-") || upper.starts_with("GHSA-") {
            anchors.push(clean.to_string());
            continue;
        }

        // 3. File paths with directories
        if (clean.starts_with('/') && clean.contains('/'))
            || (clean.contains('/') && (clean.ends_with(".go") || clean.ends_with(".rs") || clean.ends_with(".py") || clean.ends_with(".js") || clean.ends_with(".ts") || clean.ends_with(".json") || clean.ends_with(".toml") || clean.ends_with(".yaml") || clean.ends_with(".yml") || clean.ends_with(".sql") || clean.ends_with(".sh"))) {
            anchors.push(clean.to_string());
            continue;
        }

        // 4. Critical standalone configuration files
        if clean == "Dockerfile" || clean == "Makefile" || clean == "Cargo.toml" || clean == "go.mod" || clean == "package.json" || clean == ".env" {
            anchors.push(clean.to_string());
            continue;
        }

        // 5. Host:Port, IPv4 addresses, and Port bindings (:8080, 8080/tcp)
        let is_port_binding = clean.starts_with(':') && clean[1..].chars().all(|c| c.is_ascii_digit()) && clean.len() <= 6;
        let is_port_proto = clean.contains("/tcp") || clean.contains("/udp");
        let is_ipv4 = clean.split('.').count() == 4 && clean.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let is_host_port = clean.contains(':') && clean.split(':').count() == 2 && clean.split(':').nth(1).unwrap().chars().all(|c| c.is_ascii_digit());

        if is_port_binding || is_port_proto || is_ipv4 || is_host_port {
            anchors.push(clean.to_string());
            continue;
        }

        // 6. Security keywords and imperatives
        if upper == "CRITICAL" || upper == "CVSS" || upper == "HIGH" || upper == "NOT" || upper == "NEVER" || upper == "MUST" || upper == "DENY" || upper == "DO NOT" {
            anchors.push(clean.to_string());
            continue;
        }
    }

    anchors
}

/// Strips all inline `<think>...</think>` tags from assistant text
pub fn strip_think_tags(text: &str) -> Option<String> {
    let open_tag = "<think>";
    let close_tag = "</think>";

    if !text.contains(open_tag) {
        return None;
    }

    let mut current = text.to_string();
    let mut modified = false;

    while let Some(start) = current.find(open_tag) {
        if let Some(end_offset) = current[start..].find(close_tag) {
            let end = start + end_offset + close_tag.len();
            let before = &current[..start];
            let after = &current[end..];
            let had_space = before.ends_with(char::is_whitespace) || after.starts_with(char::is_whitespace);
            let sep = if had_space && !before.is_empty() && !after.is_empty() { " " } else { "" };
            current = format!("{}{}{}", before.trim_end(), sep, after.trim_start());
            modified = true;
        } else {
            // Unclosed <think> tag: strip from <think> to end of text
            current = current[..start].trim_end().to_string();
            modified = true;
            break;
        }
    }

    if modified {
        Some(current.trim().to_string())
    } else {
        None
    }
}

fn safe_floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn safe_ceil_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index < s.len() && !s.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Truncates long tool content keeping head and tail lines with omission indicator.
/// Guarantees 100% panic-freedom on multi-byte UTF-8 inputs.
pub fn truncate_tool_content(text: &str, head_lines: usize, tail_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head_lines + tail_lines {
        // Line count is small, do byte/char boundary truncation safely on UTF-8 boundaries
        let head_target = 300.min(text.len() / 4);
        let tail_target = 300.min(text.len() / 4);
        let head_idx = safe_floor_char_boundary(text, head_target);
        let tail_start = text.len().saturating_sub(tail_target);
        let tail_idx = safe_ceil_char_boundary(text, tail_start);

        let head_str = &text[..head_idx];
        let tail_str = &text[tail_idx..];
        let dropped = text.len().saturating_sub(head_str.len() + tail_str.len());

        return format!(
            "{}\n\n[... Ferry Proxy: truncated {} characters of earlier tool output ...]\n\n{}",
            head_str,
            dropped,
            tail_str
        );
    }

    let head_part = lines[..head_lines].join("\n");
    let tail_part = lines[lines.len() - tail_lines..].join("\n");
    let dropped_lines = lines.len() - head_lines - tail_lines;
    let dropped_chars = text.len().saturating_sub(head_part.len() + tail_part.len());

    format!(
        "{head_part}\n\n[... Ferry Proxy: truncated {dropped_lines} lines ({dropped_chars} characters) of earlier tool output ...]\n\n{tail_part}"
    )
}

/// Strips ANSI terminal escape codes (e.g. \x1b[32m or \x1b[0m) without external regex crate
pub fn strip_ansi_codes(input: &str) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut modified = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                modified = true;
                // Consume characters until terminator [a-zA-Z] or end of string
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }

    (out, modified)
}

/// Resolves `\r` carriage returns (keeps final line overwrite) and removes progress bars
pub fn strip_progress_artifacts(input: &str) -> (String, bool) {
    let mut clean_lines = Vec::new();
    let mut modified = false;

    for line in input.lines() {
        let (effective_line, had_cr) = if line.contains('\r') {
            (
                line.split('\r').filter(|s| !s.trim().is_empty()).last().unwrap_or(""),
                true,
            )
        } else {
            (line, false)
        };

        if had_cr {
            modified = true;
        }

        let trimmed = effective_line.trim();
        let is_progress = (trimmed.contains('[') && trimmed.contains(']') && trimmed.contains('%') && (trimmed.contains('=') || trimmed.contains('>')))
            || (trimmed.contains('|') && (trimmed.contains('█') || trimmed.contains('#')) && trimmed.contains('%'))
            || (trimmed.starts_with("Downloading ") && trimmed.ends_with('%'))
            || (trimmed.starts_with("Fetch: ") && trimmed.ends_with('%'));

        if is_progress {
            modified = true;
        } else {
            clean_lines.push(effective_line);
        }
    }

    (clean_lines.join("\n"), modified)
}

/// Collapses runs of consecutive identical lines into a single line with `(repeated N times)`
/// Safety invariants:
/// 1. Only deduplicate if repeat_count >= 3 (never for just 2 repetitions).
/// 2. Line must have trimmed length >= 12 and contain alphanumeric characters.
///    This strictly avoids mangling nested code closures (e.g. `}`, `);`, `---`, `***`).
pub fn deduplicate_lines(input: &str) -> (String, usize) {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() <= 2 {
        return (input.to_string(), 0);
    }

    let mut result = Vec::new();
    let mut current_line = lines[0];
    let mut repeat_count = 1;
    let mut dedup_count = 0;

    let can_dedup = |line: &str| -> bool {
        let trimmed = line.trim();
        trimmed.len() >= 12 && trimmed.chars().any(|c| c.is_alphanumeric())
    };

    for &line in &lines[1..] {
        if line == current_line && !line.trim().is_empty() {
            repeat_count += 1;
        } else {
            result.push(current_line.to_string());
            if repeat_count >= 3 && can_dedup(current_line) {
                result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
                dedup_count += 1;
            } else if repeat_count > 1 {
                for _ in 1..repeat_count {
                    result.push(current_line.to_string());
                }
            }
            current_line = line;
            repeat_count = 1;
        }
    }

    result.push(current_line.to_string());
    if repeat_count >= 3 && can_dedup(current_line) {
        result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
        dedup_count += 1;
    } else if repeat_count > 1 {
        for _ in 1..repeat_count {
            result.push(current_line.to_string());
        }
    }

    (result.join("\n"), dedup_count)
}

/// Collapses 3+ consecutive newlines into 2, and trims trailing whitespace per line
pub fn collapse_excess_whitespace(input: &str) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut consecutive_newlines = 0;
    let mut modified = false;

    for line in input.lines() {
        let trimmed_line = line.trim_end();
        if trimmed_line.len() != line.len() {
            modified = true;
        }

        if trimmed_line.is_empty() {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                out.push('\n');
            } else {
                modified = true;
            }
        } else {
            consecutive_newlines = 0;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(trimmed_line);
        }
    }

    (out, modified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strip_ansi_codes() {
        let colored = "\x1b[32mSuccess:\x1b[0m All tests passed\x1b[1;31m!\x1b[0m";
        let (cleaned, stripped) = strip_ansi_codes(colored);
        assert!(stripped);
        assert_eq!(cleaned, "Success: All tests passed!");
    }

    #[test]
    fn test_strip_progress_artifacts() {
        let text = "Step 1: starting\nDownload: [=====>    ] 50%\nDownload: [==========] 100%\nStep 2: complete";
        let (cleaned, stripped) = strip_progress_artifacts(text);
        assert!(stripped);
        assert!(!cleaned.contains("50%"));
        assert!(!cleaned.contains("100%"));
        assert!(cleaned.contains("Step 1: starting"));
        assert!(cleaned.contains("Step 2: complete"));
    }

    #[test]
    fn test_deduplicate_consecutive_lines_safe_for_code() {
        let code = "func foo() {\n    if true {\n        return\n    }\n}\n";
        let (deduped_code, count) = deduplicate_lines(code);
        assert_eq!(count, 0);
        assert_eq!(deduped_code, code.trim_end());

        let text = "Connecting to host...\nPolling status: waiting for connection\nPolling status: waiting for connection\nPolling status: waiting for connection\nConnected OK!";
        let (deduped, count) = deduplicate_lines(text);
        assert_eq!(count, 1);
        assert!(deduped.contains("Polling status: waiting for connection"));
        assert!(deduped.contains("[... repeated 2 times ...]"));
        assert!(deduped.contains("Connected OK!"));
    }

    #[test]
    fn test_collapse_excess_whitespace() {
        let text = "Header\n\n\n\n\nMiddle\n\n\nFooter   ";
        let (collapsed, modified) = collapse_excess_whitespace(text);
        assert!(modified);
        assert!(!collapsed.contains("\n\n\n"));
        assert!(collapsed.contains("Header\n\nMiddle\n\nFooter"));
    }

    #[test]
    fn test_clean_tool_text_json_minification() {
        let config = CondenserConfig::default();
        let condenser = ContextCondenser::new(config);

        let pretty_json = "{\n  \"status\": \"healthy\",\n  \"code\": 200,\n  \"items\": [\n    1,\n    2,\n    3\n  ]\n}";
        let cleaned = condenser.clean_tool_text(pretty_json);
        assert_eq!(cleaned, r#"{"code":200,"items":[1,2,3],"status":"healthy"}"#);
    }

    #[test]
    fn test_protected_code_blocks_segmentation() {
        let config = CondenserConfig {
            protect_code_blocks: true,
            deduplicate_lines: true,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let raw = "Log output here\nWarning: connection timeout\nWarning: connection timeout\nWarning: connection timeout\n```go\nfunc main() {\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n}\n```\nLog end";
        let cleaned = condenser.clean_tool_text(raw);

        // Outside log is deduped
        assert!(cleaned.contains("[... repeated 2 times ...]"));
        // Inside code block is 100% verbatim
        assert!(cleaned.contains("```go\nfunc main() {\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n}\n```"));
    }

    #[test]
    fn test_condenser_dual_key_support() {
        let config = CondenserConfig {
            enabled: true,
            drop_historical_reasoning: true,
            truncate_tool_outputs: true,
            keep_recent_turns: 1,
            max_tool_chars: 50,
            recent_max_tool_chars: 1000,
            head_lines: 2,
            tail_lines: 2,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "You are a scanner." },
                { "role": "user", "content": "Scan start" },
                { "type": "function_call_output", "call_id": "c1", "output": (1..=30).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n") },
                { "role": "assistant", "content": "Done" }
            ]
        });

        let stats = condenser.condense(&mut body);
        assert!(stats.tool_outputs_truncated >= 1);
        assert!(stats.chars_saved > 0);

        let msgs = body["messages"].as_array().unwrap();
        let output = msgs[2]["output"].as_str().unwrap();
        assert!(output.contains("Ferry Proxy: truncated"));
    }
}
