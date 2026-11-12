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
            let mut seen_historical_outputs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for idx in 0..cutoff_index {
                let msg = &messages[idx];
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default();
                let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default();
                if role == "tool" || msg_type == "tool_result" {
                    let content_opt = msg.get("content").and_then(|c| c.as_str()).or_else(|| msg.get("output").and_then(|o| o.as_str()));
                    if let Some(content) = content_opt {
                        let trimmed = content.trim();
                        if trimmed.len() >= 25 {
                            *seen_historical_outputs.entry(trimmed.to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }

            if seen_historical_outputs.values().any(|&count| count >= 2) {
                let mut kept_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for idx in 0..cutoff_index {
                    let msg = &mut messages[idx];
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or_default().to_string();
                    let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or_default().to_string();
                    if role == "tool" || msg_type == "tool_result" {
                        let content_opt = msg.get("content").and_then(|c| c.as_str()).or_else(|| msg.get("output").and_then(|o| o.as_str())).map(|s| s.to_string());
                        if let Some(content) = content_opt {
                            let trimmed = content.trim();
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
                                            if obj.contains_key("content") {
                                                obj.insert("content".to_string(), Value::String(folded));
                                            } else if obj.contains_key("output") {
                                                obj.insert("output".to_string(), Value::String(folded));
                                            }
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

        let mut processed = self.clean_tool_text_with_stats(initial_text, stats);

        // Truncation logic
        if self.config.truncate_tool_outputs {
            if is_historical {
                if processed.len() > self.config.max_tool_chars {
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
                    processed = truncate_tool_content_smart(
                        &processed,
                        hist_head,
                        hist_tail,
                        &mut stats.signals_extracted_count,
                    );
                    stats.tool_outputs_truncated += 1;
                }
            } else {
                if self.config.recent_max_tool_chars > 0 && processed.len() > self.config.recent_max_tool_chars {
                    let recent_head = self.config.head_lines.max(30);
                    let recent_tail = self.config.tail_lines.max(20);
                    processed = truncate_tool_content_smart(
                        &processed,
                        recent_head,
                        recent_tail,
                        &mut stats.signals_extracted_count,
                    );
                    stats.tool_outputs_truncated += 1;
                }
            }
        }

        // Permanent invariant: PromptTrim-inspired Constraint Ledger verification (Proof-of-safety)
        if !is_historical {
            let anchors = extract_tool_anchors(initial_text);
            if let Some(missing) = self.find_missing_anchor(&anchors, &processed) {
                warn!(missing_anchor = %missing, "Constraint ledger vetoed compression on recent turn; restored verbatim");
                stats.ledger_vetoed_count += 1;
                processed = initial_text.to_string();
            } else {
                stats.ledger_checks_passed += 1;
            }
        } else {
            stats.ledger_checks_passed += 1;
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

        let has_system = messages.first().and_then(|m| m.get("role")).and_then(|r| r.as_str()) == Some("system");
        let start_idx = if has_system { 1 } else { 0 };

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

        while split_idx > start_idx {
            let role = messages[split_idx].get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "tool" {
                split_idx -= 1;
            } else {
                break;
            }
        }

        if split_idx > start_idx {
            let prev_role = messages[split_idx - 1].get("role").and_then(|r| r.as_str()).unwrap_or("");
            if prev_role == "assistant" && messages[split_idx - 1].get("tool_calls").is_some() {
                split_idx -= 1;
            }
        }

        if split_idx <= start_idx + 1 {
            return None;
        }

        let mut summary_lines = Vec::new();
        let mut preserved_endpoints: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut preserved_auth: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut preserved_anomalies: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let num_compacted = split_idx - start_idx;

        for msg in &messages[start_idx..split_idx] {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            match role {
                "user" => {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        if content.starts_with("[Historical Context Checkpoint:") {
                            for line in content.lines() {
                                let trimmed = line.trim();
                                if trimmed.starts_with("• ") || trimmed.starts_with("└─ ") {
                                    summary_lines.push(trimmed.to_string());
                                }
                            }
                        } else {
                            let snippet: String = content.chars().take(250).collect();
                            summary_lines.push(format!("• [User Request]: {}", snippet.trim()));
                        }
                        collect_discovered_entities(content, &mut preserved_endpoints, &mut preserved_auth, &mut preserved_anomalies);
                    }
                }
                "assistant" => {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        for tc in tool_calls {
                            let name = tc.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str()).unwrap_or("tool");
                            let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|a| a.as_str()).unwrap_or("");

                            let args_snippet = if !args.is_empty() {
                                extract_compact_tool_arguments(args)
                            } else {
                                String::new()
                            };

                            if args_snippet.is_empty() {
                                summary_lines.push(format!("• [Executed Tool]: {}", name));
                            } else {
                                summary_lines.push(format!("• [Executed Tool]: {}({})", name, args_snippet));
                            }

                            collect_discovered_entities(args, &mut preserved_endpoints, &mut preserved_auth, &mut preserved_anomalies);
                        }
                    } else if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        let snippet: String = content.chars().take(250).collect();
                        summary_lines.push(format!("• [Assistant Finding]: {}", snippet.trim()));
                        collect_discovered_entities(content, &mut preserved_endpoints, &mut preserved_auth, &mut preserved_anomalies);
                    }
                }
                "tool" => {
                    let name = msg.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        collect_discovered_entities(content, &mut preserved_endpoints, &mut preserved_auth, &mut preserved_anomalies);

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

        let original_range_chars = count_message_chars(&messages[start_idx..split_idx]);
        if summary_text.len() >= original_range_chars {
            return None;
        }

        let checkpoint_msg = serde_json::json!({
            "role": "user",
            "content": summary_text
        });

        messages.splice(start_idx..split_idx, std::iter::once(checkpoint_msg));

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
        assert_eq!(last_user["content"], "Recent active user request");
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
}
