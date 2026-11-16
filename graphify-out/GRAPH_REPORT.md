# Graph Report - ferry  (2026-11-16)

## Corpus Check
- 43 files · ~55,713 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 4 file(s) not represented in the graph (top: (none) 2, .toml 2)

## Summary
- 796 nodes · 1460 edges · 37 communities (24 shown, 13 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 112 edges (avg confidence: 0.79)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `06937130`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- whitespace.rs
- MetricsTracker
- condenser.rs
- upstream.rs
- Router
- with
- SessionAffinityTracker
- ⛴️ Ferry
- cleaner_rules.py
- main.rs
- TestFerryCleanerDirect
- steps
- issue_17_telemetry_config_test.rs
- ContextCondenser
- condenser/mod.rs
- is_hard_logic_turn
- test_cleaner_streaming.py
- .condense
- TestFerryCleanerSubprocess
- hardware.rs
- AtomicBool
- graphify
- ferry
- Value
- AtomicBool
- truncate_tool_content_smart
- code.rs
- directory.rs
- count_message_chars

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 52 edges
2. `Config` - 22 edges
3. `Router` - 22 edges
4. `SessionAffinityTracker` - 21 edges
5. `nudge_repeated_tool_failure()` - 20 edges
6. `ContextCondenser` - 18 edges
7. `UpstreamClient` - 17 edges
8. `ResponseCache` - 16 edges
9. `inspect_and_repair_local_tool_response()` - 15 edges
10. `TestFerryCleanerDirect` - 15 edges

## Surprising Connections (you probably didn't know these)
- `test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations()` --calls--> `suppress_base64_and_hex_blobs()`  [INFERRED]
  tests/issue_16_condenser_perf_test.rs → src/condenser/whitespace.rs
- `test_compute_prefix_hash_and_orchestrator_multipart_array_content()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_compute_prefix_hash_multibyte_utf8_at_8192_boundary_no_panic()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/truncation.rs
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs

## Import Cycles
- None detected.

## Communities (37 total, 13 thin omitted)

### Community 0 - "whitespace.rs"
Cohesion: 0.17
Nodes (12): test_collapse_excess_whitespace(), test_deduplicate_consecutive_lines_safe_for_code(), test_strip_ansi_codes(), test_strip_progress_artifacts(), test_suppress_base64_and_hex_blobs(), collapse_excess_whitespace(), COMMON_CODE_EXTS, deduplicate_lines() (+4 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.05
Nodes (14): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+6 more)

### Community 2 - "condenser.rs"
Cohesion: 0.07
Nodes (35): default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines(), default_keep_recent_turns() (+27 more)

### Community 4 - "upstream.rs"
Cohesion: 0.05
Nodes (40): LocalSlotGuard, coerce_xml_param_value(), declared_param_type(), DONE_SSE_CHUNK, extract_declared_tools(), format_sse_error_frame(), GuardedStream<S>, inspect_and_repair_local_tool_response() (+32 more)

### Community 5 - "Router"
Cohesion: 0.07
Nodes (34): COGNITIVE_TOOL_CONTEXT_KEYWORDS, detect_orchestrator(), estimate_tokens(), estimated_json_len(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS (+26 more)

### Community 6 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 7 - "SessionAffinityTracker"
Cohesion: 0.08
Nodes (15): AffinityType, NewSlot, PrefixHit, SessionHit, SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse(), test_anti_eviction_hysteresis() (+7 more)

### Community 8 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 9 - "cleaner_rules.py"
Cohesion: 0.08
Nodes (20): _clean_text_chunk(), collapse_excess_whitespace(), deduplicate_consecutive_lines(), is_base64_or_hex_payload(), is_directory_listing_line(), prune_git_diff_lockfiles(), segment_markdown_code_blocks(), shield_directory_traversals() (+12 more)

### Community 10 - "main.rs"
Cohesion: 0.10
Nodes (16): AppState, chat_completions_handler(), cors_options_handler(), embeddings_handler(), extract_session_id(), health_handler(), metrics_handler(), model_detail_handler() (+8 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 13 - "issue_17_telemetry_config_test.rs"
Cohesion: 0.20
Nodes (3): repo_root(), test_config_hygiene_no_dead_keys_no_lan_secrets_and_dockerfile_locked(), test_grafana_dashboard_valid_json_and_offload_query()

### Community 14 - "ContextCondenser"
Cohesion: 0.31
Nodes (4): ContextCondenser, test_clean_tool_text_json_minification(), test_protected_code_blocks_segmentation(), CondensationStats

### Community 15 - "condenser/mod.rs"
Cohesion: 0.17
Nodes (20): align_tool_outputs(), append_nudge_to_tool_message(), extract_preceding_tool_calls(), extract_tool_output_id(), extract_tool_output_text(), LOOP_GUARD_MARKER, LOOP_GUARD_SUFFIX, normalize_tool_arguments() (+12 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.19
Nodes (7): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryRouterAdversarial

### Community 17 - "test_cleaner_streaming.py"
Cohesion: 0.23
Nodes (8): _load_module_from_path(), test_cleaner_deduplicated_not_symlink_and_under_500_loc(), test_early_stdout_stderr_close_does_not_sigterm_child(), test_hook_json_roundtrip_both_entrypoints(), test_no_pythonpath_or_cwd_namespace_shadowing(), test_process_group_killed_on_sigterm(), test_streaming_incremental_utf8_and_partial_line_buffering(), test_streaming_mode_records_actual_saved_chars()

### Community 18 - ".condense"
Cohesion: 0.33
Nodes (7): is_diagnostic_tool_failure(), test_condenser_dual_key_support(), test_historical_tool_truncation_preserves_missing_constraint_ledger_anchors(), test_loop_nudge_preserves_json_minification_and_suffix_in_condense(), test_loop_nudge_preserves_lockfile_diff_pruning_and_suffix_in_condense(), test_loop_nudge_preserves_suffix_under_large_and_small_truncation_budgets(), CondenserConfig

### Community 20 - "hardware.rs"
Cohesion: 0.14
Nodes (22): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_bind(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_name() (+14 more)

### Community 21 - "AtomicBool"
Cohesion: 0.05
Nodes (16): compute_prefix_hash(), ChildGuard, test_4mib_payload_accepted_and_condensed_by_ferry_router(), test_cloud_429_emergency_eviction_releases_slot_on_kv_budget_failure(), test_cloud_429_respects_slot_limits_and_never_ping_pongs_back_to_cloud(), test_compute_prefix_hash_and_orchestrator_multipart_array_content(), test_compute_prefix_hash_multibyte_utf8_at_8192_boundary_no_panic(), test_local_only_never_calls_cloud_on_500_or_connection_refused() (+8 more)

### Community 25 - "Value"
Cohesion: 0.23
Nodes (7): condense_output(), main(), record_stats(), run_command_wrapped(), flush_pending(), handle_term(), terminate_process_group()

### Community 31 - "truncate_tool_content_smart"
Cohesion: 0.24
Nodes (11): test_multi_region_diagnostic_signal_preservation(), test_single_line_diagnostic_signal_preservation(), DIAGNOSTIC_SIGNALS, find_diagnostic_signal_index(), find_diagnostic_signal_regions(), safe_ceil_char_boundary(), safe_floor_char_boundary(), strip_think_tags() (+3 more)

### Community 32 - "code.rs"
Cohesion: 0.24
Nodes (9): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+1 more)

### Community 33 - "directory.rs"
Cohesion: 0.25
Nodes (8): DIR_SHIELD_PATTERNS, is_directory_listing_line(), LOCKFILE_EXTENSIONS, LOCKFILE_NAMES, prune_git_diff_lockfiles(), shield_directory_traversals(), test_prune_git_diff_lockfiles(), test_shield_directory_traversals()

### Community 35 - "count_message_chars"
Cohesion: 0.24
Nodes (9): is_tool_output_message(), test_repeated_rolling_compaction_merges_checkpoints_cleanly(), test_rolling_context_compaction_preserves_security_entities(), test_rolling_context_compaction_preserves_system_and_recent(), count_message_chars(), estimate_message_tokens(), estimate_tokens_from_chars(), RollingCompactionStats (+1 more)

## Knowledge Gaps
- **20 isolated node(s):** `EventBoundary`, `AfterSingleNewline`, `AfterFirstCr`, `AfterNewlineCr`, `MidLine` (+15 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 276 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **13 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `condenser.rs` to `upstream.rs`, `Router`, `SessionAffinityTracker`, `main.rs`, `.condense`, `hardware.rs`?**
  _High betweenness centrality (0.106) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `EventBoundary`, `AfterSingleNewline`, `AfterFirstCr` to the rest of the system?**
  _20 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.05202661826981246 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `main.rs`, `upstream.rs`, `Router`, `SessionAffinityTracker`?**
  _High betweenness centrality (0.091) - this node is a cross-community bridge._
- **Are the 2 inferred relationships involving `SessionAffinityTracker` (e.g. with `UpstreamClient` and `.with_affinity()`) actually correct?**
  _`SessionAffinityTracker` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Should `condenser.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.07346938775510205 - nodes in this community are weakly interconnected._