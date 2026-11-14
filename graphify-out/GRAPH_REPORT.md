# Graph Report - ferry-15  (2026-11-13)

## Corpus Check
- 39 files · ~43,592 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 4 file(s) not represented in the graph (top: (none) 3, .toml 1)

## Summary
- 732 nodes · 1305 edges · 31 communities (24 shown, 7 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 105 edges (avg confidence: 0.79)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `7086254c`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- .clean_single_chunk
- MetricsTracker
- condenser.rs
- condenser_invariants.rs
- upstream.rs
- Router
- with
- SessionAffinityTracker
- ⛴️ Ferry
- antigravity/ferry_cleaner.py
- issue_15_sse_cache_test.rs
- routing.rs
- steps
- ferry_cleaner.py
- condenser/mod.rs
- is_hard_logic_turn
- config/mod.rs
- TestFerryCleanerDirect
- Config
- hardware.rs
- graphify
- ferry
- ContextCondenser
- truncate_tool_content_smart
- code.rs
- directory.rs
- TestFerryCleanerSubprocess
- .rolling_compact

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 49 edges
2. `Router` - 22 edges
3. `Config` - 21 edges
4. `nudge_repeated_tool_failure()` - 20 edges
5. `SessionAffinityTracker` - 19 edges
6. `ContextCondenser` - 16 edges
7. `ResponseCache` - 15 edges
8. `AppState` - 15 edges
9. `RouteDecision` - 15 edges
10. `TestFerryCleanerDirect` - 15 edges

## Surprising Connections (you probably didn't know these)
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/truncation.rs
- `Architecture & Alignment with Ferry Core` --references--> `strip_ansi()`  [INFERRED]
  integrations/antigravity/README.md → integrations/antigravity/ferry_cleaner.py
- `Architecture & Alignment with Ferry Core` --references--> `strip_progress_artifacts()`  [INFERRED]
  integrations/antigravity/README.md → integrations/antigravity/ferry_cleaner.py
- `Architecture & Alignment with Ferry Core` --references--> `collapse_excess_whitespace()`  [INFERRED]
  integrations/antigravity/README.md → integrations/antigravity/ferry_cleaner.py

## Import Cycles
- None detected.

## Communities (31 total, 7 thin omitted)

### Community 0 - ".clean_single_chunk"
Cohesion: 0.17
Nodes (13): test_collapse_excess_whitespace(), test_deduplicate_consecutive_lines_safe_for_code(), test_strip_ansi_codes(), test_strip_progress_artifacts(), test_suppress_base64_and_hex_blobs(), CondensationStats, collapse_excess_whitespace(), COMMON_CODE_EXTS (+5 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.06
Nodes (12): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+4 more)

### Community 2 - "condenser.rs"
Cohesion: 0.18
Nodes (19): default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines(), default_keep_recent_turns() (+11 more)

### Community 3 - "condenser_invariants.rs"
Cohesion: 0.04
Nodes (10): RouteOverride, RoutingMode, CloudOnly, Hybrid, LocalOnly, LocalPreferred, TargetProvider, Cloud (+2 more)

### Community 4 - "upstream.rs"
Cohesion: 0.06
Nodes (25): LocalSlotGuard, DONE_SSE_CHUNK, extract_streaming_usage(), format_sse_error_frame(), GuardedStream<S>, is_pure_sse_comment_chunk(), KEEPALIVE_SSE_CHUNK, looks_like_json_value_tail() (+17 more)

### Community 5 - "Router"
Cohesion: 0.10
Nodes (22): COGNITIVE_TOOL_CONTEXT_KEYWORDS, detect_orchestrator(), estimate_tokens(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS (+14 more)

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
Cohesion: 0.06
Nodes (27): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line(), main() (+19 more)

### Community 10 - "issue_15_sse_cache_test.rs"
Cohesion: 0.08
Nodes (17): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+9 more)

### Community 11 - "routing.rs"
Cohesion: 0.22
Nodes (10): AffinityConfig, CacheConfig, default_affinity_enabled(), default_affinity_timeout_secs(), default_cache_enabled(), default_cache_max_entries(), default_cache_ttl_secs(), default_default_route() (+2 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 13 - "ferry_cleaner.py"
Cohesion: 0.07
Nodes (21): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), handle_hook(), is_base64_or_hex_payload(), is_directory_listing_line(), main() (+13 more)

### Community 15 - "condenser/mod.rs"
Cohesion: 0.16
Nodes (22): align_tool_outputs(), append_nudge_to_tool_message(), extract_preceding_tool_calls(), extract_tool_output_id(), extract_tool_output_text(), is_diagnostic_tool_failure(), is_tool_output_message(), LOOP_GUARD_MARKER (+14 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.19
Nodes (7): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryRouterAdversarial

### Community 17 - "config/mod.rs"
Cohesion: 0.26
Nodes (6): parse_bool(), default_job_name(), default_push_interval_secs(), default_pushgateway_url(), default_telemetry_enabled(), TelemetryConfig

### Community 19 - "Config"
Cohesion: 0.28
Nodes (4): CondenserConfig, default_bind(), ServerConfig, Config

### Community 20 - "hardware.rs"
Cohesion: 0.17
Nodes (18): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_timeout_secs(), default_connect_timeout_secs() (+10 more)

### Community 30 - "ContextCondenser"
Cohesion: 0.24
Nodes (8): ContextCondenser, test_clean_tool_text_json_minification(), test_condenser_dual_key_support(), test_loop_nudge_preserves_json_minification_and_suffix_in_condense(), test_loop_nudge_preserves_lockfile_diff_pruning_and_suffix_in_condense(), test_loop_nudge_preserves_suffix_under_large_and_small_truncation_budgets(), test_protected_code_blocks_segmentation(), strip_think_tags()

### Community 31 - "truncate_tool_content_smart"
Cohesion: 0.27
Nodes (10): test_multi_region_diagnostic_signal_preservation(), test_single_line_diagnostic_signal_preservation(), DIAGNOSTIC_SIGNALS, find_diagnostic_signal_index(), find_diagnostic_signal_regions(), safe_ceil_char_boundary(), safe_floor_char_boundary(), truncate_tool_content() (+2 more)

### Community 32 - "code.rs"
Cohesion: 0.20
Nodes (10): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+2 more)

### Community 33 - "directory.rs"
Cohesion: 0.25
Nodes (8): DIR_SHIELD_PATTERNS, is_directory_listing_line(), LOCKFILE_EXTENSIONS, LOCKFILE_NAMES, prune_git_diff_lockfiles(), shield_directory_traversals(), test_prune_git_diff_lockfiles(), test_shield_directory_traversals()

### Community 35 - ".rolling_compact"
Cohesion: 0.31
Nodes (7): test_rolling_context_compaction_preserves_security_entities(), test_rolling_context_compaction_preserves_system_and_recent(), count_message_chars(), estimate_message_tokens(), estimate_tokens_from_chars(), RollingCompactionStats, test_zero_allocation_char_counting_parity()

## Knowledge Gaps
- **18 isolated node(s):** `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit`, `PrefixHit`, `NewSlot` (+13 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 260 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **7 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `Config` to `upstream.rs`, `Router`, `SessionAffinityTracker`, `issue_15_sse_cache_test.rs`, `routing.rs`, `config/mod.rs`, `hardware.rs`?**
  _High betweenness centrality (0.116) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit` to the rest of the system?**
  _18 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.055904961565338925 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `condenser_invariants.rs`, `upstream.rs`, `Router`, `SessionAffinityTracker`, `issue_15_sse_cache_test.rs`?**
  _High betweenness centrality (0.095) - this node is a cross-community bridge._
- **Are the 11 inferred relationships involving `nudge_repeated_tool_failure()` (e.g. with `test_loop_nudge_3x_identical_failing_calls_tail_only_across_formats()` and `test_loop_nudge_3x_identical_failing_parallel_tuple_nudges_tail_only()`) actually correct?**
  _`nudge_repeated_tool_failure()` has 11 INFERRED edges - model-reasoned connections that need verification._
- **Should `condenser_invariants.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.041666666666666664 - nodes in this community are weakly interconnected._