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
        // We preserve the last `keep_recent_turns` assistant messages and anything following them.
        let mut assistant_indices = Vec::new();
        for (idx, msg) in messages.iter().enumerate() {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default();
            if role == "assistant" {
                assistant_indices.push(idx);
            }
        }

        let cutoff_index = if assistant_indices.len() > self.config.keep_recent_turns {
            // Cutoff is the start of the recent window
            assistant_indices[assistant_indices.len() - self.config.keep_recent_turns]
        } else {
            // Everything falls inside the recent window; no historical turns to condense
            messages.len()
        };

        if cutoff_index == messages.len() {
            return CondensationStats::default();
        }

        let mut stats = CondensationStats::default();

        // 2. Iterate through historical messages (0 .. cutoff_index)
        for idx in 0..cutoff_index {
            // Never touch system prompts or the root user instruction at index 0
            if idx == 0 {
                continue;
            }

            let msg = &mut messages[idx];
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default().to_string();
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default().to_string();

            // A) Handle standalone reasoning message items (e.g. {"type": "reasoning", ...})
            if self.config.drop_historical_reasoning && msg_type == "reasoning" {
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
            if self.config.drop_historical_reasoning && role == "assistant" {
                if let Some(obj) = msg.as_object_mut() {
                    // Check standard `reasoning` field
                    if let Some(v) = obj.remove("reasoning") {
                        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0);
                        stats.chars_saved += len;
                        stats.reasoning_blocks_dropped += 1;
                    }
                    // Check `reasoning_content` field (used by DeepSeek / Qwen models)
                    if let Some(v) = obj.remove("reasoning_content") {
                        let len = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0);
                        stats.chars_saved += len;
                        stats.reasoning_blocks_dropped += 1;
                    }
                    // Check inline `<think>...</think>` tags inside `content`
                    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                        if let Some(stripped) = strip_think_tags(content) {
                            stats.chars_saved += content.len().saturating_sub(stripped.len());
                            stats.reasoning_blocks_dropped += 1;
                            obj.insert("content".to_string(), Value::String(stripped));
                        }
                    }
                }
            }

            // C) Handle tool / function call outputs
            if self.config.truncate_tool_outputs && (role == "tool" || role == "function") {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    if content.len() > self.config.max_tool_chars {
                        let truncated = truncate_tool_content(
                            content,
                            self.config.head_lines,
                            self.config.tail_lines,
                        );
                        let saved = content.len().saturating_sub(truncated.len());
                        if saved > 0 {
                            stats.chars_saved += saved;
                            stats.tool_outputs_truncated += 1;
                            if let Some(obj) = msg.as_object_mut() {
                                obj.insert("content".to_string(), Value::String(truncated));
                            }
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
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_strip_think_tags() {
        let text = "<think>Let me reason about this...</think>The answer is 42.";
        assert_eq!(strip_think_tags(text), Some("The answer is 42.".to_string()));

        let text_no_tag = "Just normal response";
        assert_eq!(strip_think_tags(text_no_tag), None);
    }

    #[test]
    fn test_truncate_tool_content() {
        let lines: Vec<String> = (1..=100).map(|i| format!("Line {}", i)).collect();
        let long_content = lines.join("\n");
        let truncated = truncate_tool_content(&long_content, 5, 5);

        assert!(truncated.contains("Line 1"));
        assert!(truncated.contains("Line 5"));
        assert!(truncated.contains("Line 100"));
        assert!(truncated.contains("Ferry Proxy: truncated 90 lines"));
    }

    #[test]
    fn test_condenser_preserves_recent_turns() {
        let config = CondenserConfig {
            enabled: true,
            drop_historical_reasoning: true,
            truncate_tool_outputs: true,
            keep_recent_turns: 1,
            max_tool_chars: 100,
            head_lines: 2,
            tail_lines: 2,
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "You are a pentester." },
                { "role": "user", "content": "Scan the target." },
                // Turn 1 (Historical)
                { "role": "assistant", "reasoning": "Historical thoughts", "content": "Checking files" },
                { "role": "tool", "content": (1..=50).map(|i| format!("Historical line {}", i)).collect::<Vec<_>>().join("\n") },
                // Turn 2 (Recent - must be kept intact)
                { "role": "assistant", "reasoning": "Recent thoughts", "content": "Found finding" },
                { "role": "tool", "content": (1..=50).map(|i| format!("Recent line {}", i)).collect::<Vec<_>>().join("\n") }
            ]
        });

        let stats = condenser.condense(&mut body);

        assert_eq!(stats.reasoning_blocks_dropped, 1);
        assert_eq!(stats.tool_outputs_truncated, 1);
        assert!(stats.chars_saved > 0);

        let msgs = body["messages"].as_array().unwrap();
        // Turn 1 assistant should have reasoning removed
        assert!(msgs[2].get("reasoning").is_none());
        // Turn 1 tool output should be truncated
        assert!(msgs[3]["content"].as_str().unwrap().contains("Ferry Proxy: truncated"));

        // Turn 2 assistant must still have reasoning intact!
        assert_eq!(msgs[4]["reasoning"].as_str().unwrap(), "Recent thoughts");
        // Turn 2 tool output must remain untruncated!
        assert!(!msgs[5]["content"].as_str().unwrap().contains("Ferry Proxy: truncated"));
    }

    #[test]
    fn test_condenser_disabled() {
        let config = CondenserConfig {
            enabled: false,
            drop_historical_reasoning: true,
            truncate_tool_outputs: true,
            keep_recent_turns: 1,
            max_tool_chars: 100,
            head_lines: 2,
            tail_lines: 2,
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "assistant", "reasoning": "Should stay", "content": "Test" }
            ]
        });

        let stats = condenser.condense(&mut body);
        assert_eq!(stats, CondensationStats::default());
        assert!(body["messages"][0].get("reasoning").is_some());
    }

    #[test]
    fn test_standalone_reasoning_and_reasoning_content() {
        let config = CondenserConfig {
            enabled: true,
            drop_historical_reasoning: true,
            truncate_tool_outputs: true,
            keep_recent_turns: 1,
            max_tool_chars: 100,
            head_lines: 2,
            tail_lines: 2,
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "System prompt with detailed instruction" },
                { "role": "user", "content": "Initial user task" },
                // Turn 1 with standalone reasoning object + reasoning_content
                { "type": "reasoning", "content": "Massive internal reasoning scratchpad...", "summary": ["step1"] },
                { "role": "assistant", "reasoning_content": "DeepSeek/Qwen style reasoning", "content": "Calling tool" },
                { "role": "tool", "content": "Tool result" },
                // Turn 2 (Recent)
                { "role": "assistant", "reasoning_content": "Active recent reasoning", "content": "Final answer" }
            ]
        });

        let stats = condenser.condense(&mut body);
        assert!(stats.reasoning_blocks_dropped >= 2);

        let msgs = body["messages"].as_array().unwrap();
        // Standalone reasoning content cleared
        assert_eq!(msgs[2]["content"].as_str().unwrap(), "");
        // Historical assistant reasoning_content removed
        assert!(msgs[3].get("reasoning_content").is_none());
        // Recent assistant reasoning_content preserved
        assert_eq!(msgs[5]["reasoning_content"].as_str().unwrap(), "Active recent reasoning");
        // System and User prompts unchanged
        assert_eq!(msgs[0]["content"].as_str().unwrap(), "System prompt with detailed instruction");
        assert_eq!(msgs[1]["content"].as_str().unwrap(), "Initial user task");
    }
}
