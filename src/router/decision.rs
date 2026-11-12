use std::sync::Arc;
use crate::affinity::{AffinityType, SessionAffinityTracker};
use crate::stats::MetricsTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProvider {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    LocalOnly,
    LocalPreferred,
    Hybrid,
    CloudOnly,
}

impl RoutingMode {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "local-only" | "local" => Some(Self::LocalOnly),
            "local-preferred" | "local-prefer" | "prefer-local" => Some(Self::LocalPreferred),
            "hybrid" | "hybrid-smart" | "cognitive" => Some(Self::Hybrid),
            "cloud-only" | "cloud" => Some(Self::CloudOnly),
            _ => None,
        }
    }
}

pub struct RouteOverride {
    pub route: Option<String>,
    pub queue_timeout_ms: Option<u64>,
    pub session_id: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug)]
pub struct LocalSlotGuard {
    metrics: Option<Arc<MetricsTracker>>,
    affinity_tracker: Option<Arc<SessionAffinityTracker>>,
    slot_id: Option<usize>,
    tokens: u64,
}

impl LocalSlotGuard {
    pub fn new(
        metrics: Arc<MetricsTracker>,
        affinity_tracker: Option<Arc<SessionAffinityTracker>>,
        slot_id: Option<usize>,
        tokens: u64,
    ) -> Self {
        Self {
            metrics: Some(metrics),
            affinity_tracker,
            slot_id,
            tokens,
        }
    }
}

impl Drop for LocalSlotGuard {
    fn drop(&mut self) {
        if let (Some(ref tracker), Some(slot)) = (&self.affinity_tracker, self.slot_id) {
            tracker.release_slot(slot);
        }
        if let Some(m) = self.metrics.take() {
            m.release_local_capacity(self.tokens);
        }
    }
}

#[derive(Debug)]
pub struct RouteDecision {
    pub target: TargetProvider,
    pub est_tokens: u64,
    pub is_spillover: bool,
    pub reason: String,
    pub local_only: bool,
    pub slot_guard: Option<LocalSlotGuard>,
    pub slot_id: Option<usize>,
    pub is_affinity_hit: bool,
    pub affinity_type: AffinityType,
    pub session_id: String,
}
