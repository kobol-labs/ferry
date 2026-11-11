# Graph Report - ferry  (2026-11-11)

## Corpus Check
- 21 files · ~36,989 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 3 file(s) not represented in the graph (top: (none) 2, .toml 1)

## Summary
- 538 nodes · 952 edges · 24 communities (17 shown, 7 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 17 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `c6082e35`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- config.rs
- condenser.rs
- SessionAffinityTracker
- upstream.rs
- MetricsTracker
- router.rs
- condenser_invariants.rs
- with
- antigravity/ferry_cleaner.py
- ⛴️ Ferry
- main.rs
- ferry_cleaner.py
- condense_output
- _clean_text_chunk
- test_router_adversarial.py
- Ferry Antigravity Integration
- suppress_base64_blobs
- suppress_base64_blobs
- shield_directory_traversals
- ferry
- graphify

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 44 edges
2. `Router` - 22 edges
3. `Config` - 21 edges
4. `SessionAffinityTracker` - 19 edges
5. `ContextCondenser` - 16 edges
6. `AppState` - 15 edges
7. `ResponseCache` - 14 edges
8. `UpstreamClient` - 12 edges
9. `RouteDecision` - 12 edges
10. `AffinityType` - 11 edges

## Surprising Connections (you probably didn't know these)
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser.rs
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser.rs
- `ResponseMeta` --references--> `AffinityType`  [INFERRED]
  src/upstream.rs → src/affinity.rs
- `ContextCondenser` --references--> `CondenserConfig`  [EXTRACTED]
  src/condenser.rs → src/config.rs
- `AppState` --references--> `Config`  [EXTRACTED]
  src/main.rs → src/config.rs

## Import Cycles
- None detected.

## Communities (24 total, 7 thin omitted)

### Community 0 - "config.rs"
Cohesion: 0.05
Nodes (57): AffinityConfig, CacheConfig, CloudConfig, CondenserConfig, Config, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_affinity_enabled() (+49 more)

### Community 1 - "condenser.rs"
Cohesion: 0.06
Nodes (52): collapse_excess_whitespace(), collect_discovered_entities(), COMMON_CODE_EXTS, CondensationStats, ContentSegment, Code, Text, ContextCondenser (+44 more)

### Community 2 - "SessionAffinityTracker"
Cohesion: 0.07
Nodes (15): AffinityType, NewSlot, PrefixHit, SessionHit, compute_prefix_hash(), SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse() (+7 more)

### Community 3 - "upstream.rs"
Cohesion: 0.08
Nodes (7): LocalSlotGuard, GuardedStream<S>, normalize_wire_format(), ReceiverStream<T>, ResponseMeta, transform_strata_chunk(), UpstreamClient

### Community 4 - "MetricsTracker"
Cohesion: 0.07
Nodes (3): MetricsTracker, StatsSnapshot, test_queue_metrics_tracking()

### Community 5 - "router.rs"
Cohesion: 0.11
Nodes (15): HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS, RouteDecision, RouteOverride, Router, RoutingMode, CloudOnly, Hybrid (+7 more)

### Community 7 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 8 - "antigravity/ferry_cleaner.py"
Cohesion: 0.10
Nodes (9): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_directory_listing_line(), prune_git_diff_lockfiles(), shield_directory_traversals(), strip_ansi(), strip_progress_artifacts() (+1 more)

### Community 9 - "⛴️ Ferry"
Cohesion: 0.08
Nodes (25): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+17 more)

### Community 10 - "main.rs"
Cohesion: 0.23
Nodes (10): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+2 more)

### Community 11 - "ferry_cleaner.py"
Cohesion: 0.16
Nodes (10): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+2 more)

### Community 12 - "condense_output"
Cohesion: 0.16
Nodes (9): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+1 more)

### Community 13 - "_clean_text_chunk"
Cohesion: 0.17
Nodes (6): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), prune_git_diff_lockfiles(), strip_progress_artifacts(), try_minify_json()

### Community 14 - "test_router_adversarial.py"
Cohesion: 0.43
Nodes (6): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite()

### Community 15 - "Ferry Antigravity Integration"
Cohesion: 0.29
Nodes (6): Architecture & Alignment with Ferry Core, Ferry Antigravity Integration, Installation & Configuration, Purpose, Reduction Telemetry, Running Tests

### Community 16 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

### Community 17 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

## Knowledge Gaps
- **11 isolated node(s):** `/Users/steve/.local/bin/graphify-mcp`, `NewSlot`, `PrefixHit`, `SessionHit`, `ferry` (+6 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 192 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `config.rs` to `SessionAffinityTracker`, `main.rs`, `upstream.rs`, `router.rs`?**
  _High betweenness centrality (0.120) - this node is a cross-community bridge._
- **What connects `/Users/steve/.local/bin/graphify-mcp`, `NewSlot`, `PrefixHit` to the rest of the system?**
  _11 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `config.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0518326545723806 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `config.rs`, `SessionAffinityTracker`, `upstream.rs`, `router.rs`, `main.rs`?**
  _High betweenness centrality (0.106) - this node is a cross-community bridge._
- **Should `condenser.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.06386946386946386 - nodes in this community are weakly interconnected._
- **Why does `CondenserConfig` connect `config.rs` to `condenser.rs`, `condenser_invariants.rs`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._
- **Should `SessionAffinityTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.07102040816326531 - nodes in this community are weakly interconnected._