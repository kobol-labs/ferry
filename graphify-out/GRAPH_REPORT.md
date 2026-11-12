# Graph Report - ferry  (2026-11-11)

## Corpus Check
- 29 files · ~36,904 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 3 file(s) not represented in the graph (top: (none) 2, .toml 1)

## Summary
- 582 nodes · 1014 edges · 29 communities (17 shown, 12 thin omitted)
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 57 edges (avg confidence: 0.77)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `04314d40`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- config.rs
- mod.rs
- SessionAffinityTracker
- upstream.rs
- MetricsTracker
- Router
- with
- antigravity/ferry_cleaner.py
- ⛴️ Ferry
- main.rs
- ferry_cleaner.py
- condense_output
- test_router_adversarial.py
- Ferry Antigravity Integration
- _clean_text_chunk
- suppress_base64_blobs
- suppress_base64_blobs
- shield_directory_traversals
- ferry
- graphify

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 48 edges
2. `Router` - 22 edges
3. `Config` - 21 edges
4. `SessionAffinityTracker` - 20 edges
5. `AppState` - 14 edges
6. `ResponseCache` - 14 edges
7. `ContextCondenser` - 13 edges
8. `UpstreamClient` - 12 edges
9. `AffinityType` - 12 edges
10. `RouteDecision` - 12 edges

## Surprising Connections (you probably didn't know these)
- `ResponseMeta` --references--> `AffinityType`  [INFERRED]
  src/upstream.rs → src/affinity.rs
- `test_multi_region_diagnostic_signal_preservation()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  src/condenser/mod.rs → src/condenser/truncation.rs
- `test_single_line_diagnostic_signal_preservation()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  src/condenser/mod.rs → src/condenser/truncation.rs
- `AppState` --references--> `UpstreamClient`  [EXTRACTED]
  src/main.rs → src/upstream.rs
- `UpstreamClient` --references--> `ResponseCache`  [EXTRACTED]
  src/upstream.rs → src/cache.rs

## Import Cycles
- None detected.

## Communities (29 total, 12 thin omitted)

### Community 0 - "config.rs"
Cohesion: 0.05
Nodes (55): AffinityConfig, CacheConfig, CloudConfig, Config, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_affinity_enabled(), default_affinity_timeout_secs() (+47 more)

### Community 1 - "mod.rs"
Cohesion: 0.05
Nodes (52): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+44 more)

### Community 2 - "SessionAffinityTracker"
Cohesion: 0.07
Nodes (15): AffinityType, NewSlot, PrefixHit, SessionHit, compute_prefix_hash(), SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse() (+7 more)

### Community 3 - "upstream.rs"
Cohesion: 0.07
Nodes (14): LocalSlotGuard, DONE_SSE_CHUNK, extract_streaming_usage(), GuardedStream<S>, KEEPALIVE_SSE_CHUNK, normalize_wire_format(), ReceiverStream<T>, ResponseMeta (+6 more)

### Community 4 - "MetricsTracker"
Cohesion: 0.06
Nodes (9): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), MetricsTracker, StatsSnapshot (+1 more)

### Community 5 - "Router"
Cohesion: 0.08
Nodes (26): detect_orchestrator(), estimate_tokens(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS, has_real_diagnostic_signal() (+18 more)

### Community 7 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 8 - "antigravity/ferry_cleaner.py"
Cohesion: 0.10
Nodes (9): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_directory_listing_line(), prune_git_diff_lockfiles(), shield_directory_traversals(), strip_ansi(), strip_progress_artifacts() (+1 more)

### Community 9 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 10 - "main.rs"
Cohesion: 0.23
Nodes (10): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+2 more)

### Community 11 - "ferry_cleaner.py"
Cohesion: 0.16
Nodes (10): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+2 more)

### Community 12 - "condense_output"
Cohesion: 0.16
Nodes (9): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+1 more)

### Community 14 - "test_router_adversarial.py"
Cohesion: 0.43
Nodes (6): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite()

### Community 15 - "Ferry Antigravity Integration"
Cohesion: 0.29
Nodes (6): Architecture & Alignment with Ferry Core, Ferry Antigravity Integration, Installation & Configuration, Purpose, Reduction Telemetry, Running Tests

### Community 16 - "_clean_text_chunk"
Cohesion: 0.17
Nodes (6): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), prune_git_diff_lockfiles(), strip_progress_artifacts(), try_minify_json()

### Community 17 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

### Community 18 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

## Knowledge Gaps
- **13 isolated node(s):** `NewSlot`, `PrefixHit`, `SessionHit`, `ferry`, `/Users/steve/.local/bin/graphify-mcp` (+8 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 209 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **12 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `config.rs` to `mod.rs`, `main.rs`, `upstream.rs`, `Router`?**
  _High betweenness centrality (0.127) - this node is a cross-community bridge._
- **What connects `NewSlot`, `PrefixHit`, `SessionHit` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `config.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.053923541247484906 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `config.rs`, `SessionAffinityTracker`, `upstream.rs`, `Router`, `main.rs`?**
  _High betweenness centrality (0.111) - this node is a cross-community bridge._
- **Should `mod.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05405405405405406 - nodes in this community are weakly interconnected._
- **Why does `CondenserConfig` connect `mod.rs` to `config.rs`, `condenser_invariants.rs`?**
  _High betweenness centrality (0.084) - this node is a cross-community bridge._
- **Should `SessionAffinityTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.07446808510638298 - nodes in this community are weakly interconnected._