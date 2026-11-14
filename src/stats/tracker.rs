use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub const DEFAULT_CIRCUIT_COOLDOWN_SECS: u64 = 30;

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug)]
pub struct MetricsTracker {
    pub total_requests: AtomicU64,
    pub local_requests: AtomicU64,
    pub cloud_requests: AtomicU64,
    pub spillover_requests: AtomicU64,
    pub failover_requests: AtomicU64,
    pub active_local_slots: AtomicUsize,
    pub active_local_tokens: AtomicU64,
    pub adaptive_squeezed_count: AtomicU64,
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
    pub ledger_checks_passed: AtomicU64,
    pub ledger_vetoed_count: AtomicU64,
    pub signals_extracted_count: AtomicU64,
    pub loops_collapsed_count: AtomicU64,
    pub lockfile_diffs_pruned_count: AtomicU64,
    pub base64_blobs_suppressed_count: AtomicU64,
    pub dir_traversals_shielded_count: AtomicU64,
    pub loop_nudges_total: AtomicU64,

    // Last request gauges (for real-time Grafana observation)
    pub last_prompt_tokens_original: AtomicU64,
    pub last_prompt_tokens_condensed: AtomicU64,
    pub last_savings_percent: AtomicU64, // scaled by 100, e.g. 2550 = 25.50%
    pub last_chars_saved: AtomicU64,
    // Latency & duration tracking (for Grafana p50/p90/p99 & rate queries)
    pub local_duration_ms: AtomicU64,
    pub cloud_duration_ms: AtomicU64,
    pub last_duration_ms: AtomicU64,

    // Circuit breaker & upstream hardware health
    pub strata_online: AtomicBool,
    pub circuit_open: AtomicBool,
    pub consecutive_local_failures: AtomicU32,
    pub circuit_trips_total: AtomicU64,
    pub circuit_opened_at_ms: AtomicU64,
    pub last_local_failure_ms: AtomicU64,
    pub cloud_rate_limited_until: AtomicU64,

    // Affinity & Rolling Compaction metrics
    pub affinity_hits: AtomicU64,
    pub affinity_misses: AtomicU64,
    pub rolling_compact_count: AtomicU64,
    pub rolling_compact_chars_saved: AtomicU64,

    // Contention Queue metrics
    pub queued_requests: AtomicU64,
    pub queue_wait_ms_total: AtomicU64,
    pub last_queue_wait_ms: AtomicU64,

    // Orchestrator priority metrics
    pub orchestrator_requests: AtomicU64,
    pub orchestrator_spillover: AtomicU64,

    // Sentinel Pillar metrics
    pub security_requests: AtomicU64,
    pub fuzzer_requests: AtomicU64,
    pub explorer_requests: AtomicU64,
}

impl Default for MetricsTracker {
    fn default() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            local_requests: AtomicU64::new(0),
            cloud_requests: AtomicU64::new(0),
            spillover_requests: AtomicU64::new(0),
            failover_requests: AtomicU64::new(0),
            active_local_slots: AtomicUsize::new(0),
            active_local_tokens: AtomicU64::new(0),
            adaptive_squeezed_count: AtomicU64::new(0),
            total_tokens_est: AtomicU64::new(0),
            local_tokens_est: AtomicU64::new(0),
            cloud_tokens_est: AtomicU64::new(0),

            condensed_requests: AtomicU64::new(0),
            chars_saved: AtomicU64::new(0),
            reasoning_blocks_dropped: AtomicU64::new(0),
            tool_outputs_truncated: AtomicU64::new(0),
            ansi_stripped_count: AtomicU64::new(0),
            progress_cleaned_count: AtomicU64::new(0),
            lines_deduped_count: AtomicU64::new(0),
            whitespace_collapsed_count: AtomicU64::new(0),
            json_minified_count: AtomicU64::new(0),
            ledger_checks_passed: AtomicU64::new(0),
            ledger_vetoed_count: AtomicU64::new(0),
            signals_extracted_count: AtomicU64::new(0),
            loops_collapsed_count: AtomicU64::new(0),
            lockfile_diffs_pruned_count: AtomicU64::new(0),
            base64_blobs_suppressed_count: AtomicU64::new(0),
            dir_traversals_shielded_count: AtomicU64::new(0),
            loop_nudges_total: AtomicU64::new(0),

            last_prompt_tokens_original: AtomicU64::new(0),
            last_prompt_tokens_condensed: AtomicU64::new(0),
            last_chars_saved: AtomicU64::new(0),
            last_savings_percent: AtomicU64::new(0),

            local_duration_ms: AtomicU64::new(0),
            cloud_duration_ms: AtomicU64::new(0),
            last_duration_ms: AtomicU64::new(0),

            strata_online: AtomicBool::new(true),
            circuit_open: AtomicBool::new(false),
            consecutive_local_failures: AtomicU32::new(0),
            circuit_trips_total: AtomicU64::new(0),
            circuit_opened_at_ms: AtomicU64::new(0),
            last_local_failure_ms: AtomicU64::new(0),
            cloud_rate_limited_until: AtomicU64::new(0),

            affinity_hits: AtomicU64::new(0),
            affinity_misses: AtomicU64::new(0),
            rolling_compact_count: AtomicU64::new(0),
            rolling_compact_chars_saved: AtomicU64::new(0),

            queued_requests: AtomicU64::new(0),
            queue_wait_ms_total: AtomicU64::new(0),
            last_queue_wait_ms: AtomicU64::new(0),

            orchestrator_requests: AtomicU64::new(0),
            orchestrator_spillover: AtomicU64::new(0),

            security_requests: AtomicU64::new(0),
            fuzzer_requests: AtomicU64::new(0),
            explorer_requests: AtomicU64::new(0),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatsSnapshot {
    pub total_requests: u64,
    pub local_requests: u64,
    pub cloud_requests: u64,
    pub spillover_requests: u64,
    pub failover_requests: u64,
    pub active_local_slots: usize,
    pub active_local_tokens: u64,
    pub adaptive_squeezed_count: u64,
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
    pub ledger_checks_passed: u64,
    pub ledger_vetoed_count: u64,
    pub signals_extracted_count: u64,
    pub loops_collapsed_count: u64,
    pub lockfile_diffs_pruned_count: u64,
    pub base64_blobs_suppressed_count: u64,
    pub dir_traversals_shielded_count: u64,
    pub loop_nudges_total: u64,

    pub last_prompt_tokens_original: u64,
    pub last_prompt_tokens_condensed: u64,
    pub last_chars_saved: u64,
    pub last_savings_percent: f64,

    pub local_duration_ms_total: u64,
    pub cloud_duration_ms_total: u64,
    pub last_duration_ms: u64,

    pub strata_online: bool,
    pub circuit_open: bool,
    pub consecutive_local_failures: u32,
    pub circuit_trips_total: u64,
    pub cloud_rate_limited: bool,

    pub affinity_hits: u64,
    pub affinity_misses: u64,
    pub rolling_compact_count: u64,
    pub rolling_compact_chars_saved: u64,

    pub queued_requests_total: u64,
    pub queue_wait_ms_total: u64,
    pub last_queue_wait_ms: u64,
    pub avg_queue_wait_ms: f64,

    pub orchestrator_requests_total: u64,
    pub orchestrator_spillover_total: u64,

    pub security_requests_total: u64,
    pub fuzzer_requests_total: u64,
    pub explorer_requests_total: u64,
}

impl MetricsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire_local_slot(&self, max_slots: usize) -> bool {
        self.try_acquire_local_capacity(0, max_slots, 0)
    }

    pub fn try_acquire_local_capacity(&self, tokens: u64, max_slots: usize, max_token_budget: u64) -> bool {
        if max_slots == 0 || self.active_local_slots.load(Ordering::SeqCst) >= max_slots {
            return false;
        }

        // 1. Atomically reserve token capacity first so concurrent callers cannot oversubscribe max_token_budget
        if tokens > 0 {
            let mut curr_tokens = self.active_local_tokens.load(Ordering::SeqCst);
            loop {
                let Some(next_tokens) = curr_tokens.checked_add(tokens) else {
                    return false;
                };
                if max_token_budget > 0 && next_tokens > max_token_budget {
                    return false;
                }
                match self.active_local_tokens.compare_exchange_weak(
                    curr_tokens,
                    next_tokens,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => curr_tokens = actual,
                }
            }
        }

        // 2. Atomically reserve slot; roll back reserved tokens cleanly if slots saturated concurrently
        let mut curr_slots = self.active_local_slots.load(Ordering::SeqCst);
        loop {
            if curr_slots >= max_slots {
                if tokens > 0 {
                    let mut curr = self.active_local_tokens.load(Ordering::SeqCst);
                    while curr > 0 {
                        let next = curr.saturating_sub(tokens);
                        match self.active_local_tokens.compare_exchange_weak(
                            curr,
                            next,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        ) {
                            Ok(_) => break,
                            Err(actual) => curr = actual,
                        }
                    }
                }
                return false;
            }
            match self.active_local_slots.compare_exchange_weak(
                curr_slots,
                curr_slots + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => curr_slots = actual,
            }
        }
    }

    pub fn release_local_capacity(&self, tokens: u64) {
        self.dec_local();
        let mut curr = self.active_local_tokens.load(Ordering::SeqCst);
        while curr > 0 {
            let next = curr.saturating_sub(tokens);
            match self.active_local_tokens.compare_exchange_weak(
                curr,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => curr = actual,
            }
        }
    }

    pub fn record_local_request_stats(&self, tokens: u64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.local_requests.fetch_add(1, Ordering::Relaxed);
        self.total_tokens_est.fetch_add(tokens, Ordering::Relaxed);
        self.local_tokens_est.fetch_add(tokens, Ordering::Relaxed);
    }

    pub fn inc_local(&self, tokens: u64) {
        self.record_local_request_stats(tokens);
        self.active_local_slots.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_local(&self) {
        let mut curr = self.active_local_slots.load(Ordering::Relaxed);
        while curr > 0 {
            match self.active_local_slots.compare_exchange_weak(
                curr,
                curr - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => curr = actual,
            }
        }
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

    pub fn record_local_completion(&self, duration_ms: u128) {
        self.local_duration_ms.fetch_add(duration_ms as u64, Ordering::Relaxed);
        self.last_duration_ms.store(duration_ms as u64, Ordering::Relaxed);
    }

    pub fn record_cloud_completion(&self, duration_ms: u128) {
        self.cloud_duration_ms.fetch_add(duration_ms as u64, Ordering::Relaxed);
        self.last_duration_ms.store(duration_ms as u64, Ordering::Relaxed);
    }

    pub fn record_local_success(&self) {
        self.consecutive_local_failures.store(0, Ordering::Relaxed);
        self.circuit_opened_at_ms.store(0, Ordering::Relaxed);
        self.last_local_failure_ms.store(0, Ordering::Relaxed);
        self.circuit_open.store(false, Ordering::Relaxed);
        self.strata_online.store(true, Ordering::Relaxed);
    }

    pub fn record_local_failure(&self, threshold: u32) -> bool {
        let now = now_epoch_ms();
        self.last_local_failure_ms.store(now, Ordering::Relaxed);
        let prev = self.consecutive_local_failures.fetch_add(1, Ordering::Relaxed);
        let current = prev + 1;

        if current >= threshold {
            self.circuit_opened_at_ms.store(now, Ordering::Relaxed);
            self.strata_online.store(false, Ordering::Relaxed);
            let was_open = self.circuit_open.swap(true, Ordering::Relaxed);
            if !was_open {
                self.circuit_trips_total.fetch_add(1, Ordering::Relaxed);
                return true; // Tripped just now
            }
        }
        false
    }

    /// Marks Strata reachable on `/health` 200 OK. Closes an open circuit breaker only if
    /// no active inference completion failures occurred within `cooldown_secs`.
    pub fn record_health_probe_success(&self, cooldown_secs: u64) -> bool {
        let was_offline = !self.strata_online.swap(true, Ordering::Relaxed);
        let cooldown_ms = cooldown_secs.saturating_mul(1000);
        let recent_fail_ts = self
            .circuit_opened_at_ms
            .load(Ordering::Relaxed)
            .max(self.last_local_failure_ms.load(Ordering::Relaxed));

        if recent_fail_ts > 0 && now_epoch_ms().saturating_sub(recent_fail_ts) < cooldown_ms {
            return false;
        }

        self.consecutive_local_failures.store(0, Ordering::Relaxed);
        self.circuit_opened_at_ms.store(0, Ordering::Relaxed);
        self.last_local_failure_ms.store(0, Ordering::Relaxed);
        let was_open = self.circuit_open.swap(false, Ordering::Relaxed);
        was_open || was_offline
    }

    /// Marks Strata offline when `/health` fails or returns non-2xx.
    pub fn record_health_probe_failure(&self) -> bool {
        let was_online = self.strata_online.swap(false, Ordering::Relaxed);
        self.circuit_open.store(true, Ordering::Relaxed);
        was_online
    }

    pub fn set_strata_status(&self, online: bool) {
        self.strata_online.store(online, Ordering::Relaxed);
        if !online {
            self.circuit_open.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_circuit_open(&self) -> bool {
        self.circuit_open.load(Ordering::Relaxed)
    }

    pub fn is_strata_online(&self) -> bool {
        self.strata_online.load(Ordering::Relaxed)
    }

    pub fn record_cloud_rate_limit(&self, backoff_secs: u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.cloud_rate_limited_until.store(now + backoff_secs, Ordering::Relaxed);
    }

    pub fn is_cloud_rate_limited(&self) -> bool {
        let until = self.cloud_rate_limited_until.load(Ordering::Relaxed);
        if until == 0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now < until
    }

    pub fn clear_cloud_rate_limit(&self) {
        self.cloud_rate_limited_until.store(0, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
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
        ledger_passed: usize,
        ledger_vetoed: usize,
        signals_extracted: usize,
        loops_collapsed: usize,
        lockfile_diffs_pruned: usize,
        base64_blobs_suppressed: usize,
        dir_traversals_shielded: usize,
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
        }
        if reasoning_dropped > 0 {
            self.reasoning_blocks_dropped.fetch_add(reasoning_dropped as u64, Ordering::Relaxed);
        }
        if tools_truncated > 0 {
            self.tool_outputs_truncated.fetch_add(tools_truncated as u64, Ordering::Relaxed);
        }
        if ansi_stripped > 0 {
            self.ansi_stripped_count.fetch_add(ansi_stripped as u64, Ordering::Relaxed);
        }
        if progress_cleaned > 0 {
            self.progress_cleaned_count.fetch_add(progress_cleaned as u64, Ordering::Relaxed);
        }
        if lines_deduped > 0 {
            self.lines_deduped_count.fetch_add(lines_deduped as u64, Ordering::Relaxed);
        }
        if whitespace_collapsed > 0 {
            self.whitespace_collapsed_count.fetch_add(whitespace_collapsed as u64, Ordering::Relaxed);
        }
        if json_minified > 0 {
            self.json_minified_count.fetch_add(json_minified as u64, Ordering::Relaxed);
        }
        self.ledger_checks_passed.fetch_add(ledger_passed as u64, Ordering::Relaxed);
        self.ledger_vetoed_count.fetch_add(ledger_vetoed as u64, Ordering::Relaxed);
        if signals_extracted > 0 {
            self.signals_extracted_count.fetch_add(signals_extracted as u64, Ordering::Relaxed);
        }
        if loops_collapsed > 0 {
            self.loops_collapsed_count.fetch_add(loops_collapsed as u64, Ordering::Relaxed);
        }
        if lockfile_diffs_pruned > 0 {
            self.lockfile_diffs_pruned_count.fetch_add(lockfile_diffs_pruned as u64, Ordering::Relaxed);
        }
        if base64_blobs_suppressed > 0 {
            self.base64_blobs_suppressed_count.fetch_add(base64_blobs_suppressed as u64, Ordering::Relaxed);
        }
        if dir_traversals_shielded > 0 {
            self.dir_traversals_shielded_count.fetch_add(dir_traversals_shielded as u64, Ordering::Relaxed);
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

        let queued = self.queued_requests.load(Ordering::Relaxed);
        let queue_wait_total = self.queue_wait_ms_total.load(Ordering::Relaxed);
        let last_queue_wait = self.last_queue_wait_ms.load(Ordering::Relaxed);
        let avg_queue_wait = if queued > 0 {
            (queue_wait_total as f64 / queued as f64 * 100.0).round() / 100.0
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
            active_local_tokens: self.active_local_tokens.load(Ordering::Relaxed),
            adaptive_squeezed_count: self.adaptive_squeezed_count.load(Ordering::Relaxed),
            total_tokens_est: self.total_tokens_est.load(Ordering::Relaxed),
            local_tokens_est: self.local_tokens_est.load(Ordering::Relaxed),
            cloud_tokens_est: self.cloud_tokens_est.load(Ordering::Relaxed),
            local_ratio_percent: (ratio * 100.0).round() / 100.0,

            condensed_requests: self.condensed_requests.load(Ordering::Relaxed),
            chars_saved,
            tokens_saved_est: crate::condenser::estimate_tokens_from_chars(chars_saved as usize),
            reasoning_blocks_dropped: self.reasoning_blocks_dropped.load(Ordering::Relaxed),
            tool_outputs_truncated: self.tool_outputs_truncated.load(Ordering::Relaxed),
            ansi_stripped_count: self.ansi_stripped_count.load(Ordering::Relaxed),
            progress_cleaned_count: self.progress_cleaned_count.load(Ordering::Relaxed),
            lines_deduped_count: self.lines_deduped_count.load(Ordering::Relaxed),
            whitespace_collapsed_count: self.whitespace_collapsed_count.load(Ordering::Relaxed),
            json_minified_count: self.json_minified_count.load(Ordering::Relaxed),
            ledger_checks_passed: self.ledger_checks_passed.load(Ordering::Relaxed),
            ledger_vetoed_count: self.ledger_vetoed_count.load(Ordering::Relaxed),
            signals_extracted_count: self.signals_extracted_count.load(Ordering::Relaxed),
            loops_collapsed_count: self.loops_collapsed_count.load(Ordering::Relaxed),
            lockfile_diffs_pruned_count: self.lockfile_diffs_pruned_count.load(Ordering::Relaxed),
            base64_blobs_suppressed_count: self.base64_blobs_suppressed_count.load(Ordering::Relaxed),
            dir_traversals_shielded_count: self.dir_traversals_shielded_count.load(Ordering::Relaxed),
            loop_nudges_total: self.loop_nudges_total.load(Ordering::Relaxed),

            last_prompt_tokens_original: self.last_prompt_tokens_original.load(Ordering::Relaxed),
            last_prompt_tokens_condensed: self.last_prompt_tokens_condensed.load(Ordering::Relaxed),
            last_chars_saved: self.last_chars_saved.load(Ordering::Relaxed),
            last_savings_percent: pct_scaled as f64 / 100.0,

            local_duration_ms_total: self.local_duration_ms.load(Ordering::Relaxed),
            cloud_duration_ms_total: self.cloud_duration_ms.load(Ordering::Relaxed),
            last_duration_ms: self.last_duration_ms.load(Ordering::Relaxed),

            strata_online: self.strata_online.load(Ordering::Relaxed),
            circuit_open: self.circuit_open.load(Ordering::Relaxed),
            consecutive_local_failures: self.consecutive_local_failures.load(Ordering::Relaxed),
            circuit_trips_total: self.circuit_trips_total.load(Ordering::Relaxed),
            cloud_rate_limited: self.is_cloud_rate_limited(),

            affinity_hits: self.affinity_hits.load(Ordering::Relaxed),
            affinity_misses: self.affinity_misses.load(Ordering::Relaxed),
            rolling_compact_count: self.rolling_compact_count.load(Ordering::Relaxed),
            rolling_compact_chars_saved: self.rolling_compact_chars_saved.load(Ordering::Relaxed),

            queued_requests_total: queued,
            queue_wait_ms_total: queue_wait_total,
            last_queue_wait_ms: last_queue_wait,
            avg_queue_wait_ms: avg_queue_wait,

            orchestrator_requests_total: self.orchestrator_requests.load(Ordering::Relaxed),
            orchestrator_spillover_total: self.orchestrator_spillover.load(Ordering::Relaxed),

            security_requests_total: self.security_requests.load(Ordering::Relaxed),
            fuzzer_requests_total: self.fuzzer_requests.load(Ordering::Relaxed),
            explorer_requests_total: self.explorer_requests.load(Ordering::Relaxed),
        }
    }

    pub fn prometheus_metrics(&self) -> String {
        super::prometheus::render_prometheus(&self.snapshot())
    }

    pub fn record_pillar(&self, pillar: &str) {
        match pillar.to_lowercase().as_str() {
            "security" => { self.security_requests.fetch_add(1, Ordering::Relaxed); }
            "fuzzer" => { self.fuzzer_requests.fetch_add(1, Ordering::Relaxed); }
            "explorer" => { self.explorer_requests.fetch_add(1, Ordering::Relaxed); }
            _ => {}
        }
    }

    pub fn record_affinity(&self, is_hit: bool) {
        if is_hit {
            self.affinity_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.affinity_misses.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_rolling_compaction(&self, chars_saved: usize) {
        self.rolling_compact_count.fetch_add(1, Ordering::Relaxed);
        self.rolling_compact_chars_saved.fetch_add(chars_saved as u64, Ordering::Relaxed);
    }

    pub fn record_queue_wait(&self, wait_ms: u64) {
        self.queued_requests.fetch_add(1, Ordering::Relaxed);
        self.queue_wait_ms_total.fetch_add(wait_ms, Ordering::Relaxed);
        self.last_queue_wait_ms.store(wait_ms, Ordering::Relaxed);
    }

    pub fn record_orchestrator(&self, is_spillover: bool) {
        self.orchestrator_requests.fetch_add(1, Ordering::Relaxed);
        if is_spillover {
            self.orchestrator_spillover.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_loop_nudge(&self) {
        self.loop_nudges_total.fetch_add(1, Ordering::Relaxed);
    }
}
