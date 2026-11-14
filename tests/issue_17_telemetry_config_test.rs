use ferry::condenser::ContextCondenser;
use ferry::config::{CondenserConfig, Config};
use ferry::stats::MetricsTracker;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn test_prometheus_orthogonal_metrics_no_mixed_label_families() {
    let tracker = MetricsTracker::new();
    tracker.inc_local(300);
    tracker.inc_local(300);
    tracker.inc_local(300);
    tracker.inc_cloud(500, true);
    tracker.inc_failover();

    let prom = tracker.prometheus_metrics();

    assert!(prom.contains("ferry_requests_dispatched_total 4\n"));
    assert!(prom.contains("ferry_provider_requests_total{provider=\"local\"} 3\n"));
    assert!(prom.contains("ferry_provider_requests_total{provider=\"cloud\"} 1\n"));
    assert!(prom.contains("ferry_routing_events_total{event=\"spillover\"} 1\n"));
    assert!(prom.contains("ferry_routing_events_total{event=\"failover\"} 1\n"));
    assert!(prom.contains("ferry_tokens_estimated 1400\n"));
    assert!(prom.contains("ferry_provider_tokens_estimated_total{provider=\"local\"} 900\n"));
    assert!(prom.contains("ferry_provider_tokens_estimated_total{provider=\"cloud\"} 500\n"));
    assert!(!prom.contains("ferry_requests_total"));

    // Verify PromQL safety invariant: within any metric family, every sample has the exact same label keys.
    let mut family_label_keys: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    for line in prom.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let metric_part = trimmed.split_whitespace().next().unwrap();
        let (name, keys) = if let Some(brace_idx) = metric_part.find('{') {
            let name = metric_part[..brace_idx].to_string();
            let inside = metric_part[brace_idx + 1..metric_part.len() - 1].trim();
            let keys: BTreeSet<String> = inside
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|kv| kv.split('=').next().unwrap().trim().to_string())
                .collect();
            (name, keys)
        } else {
            (metric_part.to_string(), BTreeSet::new())
        };
        family_label_keys.entry(name).or_default().push(keys);
    }

    for (family, key_sets) in &family_label_keys {
        let first = &key_sets[0];
        for ks in key_sets.iter().skip(1) {
            assert_eq!(
                first, ks,
                "Metric family `{family}` mixes incompatible label key sets ({first:?} vs {ks:?}), causing PromQL label-matching bugs"
            );
        }
    }
}

#[test]
fn test_grafana_dashboard_valid_json_and_offload_query() {
    let raw = std::fs::read_to_string(repo_root().join("grafana/dashboard.json"))
        .expect("grafana/dashboard.json must be readable");
    let dash: Value = serde_json::from_str(&raw).expect("grafana/dashboard.json must be valid JSON");

    assert!(
        !raw.contains("ferry_requests_total"),
        "grafana/dashboard.json must not reference ambiguous ferry_requests_total"
    );

    let panels = dash["panels"].as_array().expect("panels array");
    let offload_panel = panels
        .iter()
        .find(|p| p["title"] == "Local Offload Ratio")
        .expect("Local Offload Ratio panel must exist");
    let expr = offload_panel["targets"][0]["expr"]
        .as_str()
        .expect("Local Offload Ratio expr");

    assert!(expr.contains("ferry_provider_requests_total"));
    assert!(expr.contains("clamp_min"));
    assert!(!expr.contains("or vector(100)"));

    let routing_pie = panels
        .iter()
        .find(|p| p["title"] == "Overall Routing Distribution")
        .expect("Overall Routing Distribution panel must exist");
    let pie_targets = routing_pie["targets"].as_array().expect("pie targets");
    assert_eq!(pie_targets.len(), 3);
    assert_eq!(
        pie_targets[0]["expr"].as_str().unwrap(),
        "sum(ferry_provider_requests_total{provider=\"local\"})"
    );
    assert_eq!(
        pie_targets[1]["expr"].as_str().unwrap(),
        "clamp_min(sum(ferry_provider_requests_total{provider=\"cloud\"}) - sum(ferry_routing_events_total{event=\"spillover\"}), 0)"
    );
    assert_eq!(
        pie_targets[2]["expr"].as_str().unwrap(),
        "sum(ferry_routing_events_total{event=\"spillover\"})"
    );
}

#[test]
fn test_missing_condensation_counters_wired_to_tracker_and_prometheus() {
    let condenser = ContextCondenser::new(CondenserConfig::default());
    let lockfile_diff = format!(
        "diff --git a/package-lock.json b/package-lock.json\n--- a/package-lock.json\n+++ b/package-lock.json\n@@ -1,20 +1,20 @@\n{}",
        (0..15)
            .map(|i| format!("+  \"dep-{i}\": \"1.0.{i}\",\n"))
            .collect::<String>()
    );
    let b64_blob = format!("data:image/png;base64,{}", "A".repeat(200));
    let dir_list = (0..6)
        .map(|i| format!("./node_modules/pkg-{i}/index.js"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut body = json!({
        "messages": [
            { "role": "user", "content": "Inspect workspace" },
            { "role": "assistant", "content": "Checking git diff" },
            { "role": "tool", "content": lockfile_diff },
            { "role": "assistant", "content": "Checking image log" },
            { "role": "tool", "content": b64_blob },
            { "role": "assistant", "content": "Checking directory tree" },
            { "role": "tool", "content": dir_list },
            { "role": "assistant", "content": "Done" }
        ]
    });

    let stats = condenser.condense(&mut body);
    assert!(stats.lockfile_diffs_pruned_count >= 1);
    assert!(stats.base64_blobs_suppressed_count >= 1);
    assert!(stats.dir_traversals_shielded_count >= 1);

    let tracker = MetricsTracker::new();
    tracker.record_condensation(
        500,
        120,
        stats.reasoning_blocks_dropped,
        stats.tool_outputs_truncated,
        stats.chars_saved,
        stats.ansi_stripped_count,
        stats.progress_cleaned_count,
        stats.lines_deduped_count,
        stats.whitespace_collapsed_count,
        stats.json_minified_count,
        stats.ledger_checks_passed,
        stats.ledger_vetoed_count,
        stats.signals_extracted_count,
        stats.loops_collapsed_count,
        stats.lockfile_diffs_pruned_count,
        stats.base64_blobs_suppressed_count,
        stats.dir_traversals_shielded_count,
    );

    let snap = tracker.snapshot();
    assert_eq!(snap.lockfile_diffs_pruned_count, stats.lockfile_diffs_pruned_count as u64);
    assert_eq!(snap.base64_blobs_suppressed_count, stats.base64_blobs_suppressed_count as u64);
    assert_eq!(snap.dir_traversals_shielded_count, stats.dir_traversals_shielded_count as u64);

    let prom = tracker.prometheus_metrics();
    assert!(prom.contains(&format!(
        "ferry_condensation_cleaner_actions_total{{action=\"lockfile_diffs_pruned\"}} {}",
        stats.lockfile_diffs_pruned_count
    )));
    assert!(prom.contains(&format!(
        "ferry_condensation_cleaner_actions_total{{action=\"base64_blobs_suppressed\"}} {}",
        stats.base64_blobs_suppressed_count
    )));
    assert!(prom.contains(&format!(
        "ferry_condensation_cleaner_actions_total{{action=\"dir_traversals_shielded\"}} {}",
        stats.dir_traversals_shielded_count
    )));
}

#[test]
fn test_health_probe_failure_and_circuit_breaker_cooldown() {
    let tracker = MetricsTracker::new();

    // 1. Cold-boot /health failure marks strata_online = false and opens circuit breaker
    assert!(tracker.record_health_probe_failure());
    assert!(!tracker.is_strata_online());
    assert!(tracker.is_circuit_open());

    // 2. When no inference failures occurred, /health 200 OK immediately restores local routing
    assert!(tracker.record_health_probe_success(30));
    assert!(tracker.is_strata_online());
    assert!(!tracker.is_circuit_open());

    // 3. Sub-threshold inference failures (2 < 3) must NOT be wiped by /health 200 OK within cooldown
    assert!(!tracker.record_local_failure(3));
    assert!(!tracker.record_local_failure(3));
    assert!(!tracker.record_health_probe_success(30));
    // 3rd inference failure trips the circuit breaker
    assert!(tracker.record_local_failure(3));
    assert!(tracker.is_circuit_open());

    // 4. While inference circuit breaker is in cooldown (30s), /health 200 OK marks strata_online = true
    //    but MUST NOT close circuit_open or reset consecutive_local_failures!
    assert!(!tracker.record_health_probe_success(30));
    assert!(tracker.is_strata_online());
    assert!(tracker.is_circuit_open());
    assert_eq!(tracker.snapshot().consecutive_local_failures, 3);

    // 5. Once cooldown window elapses (cooldown_secs = 0), /health 200 OK closes breaker
    assert!(tracker.record_health_probe_success(0));
    assert!(tracker.is_strata_online());
    assert!(!tracker.is_circuit_open());
    assert_eq!(tracker.snapshot().consecutive_local_failures, 0);
}

#[test]
fn test_config_hygiene_no_dead_keys_no_lan_secrets_and_dockerfile_locked() {
    let default_cfg = Config::default();
    assert!(!default_cfg.local.api_base.contains("192.168."));
    assert!(!default_cfg.local.api_base.contains("10.10."));
    assert!(default_cfg.local.api_key.is_empty());
    assert!(!default_cfg.telemetry.pushgateway_url.contains("10.10."));
    assert!(!default_cfg.telemetry.pushgateway_url.contains("192.168."));

    for filename in ["ferry.toml", "ferry.example.toml"] {
        let raw = std::fs::read_to_string(repo_root().join(filename))
            .unwrap_or_else(|_| panic!("{filename} must exist"));
        assert!(!raw.contains("192.168."), "{filename} contains private LAN IP");
        assert!(!raw.contains("10.10."), "{filename} contains private LAN IP");
        assert!(!raw.contains("sk-strata"), "{filename} contains hardcoded LAN API key");
        assert!(!raw.contains("sk-ferry"), "{filename} contains hardcoded LAN API key");

        let file_val: toml::Value = toml::from_str(&raw).expect("valid TOML");
        let parsed_cfg: Config = toml::from_str(&raw).expect("deserializes to Config");
        let struct_val = toml::Value::try_from(&parsed_cfg).expect("serializes to toml::Value");

        let file_root = file_val.as_table().expect("root table");
        let struct_root = struct_val.as_table().expect("struct root table");
        for (section, section_val) in file_root {
            let struct_sec = struct_root
                .get(section)
                .and_then(|v| v.as_table())
                .unwrap_or_else(|| panic!("Unknown section [{section}] in {filename}"));
            if let Some(sec_tbl) = section_val.as_table() {
                for key in sec_tbl.keys() {
                    assert!(
                        struct_sec.contains_key(key),
                        "Dead/unwired config key `[{section}].{key}` in {filename} would be silently dropped"
                    );
                }
            }
        }
    }

    // Legacy alias `strip_ansi_escapes = false` maps cleanly to `strip_terminal_noise`
    let alias_cfg: Config = toml::from_str("[condensation]\nstrip_ansi_escapes = false\n").unwrap();
    assert!(!alias_cfg.condensation.strip_terminal_noise);

    let dockerfile = std::fs::read_to_string(repo_root().join("Dockerfile"))
        .expect("Dockerfile must exist");
    assert!(dockerfile.contains("Cargo.lock"), "Dockerfile must COPY Cargo.lock");
    assert!(
        dockerfile.contains("cargo build --release --locked"),
        "Dockerfile must build with --locked"
    );
}
