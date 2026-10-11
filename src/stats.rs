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

    pub fn snapshot(&self) -> StatsSnapshot {
        let total = self.total_requests.load(Ordering::Relaxed);
        let local = self.local_requests.load(Ordering::Relaxed);
        let ratio = if total > 0 {
            (local as f64 / total as f64) * 100.0
        } else {
            0.0
        };

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
             ferry_tokens_estimated{{provider=\"cloud\"}} {ctokens}\n",
            total = s.total_requests,
            local = s.local_requests,
            cloud = s.cloud_requests,
            spill = s.spillover_requests,
            fail = s.failover_requests,
            slots = s.active_local_slots,
            tokens = s.total_tokens_est,
            ltokens = s.local_tokens_est,
            ctokens = s.cloud_tokens_est,
        )
    }
}
