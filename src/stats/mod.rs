pub mod json;
pub mod prometheus;
pub mod tracker;

pub use json::*;
pub use prometheus::*;
pub use tracker::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_queue_metrics_tracking() {
        let tracker = MetricsTracker::new();
        assert_eq!(tracker.queued_requests.load(Ordering::Relaxed), 0);
        tracker.record_queue_wait(150);
        tracker.record_queue_wait(350);

        let snap = tracker.snapshot();
        assert_eq!(snap.queued_requests_total, 2);
        assert_eq!(snap.queue_wait_ms_total, 500);
        assert_eq!(snap.last_queue_wait_ms, 350);
        assert_eq!(snap.avg_queue_wait_ms, 250.0);
    }

    #[test]
    fn test_prometheus_exposition_decoupled() {
        let tracker = MetricsTracker::new();
        tracker.inc_local(100);
        tracker.inc_cloud(200, false);
        let snap = tracker.snapshot();
        let prom = render_prometheus(&snap);

        assert!(prom.contains("ferry_requests_total 2"));
        assert!(prom.contains("ferry_requests_total{provider=\"local\"} 1"));
        assert!(prom.contains("ferry_requests_total{provider=\"cloud\"} 1"));
        assert!(prom.contains("ferry_tokens_estimated 300"));
    }

    #[test]
    fn test_json_exposition_decoupled() {
        let tracker = MetricsTracker::new();
        tracker.inc_local(50);
        let snap = tracker.snapshot();
        let json_val = render_json(&snap);

        assert_eq!(json_val["total_requests"], 1);
        assert_eq!(json_val["local_requests"], 1);
        assert_eq!(json_val["cloud_requests"], 0);
    }
}
