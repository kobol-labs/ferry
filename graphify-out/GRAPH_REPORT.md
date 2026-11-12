# Graph Report - ferry  (2026-11-11)

## Corpus Check
- 36 files · ~37,048 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 3 file(s) not represented in the graph (top: (none) 2, .toml 1)

## Summary
- 595 nodes · 1005 edges · 27 communities (16 shown, 11 thin omitted)
- Extraction: 93% EXTRACTED · 7% INFERRED · 0% AMBIGUOUS · INFERRED: 68 edges (avg confidence: 0.75)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `b512bcd2`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- condenser.rs
- mod.rs
- SessionAffinityTracker
- upstream.rs
- MetricsTracker
- Router
- condenser_invariants.rs
- with
- antigravity/ferry_cleaner.py
- ⛴️ Ferry
- main.rs
- ferry_cleaner.py
- hardware.rs
- test_router_adversarial.py
- Ferry Antigravity Integration
- ResponseCache
- ferry
- graphify

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 33 edges
2. `SessionAffinityTracker` - 20 edges
3. `Router` - 20 edges
4. `ResponseCache` - 14 edges
5. `Config` - 12 edges
6. `ContextCondenser` - 12 edges
7. `AppState` - 12 edges
8. `AffinityType` - 12 edges
9. `RouteDecision` - 12 edges
10. `with` - 11 edges

## Surprising Connections (you probably didn't know these)
- `Config` --references--> `ServerConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs
- `Config` --references--> `LocalConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs
- `Config` --references--> `CloudConfig`  [INFERRED]
  src/config/mod.rs → src/config/hardware.rs
- `ResponseMeta` --references--> `AffinityType`  [INFERRED]
  src/upstream.rs → src/affinity.rs
- `test_multi_region_diagnostic_signal_preservation()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  src/condenser/mod.rs → src/condenser/truncation.rs

## Import Cycles
- None detected.

## Communities (27 total, 11 thin omitted)

### Community 0 - "condenser.rs"
Cohesion: 0.07
Nodes (36): CondenserConfig, default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines() (+28 more)

### Community 1 - "mod.rs"
Cohesion: 0.05
Nodes (51): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+43 more)

### Community 2 - "SessionAffinityTracker"
Cohesion: 0.13
Nodes (12): AffinityType, NewSlot, PrefixHit, SessionHit, compute_prefix_hash(), SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse() (+4 more)

### Community 3 - "upstream.rs"
Cohesion: 0.07
Nodes (14): LocalSlotGuard, DONE_SSE_CHUNK, extract_streaming_usage(), GuardedStream<S>, KEEPALIVE_SSE_CHUNK, normalize_wire_format(), ReceiverStream<T>, ResponseMeta (+6 more)

### Community 4 - "MetricsTracker"
Cohesion: 0.05
Nodes (12): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+4 more)

### Community 5 - "Router"
Cohesion: 0.12
Nodes (17): detect_orchestrator(), estimate_tokens(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS, has_real_diagnostic_signal() (+9 more)

### Community 6 - "condenser_invariants.rs"
Cohesion: 0.04
Nodes (9): RouteOverride, RoutingMode, CloudOnly, Hybrid, LocalOnly, LocalPreferred, TargetProvider, Cloud (+1 more)

### Community 7 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 8 - "antigravity/ferry_cleaner.py"
Cohesion: 0.07
Nodes (21): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line(), main() (+13 more)

### Community 9 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 10 - "main.rs"
Cohesion: 0.20
Nodes (10): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+2 more)

### Community 11 - "ferry_cleaner.py"
Cohesion: 0.06
Nodes (21): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line(), main() (+13 more)

### Community 12 - "hardware.rs"
Cohesion: 0.14
Nodes (20): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_bind(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_timeout_secs() (+12 more)

### Community 14 - "test_router_adversarial.py"
Cohesion: 0.43
Nodes (6): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite()

### Community 15 - "Ferry Antigravity Integration"
Cohesion: 0.29
Nodes (6): Architecture & Alignment with Ferry Core, Ferry Antigravity Integration, Installation & Configuration, Purpose, Reduction Telemetry, Running Tests

### Community 16 - "ResponseCache"
Cohesion: 0.15
Nodes (3): CachedResponse, hash_json_value(), ResponseCache

## Knowledge Gaps
- **13 isolated node(s):** `NewSlot`, `PrefixHit`, `SessionHit`, `ferry`, `/Users/steve/.local/bin/graphify-mcp` (+8 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 212 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **11 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SessionAffinityTracker` connect `SessionAffinityTracker` to `main.rs`, `upstream.rs`, `Router`, `condenser_invariants.rs`?**
  _High betweenness centrality (0.058) - this node is a cross-community bridge._
- **What connects `NewSlot`, `PrefixHit`, `SessionHit` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `condenser.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07058823529411765 - nodes in this community are weakly interconnected._
- **Why does `ResponseCache` connect `ResponseCache` to `main.rs`, `upstream.rs`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Should `mod.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05441400304414003 - nodes in this community are weakly interconnected._
- **Should `SessionAffinityTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.13054187192118227 - nodes in this community are weakly interconnected._
- **Should `upstream.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0730804810360777 - nodes in this community are weakly interconnected._