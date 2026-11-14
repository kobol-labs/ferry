pub mod code;
pub mod directory;
pub mod stats;
pub mod truncation;
pub mod whitespace;

pub use code::*;
pub use directory::*;
pub use stats::*;
pub use truncation::*;
pub use whitespace::*;

use crate::config::CondenserConfig;
use serde_json::Value;
use tracing::{debug, warn};

pub const LOOP_GUARD_MARKER: &str = "[Ferry Loop Guard]:";
pub const LOOP_GUARD_SUFFIX: &str = "\n\n[Ferry Loop Guard]: This exact tool call has failed 3 times consecutively with the same result. Do not repeat it; inspect the root cause or pivot to a different approach.";

struct ParsedToolCall {
    id: Option<String>,
    signature: (String, String),
}

struct ParsedToolOutput {
    id: Option<String>,
    clean_text: String,
}

struct TurnTuple {
    signatures: Vec<(String, String)>,
    outputs: Vec<String>,
}

/// Stateless tail-only loop guard: detects 3 consecutive identical failing `(assistant -> tool)` turns
/// at the end of `body.messages` and appends a single redirect line ONLY to the final `tool` message.
pub fn nudge_repeated_tool_failure(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return false;
    };
    if messages.len() < 6 {
        return false;
    }

    // Fast path & idempotency guard: inspect tail message first.
    let last_msg = &messages[messages.len() - 1];
    if !is_tool_output_message(last_msg) {
        return false;
    }
    let Some(last_raw_text) = extract_tool_output_text(last_msg) else {
        return false;
    };
    if last_raw_text.contains(LOOP_GUARD_MARKER) {
        return false;
    }

    // Walk backward over the last 3 completed (assistant -> k tool outputs) turns.
    let mut cursor = messages.len();
    let mut turns: Vec<TurnTuple> = Vec::with_capacity(3);
    for step in 0..3 {
        let mut tool_start = cursor;
        while tool_start > 0 && is_tool_output_message(&messages[tool_start - 1]) {
            tool_start -= 1;
        }
        let k = cursor - tool_start;
        if k == 0 || tool_start == 0 {
            return false;
        }

        let mut raw_outputs = Vec::with_capacity(k);
        for msg in &messages[tool_start..cursor] {
            let Some(raw_out) = extract_tool_output_text(msg) else {
                return false;
            };
            if step == 0 && raw_out.contains(LOOP_GUARD_MARKER) {
                return false;
            }
            let clean_text = raw_out
                .strip_suffix(LOOP_GUARD_SUFFIX)
                .unwrap_or(&raw_out)
                .to_string();
            raw_outputs.push(ParsedToolOutput {
                id: extract_tool_output_id(msg),
                clean_text,
            });
        }

        let Some((next_cursor, calls)) = extract_preceding_tool_calls(messages, tool_start, k)
        else {
            return false;
        };
        let Some(aligned_outputs) = align_tool_outputs(&calls, &raw_outputs) else {
            return false;
        };

        let signatures = calls.into_iter().map(|c| c.signature).collect();
        turns.push(TurnTuple {
            signatures,
            outputs: aligned_outputs,
        });
        cursor = next_cursor;
    }

    if turns[0].signatures != turns[1].signatures || turns[1].signatures != turns[2].signatures {
        return false;
    }

    let tuple_len = turns[0].outputs.len();
    for slot in 0..tuple_len {
        let o0 = &turns[0].outputs[slot];
        let o1 = &turns[1].outputs[slot];
        let o2 = &turns[2].outputs[slot];

        let all_diagnostic_failures = is_diagnostic_tool_failure(o0)
            && is_diagnostic_tool_failure(o1)
            && is_diagnostic_tool_failure(o2);
        let all_identical_non_empty = !o0.trim().is_empty() && o0 == o1 && o1 == o2;

        if !all_diagnostic_failures && !all_identical_non_empty {
            return false;
        }
    }

    // Non-destructive tail-only mutation: ONLY messages.last_mut() is ever modified.
    if let Some(tail) = messages.last_mut() {
        return append_nudge_to_tool_message(tail);
    }
    false
}

fn is_tool_output_message(msg: &Value) -> bool {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default();
    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default();
    matches!(role, "tool" | "function") || matches!(msg_type, "function_call_output" | "tool_result")
}

fn extract_tool_output_id(msg: &Value) -> Option<String> {
    msg.get("tool_call_id")
        .or_else(|| msg.get("call_id"))
        .or_else(|| msg.get("tool_use_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn extract_tool_output_text(msg: &Value) -> Option<String> {
    if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
        return Some(c.to_string());
    }
    if let Some(o) = msg.get("output").and_then(|o| o.as_str()) {
        return Some(o.to_string());
    }
    if let Some(parts) = msg.get("content").and_then(|c| c.as_array()) {
        let texts: Vec<&str> = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect();
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    None
}

fn append_nudge_to_tool_message(msg: &mut Value) -> bool {
    let Some(obj) = msg.as_object_mut() else {
        return false;
    };
    if let Some(Value::String(c)) = obj.get_mut("content") {
        c.push_str(LOOP_GUARD_SUFFIX);
        return true;
    }
    if let Some(Value::String(o)) = obj.get_mut("output") {
        o.push_str(LOOP_GUARD_SUFFIX);
        return true;
    }
    if let Some(Value::Array(parts)) = obj.get_mut("content") {
        for part in parts.iter_mut().rev() {
            if let Some(Value::String(t)) = part.get_mut("text") {
                t.push_str(LOOP_GUARD_SUFFIX);
                return true;
            }
        }
    }
    false
}

fn extract_preceding_tool_calls(
    messages: &[Value],
    tool_start: usize,
    expected_k: usize,
) -> Option<(usize, Vec<ParsedToolCall>)> {
    if tool_start == 0 || expected_k == 0 {
        return None;
    }

    let prev = &messages[tool_start - 1];
    let role = prev.get("role").and_then(|r| r.as_str()).unwrap_or_default();

    if role == "assistant" {
        if let Some(arr) = prev.get("tool_calls").and_then(|tc| tc.as_array()) {
            if arr.len() != expected_k {
                return None;
            }
            let mut calls = Vec::with_capacity(expected_k);
            for item in arr {
                let id = item
                    .get("id")
                    .or_else(|| item.get("call_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let func = item.get("function").unwrap_or(item);
                let name = func.get("name").and_then(|n| n.as_str())?.trim();
                if name.is_empty() {
                    return None;
                }
                let args = normalize_tool_arguments(func.get("arguments"));
                calls.push(ParsedToolCall {
                    id,
                    signature: (name.to_lowercase(), args),
                });
            }
            return Some((tool_start - 1, calls));
        }

        if let Some(fc) = prev.get("function_call").filter(|_| expected_k == 1) {
            let name = fc.get("name").and_then(|n| n.as_str())?.trim();
            if name.is_empty() {
                return None;
            }
            let args = normalize_tool_arguments(fc.get("arguments"));
            return Some((
                tool_start - 1,
                vec![ParsedToolCall {
                    id: None,
                    signature: (name.to_lowercase(), args),
                }],
            ));
        }

        return None;
    }

    let mut fc_start = tool_start;
    while fc_start > 0
        && messages[fc_start - 1]
            .get("type")
            .and_then(|t| t.as_str())
            == Some("function_call")
    {
        fc_start -= 1;
    }
    let fc_count = tool_start - fc_start;
    if fc_count > 0 {
        if fc_count != expected_k {
            return None;
        }
        let mut calls = Vec::with_capacity(expected_k);
        for item in &messages[fc_start..tool_start] {
            let id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let name = item.get("name").and_then(|n| n.as_str())?.trim();
            if name.is_empty() {
                return None;
            }
            let args = normalize_tool_arguments(item.get("arguments"));
            calls.push(ParsedToolCall {
                id,
                signature: (name.to_lowercase(), args),
            });
        }
        return Some((fc_start, calls));
    }

    None
}

fn align_tool_outputs(
    calls: &[ParsedToolCall],
    outputs: &[ParsedToolOutput],
) -> Option<Vec<String>> {
    let k = calls.len();
    if k == 0 || outputs.len() != k {
        return None;
    }

    let mut aligned: Vec<Option<String>> = vec![None; k];
    let mut used_out = vec![false; k];

    // Pass 1: exact match whenever both call_id and tool_call_id are present and non-empty.
    for (ci, call) in calls.iter().enumerate() {
        let Some(cid) = call.id.as_deref() else {
            continue;
        };
        if let Some(oi) = (0..k).find(|&oi| !used_out[oi] && outputs[oi].id.as_deref() == Some(cid))
        {
            used_out[oi] = true;
            aligned[ci] = Some(outputs[oi].clean_text.clone());
        }
    }

    // Pass 2: positional fallback only when at least one side omits an ID.
    let mut next_unused = 0usize;
    for (ci, call) in calls.iter().enumerate() {
        if aligned[ci].is_some() {
            continue;
        }
        while next_unused < k && used_out[next_unused] {
            next_unused += 1;
        }
        if next_unused >= k {
            return None;
        }
        let out = &outputs[next_unused];
        if call.id.is_some() && out.id.is_some() {
            return None;
        }
        used_out[next_unused] = true;
        aligned[ci] = Some(out.clean_text.clone());
    }

    aligned.into_iter().collect()
}

fn normalize_tool_arguments(args_val: Option<&Value>) -> String {
    match args_val {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                serde_json::to_string(&parsed).unwrap_or_else(|_| trimmed.to_string())
            } else {
                trimmed.to_string()
            }
        }
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn is_diagnostic_tool_failure(text: &str) -> bool {
    if crate::router::classifier::has_real_diagnostic_signal(text).is_some() {
        return true;
    }
    let prefix_lower: String = text
        .trim_start()
        .chars()
        .take(24)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    prefix_lower.starts_with("error:")
        || prefix_lower.starts_with("file not found")
        || prefix_lower.starts_with("no such file")
        || prefix_lower.starts_with("permission denied")
        || prefix_lower.starts_with("notfound")
}

pub struct ContextCondenser {
    config: CondenserConfig,
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

        let orig_chars = count_message_chars(messages);

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

        // 1.5. Runaway Repetition Folding:
        // Identifies identical tool outputs across historical turns and folds redundant repetitions
        if self.config.fold_repeated_tool_failures && cutoff_index > 1 {
            let is_foldable_failure = |text: &str| -> bool {
                if text.starts_with("[... Ferry Proxy: Repeated identical tool failure suppressed") {
                    return false;
                }
                if is_diagnostic_tool_failure(text) {
                    return true;
                }
                for raw_line in text.lines() {
                    let lower = raw_line.to_ascii_lowercase();
                    if crate::router::classifier::is_source_code_or_comment_line(&lower) {
                        continue;
                    }
                    if lower.contains("connection refused")
                        || lower.contains("failed to connect")
                        || lower.contains("command not found")
                        || lower.contains("connection timed out")
                        || lower.contains("access denied")
                        || lower.contains("cannot find module")
                        || lower.contains("module not found")
                        || lower.contains("assertionerror")
                        || lower.contains("=== failures ===")
                        || lower.contains("test result: failed")
                        || lower.trim_start().starts_with("failed ")
                    {
                        return true;
                    }
                    for marker in ["exit status ", "exit code "] {
                        if let Some(pos) = lower.find(marker) {
                            let rest = lower[pos + marker.len()..].trim_start();
                            if let Some(first) = rest.chars().next() {
                                if first.is_ascii_digit() && first != '0' {
                                    return true;
                                }
                            }
                        }
                    }
                }
                false
            };

            let mut seen_historical_outputs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for msg in &messages[0..cutoff_index] {
                if is_tool_output_message(msg) {
                    if let Some(raw_content) = extract_tool_output_text(msg) {
                        let base = raw_content.strip_suffix(LOOP_GUARD_SUFFIX).unwrap_or(&raw_content);
                        let trimmed = base.trim();
                        if trimmed.len() >= 25 && is_foldable_failure(trimmed) {
                            *seen_historical_outputs.entry(trimmed.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }

            if seen_historical_outputs.values().any(|&count| count >= 2) {
                let mut kept_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for msg in &mut messages[0..cutoff_index] {
                    if is_tool_output_message(msg) {
                        if let Some(raw_content) = extract_tool_output_text(msg) {
                            let base = raw_content.strip_suffix(LOOP_GUARD_SUFFIX).unwrap_or(&raw_content);
                            let trimmed = base.trim();
                            if let Some(&total_count) = seen_historical_outputs.get(trimmed) {
                                if total_count >= 2 {
                                    let seen_so_far = kept_counts.entry(trimmed.to_string()).or_insert(0);
                                    *seen_so_far += 1;
                                    if *seen_so_far > 1 {
                                        if let Some(obj) = msg.as_object_mut() {
                                            let folded = format!(
                                                "[... Ferry Proxy: Repeated identical tool failure suppressed (attempt #{}); see earlier turn for output ...]",
                                                *seen_so_far
                                            );
                                            let mut replaced = false;
                                            if matches!(obj.get("content"), Some(Value::String(_))) {
                                                obj.insert("content".to_string(), Value::String(folded));
                                                replaced = true;
                                            } else if matches!(obj.get("output"), Some(Value::String(_))) {
                                                obj.insert("output".to_string(), Value::String(folded));
                                                replaced = true;
                                            } else if let Some(Value::Array(parts)) = obj.get_mut("content") {
                                                let mut placed = false;
                                                parts.retain_mut(|part| {
                                                    if part.get("text").and_then(|t| t.as_str()).is_some() {
                                                        if !placed {
                                                            if let Some(po) = part.as_object_mut() {
                                                                po.insert("text".to_string(), Value::String(folded.clone()));
                                                            }
                                                            placed = true;
                                                            true
                                                        } else {
                                                            false
                                                        }
                                                    } else {
                                                        true
                                                    }
                                                });
                                                replaced = placed;
                                            }
                                            if replaced {
                                                stats.loops_collapsed_count += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Iterate through all messages
        let msg_count = messages.len();
        for idx in 0..msg_count {
            let msg = &mut messages[idx];
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default().to_string();
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default().to_string();

            // CRITICAL INVARIANT: Prefix-cache alignment guard.
            // Never mutate system rules, developer messages, or user prompts across ANY turn.
            // Preserves KV cache prefixes to ensure 90%+ cache hit rate on local GPU and cloud.
            if role == "system" || role == "user" || role == "developer" {
                continue;
            }

            let is_historical = idx < cutoff_index;
            let is_completed_turn = is_historical || (idx + 1 < msg_count);

            // A) Handle standalone reasoning message items
            if is_completed_turn && self.config.drop_historical_reasoning && msg_type == "reasoning" {
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
            if is_completed_turn && self.config.drop_historical_reasoning && (role == "assistant" || msg_type == "message") {
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
                if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
                    let initial_text = c.to_string();
                    let processed = self.condense_tool_string(&initial_text, is_historical, &mut stats);
                    if processed != initial_text {
                        if let Some(obj) = msg.as_object_mut() {
                            obj.insert("content".to_string(), Value::String(processed));
                        }
                    }
                } else if let Some(o) = msg.get("output").and_then(|o| o.as_str()) {
                    let initial_text = o.to_string();
                    let processed = self.condense_tool_string(&initial_text, is_historical, &mut stats);
                    if processed != initial_text {
                        if let Some(obj) = msg.as_object_mut() {
                            obj.insert("output".to_string(), Value::String(processed));
                        }
                    }
                } else if let Some(content_arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                    for part in content_arr {
                        if let Some(text_val) = part.get("text").and_then(|t| t.as_str()) {
                            let initial_text = text_val.to_string();
                            let processed = self.condense_tool_string(&initial_text, is_historical, &mut stats);
                            if processed != initial_text {
                                if let Some(part_obj) = part.as_object_mut() {
                                    part_obj.insert("text".to_string(), Value::String(processed));
                                }
                            }
                        }
                    }
                }
            }
        }

        let cond_chars = count_message_chars(messages);
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

    /// Condenses a single tool output string according to historical vs recent policies and constraint ledger.
    pub fn condense_tool_string(
        &self,
        initial_text: &str,
        is_historical: bool,
        stats: &mut CondensationStats,
    ) -> String {
        if initial_text.is_empty() {
            return String::new();
        }

        let (base_text, has_loop_nudge) = match initial_text.strip_suffix(LOOP_GUARD_SUFFIX) {
            Some(s) => (s, true),
            None => (initial_text, false),
        };

        let mut processed = if base_text.is_empty() {
            String::new()
        } else {
            let mut p = self.clean_tool_text_with_stats(base_text, stats);

            // Truncation logic
            if self.config.truncate_tool_outputs {
                if is_historical {
                    if p.len() > self.config.max_tool_chars {
                        let hist_head = if self.config.max_tool_chars >= 8000 {
                            self.config.head_lines.max(30)
                        } else {
                            self.config.head_lines
                        };
                        let hist_tail = if self.config.max_tool_chars >= 8000 {
                            self.config.tail_lines.max(20)
                        } else {
                            self.config.tail_lines
                        };
                        p = truncate_tool_content_smart(
                            &p,
                            hist_head,
                            hist_tail,
                            &mut stats.signals_extracted_count,
                        );
                        stats.tool_outputs_truncated += 1;
                    }
                } else if self.config.recent_max_tool_chars > 0 && p.len() > self.config.recent_max_tool_chars {
                    let recent_head = self.config.head_lines.max(30);
                    let recent_tail = self.config.tail_lines.max(20);
                    p = truncate_tool_content_smart(
                        &p,
                        recent_head,
                        recent_tail,
                        &mut stats.signals_extracted_count,
                    );
                    stats.tool_outputs_truncated += 1;
                }
            }

            // Permanent invariant: PromptTrim-inspired Constraint Ledger verification (Proof-of-safety)
            if !is_historical {
                let anchors = extract_tool_anchors(base_text);
                if let Some(missing) = self.find_missing_anchor(&anchors, &p) {
                    warn!(missing_anchor = %missing, "Constraint ledger vetoed compression on recent turn; restored verbatim");
                    stats.ledger_vetoed_count += 1;
                    p = base_text.to_string();
                } else {
                    stats.ledger_checks_passed += 1;
                }
            } else {
                stats.ledger_checks_passed += 1;
            }
            p
        };

        if has_loop_nudge {
            processed.push_str(LOOP_GUARD_SUFFIX);
        }

        processed
    }

    /// Applies inline text cleaners with 100% verbatim markdown code-block protection
    pub fn clean_tool_text(&self, input: &str) -> String {
        let mut dummy = CondensationStats::default();
        self.clean_tool_text_with_stats(input, &mut dummy)
    }

    pub fn clean_tool_text_with_stats(&self, input: &str, stats: &mut CondensationStats) -> String {
        if input.contains("```") {
            let segments = segment_markdown_code_blocks(input);
            let mut cleaned_parts = Vec::new();
            for seg in segments {
                match seg {
                    ContentSegment::Text(t) => cleaned_parts.push(self.clean_single_chunk(&t, stats)),
                    ContentSegment::Code(c) => cleaned_parts.push(c),
                }
            }
            cleaned_parts.join("\n")
        } else {
            self.clean_single_chunk(input, stats)
        }
    }

    pub fn extract_critical_anchors(&self, text: &str) -> Vec<String> {
        code::extract_critical_anchors(text)
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

    /// Secondary adaptive squeezing pass: shrinks older historical tool outputs from 2000 chars down to 500 chars
    pub fn adaptive_squeeze(
        &self,
        body: &mut Value,
        target_max_tokens: u64,
    ) -> Option<usize> {
        let messages = body.get_mut("messages")?.as_array_mut()?;
        let orig_chars = count_message_chars(messages);
        let orig_tokens = estimate_tokens_from_chars(orig_chars);

        if orig_tokens <= target_max_tokens {
            return None;
        }

        let mut total_squeezed_chars = 0;
        let num_messages = messages.len();
        let recent_boundary = if num_messages > self.config.keep_recent_turns * 2 {
            num_messages - (self.config.keep_recent_turns * 2)
        } else {
            0
        };

        for (idx, msg) in messages.iter_mut().enumerate() {
            if idx >= recent_boundary {
                continue;
            }

            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "tool" || role == "function" {
                if let Some(content) = msg.get_mut("content") {
                    if let Some(s) = content.as_str() {
                        if s.len() > 600 {
                            let mut dummy_signals = 0;
                            let squeezed = truncate_tool_content_smart(s, 5, 5, &mut dummy_signals);
                            if squeezed.len() < s.len() {
                                total_squeezed_chars += s.len() - squeezed.len();
                                *content = Value::String(squeezed);
                            }
                        }
                    }
                }
            }
        }

        if total_squeezed_chars > 0 {
            Some(total_squeezed_chars)
        } else {
            None
        }
    }

    /// Rolling Context Compaction:
    /// Compacts older historical turns into a structured checkpoint block, leaving system prompt and recent turns intact.
    pub fn rolling_compact(&self, body: &mut Value) -> Option<RollingCompactionStats> {
        if !self.config.rolling_compaction {
            return None;
        }

        let messages = body.get_mut("messages")?.as_array_mut()?;
        let orig_chars = count_message_chars(messages);

        let total_msgs = messages.len();
        let keep_recent = self.config.compact_keep_recent_turns * 2;
        if total_msgs <= keep_recent + 2 {
            return None;
        }

        let mut start_idx = 0;
        while start_idx < total_msgs {
            let role = messages[start_idx]
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if role == "system" || role == "developer" {
                start_idx += 1;
            } else {
                break;
            }
        }
        if start_idx >= total_msgs {
            return None;
        }

        let conv_chars = count_message_chars(&messages[start_idx..]);
        let conv_tokens = estimate_tokens_from_chars(conv_chars);

        if conv_tokens <= self.config.compact_threshold_tokens {
            return None;
        }

        let mut split_idx = total_msgs.saturating_sub(keep_recent);
        if split_idx <= start_idx {
            return None;
        }

        let raw_compacted = split_idx - start_idx;
        if raw_compacted >= 16 {
            let quantized = (raw_compacted / 16) * 16;
            split_idx = start_idx + quantized;
        }

        let is_compact_tool_output = |msg: &Value| -> bool {
            if is_tool_output_message(msg) {
                return true;
            }
            msg.get("content")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    !arr.is_empty()
                        && arr.iter().all(|p| {
                            p.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                        })
                })
                .unwrap_or(false)
        };

        let is_compact_tool_call = |msg: &Value| -> bool {
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if matches!(msg_type, "function_call" | "tool_use") {
                return true;
            }
            if msg
                .get("tool_calls")
                .and_then(|tc| tc.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                return true;
            }
            if msg.get("function_call").map(|fc| fc.is_object()).unwrap_or(false) {
                return true;
            }
            msg.get("content")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter().any(|p| {
                        matches!(
                            p.get("type").and_then(|t| t.as_str()),
                            Some("tool_use" | "function_call")
                        )
                    })
                })
                .unwrap_or(false)
        };

        let collect_output_call_ids = |msg: &Value, out: &mut Vec<String>| {
            if let Some(id) = extract_tool_output_id(msg) {
                out.push(id);
            }
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for p in arr {
                    if p.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        if let Some(id) = p
                            .get("tool_use_id")
                            .or_else(|| p.get("call_id"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            out.push(id.to_string());
                        }
                    }
                }
            }
        };

        let collect_call_ids_from_msg = |msg: &Value, out: &mut Vec<String>| {
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if matches!(msg_type, "function_call" | "tool_use") {
                if let Some(id) = msg
                    .get("call_id")
                    .or_else(|| msg.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    out.push(id.to_string());
                }
            }
            if let Some(arr) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                for item in arr {
                    if let Some(id) = item
                        .get("id")
                        .or_else(|| item.get("call_id"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        out.push(id.to_string());
                    }
                }
            }
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for p in arr {
                    if matches!(
                        p.get("type").and_then(|t| t.as_str()),
                        Some("tool_use" | "function_call")
                    ) {
                        if let Some(id) = p
                            .get("id")
                            .or_else(|| p.get("call_id"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            out.push(id.to_string());
                        }
                    }
                }
            }
        };

        loop {
            let prev_split = split_idx;

            while split_idx > start_idx
                && split_idx < total_msgs
                && is_compact_tool_output(&messages[split_idx])
            {
                split_idx -= 1;
            }

            while split_idx > start_idx && is_compact_tool_call(&messages[split_idx - 1]) {
                split_idx -= 1;
            }

            while split_idx > start_idx
                && split_idx < total_msgs
                && is_compact_tool_call(&messages[split_idx])
                && messages[split_idx - 1].get("type").and_then(|t| t.as_str()) == Some("reasoning")
            {
                split_idx -= 1;
            }

            if split_idx > start_idx && split_idx < total_msgs {
                let mut kept_output_ids = Vec::new();
                for m in &messages[split_idx..] {
                    if is_compact_tool_output(m) {
                        collect_output_call_ids(m, &mut kept_output_ids);
                    }
                }
                if !kept_output_ids.is_empty() {
                    for idx in start_idx..split_idx {
                        let mut defined_ids = Vec::new();
                        collect_call_ids_from_msg(&messages[idx], &mut defined_ids);
                        if defined_ids
                            .iter()
                            .any(|id| kept_output_ids.iter().any(|oid| oid == id))
                        {
                            split_idx = idx;
                            break;
                        }
                    }
                }
            }

            if split_idx == prev_split || split_idx <= start_idx {
                break;
            }
        }

        if split_idx <= start_idx + 1 {
            return None;
        }

        let extract_compact_text = |msg: &Value| -> Option<String> {
            let mut parts_buf: Vec<&str> = Vec::new();
            if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
                if !c.is_empty() {
                    parts_buf.push(c);
                }
            } else if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for part in arr {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        if !t.is_empty() {
                            parts_buf.push(t);
                        }
                    } else if part.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        if let Some(tc) = part.get("content").and_then(|c| c.as_str()) {
                            if !tc.is_empty() {
                                parts_buf.push(tc);
                            }
                        } else if let Some(tarr) = part.get("content").and_then(|c| c.as_array()) {
                            for sub in tarr {
                                if let Some(st) = sub.get("text").and_then(|t| t.as_str()) {
                                    if !st.is_empty() {
                                        parts_buf.push(st);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(o) = msg.get("output").and_then(|o| o.as_str()) {
                if !o.is_empty() {
                    parts_buf.push(o);
                }
            } else if let Some(arr) = msg.get("output").and_then(|o| o.as_array()) {
                for part in arr {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        if !t.is_empty() {
                            parts_buf.push(t);
                        }
                    }
                }
            }
            if parts_buf.is_empty() {
                None
            } else {
                Some(parts_buf.join("\n"))
            }
        };

        let mut summary_lines = Vec::new();
        let mut preserved_endpoints: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut preserved_auth: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut preserved_anomalies: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let num_compacted = split_idx - start_idx;

        let format_tool_call_line = |name: &str, args: &str| -> String {
            let args_snippet = if !args.is_empty() {
                extract_compact_tool_arguments(args)
            } else {
                String::new()
            };
            if args_snippet.is_empty() {
                format!("• [Executed Tool]: {}", name)
            } else {
                format!("• [Executed Tool]: {}({})", name, args_snippet)
            }
        };

        for msg in &messages[start_idx..split_idx] {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");

            if is_compact_tool_output(msg) {
                let name = msg.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                if let Some(content) = extract_compact_text(msg) {
                    collect_discovered_entities(
                        &content,
                        &mut preserved_endpoints,
                        &mut preserved_auth,
                        &mut preserved_anomalies,
                    );

                    let signal_line = content.lines().find(|l| {
                        let lower = l.to_ascii_lowercase();
                        DIAGNOSTIC_SIGNALS.iter().any(|sig| lower.contains(sig))
                    });

                    if let Some(sig) = signal_line {
                        let snippet: String = sig.trim().chars().take(180).collect();
                        summary_lines.push(format!("  └─ [{name} result]: ⚠️ Signal: {}", snippet));
                    } else {
                        let first_line = content.lines().next().unwrap_or("").trim();
                        let snippet: String = first_line.chars().take(150).collect();
                        summary_lines.push(format!("  └─ [{name} result]: {}", snippet));
                    }
                }
                continue;
            }

            if msg_type == "function_call" {
                let name = msg.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                let args_owned = normalize_tool_arguments(msg.get("arguments"));
                summary_lines.push(format_tool_call_line(name, &args_owned));
                collect_discovered_entities(
                    &args_owned,
                    &mut preserved_endpoints,
                    &mut preserved_auth,
                    &mut preserved_anomalies,
                );
                continue;
            }

            match role {
                "user" => {
                    if let Some(content) = extract_compact_text(msg) {
                        if content.starts_with("[Historical Context Checkpoint:") {
                            let (inside_cp, after_cp) = match content.split_once("[End of Historical Checkpoint]") {
                                Some((before, after)) => (before, Some(after)),
                                None => (content.as_str(), None),
                            };
                            for line in inside_cp.lines() {
                                let trimmed = line.trim();
                                if trimmed == "[Preserved Security Discoveries & Entities]" {
                                    break;
                                }
                                if trimmed.starts_with("• ") || trimmed.starts_with("└─ ") {
                                    summary_lines.push(trimmed.to_string());
                                }
                            }
                            if let Some(after) = after_cp {
                                let trailing = after.trim();
                                if !trailing.is_empty() {
                                    let snippet: String = trailing.chars().take(250).collect();
                                    summary_lines.push(format!("• [User Request]: {}", snippet.trim()));
                                }
                            }
                        } else {
                            let snippet: String = content.chars().take(250).collect();
                            summary_lines.push(format!("• [User Request]: {}", snippet.trim()));
                        }
                        collect_discovered_entities(
                            &content,
                            &mut preserved_endpoints,
                            &mut preserved_auth,
                            &mut preserved_anomalies,
                        );
                    }
                }
                "assistant" => {
                    let mut emitted_tool_call = false;
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        for tc in tool_calls {
                            let func = tc.get("function").unwrap_or(tc);
                            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                            let args = func.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                            summary_lines.push(format_tool_call_line(name, args));
                            collect_discovered_entities(
                                args,
                                &mut preserved_endpoints,
                                &mut preserved_auth,
                                &mut preserved_anomalies,
                            );
                            emitted_tool_call = true;
                        }
                    }
                    if let Some(fc) = msg.get("function_call").filter(|v| v.is_object()) {
                        let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        let args_owned = normalize_tool_arguments(fc.get("arguments"));
                        summary_lines.push(format_tool_call_line(name, &args_owned));
                        collect_discovered_entities(
                            &args_owned,
                            &mut preserved_endpoints,
                            &mut preserved_auth,
                            &mut preserved_anomalies,
                        );
                        emitted_tool_call = true;
                    }
                    if let Some(content) = extract_compact_text(msg) {
                        if !emitted_tool_call {
                            let snippet: String = content.chars().take(250).collect();
                            summary_lines.push(format!("• [Assistant Finding]: {}", snippet.trim()));
                        }
                        collect_discovered_entities(
                            &content,
                            &mut preserved_endpoints,
                            &mut preserved_auth,
                            &mut preserved_anomalies,
                        );
                    }
                }
                _ if role.is_empty() && msg_type == "message" => {
                    if let Some(content) = extract_compact_text(msg) {
                        let snippet: String = content.chars().take(250).collect();
                        summary_lines.push(format!("• [Assistant Finding]: {}", snippet.trim()));
                        collect_discovered_entities(
                            &content,
                            &mut preserved_endpoints,
                            &mut preserved_auth,
                            &mut preserved_anomalies,
                        );
                    }
                }
                _ => {}
            }
        }

        if summary_lines.len() > 35 {
            let head = summary_lines[..10].to_vec();
            let tail = summary_lines[summary_lines.len() - 20..].to_vec();
            let omitted = summary_lines.len() - 30;
            let mut bounded = head;
            bounded.push(format!("• [... {} earlier milestones collapsed ...]", omitted));
            bounded.extend(tail);
            summary_lines = bounded;
        }

        let mut entity_lines = Vec::new();
        if !preserved_endpoints.is_empty() {
            let ep_list: Vec<&str> = preserved_endpoints.iter().take(10).map(|s| s.as_str()).collect();
            entity_lines.push(format!("• Discovered Endpoints: {}", ep_list.join(", ")));
        }
        if !preserved_auth.is_empty() {
            let auth_list: Vec<&str> = preserved_auth.iter().take(8).map(|s| s.as_str()).collect();
            entity_lines.push(format!("• Active Auth/Session Artifacts: {}", auth_list.join(", ")));
        }
        if !preserved_anomalies.is_empty() {
            let anom_list: Vec<&str> = preserved_anomalies.iter().take(8).map(|s| s.as_str()).collect();
            entity_lines.push(format!("• Flagged Security Discoveries: {}", anom_list.join("; ")));
        }
        if !entity_lines.is_empty() {
            summary_lines.push("\n[Preserved Security Discoveries & Entities]".to_string());
            summary_lines.extend(entity_lines);
        }

        let summary_text = format!(
            "[Historical Context Checkpoint: {} earlier turns compacted]\n{}\n[End of Historical Checkpoint]",
            num_compacted,
            summary_lines.join("\n")
        );

        let split_is_user = messages
            .get(split_idx)
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("user")
            && !is_compact_tool_output(&messages[split_idx]);

        let original_range_chars = count_message_chars(&messages[start_idx..split_idx]);
        let added_chars = if split_is_user {
            summary_text.len() + 2
        } else {
            summary_text.len()
        };
        if added_chars >= original_range_chars {
            return None;
        }

        if split_is_user {
            let mut summary_opt = Some(summary_text);
            if let Some(user_obj) = messages[split_idx].as_object_mut() {
                match user_obj.get_mut("content") {
                    Some(Value::String(existing)) => {
                        let st = summary_opt.take().unwrap_or_default();
                        if existing.is_empty() {
                            *existing = st;
                        } else {
                            *existing = format!("{st}\n\n{existing}");
                        }
                    }
                    Some(Value::Array(parts)) => {
                        let st = summary_opt.take().unwrap_or_default();
                        let mut merged_into_part = false;
                        for part in parts.iter_mut() {
                            if let Some(part_obj) = part.as_object_mut() {
                                if let Some(Value::String(existing_text)) = part_obj.get_mut("text") {
                                    if existing_text.is_empty() {
                                        *existing_text = st.clone();
                                    } else {
                                        *existing_text = format!("{st}\n\n{existing_text}");
                                    }
                                    merged_into_part = true;
                                    break;
                                }
                            }
                        }
                        if !merged_into_part {
                            parts.insert(0, serde_json::json!({ "type": "text", "text": st }));
                        }
                    }
                    None => {
                        let st = summary_opt.take().unwrap_or_default();
                        user_obj.insert("content".to_string(), Value::String(st));
                    }
                    _ => {}
                }
            }
            if let Some(unmerged) = summary_opt {
                let checkpoint_msg = serde_json::json!({
                    "role": "user",
                    "content": unmerged
                });
                messages.splice(start_idx..split_idx, std::iter::once(checkpoint_msg));
            } else {
                messages.drain(start_idx..split_idx);
            }
        } else {
            let checkpoint_msg = serde_json::json!({
                "role": "user",
                "content": summary_text
            });
            messages.splice(start_idx..split_idx, std::iter::once(checkpoint_msg));
        }

        let new_chars = count_message_chars(messages);
        let chars_saved = orig_chars.saturating_sub(new_chars);

        Some(RollingCompactionStats {
            turns_compacted: num_compacted,
            chars_saved,
            original_chars: orig_chars,
            compacted_chars: new_chars,
        })
    }

    fn clean_single_chunk(&self, input: &str, stats: &mut CondensationStats) -> String {
        let mut text = input.to_string();

        if self.config.strip_terminal_noise {
            if text.contains('\x1b') {
                let (cleaned, stripped) = strip_ansi_codes(&text);
                if stripped {
                    text = cleaned;
                    stats.ansi_stripped_count += 1;
                }
            }

            if text.contains('\r') || text.contains('%') || text.contains('█') {
                let (cleaned, stripped) = strip_progress_artifacts(&text);
                if stripped {
                    text = cleaned;
                    stats.progress_cleaned_count += 1;
                }
            }

            let (cleaned, collapsed) = collapse_excess_whitespace(&text);
            if collapsed {
                text = cleaned;
                stats.whitespace_collapsed_count += 1;
            }
        }

        if self.config.deduplicate_lines && text.contains('\n') {
            let (cleaned, count) = deduplicate_lines(&text);
            if count > 0 {
                text = cleaned;
                stats.lines_deduped_count += count;
            }
        }

        if self.config.minify_json_outputs {
            let trimmed = text.trim();
            if (trimmed.contains('\n') || trimmed.contains("  "))
                && ((trimmed.starts_with('{') && trimmed.ends_with('}'))
                    || (trimmed.starts_with('[') && trimmed.ends_with(']')))
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

        if self.config.prune_lockfile_diffs && text.contains("diff --git ") {
            let (cleaned, count) = prune_git_diff_lockfiles(&text);
            if count > 0 {
                text = cleaned;
                stats.lockfile_diffs_pruned_count += count;
            }
        }

        if self.config.suppress_base64_blobs && text.len() >= 120 {
            let (cleaned, count) = suppress_base64_and_hex_blobs(&text);
            if count > 0 {
                text = cleaned;
                stats.base64_blobs_suppressed_count += count;
            }
        }

        if self.config.shield_dir_traversals && text.contains('\n') {
            let (cleaned, count) = shield_directory_traversals(&text);
            if count > 0 {
                text = cleaned;
                stats.dir_traversals_shielded_count += count;
            }
        }

        text
    }
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
            deduplicate_lines: true,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let raw = "Log output here\nWarning: connection timeout\nWarning: connection timeout\nWarning: connection timeout\n```go\nfunc main() {\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n\t// Repeated comments inside code fence must NOT be touched\n}\n```\nLog end";
        let cleaned = condenser.clean_tool_text(raw);

        assert!(cleaned.contains("[... repeated 2 times ...]"));
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

    #[test]
    fn test_rolling_context_compaction_preserves_system_and_recent() {
        let config = CondenserConfig {
            rolling_compaction: true,
            compact_threshold_tokens: 50,
            compact_keep_recent_turns: 1,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "You are a pentester." },
                { "role": "user", "content": "Explore target 1 in deep detail" },
                { "role": "assistant", "content": "Investigating target 1 with network security tools" },
                { "role": "tool", "name": "nmap", "content": "PORT 80 open, Apache 2.4 web server details and directory listing\n".repeat(30) },
                { "role": "user", "content": "Explore target 2 in deep detail" },
                { "role": "assistant", "content": "Investigating target 2 with web client tools" },
                { "role": "tool", "name": "curl", "content": "HTTP 200 OK headers and body content with detailed debug responses\n".repeat(30) },
                { "role": "user", "content": "Recent active user request" },
                { "role": "assistant", "content": "Recent active assistant reply" }
            ]
        });

        let stats = condenser.rolling_compact(&mut body);
        assert!(stats.is_some(), "Expected rolling compaction to trigger");
        let s = stats.unwrap();
        assert!(s.turns_compacted >= 4);
        assert!(s.chars_saved > 0);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are a pentester.");

        assert_eq!(msgs[1]["role"], "user");
        let checkpoint = msgs[1]["content"].as_str().unwrap();
        assert!(checkpoint.contains("Historical Context Checkpoint"));
        assert!(checkpoint.contains("Explore target 1"));

        let last_user = &msgs[msgs.len() - 2];
        let last_asst = &msgs[msgs.len() - 1];
        assert_eq!(last_user["role"], "user");
        assert!(last_user["content"].as_str().unwrap().ends_with("Recent active user request"));
        assert_eq!(last_asst["role"], "assistant");
        assert_eq!(last_asst["content"], "Recent active assistant reply");
    }

    #[test]
    fn test_prune_git_diff_lockfiles() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -10,30 +10,30 @@\n+name = \"foo\"\n+version = \"1.0.0\"\n+checksum = \"abc123def456\"\n+name = \"bar\"\n+version = \"2.0.0\"\n+checksum = \"789xyz\"\n+name = \"baz\"\n+version = \"3.0.0\"\n+checksum = \"456qwe\"\n+name = \"qux\"\n+version = \"4.0.0\"\n+checksum = \"rtyuiop\"\n";
        let (pruned, count) = prune_git_diff_lockfiles(diff);
        assert_eq!(count, 1);
        assert!(pruned.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(pruned.contains("+new"));
        assert!(pruned.contains("diff --git a/Cargo.lock b/Cargo.lock"));
        assert!(pruned.contains("[... Ferry Proxy: lockfile/asset diff suppressed"));
        assert!(!pruned.contains("checksum = \"abc123def456\""));
    }

    #[test]
    fn test_suppress_base64_and_hex_blobs() {
        let raw = format!("data:image/png;base64,{}\nNormal log line\nRaw token: {}", "A".repeat(150), "B".repeat(160));
        let (cleaned, count) = suppress_base64_and_hex_blobs(&raw);
        assert!(count >= 2);
        assert!(cleaned.contains("[... Ferry Proxy: base64 payload (150 chars) suppressed ...]"));
        assert!(cleaned.contains("[... Ferry Proxy: base64/hex blob (160 chars) suppressed ...]"));
        assert!(cleaned.contains("Normal log line"));
    }

    #[test]
    fn test_shield_directory_traversals() {
        let listing = "./src/main.rs\n./node_modules/a/index.js\n./node_modules/b/index.js\n./node_modules/c/index.js\n./node_modules/d/index.js\n./node_modules/e/index.js\n./Cargo.toml\n";
        let (cleaned, count) = shield_directory_traversals(listing);
        assert_eq!(count, 5);
        assert!(cleaned.contains("./src/main.rs"));
        assert!(cleaned.contains("[... Ferry Proxy: 5 directory entries in node_modules/ suppressed ...]"));
        assert!(cleaned.contains("./Cargo.toml"));
        assert!(!cleaned.contains("./node_modules/c/index.js"));
    }

    #[test]
    fn test_multi_region_diagnostic_signal_preservation() {
        let mut lines = Vec::new();
        for i in 0..5 {
            lines.push(format!("HEAD line {}", i));
        }
        for i in 0..30 {
            lines.push(format!("filler passing log {}", i));
        }
        lines.push("CRITICAL ERROR: SQL syntax error near 'OR 1=1' in table users".to_string());
        for i in 0..30 {
            lines.push(format!("more passing info {}", i));
        }
        lines.push("ALERT: 500 Internal Server Error: unhandled nullpointer exception".to_string());
        for i in 0..30 {
            lines.push(format!("tail filler {}", i));
        }
        for i in 0..5 {
            lines.push(format!("TAIL line {}", i));
        }

        let full_text = lines.join("\n");
        let mut signals_extracted = 0;
        let truncated = truncate_tool_content_smart(&full_text, 5, 5, &mut signals_extracted);

        assert!(signals_extracted >= 2);
        assert!(truncated.contains("HEAD line 0"));
        assert!(truncated.contains("TAIL line 4"));
        assert!(truncated.contains("SQL syntax error near 'OR 1=1'"));
        assert!(truncated.contains("500 Internal Server Error"));
        assert!(truncated.contains("truncated"));
    }

    #[test]
    fn test_single_line_diagnostic_signal_preservation() {
        let prefix = "A".repeat(400);
        let signal = "status 500: Database transaction failed with sqlite3.OperationalError";
        let suffix = "B".repeat(400);
        let single_line = format!("{}{}{}", prefix, signal, suffix);

        let mut signals_extracted = 0;
        let truncated = truncate_tool_content_smart(&single_line, 10, 10, &mut signals_extracted);

        assert_eq!(signals_extracted, 1);
        assert!(truncated.contains("sqlite3.OperationalError"));
        assert!(truncated.contains("truncated"));
    }

    #[test]
    fn test_rolling_context_compaction_preserves_security_entities() {
        let config = CondenserConfig {
            rolling_compaction: true,
            compact_threshold_tokens: 50,
            compact_keep_recent_turns: 1,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "Security Audit System" },
                { "role": "user", "content": "Analyze authorization endpoints" },
                {
                    "role": "assistant",
                    "content": "Probing backup endpoints",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "run_command",
                                "arguments": "{\"cmd\": \"curl -s http://sentinel-target:3000/api/backup -H 'Authorization: Bearer sps_session=xyz123'\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "name": "run_command",
                    "content": "HTTP/1.1 401 Unauthorized\n{\"error\": \"Invalid token_version for CVE-2026-9999\"}\nMore lines\n".repeat(20)
                },
                { "role": "user", "content": "Next step user" },
                { "role": "assistant", "content": "Next step reply" }
            ]
        });

        let stats = condenser.rolling_compact(&mut body);
        assert!(stats.is_some());

        let msgs = body["messages"].as_array().unwrap();
        let checkpoint = msgs[1]["content"].as_str().unwrap();

        assert!(checkpoint.contains("[Preserved Security Discoveries & Entities]"));
        assert!(checkpoint.contains("http://sentinel-target:3000/api/backup") || checkpoint.contains("/api/backup"));
        assert!(checkpoint.contains("sps_session") || checkpoint.contains("token_version"));
        assert!(checkpoint.contains("CVE-2026-9999"));
        assert!(checkpoint.contains("⚠️ Signal:"));
    }

    #[test]
    fn test_repeated_rolling_compaction_merges_checkpoints_cleanly() {
        let config = CondenserConfig {
            rolling_compaction: true,
            compact_threshold_tokens: 50,
            compact_keep_recent_turns: 1,
            ..Default::default()
        };
        let condenser = ContextCondenser::new(config);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "You are a scanner." },
                { "role": "user", "content": "First audit step" },
                { "role": "assistant", "content": "Initial probing" },
                { "role": "tool", "name": "scan", "content": "First tool output findings\n".repeat(30) },
                { "role": "user", "content": "Recent active user request" },
                { "role": "assistant", "content": "Recent active assistant reply" }
            ]
        });

        let s1 = condenser.rolling_compact(&mut body);
        assert!(s1.is_some());

        let msgs = body["messages"].as_array_mut().unwrap();
        msgs.push(json!({ "role": "user", "content": "Second audit step" }));
        msgs.push(json!({ "role": "assistant", "content": "Second probing phase" }));
        msgs.push(json!({ "role": "tool", "name": "scan2", "content": "Second tool output findings\n".repeat(30) }));
        msgs.push(json!({ "role": "user", "content": "Latest user request" }));
        msgs.push(json!({ "role": "assistant", "content": "Latest assistant reply" }));

        let s2 = condenser.rolling_compact(&mut body);
        assert!(s2.is_some());

        let final_msgs = body["messages"].as_array().unwrap();
        assert_eq!(final_msgs[0]["role"], "system");
        assert_eq!(final_msgs[1]["role"], "user");

        let checkpoint = final_msgs[1]["content"].as_str().unwrap();
        assert!(!checkpoint.contains("• [User Request]: [Historical Context Checkpoint"));
        assert!(checkpoint.contains("First audit step"));
        assert!(checkpoint.contains("Second audit step"));

        let tool_anchors = extract_tool_anchors("// Note: user cannot modify /api/cards without CVE-2026-1111");
        assert!(!tool_anchors.iter().any(|a| a.eq_ignore_ascii_case("cannot")));
        assert!(tool_anchors.iter().any(|a| a == "/api/cards"));
        assert!(tool_anchors.iter().any(|a| a == "CVE-2026-1111"));
    }

    #[test]
    fn test_loop_nudge_3x_identical_failing_calls_tail_only_across_formats() {
        // 1a. Diagnostic failure signal (even with non-identical timestamps across attempts) + JSON key-order normalization
        let tc1 = json!([{ "id": "c1", "type": "function", "function": { "name": "run_command", "arguments": "{\"cmd\": \"pytest tests/\", \"timeout\": 30}" } }]);
        let tc2 = json!([{ "id": "c2", "type": "function", "function": { "name": "run_command", "arguments": "{\n  \"timeout\": 30,\n  \"cmd\": \"pytest tests/\"\n}" } }]);
        let tc3 = json!([{ "id": "c3", "type": "function", "function": { "name": "run_command", "arguments": "{\"cmd\":\"pytest tests/\",\"timeout\":30}" } }]);

        let mut body = json!({
            "messages": [
                { "role": "system", "content": "You are an autonomous agent." },
                { "role": "user", "content": "Run tests" },
                { "role": "assistant", "content": null, "tool_calls": tc1 },
                { "role": "tool", "tool_call_id": "c1", "content": "test result: test failed in 0.11s\nAssertionError: 1 != 2" },
                { "role": "assistant", "content": null, "tool_calls": tc2 },
                { "role": "tool", "tool_call_id": "c2", "content": "test result: test failed in 0.14s\nAssertionError: 1 != 2" },
                { "role": "assistant", "content": null, "tool_calls": tc3 },
                { "role": "tool", "tool_call_id": "c3", "content": "test result: test failed in 0.12s\nAssertionError: 1 != 2" }
            ]
        });
        let snapshot_before = body["messages"].as_array().unwrap()[..7].to_vec();

        assert!(nudge_repeated_tool_failure(&mut body));

        let msgs = body["messages"].as_array().unwrap();
        // Historical messages (< len - 1) and all assistant.tool_calls are 100% unmutated
        assert_eq!(&msgs[..7], snapshot_before.as_slice());
        assert_eq!(msgs[2]["tool_calls"], tc1);
        assert_eq!(msgs[4]["tool_calls"], tc2);
        assert_eq!(msgs[6]["tool_calls"], tc3);
        assert_eq!(
            msgs[7]["content"].as_str().unwrap(),
            format!("test result: test failed in 0.12s\nAssertionError: 1 != 2{LOOP_GUARD_SUFFIX}")
        );

        // 1b. Read-tool error prefix per classifier.rs:611-615 on `output` string format (`function_call_output`)
        let mut body_output = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "r1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"/missing.txt\"}" } }] },
                { "type": "function_call_output", "call_id": "r1", "output": "File not found: /missing.txt (attempt 1)" },
                { "role": "assistant", "tool_calls": [{ "id": "r2", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"/missing.txt\"}" } }] },
                { "type": "function_call_output", "call_id": "r2", "output": "File not found: /missing.txt (attempt 2)" },
                { "role": "assistant", "tool_calls": [{ "id": "r3", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"/missing.txt\"}" } }] },
                { "type": "function_call_output", "call_id": "r3", "output": "File not found: /missing.txt (attempt 3)" }
            ]
        });
        let hist_output = body_output["messages"].as_array().unwrap()[..5].to_vec();
        assert!(nudge_repeated_tool_failure(&mut body_output));
        let msgs_out = body_output["messages"].as_array().unwrap();
        assert_eq!(&msgs_out[..5], hist_output.as_slice());
        assert_eq!(
            msgs_out[5]["output"].as_str().unwrap(),
            format!("File not found: /missing.txt (attempt 3){LOOP_GUARD_SUFFIX}")
        );

        // 1c. Byte-identical non-empty output (without diagnostic signal) on `content` text-part array
        let mut body_parts = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "p1", "type": "function", "function": { "name": "run_command", "arguments": "{\"cmd\":\"which missing_bin\"}" } }] },
                { "role": "tool", "content": [{ "type": "text", "text": "missing_bin not installed" }] },
                { "role": "assistant", "tool_calls": [{ "id": "p2", "type": "function", "function": { "name": "run_command", "arguments": "{\"cmd\":\"which missing_bin\"}" } }] },
                { "role": "tool", "content": [{ "type": "text", "text": "missing_bin not installed" }] },
                { "role": "assistant", "tool_calls": [{ "id": "p3", "type": "function", "function": { "name": "run_command", "arguments": "{\"cmd\":\"which missing_bin\"}" } }] },
                { "role": "tool", "content": [{ "type": "text", "text": "missing_bin not installed" }] }
            ]
        });
        let hist_parts = body_parts["messages"].as_array().unwrap()[..5].to_vec();
        assert!(nudge_repeated_tool_failure(&mut body_parts));
        let msgs_parts = body_parts["messages"].as_array().unwrap();
        assert_eq!(&msgs_parts[..5], hist_parts.as_slice());
        assert_eq!(
            msgs_parts[5]["content"][0]["text"].as_str().unwrap(),
            format!("missing_bin not installed{LOOP_GUARD_SUFFIX}")
        );
    }

    #[test]
    fn test_loop_nudge_idempotent_no_double_nudge() {
        let mut body = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "content": "error: exit status 1" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "content": "error: exit status 1" },
                { "role": "assistant", "tool_calls": [{ "id": "c3", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "content": "error: exit status 1" }
            ]
        });

        assert!(nudge_repeated_tool_failure(&mut body));
        let after_first = body.clone();

        // Second invocation must be a no-op and return false
        assert!(!nudge_repeated_tool_failure(&mut body));
        assert_eq!(body, after_first);
        let tail = body["messages"][5]["content"].as_str().unwrap();
        assert_eq!(tail.matches(LOOP_GUARD_MARKER).count(), 1);
    }

    #[test]
    fn test_loop_nudge_same_tool_different_arguments_does_not_nudge() {
        let mut body = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" } }] },
                { "role": "tool", "content": "File not found" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"b.txt\"}" } }] },
                { "role": "tool", "content": "File not found" },
                { "role": "assistant", "tool_calls": [{ "id": "c3", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"c.txt\"}" } }] },
                { "role": "tool", "content": "File not found" }
            ]
        });
        let orig = body.clone();
        assert!(!nudge_repeated_tool_failure(&mut body));
        assert_eq!(body, orig);
    }

    #[test]
    fn test_loop_nudge_only_2_consecutive_failures_does_not_nudge() {
        let mut body = json!({
            "messages": [
                { "role": "system", "content": "System" },
                { "role": "user", "content": "Run" },
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"curl http://localhost:9999\"}" } }] },
                { "role": "tool", "content": "error: connection refused" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"curl http://localhost:9999\"}" } }] },
                { "role": "tool", "content": "error: connection refused" }
            ]
        });
        let orig = body.clone();
        assert!(!nudge_repeated_tool_failure(&mut body));
        assert_eq!(body, orig);
    }

    #[test]
    fn test_loop_nudge_changing_non_error_outputs_does_not_nudge() {
        let mut body = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "poll_job", "arguments": "{\"job_id\":\"42\"}" } }] },
                { "role": "tool", "content": "pending 1/3" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "poll_job", "arguments": "{\"job_id\":\"42\"}" } }] },
                { "role": "tool", "content": "pending 2/3" },
                { "role": "assistant", "tool_calls": [{ "id": "c3", "type": "function", "function": { "name": "poll_job", "arguments": "{\"job_id\":\"42\"}" } }] },
                { "role": "tool", "content": "ready 3/3" }
            ]
        });
        let orig = body.clone();
        assert!(!nudge_repeated_tool_failure(&mut body));
        assert_eq!(body, orig);
    }

    #[test]
    fn test_loop_nudge_preserves_json_minification_and_suffix_in_condense() {
        let condenser = ContextCondenser::new(CondenserConfig::default());
        let pretty_json_err = "{\n  \"error\": \"status 500\",\n  \"detail\": \"upstream gateway failure\"\n}";
        let mut body = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "j1", "type": "function", "function": { "name": "http_get", "arguments": "{\"url\":\"http://api.local/health\"}" } }] },
                { "role": "tool", "tool_call_id": "j1", "content": pretty_json_err },
                { "role": "assistant", "tool_calls": [{ "id": "j2", "type": "function", "function": { "name": "http_get", "arguments": "{\"url\":\"http://api.local/health\"}" } }] },
                { "role": "tool", "tool_call_id": "j2", "content": pretty_json_err },
                { "role": "assistant", "tool_calls": [{ "id": "j3", "type": "function", "function": { "name": "http_get", "arguments": "{\"url\":\"http://api.local/health\"}" } }] },
                { "role": "tool", "tool_call_id": "j3", "content": pretty_json_err }
            ]
        });

        assert!(nudge_repeated_tool_failure(&mut body));
        let stats = condenser.condense(&mut body);
        assert!(stats.json_minified_count >= 1);

        let tail = body["messages"][5]["content"].as_str().unwrap();
        assert!(tail.ends_with(LOOP_GUARD_SUFFIX));
        let prefix = tail.strip_suffix(LOOP_GUARD_SUFFIX).unwrap();
        assert!(!prefix.contains('\n'));
        assert!(prefix.contains("\"error\":\"status 500\""));
    }

    #[test]
    fn test_loop_nudge_preserves_lockfile_diff_pruning_and_suffix_in_condense() {
        let condenser = ContextCondenser::new(CondenserConfig::default());
        let mut diff_lines = vec![
            "diff --git a/Cargo.lock b/Cargo.lock".to_string(),
            "--- a/Cargo.lock".to_string(),
            "+++ b/Cargo.lock".to_string(),
            "@@ -1,15 +1,15 @@".to_string(),
        ];
        for i in 0..15 {
            diff_lines.push(format!("+checksum-{i} = \"deadbeef\""));
        }
        let lockfile_diff = diff_lines.join("\n");

        let mut body = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "d1", "type": "function", "function": { "name": "git_diff", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "d1", "content": lockfile_diff },
                { "role": "assistant", "tool_calls": [{ "id": "d2", "type": "function", "function": { "name": "git_diff", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "d2", "content": lockfile_diff },
                { "role": "assistant", "tool_calls": [{ "id": "d3", "type": "function", "function": { "name": "git_diff", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "d3", "content": lockfile_diff }
            ]
        });

        assert!(nudge_repeated_tool_failure(&mut body));
        let stats = condenser.condense(&mut body);
        assert!(stats.lockfile_diffs_pruned_count >= 1);

        let tail = body["messages"][5]["content"].as_str().unwrap();
        assert!(tail.contains("[... Ferry Proxy: lockfile/asset diff suppressed"));
        assert!(tail.ends_with(LOOP_GUARD_SUFFIX));
    }

    #[test]
    fn test_loop_nudge_preserves_suffix_under_large_and_small_truncation_budgets() {
        // 1. Default condenser (> 24,000 char failing output)
        let condenser_default = ContextCondenser::new(CondenserConfig::default());
        let mut big_lines = vec!["error: build failed with multiple diagnostics".to_string()];
        for i in 0..380 {
            big_lines.push(format!(
                "log entry {i:04}: compiler diagnostic frame output padding {}",
                "x".repeat(50)
            ));
        }
        let big_err = big_lines.join("\n");
        assert!(big_err.len() > 24_000);

        let mut body_big = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "b1", "type": "function", "function": { "name": "cargo_build", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "b1", "content": big_err },
                { "role": "assistant", "tool_calls": [{ "id": "b2", "type": "function", "function": { "name": "cargo_build", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "b2", "content": big_err },
                { "role": "assistant", "tool_calls": [{ "id": "b3", "type": "function", "function": { "name": "cargo_build", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "b3", "content": big_err }
            ]
        });

        assert!(nudge_repeated_tool_failure(&mut body_big));
        let stats_big = condenser_default.condense(&mut body_big);
        assert!(stats_big.tool_outputs_truncated >= 1);
        let tail_big = body_big["messages"][5]["content"].as_str().unwrap();
        assert!(tail_big.ends_with(LOOP_GUARD_SUFFIX));

        // 2. Small `recent_max_tool_chars = 200` boundary
        let small_cfg = CondenserConfig {
            recent_max_tool_chars: 200,
            ..CondenserConfig::default()
        };
        let condenser_small = ContextCondenser::new(small_cfg);
        let mut med_lines = vec!["error: command exited with status 1".to_string()];
        for i in 0..80 {
            med_lines.push(format!("frame {i:03}: stack trace detail line"));
        }
        let med_err = med_lines.join("\n");
        assert!(med_err.len() > 200);

        let mut body_small = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "s1", "type": "function", "function": { "name": "run_cmd", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "s1", "content": med_err },
                { "role": "assistant", "tool_calls": [{ "id": "s2", "type": "function", "function": { "name": "run_cmd", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "s2", "content": med_err },
                { "role": "assistant", "tool_calls": [{ "id": "s3", "type": "function", "function": { "name": "run_cmd", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "s3", "content": med_err }
            ]
        });

        assert!(nudge_repeated_tool_failure(&mut body_small));
        let stats_small = condenser_small.condense(&mut body_small);
        assert!(stats_small.tool_outputs_truncated >= 1);
        let tail_small = body_small["messages"][5]["content"].as_str().unwrap();
        assert!(tail_small.ends_with(LOOP_GUARD_SUFFIX));
    }

    #[test]
    fn test_loop_nudge_mismatched_tool_call_id_and_partial_parallel_batch_do_not_nudge() {
        // 1. Mismatched `tool_call_id` ("c1" vs "c99")
        let mut body_mismatch = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c1", "content": "error: exit status 1" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c2", "content": "error: exit status 1" },
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c99", "content": "error: exit status 1" }
            ]
        });
        let orig_mismatch = body_mismatch.clone();
        assert!(!nudge_repeated_tool_failure(&mut body_mismatch));
        assert_eq!(body_mismatch, orig_mismatch);

        // 2. Partial parallel batch (2 `tool_calls` + only 1 `tool` message)
        let two_calls = json!([
            { "id": "p1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"a.rs\"}" } },
            { "id": "p2", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"b.rs\"}" } }
        ]);
        let mut body_partial = json!({
            "messages": [
                { "role": "assistant", "tool_calls": two_calls },
                { "role": "tool", "tool_call_id": "p1", "content": "File not found" },
                { "role": "assistant", "tool_calls": two_calls },
                { "role": "tool", "tool_call_id": "p1", "content": "File not found" },
                { "role": "assistant", "tool_calls": two_calls },
                { "role": "tool", "tool_call_id": "p1", "content": "File not found" }
            ]
        });
        let orig_partial = body_partial.clone();
        assert!(!nudge_repeated_tool_failure(&mut body_partial));
        assert_eq!(body_partial, orig_partial);
    }

    #[test]
    fn test_loop_nudge_3x_identical_failing_parallel_tuple_nudges_tail_only() {
        let mut body = json!({
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "a1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"x.rs\"}" } },
                        { "id": "b1", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"y.rs\"}" } }
                    ]
                },
                { "role": "tool", "tool_call_id": "a1", "content": "error: 1" },
                { "role": "tool", "tool_call_id": "b1", "content": "error: 2" },
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "a2", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"x.rs\"}" } },
                        { "id": "b2", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"y.rs\"}" } }
                    ]
                },
                { "role": "tool", "tool_call_id": "a2", "content": "error: 1" },
                { "role": "tool", "tool_call_id": "b2", "content": "error: 2" },
                {
                    "role": "assistant",
                    "tool_calls": [
                        { "id": "a3", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"x.rs\"}" } },
                        { "id": "b3", "type": "function", "function": { "name": "read_file", "arguments": "{\"path\":\"y.rs\"}" } }
                    ]
                },
                { "role": "tool", "tool_call_id": "a3", "content": "error: 1" },
                { "role": "tool", "tool_call_id": "b3", "content": "error: 2" }
            ]
        });
        let snapshot_before_tail = body["messages"].as_array().unwrap()[..8].to_vec();

        assert!(nudge_repeated_tool_failure(&mut body));

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(&msgs[..8], snapshot_before_tail.as_slice());
        assert_eq!(msgs[7]["content"].as_str().unwrap(), "error: 1");
        assert_eq!(
            msgs[8]["content"].as_str().unwrap(),
            format!("error: 2{LOOP_GUARD_SUFFIX}")
        );

        // Idempotent on subsequent pass
        assert!(!nudge_repeated_tool_failure(&mut body));
    }

    #[test]
    fn test_loop_nudge_interleaved_user_and_empty_whitespace_outputs_do_not_nudge() {
        // 1. Interleaved `user` message breaking the 3-turn consecutive streak
        let mut body_interleaved = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c1", "content": "error: exit status 1" },
                { "role": "user", "content": "Try again please" },
                { "role": "assistant", "tool_calls": [{ "id": "c2", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c2", "content": "error: exit status 1" },
                { "role": "assistant", "tool_calls": [{ "id": "c3", "type": "function", "function": { "name": "bash", "arguments": "{\"cmd\":\"false\"}" } }] },
                { "role": "tool", "tool_call_id": "c3", "content": "error: exit status 1" }
            ]
        });
        let orig_interleaved = body_interleaved.clone();
        assert!(!nudge_repeated_tool_failure(&mut body_interleaved));
        assert_eq!(body_interleaved, orig_interleaved);

        // 2. 3x empty outputs
        let mut body_empty = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "e1", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "e1", "content": "" },
                { "role": "assistant", "tool_calls": [{ "id": "e2", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "e2", "content": "" },
                { "role": "assistant", "tool_calls": [{ "id": "e3", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "e3", "content": "" }
            ]
        });
        let orig_empty = body_empty.clone();
        assert!(!nudge_repeated_tool_failure(&mut body_empty));
        assert_eq!(body_empty, orig_empty);

        // 3. 3x whitespace-only outputs
        let mut body_ws = json!({
            "messages": [
                { "role": "assistant", "tool_calls": [{ "id": "w1", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "w1", "content": "   \n\t  " },
                { "role": "assistant", "tool_calls": [{ "id": "w2", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "w2", "content": "   \n\t  " },
                { "role": "assistant", "tool_calls": [{ "id": "w3", "type": "function", "function": { "name": "noop", "arguments": "{}" } }] },
                { "role": "tool", "tool_call_id": "w3", "content": "   \n\t  " }
            ]
        });
        let orig_ws = body_ws.clone();
        assert!(!nudge_repeated_tool_failure(&mut body_ws));
        assert_eq!(body_ws, orig_ws);
    }
}
