use crate::config::Config;
use crate::stats::MetricsTracker;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetProvider {
    Local,
    Cloud,
}

#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub target: TargetProvider,
    pub est_tokens: u64,
    pub is_spillover: bool,
    pub reason: String,
}

pub struct Router {
    config: Arc<Config>,
    metrics: Arc<MetricsTracker>,
}

impl Router {
    pub fn new(config: Arc<Config>, metrics: Arc<MetricsTracker>) -> Self {
        Self { config, metrics }
    }

    pub fn estimate_tokens(body: &Value) -> u64 {
        let mut char_count = 0usize;
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    char_count += content.len();
                } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                    for part in content_arr {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            char_count += text.len();
                        }
                    }
                }
                // Handle OpenAI Responses format and dual-key tool outputs
                if let Some(output) = msg.get("output").and_then(|o| o.as_str()) {
                    char_count += output.len();
                }
            }
        }
        (char_count as f64 / 3.0).ceil() as u64
    }

    pub fn decide(&self, body: &Value) -> RouteDecision {
        let est_tokens = Self::estimate_tokens(body);
        let max_context = self.config.local.max_context_tokens as u64;
        let max_slots = self.config.local.max_slots;
        let active_slots = self.metrics.active_local_slots.load(std::sync::atomic::Ordering::Relaxed);

        if est_tokens > max_context {
            RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: false,
                reason: format!("context tokens ({est_tokens}) > local max ({max_context})"),
            }
        } else if active_slots < max_slots {
            RouteDecision {
                target: TargetProvider::Local,
                est_tokens,
                is_spillover: false,
                reason: format!("local slot available ({active_slots}/{max_slots})"),
            }
        } else {
            RouteDecision {
                target: TargetProvider::Cloud,
                est_tokens,
                is_spillover: true,
                reason: format!("local slots saturated ({active_slots}/{max_slots}) -> cloud spillover"),
            }
        }
    }
}
