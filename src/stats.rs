use serde::Serialize;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Debug, Default)]
pub struct MetricsTracker {
    pub total_requests: AtomicU64,
    pub local_requests: AtomicU64,
    pub cloud_requests: AtomicU64,
    pub spillover_requests: AtomicU64,
    pub failover_requests: AtomicU64,
    pub active_local_slots: AtomicUsize,
    pub total_tokens_est: AtomicU64,
    pub local_tokens_est: AtomicU64,
    pub cloud_tokens_est: AtomicU64,

    // Condensation statistics (Lifetime counters)
    pub condensed_requests: AtomicU64,
    pub chars_saved: AtomicU64,
    pub reasoning_blocks_dropped: AtomicU64,
    pub tool_outputs_truncated: AtomicU64,
    pub ansi_stripped_count: AtomicU64,
    pub progress_cleaned_count: AtomicU64,
    pub lines_deduped_count: AtomicU64,
    pub whitespace_collapsed_count: AtomicU64,
    pub json_minified_count: AtomicU64,

    // Last request gauges (for real-time Grafana observation)
    pub last_prompt_tokens_original: AtomicU64,
    pub last_prompt_tokens_condensed: AtomicU64,
    pub last_chars_saved: AtomicU64,
    pub last_savings_percent: AtomicU64, // scaled by 100, e.g. 2550 = 25.50%
}

#[derive(Debug, Serialize)]
pub struct StatsSnapshot {
    pub total_requests: u64,
    pub local_requests: u64,
    pub cloud_requests: u64,
    pub spillover_requests: u64,
    pub failover_requests: u64,
    pub active_local_slots: usize,
    pub total_tokens_est: u64,
    pub local_tokens_est: u64,
    pub cloud_tokens_est: u64,
    pub local_ratio_percent: f64,

    pub condensed_requests: u64,
    pub chars_saved: u64,
    pub tokens_saved_est: u64,
    pub reasoning_blocks_dropped: u64,
    pub tool_outputs_truncated: u64,
    pub ansi_stripped_count: u64,
    pub progress_cleaned_count: u64,
    pub lines_deduped_count: u64,
    pub whitespace_collapsed_count: u64,
    pub json_minified_count: u64,

    pub last_prompt_tokens_original: u64,
    pub last_prompt_tokens_condensed: u64,
    pub last_chars_saved: u64,
    pub last_savings_percent: f64,
}

impl MetricsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_local(&self, tokens: u64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.local_requests.fetch_add(1, Ordering::Relaxed);
        self.total_tokens_est.fetch_add(tokens, Ordering::Relaxed);
        self.local_tokens_est.fetch_add(tokens, Ordering::Relaxed);
        self.active_local_slots.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_local(&self) {
        self.active_local_slots.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn inc_cloud(&self, tokens: u64, is_spillover: bool) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.cloud_requests.fetch_add(1, Ordering::Relaxed);
        self.total_tokens_est.fetch_add(tokens, Ordering::Relaxed);
        self.cloud_tokens_est.fetch_add(tokens, Ordering::Relaxed);
        if is_spillover {
            self.spillover_requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn inc_failover(&self) {
        self.failover_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_condensation(
        &self,
        orig_tokens: u64,
        cond_tokens: u64,
        reasoning_dropped: usize,
        tools_truncated: usize,
        chars_saved: usize,
        ansi_stripped: usize,
        progress_cleaned: usize,
        lines_deduped: usize,
        whitespace_collapsed: usize,
        json_minified: usize,
    ) {
        self.last_prompt_tokens_original.store(orig_tokens, Ordering::Relaxed);
        self.last_prompt_tokens_condensed.store(cond_tokens, Ordering::Relaxed);
        self.last_chars_saved.store(chars_saved as u64, Ordering::Relaxed);

        let pct = if orig_tokens > 0 && orig_tokens >= cond_tokens {
            let diff = orig_tokens - cond_tokens;
            (diff as f64 / orig_tokens as f64) * 100.0
        } else {
            0.0
        };
        self.last_savings_percent.store((pct * 100.0).round() as u64, Ordering::Relaxed);

        if chars_saved > 0 {
            self.condensed_requests.fetch_add(1, Ordering::Relaxed);
            self.chars_saved.fetch_add(chars_saved as u64, Ordering::Relaxed);
            self.reasoning_blocks_dropped.fetch_add(reasoning_dropped as u64, Ordering::Relaxed);
            self.tool_outputs_truncated.fetch_add(tools_truncated as u64, Ordering::Relaxed);
            self.ansi_stripped_count.fetch_add(ansi_stripped as u64, Ordering::Relaxed);
            self.progress_cleaned_count.fetch_add(progress_cleaned as u64, Ordering::Relaxed);
            self.lines_deduped_count.fetch_add(lines_deduped as u64, Ordering::Relaxed);
            self.whitespace_collapsed_count.fetch_add(whitespace_collapsed as u64, Ordering::Relaxed);
            self.json_minified_count.fetch_add(json_minified as u64, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        let total = self.total_requests.load(Ordering::Relaxed);
        let local = self.local_requests.load(Ordering::Relaxed);
        let ratio = if total > 0 {
            (local as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let chars_saved = self.chars_saved.load(Ordering::Relaxed);
        let pct_scaled = self.last_savings_percent.load(Ordering::Relaxed);

        StatsSnapshot {
            total_requests: total,
            local_requests: local,
            cloud_requests: self.cloud_requests.load(Ordering::Relaxed),
            spillover_requests: self.spillover_requests.load(Ordering::Relaxed),
            failover_requests: self.failover_requests.load(Ordering::Relaxed),
            active_local_slots: self.active_local_slots.load(Ordering::Relaxed),
            total_tokens_est: self.total_tokens_est.load(Ordering::Relaxed),
            local_tokens_est: self.local_tokens_est.load(Ordering::Relaxed),
            cloud_tokens_est: self.cloud_tokens_est.load(Ordering::Relaxed),
            local_ratio_percent: (ratio * 100.0).round() / 100.0,

            condensed_requests: self.condensed_requests.load(Ordering::Relaxed),
            chars_saved,
            tokens_saved_est: chars_saved / 4,
            reasoning_blocks_dropped: self.reasoning_blocks_dropped.load(Ordering::Relaxed),
            tool_outputs_truncated: self.tool_outputs_truncated.load(Ordering::Relaxed),
            ansi_stripped_count: self.ansi_stripped_count.load(Ordering::Relaxed),
            progress_cleaned_count: self.progress_cleaned_count.load(Ordering::Relaxed),
            lines_deduped_count: self.lines_deduped_count.load(Ordering::Relaxed),
            whitespace_collapsed_count: self.whitespace_collapsed_count.load(Ordering::Relaxed),
            json_minified_count: self.json_minified_count.load(Ordering::Relaxed),

            last_prompt_tokens_original: self.last_prompt_tokens_original.load(Ordering::Relaxed),
            last_prompt_tokens_condensed: self.last_prompt_tokens_condensed.load(Ordering::Relaxed),
            last_chars_saved: self.last_chars_saved.load(Ordering::Relaxed),
            last_savings_percent: pct_scaled as f64 / 100.0,
        }
    }

    pub fn prometheus_metrics(&self) -> String {
        let s = self.snapshot();
        format!(
            "# HELP ferry_requests_total Total number of LLM requests\n\
             # TYPE ferry_requests_total counter\n\
             ferry_requests_total {total}\n\
             ferry_requests_total{{provider=\"local\"}} {local}\n\
             ferry_requests_total{{provider=\"cloud\"}} {cloud}\n\
             ferry_requests_total{{type=\"spillover\"}} {spill}\n\
             ferry_requests_total{{type=\"failover\"}} {fail}\n\
             # HELP ferry_active_slots Current active local hardware slots\n\
             # TYPE ferry_active_slots gauge\n\
             ferry_active_slots {slots}\n\
             # HELP ferry_tokens_estimated Total estimated tokens processed\n\
             # TYPE ferry_tokens_estimated counter\n\
             ferry_tokens_estimated {tokens}\n\
             ferry_tokens_estimated{{provider=\"local\"}} {ltokens}\n\
             ferry_tokens_estimated{{provider=\"cloud\"}} {ctokens}\n\
             # HELP ferry_condensed_requests_total Total requests optimized by condensation\n\
             # TYPE ferry_condensed_requests_total counter\n\
             ferry_condensed_requests_total {condensed_reqs}\n\
             # HELP ferry_chars_saved_total Total characters pruned from prompts\n\
             # TYPE ferry_chars_saved_total counter\n\
             ferry_chars_saved_total {chars_saved}\n\
             # HELP ferry_tokens_saved_estimated_total Total tokens estimated saved\n\
             # TYPE ferry_tokens_saved_estimated_total counter\n\
             ferry_tokens_saved_estimated_total {tokens_saved}\n\
             # HELP ferry_condensation_cleaner_actions_total Total actions performed per cleaner filter\n\
             # TYPE ferry_condensation_cleaner_actions_total counter\n\
             ferry_condensation_cleaner_actions_total{{action=\"reasoning_dropped\"}} {reasoning_dropped}\n\
             ferry_condensation_cleaner_actions_total{{action=\"tool_truncated\"}} {tools_truncated}\n\
             ferry_condensation_cleaner_actions_total{{action=\"ansi_stripped\"}} {ansi_stripped}\n\
             ferry_condensation_cleaner_actions_total{{action=\"progress_cleaned\"}} {progress_cleaned}\n\
             ferry_condensation_cleaner_actions_total{{action=\"lines_deduped\"}} {lines_deduped}\n\
             ferry_condensation_cleaner_actions_total{{action=\"whitespace_collapsed\"}} {whitespace_collapsed}\n\
             ferry_condensation_cleaner_actions_total{{action=\"json_minified\"}} {json_minified}\n\
             # HELP ferry_last_prompt_tokens_original Estimated tokens before condensation on most recent request\n\
             # TYPE ferry_last_prompt_tokens_original gauge\n\
             ferry_last_prompt_tokens_original {last_orig}\n\
             # HELP ferry_last_prompt_tokens_condensed Estimated tokens after condensation on most recent request\n\
             # TYPE ferry_last_prompt_tokens_condensed gauge\n\
             ferry_last_prompt_tokens_condensed {last_cond}\n\
             # HELP ferry_last_condensation_savings_percent Percentage of prompt tokens pruned on most recent request\n\
             # TYPE ferry_last_condensation_savings_percent gauge\n\
             ferry_last_condensation_savings_percent {last_pct:.2}\n\
             # HELP ferry_last_condensation_chars_saved Characters pruned on most recent request\n\
             # TYPE ferry_last_condensation_chars_saved gauge\n\
             ferry_last_condensation_chars_saved {last_chars}\n",
            total = s.total_requests,
            local = s.local_requests,
            cloud = s.cloud_requests,
            spill = s.spillover_requests,
            fail = s.failover_requests,
            slots = s.active_local_slots,
            tokens = s.total_tokens_est,
            ltokens = s.local_tokens_est,
            ctokens = s.cloud_tokens_est,
            condensed_reqs = s.condensed_requests,
            chars_saved = s.chars_saved,
            tokens_saved = s.tokens_saved_est,
            reasoning_dropped = s.reasoning_blocks_dropped,
            tools_truncated = s.tool_outputs_truncated,
            ansi_stripped = s.ansi_stripped_count,
            progress_cleaned = s.progress_cleaned_count,
            lines_deduped = s.lines_deduped_count,
            whitespace_collapsed = s.whitespace_collapsed_count,
            json_minified = s.json_minified_count,
            last_orig = s.last_prompt_tokens_original,
            last_cond = s.last_prompt_tokens_condensed,
            last_pct = s.last_savings_percent,
            last_chars = s.last_chars_saved,
        )
    }
}
