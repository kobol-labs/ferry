# Graph Report - ferry-14  (2026-11-13)

## Corpus Check
- 41 files · ~51,643 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 4 file(s) not represented in the graph (top: (none) 3, .toml 1)

## Summary
- 779 nodes · 1416 edges · 39 communities (29 shown, 10 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 112 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `7d9be226`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- whitespace.rs
- MetricsTracker
- condenser.rs
- upstream.rs
- classifier.rs
- with
- SessionAffinityTracker
- ⛴️ Ferry
- _clean_text_chunk
- issue_15_sse_cache_test.rs
- routing.rs
- steps
- _clean_text_chunk
- condenser/mod.rs
- is_hard_logic_turn
- config/mod.rs
- TestFerryCleanerDirect
- Config
- hardware.rs
- issue_16_condenser_perf_test.rs
- graphify
- ferry
- condense_output
- condense_output
- .new
- ContextCondenser
- truncate_tool_content_smart
- code.rs
- directory.rs
- TestFerryCleanerSubprocess
- .rolling_compact
- suppress_base64_blobs
- Ferry Antigravity Integration
- suppress_base64_blobs

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 50 edges
2. `Config` - 22 edges
3. `Router` - 22 edges
4. `SessionAffinityTracker` - 21 edges
5. `nudge_repeated_tool_failure()` - 20 edges
6. `ContextCondenser` - 17 edges
7. `ResponseCache` - 16 edges
8. `UpstreamClient` - 16 edges
9. `AppState` - 15 edges
10. `RouteDecision` - 15 edges

## Surprising Connections (you probably didn't know these)
- `Architecture & Alignment with Ferry Core` --references--> `suppress_base64_blobs()`  [INFERRED]
  integrations/antigravity/README.md → integrations/antigravity/ferry_cleaner.py
- `Architecture & Alignment with Ferry Core` --references--> `truncate_signal_aware()`  [INFERRED]
  integrations/antigravity/README.md → integrations/antigravity/ferry_cleaner.py
- `test_compute_prefix_hash_and_orchestrator_multipart_array_content()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_compute_prefix_hash_multibyte_utf8_at_8192_boundary_no_panic()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs

## Import Cycles
- None detected.

## Communities (39 total, 10 thin omitted)

### Community 0 - "whitespace.rs"
Cohesion: 0.17
Nodes (12): test_collapse_excess_whitespace(), test_deduplicate_consecutive_lines_safe_for_code(), test_strip_ansi_codes(), test_strip_progress_artifacts(), test_suppress_base64_and_hex_blobs(), collapse_excess_whitespace(), COMMON_CODE_EXTS, deduplicate_lines() (+4 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.06
Nodes (12): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+4 more)

### Community 2 - "condenser.rs"
Cohesion: 0.17
Nodes (20): CondenserConfig, default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines() (+12 more)

### Community 4 - "upstream.rs"
Cohesion: 0.06
Nodes (24): LocalSlotGuard, DONE_SSE_CHUNK, extract_streaming_usage(), format_sse_error_frame(), GuardedStream<S>, is_pure_sse_comment_chunk(), KEEPALIVE_SSE_CHUNK, looks_like_json_value_tail() (+16 more)

### Community 5 - "classifier.rs"
Cohesion: 0.07
Nodes (33): COGNITIVE_TOOL_CONTEXT_KEYWORDS, detect_orchestrator(), estimate_tokens(), estimated_json_len(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS (+25 more)

### Community 6 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 7 - "SessionAffinityTracker"
Cohesion: 0.08
Nodes (15): AffinityType, NewSlot, PrefixHit, SessionHit, SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse(), test_anti_eviction_hysteresis() (+7 more)

### Community 8 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 9 - "_clean_text_chunk"
Cohesion: 0.14
Nodes (10): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_directory_listing_line(), prune_git_diff_lockfiles(), shield_directory_traversals(), strip_ansi(), strip_progress_artifacts() (+2 more)

### Community 10 - "issue_15_sse_cache_test.rs"
Cohesion: 0.06
Nodes (24): compute_prefix_hash(), AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler() (+16 more)

### Community 11 - "routing.rs"
Cohesion: 0.22
Nodes (10): AffinityConfig, CacheConfig, default_affinity_enabled(), default_affinity_timeout_secs(), default_cache_enabled(), default_cache_max_entries(), default_cache_ttl_secs(), default_default_route() (+2 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 13 - "_clean_text_chunk"
Cohesion: 0.11
Nodes (9): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_directory_listing_line(), prune_git_diff_lockfiles(), shield_directory_traversals(), strip_ansi(), strip_progress_artifacts() (+1 more)

### Community 15 - "condenser/mod.rs"
Cohesion: 0.14
Nodes (21): align_tool_outputs(), append_nudge_to_tool_message(), extract_tool_output_text(), is_diagnostic_tool_failure(), LOOP_GUARD_MARKER, LOOP_GUARD_SUFFIX, nudge_repeated_tool_failure(), ParsedToolCall (+13 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.19
Nodes (7): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryRouterAdversarial

### Community 17 - "config/mod.rs"
Cohesion: 0.26
Nodes (6): parse_bool(), default_job_name(), default_push_interval_secs(), default_pushgateway_url(), default_telemetry_enabled(), TelemetryConfig

### Community 19 - "Config"
Cohesion: 0.24
Nodes (4): default_bind(), ServerConfig, Config, UpstreamClient

### Community 20 - "hardware.rs"
Cohesion: 0.17
Nodes (18): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_timeout_secs(), default_connect_timeout_secs() (+10 more)

### Community 21 - "issue_16_condenser_perf_test.rs"
Cohesion: 0.08
Nodes (7): ChildGuard, count_allocations(), CountingAlloc, GLOBAL_ALLOC, test_ac3_x_ferry_condense_false_and_cloud_target_exemptions(), test_ac3_x_ferry_condense_false_bypasses_adaptive_squeeze(), test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations()

### Community 25 - "condense_output"
Cohesion: 0.16
Nodes (9): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+1 more)

### Community 27 - "condense_output"
Cohesion: 0.16
Nodes (9): condense_output(), handle_hook(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), segment_markdown_code_blocks() (+1 more)

### Community 29 - ".new"
Cohesion: 0.29
Nodes (6): test_clean_tool_text_json_minification(), test_condenser_dual_key_support(), test_protected_code_blocks_segmentation(), test_repeated_rolling_compaction_merges_checkpoints_cleanly(), test_rolling_context_compaction_preserves_security_entities(), test_rolling_context_compaction_preserves_system_and_recent()

### Community 31 - "truncate_tool_content_smart"
Cohesion: 0.24
Nodes (11): test_multi_region_diagnostic_signal_preservation(), test_single_line_diagnostic_signal_preservation(), DIAGNOSTIC_SIGNALS, find_diagnostic_signal_index(), find_diagnostic_signal_regions(), safe_ceil_char_boundary(), safe_floor_char_boundary(), strip_think_tags() (+3 more)

### Community 32 - "code.rs"
Cohesion: 0.22
Nodes (9): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+1 more)

### Community 33 - "directory.rs"
Cohesion: 0.25
Nodes (8): DIR_SHIELD_PATTERNS, is_directory_listing_line(), LOCKFILE_EXTENSIONS, LOCKFILE_NAMES, prune_git_diff_lockfiles(), shield_directory_traversals(), test_prune_git_diff_lockfiles(), test_shield_directory_traversals()

### Community 35 - ".rolling_compact"
Cohesion: 0.25
Nodes (10): extract_preceding_tool_calls(), extract_tool_output_id(), is_tool_output_message(), normalize_tool_arguments(), count_message_chars(), estimate_message_tokens(), estimate_tokens_from_chars(), RollingCompactionStats (+2 more)

### Community 36 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

### Community 37 - "Ferry Antigravity Integration"
Cohesion: 0.33
Nodes (5): Ferry Antigravity Integration, Installation & Configuration, Purpose, Reduction Telemetry, Running Tests

### Community 38 - "suppress_base64_blobs"
Cohesion: 0.33
Nodes (3): is_base64_or_hex_payload(), suppress_base64_blobs(), _raw_base64_sub()

## Knowledge Gaps
- **18 isolated node(s):** `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit`, `PrefixHit`, `NewSlot` (+13 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 272 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `Config` to `condenser.rs`, `upstream.rs`, `classifier.rs`, `issue_15_sse_cache_test.rs`, `routing.rs`, `config/mod.rs`, `hardware.rs`?**
  _High betweenness centrality (0.109) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit` to the rest of the system?**
  _18 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.05587808417997097 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `upstream.rs`, `classifier.rs`, `SessionAffinityTracker`, `issue_15_sse_cache_test.rs`, `Config`?**
  _High betweenness centrality (0.088) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `SessionAffinityTracker` (e.g. with `UpstreamClient` and `.with_affinity()`) actually correct?**
  _`SessionAffinityTracker` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Should `condenser_invariants.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05714285714285714 - nodes in this community are weakly interconnected._