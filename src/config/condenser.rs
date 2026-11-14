use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondenserConfig {
    #[serde(default = "default_condensation_enabled")]
    pub enabled: bool,
    #[serde(default = "default_drop_historical_reasoning")]
    pub drop_historical_reasoning: bool,
    #[serde(default = "default_truncate_tool_outputs")]
    pub truncate_tool_outputs: bool,
    #[serde(default = "default_keep_recent_turns")]
    pub keep_recent_turns: usize,
    #[serde(default = "default_max_tool_chars")]
    pub max_tool_chars: usize,
    #[serde(default = "default_recent_max_tool_chars")]
    pub recent_max_tool_chars: usize,
    #[serde(default = "default_head_lines")]
    pub head_lines: usize,
    #[serde(default = "default_tail_lines")]
    pub tail_lines: usize,
    #[serde(default = "default_strip_terminal_noise", alias = "strip_ansi_escapes")]
    pub strip_terminal_noise: bool,
    #[serde(default = "default_deduplicate_lines")]
    pub deduplicate_lines: bool,
    #[serde(default = "default_minify_json")]
    pub minify_json_outputs: bool,
    #[serde(default = "default_prune_lockfile_diffs")]
    pub prune_lockfile_diffs: bool,
    #[serde(default = "default_suppress_base64_blobs")]
    pub suppress_base64_blobs: bool,
    #[serde(default = "default_shield_dir_traversals")]
    pub shield_dir_traversals: bool,
    #[serde(default = "default_fold_repeated_tool_failures")]
    pub fold_repeated_tool_failures: bool,
    #[serde(default = "default_true")]
    pub nudge_repeated_failures: bool,
    #[serde(default = "default_rolling_compaction")]
    pub rolling_compaction: bool,
    #[serde(default = "default_compact_threshold_tokens")]
    pub compact_threshold_tokens: u64,
    #[serde(default = "default_compact_keep_recent_turns")]
    pub compact_keep_recent_turns: usize,
}

fn default_condensation_enabled() -> bool { true }
fn default_drop_historical_reasoning() -> bool { true }
fn default_truncate_tool_outputs() -> bool { true }
fn default_keep_recent_turns() -> usize { 1 }
fn default_max_tool_chars() -> usize { 12000 }
fn default_recent_max_tool_chars() -> usize { 12000 }
fn default_head_lines() -> usize { 15 }
fn default_tail_lines() -> usize { 10 }
fn default_strip_terminal_noise() -> bool { true }
fn default_deduplicate_lines() -> bool { true }
fn default_minify_json() -> bool { true }
fn default_prune_lockfile_diffs() -> bool { true }
fn default_suppress_base64_blobs() -> bool { true }
fn default_shield_dir_traversals() -> bool { true }
fn default_fold_repeated_tool_failures() -> bool { true }
fn default_true() -> bool { true }
fn default_rolling_compaction() -> bool { true }
fn default_compact_threshold_tokens() -> u64 { 36000 }
fn default_compact_keep_recent_turns() -> usize { 4 }

impl Default for CondenserConfig {
    fn default() -> Self {
        Self {
            enabled: default_condensation_enabled(),
            drop_historical_reasoning: default_drop_historical_reasoning(),
            truncate_tool_outputs: default_truncate_tool_outputs(),
            keep_recent_turns: default_keep_recent_turns(),
            max_tool_chars: default_max_tool_chars(),
            recent_max_tool_chars: default_recent_max_tool_chars(),
            head_lines: default_head_lines(),
            tail_lines: default_tail_lines(),
            strip_terminal_noise: default_strip_terminal_noise(),
            deduplicate_lines: default_deduplicate_lines(),
            minify_json_outputs: default_minify_json(),
            prune_lockfile_diffs: default_prune_lockfile_diffs(),
            suppress_base64_blobs: default_suppress_base64_blobs(),
            shield_dir_traversals: default_shield_dir_traversals(),
            fold_repeated_tool_failures: default_fold_repeated_tool_failures(),
            nudge_repeated_failures: default_true(),
            rolling_compaction: default_rolling_compaction(),
            compact_threshold_tokens: default_compact_threshold_tokens(),
            compact_keep_recent_turns: default_compact_keep_recent_turns(),
        }
    }
}
