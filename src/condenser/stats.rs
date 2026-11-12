use serde_json::Value;

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
    pub signals_extracted_count: usize,
    pub loops_collapsed_count: usize,
    pub lockfile_diffs_pruned_count: usize,
    pub base64_blobs_suppressed_count: usize,
    pub dir_traversals_shielded_count: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RollingCompactionStats {
    pub turns_compacted: usize,
    pub chars_saved: usize,
    pub original_chars: usize,
    pub compacted_chars: usize,
}

/// Zero-allocation character counter across all OpenAI-compatible message formats.
/// Traverses existing in-memory JSON values without serializing to strings.
pub fn count_message_chars(messages: &[Value]) -> usize {
    let mut total = 0;
    for msg in messages {
        if let Some(c) = msg.get("content").and_then(|c| c.as_str()) {
            total += c.len();
        } else if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
            for part in arr {
                if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                    total += t.len();
                }
            }
        }
        if let Some(o) = msg.get("output").and_then(|o| o.as_str()) {
            total += o.len();
        }
        if let Some(r) = msg.get("reasoning").and_then(|r| r.as_str()) {
            total += r.len();
        }
        if let Some(rc) = msg.get("reasoning_content").and_then(|rc| rc.as_str()) {
            total += rc.len();
        }
        if let Some(tc) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            for call in tc {
                if let Some(args) = call.get("function").and_then(|f| f.get("arguments")).and_then(|a| a.as_str()) {
                    total += args.len();
                }
            }
        }
    }
    total
}

/// Unified token estimation ratio: 3.5 characters per token for mixed code, tool JSON, and natural text.
pub fn estimate_tokens_from_chars(chars: usize) -> u64 {
    if chars == 0 {
        return 0;
    }
    (chars as f64 / 3.5).ceil() as u64
}

/// Estimates token count for an array of messages using unified 3.5 chars/token ratio.
pub fn estimate_message_tokens(messages: &[Value]) -> u64 {
    let chars = count_message_chars(messages);
    estimate_tokens_from_chars(chars)
}
