# Graph Report - ferry-17  (2026-11-13)

## Corpus Check
- 45 files · ~53,202 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 771 nodes · 1407 edges · 33 communities (24 shown, 9 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 111 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `b437e4cc`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- .clean_single_chunk
- MetricsTracker
- condenser.rs
- condenser_invariants.rs
- upstream.rs
- classifier.rs
- with
- SessionAffinityTracker
- ⛴️ Ferry
- issue_15_sse_cache_test.rs
- steps
- condenser/mod.rs
- is_hard_logic_turn
- test_cleaner_streaming.py
- TestFerryCleanerDirect
- Config
- hardware.rs
- issue_16_condenser_perf_test.rs
- graphify
- ferry
- condense_output
- antigravity/ferry_cleaner.py
- ContextCondenser
- truncate_tool_content_smart
- code.rs
- directory.rs
- TestFerryCleanerSubprocess
- .rolling_compact
- cleaner_rules.py

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 52 edges
2. `Config` - 22 edges
3. `Router` - 22 edges
4. `SessionAffinityTracker` - 21 edges
5. `nudge_repeated_tool_failure()` - 20 edges
6. `ContextCondenser` - 18 edges
7. `ResponseCache` - 16 edges
8. `UpstreamClient` - 16 edges
9. `AppState` - 15 edges
10. `RouteDecision` - 15 edges

## Surprising Connections (you probably didn't know these)
- `test_compute_prefix_hash_and_orchestrator_multipart_array_content()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_compute_prefix_hash_multibyte_utf8_at_8192_boundary_no_panic()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs
- `test_ac4_estimated_json_len_zero_allocation_and_exact_parity()` --calls--> `estimate_tokens_from_chars()`  [INFERRED]
  tests/issue_16_condenser_perf_test.rs → src/condenser/stats.rs
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/truncation.rs

## Import Cycles
- None detected.

## Communities (33 total, 9 thin omitted)

### Community 0 - ".clean_single_chunk"
Cohesion: 0.17
Nodes (13): test_collapse_excess_whitespace(), test_deduplicate_consecutive_lines_safe_for_code(), test_strip_ansi_codes(), test_strip_progress_artifacts(), test_suppress_base64_and_hex_blobs(), CondensationStats, collapse_excess_whitespace(), COMMON_CODE_EXTS (+5 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.05
Nodes (14): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+6 more)

### Community 2 - "condenser.rs"
Cohesion: 0.08
Nodes (34): CondenserConfig, default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines() (+26 more)

### Community 3 - "condenser_invariants.rs"
Cohesion: 0.05
Nodes (3): repo_root(), test_config_hygiene_no_dead_keys_no_lan_secrets_and_dockerfile_locked(), test_grafana_dashboard_valid_json_and_offload_query()

### Community 4 - "upstream.rs"
Cohesion: 0.06
Nodes (20): LocalSlotGuard, DONE_SSE_CHUNK, format_sse_error_frame(), GuardedStream<S>, is_pure_sse_comment_chunk(), KEEPALIVE_SSE_CHUNK, looks_like_json_value_tail(), normalize_wire_format() (+12 more)

### Community 5 - "classifier.rs"
Cohesion: 0.07
Nodes (33): COGNITIVE_TOOL_CONTEXT_KEYWORDS, detect_orchestrator(), estimate_tokens(), estimated_json_len(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS (+25 more)

### Community 6 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 7 - "SessionAffinityTracker"
Cohesion: 0.14
Nodes (12): AffinityType, NewSlot, PrefixHit, SessionHit, SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse(), test_anti_eviction_hysteresis() (+4 more)

### Community 8 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 10 - "issue_15_sse_cache_test.rs"
Cohesion: 0.06
Nodes (24): compute_prefix_hash(), AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler() (+16 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 15 - "condenser/mod.rs"
Cohesion: 0.16
Nodes (22): align_tool_outputs(), append_nudge_to_tool_message(), extract_preceding_tool_calls(), extract_tool_output_id(), extract_tool_output_text(), is_diagnostic_tool_failure(), is_tool_output_message(), LOOP_GUARD_MARKER (+14 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.19
Nodes (7): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryRouterAdversarial

### Community 17 - "test_cleaner_streaming.py"
Cohesion: 0.23
Nodes (8): _load_module_from_path(), test_cleaner_deduplicated_not_symlink_and_under_500_loc(), test_early_stdout_stderr_close_does_not_sigterm_child(), test_hook_json_roundtrip_both_entrypoints(), test_no_pythonpath_or_cwd_namespace_shadowing(), test_process_group_killed_on_sigterm(), test_streaming_incremental_utf8_and_partial_line_buffering(), test_streaming_mode_records_actual_saved_chars()

### Community 19 - "Config"
Cohesion: 0.12
Nodes (6): CachedResponse, hash_json_value(), ResponseCache, Config, parse_bool(), UpstreamClient

### Community 20 - "hardware.rs"
Cohesion: 0.14
Nodes (22): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_bind(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_name() (+14 more)

### Community 21 - "issue_16_condenser_perf_test.rs"
Cohesion: 0.08
Nodes (8): ChildGuard, count_allocations(), CountingAlloc, GLOBAL_ALLOC, test_ac3_x_ferry_condense_false_and_cloud_target_exemptions(), test_ac3_x_ferry_condense_false_bypasses_adaptive_squeeze(), test_ac4_estimated_json_len_zero_allocation_and_exact_parity(), test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations()

### Community 25 - "condense_output"
Cohesion: 0.23
Nodes (7): condense_output(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), terminate_process_group()

### Community 29 - "ContextCondenser"
Cohesion: 0.20
Nodes (10): ContextCondenser, test_clean_tool_text_json_minification(), test_condenser_dual_key_support(), test_loop_nudge_preserves_json_minification_and_suffix_in_condense(), test_loop_nudge_preserves_lockfile_diff_pruning_and_suffix_in_condense(), test_loop_nudge_preserves_suffix_under_large_and_small_truncation_budgets(), test_protected_code_blocks_segmentation(), test_rolling_context_compaction_preserves_security_entities() (+2 more)

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
Cohesion: 0.42
Nodes (5): count_message_chars(), estimate_message_tokens(), estimate_tokens_from_chars(), RollingCompactionStats, test_zero_allocation_char_counting_parity()

### Community 37 - "cleaner_rules.py"
Cohesion: 0.08
Nodes (20): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_base64_or_hex_payload(), is_directory_listing_line(), prune_git_diff_lockfiles(), segment_markdown_code_blocks(), shield_directory_traversals() (+12 more)

## Knowledge Gaps
- **18 isolated node(s):** `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit`, `PrefixHit`, `NewSlot` (+13 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 270 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **9 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `Config` to `condenser.rs`, `upstream.rs`, `classifier.rs`, `issue_15_sse_cache_test.rs`, `hardware.rs`?**
  _High betweenness centrality (0.107) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `/Users/steve/.local/bin/graphify-mcp`, `ferry`, `SessionHit` to the rest of the system?**
  _18 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.05202661826981246 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `upstream.rs`, `classifier.rs`, `SessionAffinityTracker`, `issue_15_sse_cache_test.rs`, `Config`?**
  _High betweenness centrality (0.093) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `SessionAffinityTracker` (e.g. with `UpstreamClient` and `.with_affinity()`) actually correct?**
  _`SessionAffinityTracker` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Should `condenser.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0797979797979798 - nodes in this community are weakly interconnected._