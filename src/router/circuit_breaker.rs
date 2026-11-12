use std::sync::atomic::Ordering;
use crate::stats::MetricsTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open { failures: u32 },
}

pub fn evaluate_circuit_state(metrics: &MetricsTracker) -> CircuitState {
    if metrics.is_circuit_open() {
        let failures = metrics.consecutive_local_failures.load(Ordering::Relaxed);
        CircuitState::Open { failures }
    } else {
        CircuitState::Closed
    }
}

pub fn circuit_open_reason(metrics: &MetricsTracker) -> Option<String> {
    match evaluate_circuit_state(metrics) {
        CircuitState::Open { failures } => {
            Some(format!("circuit breaker open (Strata offline, {failures} failures)"))
        }
        CircuitState::Closed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_state_evaluation() {
        let metrics = MetricsTracker::default();
        assert_eq!(evaluate_circuit_state(&metrics), CircuitState::Closed);
        assert_eq!(circuit_open_reason(&metrics), None);

        // Record failures until trip threshold
        for _ in 0..5 {
            metrics.record_local_failure(5);
        }
        assert!(metrics.is_circuit_open());
        assert_eq!(evaluate_circuit_state(&metrics), CircuitState::Open { failures: 5 });
        assert_eq!(
            circuit_open_reason(&metrics),
            Some("circuit breaker open (Strata offline, 5 failures)".to_string())
        );

        // Reset on success
        metrics.record_local_success();
        assert!(!metrics.is_circuit_open());
        assert_eq!(evaluate_circuit_state(&metrics), CircuitState::Closed);
        assert_eq!(circuit_open_reason(&metrics), None);
    }
}
