# Graph Report - ferry  (2026-11-12)

## Corpus Check
- 37 files · ~38,892 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 3 file(s) not represented in the graph (top: (none) 2, .toml 1)

## Summary
- 664 nodes · 1145 edges · 25 communities (16 shown, 9 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 79 edges (avg confidence: 0.77)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `f7a7f6e1`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- condenser/mod.rs
- MetricsTracker
- hardware.rs
- condenser_invariants.rs
- upstream.rs
- Router
- with
- SessionAffinityTracker
- ⛴️ Ferry
- antigravity/ferry_cleaner.py
- main.rs
- hardware.rs
- steps
- ferry_cleaner.py
- is_hard_logic_turn
- Ferry Antigravity Integration
- graphify
- ferry

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 48 edges
2. `Router` - 22 edges
3. `Config` - 21 edges
4. `SessionAffinityTracker` - 19 edges
5. `ContextCondenser` - 16 edges
6. `TestFerryCleanerDirect` - 15 edges
7. `AppState` - 15 edges
8. `is_hard_logic_turn()` - 14 edges
9. `ResponseCache` - 14 edges
10. `is_hard_logic_turn()` - 12 edges

## Surprising Connections (you probably didn't know these)
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/truncation.rs
- `Config` --references--> `CloudConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs
- `Config` --references--> `LocalConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs
- `Config` --references--> `ServerConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs

## Import Cycles
- None detected.

## Communities (25 total, 9 thin omitted)

### Community 0 - "condenser/mod.rs"
Cohesion: 0.05
Nodes (53): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+45 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.06
Nodes (12): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+4 more)

### Community 2 - "hardware.rs"
Cohesion: 0.07
Nodes (36): CondenserConfig, default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines() (+28 more)

### Community 3 - "condenser_invariants.rs"
Cohesion: 0.04
Nodes (10): RouteOverride, RoutingMode, CloudOnly, Hybrid, LocalOnly, LocalPreferred, TargetProvider, Cloud (+2 more)

### Community 4 - "upstream.rs"
Cohesion: 0.07
Nodes (14): LocalSlotGuard, DONE_SSE_CHUNK, extract_streaming_usage(), GuardedStream<S>, KEEPALIVE_SSE_CHUNK, normalize_wire_format(), ReceiverStream<T>, ResponseMeta (+6 more)

### Community 5 - "Router"
Cohesion: 0.10
Nodes (21): detect_orchestrator(), estimate_tokens(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS, has_real_diagnostic_signal() (+13 more)

### Community 6 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 7 - "SessionAffinityTracker"
Cohesion: 0.07
Nodes (15): AffinityType, NewSlot, PrefixHit, SessionHit, compute_prefix_hash(), SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse() (+7 more)

### Community 8 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 9 - "antigravity/ferry_cleaner.py"
Cohesion: 0.07
Nodes (21): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line(), main() (+13 more)

### Community 10 - "main.rs"
Cohesion: 0.21
Nodes (10): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+2 more)

### Community 11 - "hardware.rs"
Cohesion: 0.14
Nodes (20): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_bind(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_timeout_secs() (+12 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 13 - "ferry_cleaner.py"
Cohesion: 0.05
Nodes (22): TestFerryCleanerDirect, _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line() (+14 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.08
Nodes (8): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryCleanerSubprocess, TestFerryRouterAdversarial

### Community 17 - "Ferry Antigravity Integration"
Cohesion: 0.29
Nodes (6): Architecture & Alignment with Ferry Core, Ferry Antigravity Integration, Installation & Configuration, Purpose, Reduction Telemetry, Running Tests

## Knowledge Gaps
- **13 isolated node(s):** `Closed`, `Open`, `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `CloudOnly` (+8 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 230 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **9 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `hardware.rs` to `upstream.rs`, `Router`, `SessionAffinityTracker`, `main.rs`, `hardware.rs`?**
  _High betweenness centrality (0.120) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Closed`, `Open`, `/Users/steve/.local/bin/graphify-mcp` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `condenser/mod.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.051228070175438595 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `condenser_invariants.rs`, `upstream.rs`, `Router`, `SessionAffinityTracker`, `main.rs`?**
  _High betweenness centrality (0.102) - this node is a cross-community bridge._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.05587808417997097 - nodes in this community are weakly interconnected._
- **Why does `ContextCondenser` connect `condenser/mod.rs` to `condenser_invariants.rs`, `hardware.rs`, `main.rs`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._