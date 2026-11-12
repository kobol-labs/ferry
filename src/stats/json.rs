use super::tracker::StatsSnapshot;
use serde_json::Value;

pub fn render_json(s: &StatsSnapshot) -> Value {
    serde_json::to_value(s).unwrap_or(Value::Null)
}
