use crate::config::CondenserConfig;
use serde_json::Value;
use tracing::debug;

pub struct ContextCondenser {
    config: CondenserConfig,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CondensationStats {
    pub reasoning_blocks_dropped: usize,
    pub tool_outputs_truncated: usize,
    pub chars_saved: usize,
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

        let cutoff_index = if assistant_indices.len() > self.config.keep_recent_turns {
            assistant_indices[assistant_indices.len() - self.config.keep_recent_turns]
        } else {
            messages.len()
        };

        let mut stats = CondensationStats::default();

        // 2. Iterate through all messages
        for idx in 0..messages.len() {
            // Never touch system prompts or root user prompt at index 0
            if idx == 0 {
                continue;
            }

            let is_historical = idx < cutoff_index;
            let msg = &mut messages[idx];
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default().to_string();
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default().to_string();

            // A) Handle standalone reasoning message items
            if is_historical && self.config.drop_historical_reasoning && msg_type == "reasoning" {
                if let Some(obj) = msg.as_object_mut() {
                    let prev_len = serde_json::to_string(obj).map(|s| s.len()).unwrap_or(0);
                    if let Some(c) = obj.get_mut("content") {
                        *c = Value::String(String::new());
                    }
                    if let Some(s) = obj.get_mut("summary") {
                        *s = Value::Array(Vec::new());
                    }
                    let new_len = serde_json::to_string(obj).map(|s| s.len()).unwrap_or(0);
                    if prev_len > new_len {
                        stats.chars_saved += prev_len - new_len;
                    }
                    stats.reasoning_blocks_dropped += 1;
                }
                continue;
            }

            // B) Handle assistant messages with reasoning fields or <think> tags
            if is_historical && self.config.drop_historical_reasoning && (role == "assistant" || msg_type == "message") {
                if let Some(obj) = msg.as_object_mut() {
                    if let Some(v) = obj.remove("reasoning") {
                        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0);
                        stats.chars_saved += len;
                        stats.reasoning_blocks_dropped += 1;
                    }
                    if let Some(v) = obj.remove("reasoning_content") {
                        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0);
                        stats.chars_saved += len;
                        stats.reasoning_blocks_dropped += 1;
                    }
                    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                        if let Some(stripped) = strip_think_tags(content) {
                            stats.chars_saved += content.len().saturating_sub(stripped.len());
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
                    let mut processed = self.clean_tool_text(&initial_text);

                    // Truncation logic
                    if self.config.truncate_tool_outputs {
                        if is_historical {
                            // Aggressive truncation for older turns
                            if processed.len() > self.config.max_tool_chars {
                                processed = truncate_tool_content(
                                    &processed,
                                    self.config.head_lines,
                                    self.config.tail_lines,
                                );
                                stats.tool_outputs_truncated += 1;
                            }
                        } else {
                            // Sanity cap for recent turns (prevents rogue 50k dumps)
                            if processed.len() > self.config.recent_max_tool_chars {
                                processed = truncate_tool_content(&processed, 20, 10);
                                stats.tool_outputs_truncated += 1;
                            }
                        }
                    }

                    let saved = initial_text.len().saturating_sub(processed.len());
                    if saved > 0 {
                        stats.chars_saved += saved;
                        if let Some(obj) = msg.as_object_mut() {
                            obj.insert(target_key.to_string(), Value::String(processed));
                        }
                    }
                }
            }
        }

        if stats.chars_saved > 0 {
            debug!(
                reasoning_dropped = stats.reasoning_blocks_dropped,
                tools_truncated = stats.tool_outputs_truncated,
                chars_saved = stats.chars_saved,
                "Context condensed successfully"
            );
        }

        stats
    }

    /// Applies RTK-inspired inline text cleaners (ANSI strip, progress bar removal,
    /// line deduplication, whitespace collapsing, JSON minification).
    pub fn clean_tool_text(&self, input: &str) -> String {
        let mut text = input.to_string();

        // 1. Strip ANSI escape sequences
        if self.config.strip_ansi_escapes && text.contains('\x1b') {
            text = strip_ansi_codes(&text);
        }

        // 2. Strip carriage-return progress overwrites and progress bars
        if self.config.strip_progress_bars && (text.contains('\r') || text.contains('%') || text.contains('█')) {
            text = strip_progress_artifacts(&text);
        }

        // 3. Deduplicate consecutive identical lines (RTK signature feature)
        if self.config.deduplicate_lines && text.contains('\n') {
            text = deduplicate_lines(&text);
        }

        // 4. Collapse runs of 3+ newlines and strip trailing whitespace
        if self.config.collapse_whitespace {
            text = collapse_excess_whitespace(&text);
        }

        // 5. Minify pretty-printed JSON outputs
        if self.config.minify_json_outputs {
            let trimmed = text.trim();
            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            {
                if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                    if let Ok(minified) = serde_json::to_string(&val) {
                        // Only replace if it saves significant space (> 15% reduction)
                        if minified.len() + 20 < trimmed.len() {
                            text = minified;
                        }
                    }
                }
            }
        }

        text
    }
}

/// Strips inline `<think>...</think>` tags
pub fn strip_think_tags(text: &str) -> Option<String> {
    let open_tag = "<think>";
    let close_tag = "</think>";

    let start = text.find(open_tag)?;
    let end = text.find(close_tag)?;

    if end <= start {
        return None;
    }

    let before = &text[..start];
    let after = &text[end + close_tag.len()..];
    let combined = format!("{}{}", before.trim_end(), after);
    Some(combined.trim().to_string())
}

/// Truncates long tool content keeping head and tail lines with omission indicator
pub fn truncate_tool_content(text: &str, head_lines: usize, tail_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head_lines + tail_lines {
        // Line count is small, do byte/char boundary truncation
        let head_chars = 300.min(text.len() / 4);
        let tail_chars = 300.min(text.len() / 4);
        let dropped = text.len().saturating_sub(head_chars + tail_chars);
        return format!(
            "{}\n\n[... Ferry Proxy: truncated {} characters of earlier tool output ...]\n\n{}",
            &text[..head_chars],
            dropped,
            &text[text.len() - tail_chars..]
        );
    }

    let head_part = lines[..head_lines].join("\n");
    let tail_part = lines[lines.len() - tail_lines..].join("\n");
    let dropped_lines = lines.len() - head_lines - tail_lines;
    let dropped_chars = text.len().saturating_sub(head_part.len() + tail_part.len());

    format!(
        "{}\n\n[... Ferry Proxy: truncated {} lines ({} chars) of earlier tool output ...]\n\n{}",
        head_part, dropped_lines, dropped_chars, tail_part
    )
}

/// Strips ANSI terminal escape codes (e.g. \x1b[32m or \x1b[0m) without external regex crate
pub fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
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

    out
}

/// Resolves `\r` carriage returns (keeps final line overwrite) and removes progress bars
pub fn strip_progress_artifacts(input: &str) -> String {
    let mut clean_lines = Vec::new();

    for line in input.lines() {
        // If line contains carriage return, take the last segment
        let effective_line = if line.contains('\r') {
            line.split('\r').filter(|s| !s.trim().is_empty()).last().unwrap_or("")
        } else {
            line
        };

        // Filter out pure progress bar lines like `[=====>   ] 45%` or `100% |██████████|`
        let trimmed = effective_line.trim();
        // Detect progress bars: contains [==>...] or |███...| with %, or ends with progress percentage
        let is_progress = (trimmed.contains('[') && trimmed.contains(']') && trimmed.contains('%') && (trimmed.contains('=') || trimmed.contains('>')))
            || (trimmed.contains('|') && (trimmed.contains('█') || trimmed.contains('#')) && trimmed.contains('%'))
            || (trimmed.starts_with("Downloading ") && trimmed.ends_with('%'))
            || (trimmed.starts_with("Fetch: ") && trimmed.ends_with('%'));

        if !is_progress {
            clean_lines.push(effective_line);
        }
    }

    clean_lines.join("\n")
}

/// Collapses runs of consecutive identical lines into a single line with `(repeated N times)`
pub fn deduplicate_lines(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() <= 1 {
        return input.to_string();
    }

    let mut result = Vec::new();
    let mut current_line = lines[0];
    let mut repeat_count = 1;

    for &line in &lines[1..] {
        if line == current_line && !line.trim().is_empty() {
            repeat_count += 1;
        } else {
            result.push(current_line.to_string());
            if repeat_count > 1 {
                result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
            }
            current_line = line;
            repeat_count = 1;
        }
    }

    result.push(current_line.to_string());
    if repeat_count > 1 {
        result.push(format!("    [... repeated {} times ...]", repeat_count - 1));
    }

    result.join("\n")
}

/// Collapses 3+ consecutive newlines into 2, and trims trailing whitespace per line
pub fn collapse_excess_whitespace(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut consecutive_newlines = 0;

    for line in input.lines() {
        let trimmed_line = line.trim_end();
        if trimmed_line.is_empty() {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                out.push('\n');
            }
        } else {
            consecutive_newlines = 0;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(trimmed_line);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strip_ansi_codes() {
        let colored = "\x1b[32mSuccess:\x1b[0m All tests passed\x1b[1;31m!\x1b[0m";
        assert_eq!(strip_ansi_codes(colored), "Success: All tests passed!");
    }

    #[test]
    fn test_strip_progress_artifacts() {
        let text = "Step 1: starting\nDownload: [=====>    ] 50%\nDownload: [==========] 100%\nStep 2: complete";
        let cleaned = strip_progress_artifacts(text);
        assert!(!cleaned.contains("50%"));
        assert!(!cleaned.contains("100%"));
        assert!(cleaned.contains("Step 1: starting"));
        assert!(cleaned.contains("Step 2: complete"));
    }

    #[test]
    fn test_deduplicate_consecutive_lines() {
        let text = "Connecting to host...\nPolling status: wait\nPolling status: wait\nPolling status: wait\nConnected OK!";
        let deduped = deduplicate_lines(text);
        assert!(deduped.contains("Polling status: wait"));
        assert!(deduped.contains("[... repeated 2 times ...]"));
        assert!(deduped.contains("Connected OK!"));
    }

    #[test]
    fn test_collapse_excess_whitespace() {
        let text = "Header\n\n\n\n\nMiddle\n\n\nFooter   ";
        let collapsed = collapse_excess_whitespace(text);
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
                // Turn 1 (Historical) using OpenAI Responses function_call_output
                { "type": "function_call_output", "call_id": "c1", "output": (1..=30).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n") },
                // Turn 2 (Recent)
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
