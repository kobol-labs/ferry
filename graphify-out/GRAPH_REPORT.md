# Graph Report - ferry  (2026-11-22)

## Corpus Check
- 45 files · ~65,765 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 4 file(s) not represented in the graph (top: (none) 2, .toml 2)

## Summary
- 894 nodes · 1742 edges · 43 communities (29 shown, 14 thin omitted)
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 133 edges (avg confidence: 0.78)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `9103ee8c`
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
- count_message_chars
- .condense
- condenser/mod.rs
- is_hard_logic_turn
- antigravity/ferry_cleaner.py
- LocalToolInspection
- issue_17_telemetry_config_test.rs
- hardware.rs
- issue_16_condenser_perf_test.rs
- graphify
- ferry
- truncate_tool_content_smart
- code.rs
- directory.rs
- .forward_chat_with_cloud_fallback
- Value
- SseFrameTracker
- TargetProvider
- .poll_next
- SseScanState
- issue_37_security_auth_cors_test.rs

## God Nodes (most connected - your core abstractions)
1. `MetricsTracker` - 61 edges
2. `SessionAffinityTracker` - 27 edges
3. `Router` - 24 edges
4. `Config` - 22 edges
5. `UpstreamClient` - 20 edges
6. `ContextCondenser` - 20 edges
7. `nudge_repeated_tool_failure()` - 20 edges
8. `inspect_and_repair_local_tool_response()` - 19 edges
9. `AffinityType` - 19 edges
10. `AppState` - 18 edges

## Surprising Connections (you probably didn't know these)
- `test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations()` --calls--> `suppress_base64_and_hex_blobs()`  [INFERRED]
  tests/issue_16_condenser_perf_test.rs → src/condenser/whitespace.rs
- `test_zero_allocation_char_counting_parity()` --calls--> `count_message_chars()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/stats.rs
- `test_ac4_estimated_json_len_zero_allocation_and_exact_parity()` --calls--> `estimate_tokens_from_chars()`  [INFERRED]
  tests/issue_16_condenser_perf_test.rs → src/condenser/stats.rs
- `test_signal_aware_truncation_preserves_middle_error()` --calls--> `truncate_tool_content_smart()`  [INFERRED]
  tests/condenser_invariants.rs → src/condenser/truncation.rs
- `test_compute_prefix_hash_and_orchestrator_multipart_array_content()` --calls--> `compute_prefix_hash()`  [INFERRED]
  tests/issue_14_router_affinity_test.rs → src/affinity.rs

## Import Cycles
- None detected.

## Communities (43 total, 14 thin omitted)

### Community 0 - "whitespace.rs"
Cohesion: 0.13
Nodes (16): ContextCondenser, test_clean_tool_text_json_minification(), test_collapse_excess_whitespace(), test_deduplicate_consecutive_lines_safe_for_code(), test_protected_code_blocks_segmentation(), test_strip_ansi_codes(), test_strip_progress_artifacts(), test_suppress_base64_and_hex_blobs() (+8 more)

### Community 1 - "MetricsTracker"
Cohesion: 0.05
Nodes (17): circuit_open_reason(), CircuitState, Closed, Open, evaluate_circuit_state(), test_circuit_state_evaluation(), render_json(), test_json_exposition_decoupled() (+9 more)

### Community 2 - "condenser.rs"
Cohesion: 0.07
Nodes (42): CondenserConfig, default_compact_keep_recent_turns(), default_compact_threshold_tokens(), default_condensation_enabled(), default_deduplicate_lines(), default_drop_historical_reasoning(), default_fold_repeated_tool_failures(), default_head_lines() (+34 more)

### Community 4 - "upstream.rs"
Cohesion: 0.11
Nodes (21): coerce_xml_param_value(), declared_param_type(), DONE_SSE_CHUNK, extract_assistant_content_str(), extract_declared_tools(), inspect_and_repair_local_tool_response(), is_valid_tool_identifier(), KEEPALIVE_SSE_CHUNK (+13 more)

### Community 5 - "Router"
Cohesion: 0.08
Nodes (29): COGNITIVE_TOOL_CONTEXT_KEYWORDS, detect_orchestrator(), estimated_json_len(), extract_message_text(), get_last_tool_args(), get_last_tool_name(), HARD_LOGIC_DIAGNOSTIC_SIGNALS, HARD_LOGIC_USER_KEYWORDS (+21 more)

### Community 6 - "with"
Cohesion: 0.06
Nodes (33): permissions, runs-on, steps, env, IMAGE_NAME, REGISTRY, jobs, build-and-push (+25 more)

### Community 7 - "SessionAffinityTracker"
Cohesion: 0.06
Nodes (31): AffinityType, NewSlot, PrefixHit, SessionHit, compute_prefix_hash(), SessionAffinityTracker, SessionEntry, test_affinity_stickiness_and_slot_reuse() (+23 more)

### Community 8 - "⛴️ Ferry"
Cohesion: 0.07
Nodes (28): 1. Configuration File (`ferry.toml`), 1. Health Status (`GET /health`), 2. JSON Statistics (`GET /stats`), 2. Tuning Parameters for Your Hardware, 3. Client Control Headers & Observability, 3. Prometheus Scrape Target (`GET /metrics`), 4. Environment Variable Overrides (Zero-Config Docker), 4. Grafana Integration & Dashboard (+20 more)

### Community 9 - "cleaner_rules.py"
Cohesion: 0.06
Nodes (28): _clean_text_chunk(), collapse_excess_whitespace(), condense_output(), deduplicate_consecutive_lines(), is_base64_or_hex_payload(), is_directory_listing_line(), prune_git_diff_lockfiles(), segment_markdown_code_blocks() (+20 more)

### Community 10 - "main.rs"
Cohesion: 0.06
Nodes (32): AppState, auth_middleware(), build_router(), chat_completions_handler(), cors_middleware(), cors_options_handler(), embeddings_handler(), extract_session_id() (+24 more)

### Community 12 - "steps"
Cohesion: 0.10
Nodes (22): jobs, test-python, test-rust, name, on, pull_request, push, permissions (+14 more)

### Community 13 - "count_message_chars"
Cohesion: 0.27
Nodes (8): test_repeated_rolling_compaction_merges_checkpoints_cleanly(), test_rolling_context_compaction_preserves_security_entities(), test_rolling_context_compaction_preserves_system_and_recent(), count_message_chars(), estimate_message_tokens(), estimate_tokens_from_chars(), RollingCompactionStats, test_zero_allocation_char_counting_parity()

### Community 14 - ".condense"
Cohesion: 0.25
Nodes (9): extract_tool_output_text(), is_diagnostic_tool_failure(), test_adaptive_squeeze_preserves_parallel_tail_turn_and_squeezes_output_and_array_content(), test_condenser_dual_key_support(), test_emergency_compact_to_fit_responses_api_and_anthropic_tool_pairs(), test_historical_tool_truncation_preserves_missing_constraint_ledger_anchors(), test_loop_nudge_preserves_json_minification_and_suffix_in_condense(), test_loop_nudge_preserves_lockfile_diff_pruning_and_suffix_in_condense() (+1 more)

### Community 15 - "condenser/mod.rs"
Cohesion: 0.17
Nodes (18): align_tool_outputs(), append_nudge_to_tool_message(), extract_preceding_tool_calls(), LOOP_GUARD_MARKER, LOOP_GUARD_SUFFIX, normalize_tool_arguments(), nudge_repeated_tool_failure(), ParsedToolCall (+10 more)

### Community 16 - "is_hard_logic_turn"
Cohesion: 0.19
Nodes (7): extract_message_text(), get_last_tool_name(), has_real_diagnostic_signal(), is_hard_logic_turn(), is_mechanical_user_command(), run_adversarial_suite(), TestFerryRouterAdversarial

### Community 17 - "antigravity/ferry_cleaner.py"
Cohesion: 0.06
Nodes (9): _load_module_from_path(), test_cleaner_deduplicated_not_symlink_and_under_500_loc(), test_early_stdout_stderr_close_does_not_sigterm_child(), test_hook_json_roundtrip_both_entrypoints(), test_no_pythonpath_or_cwd_namespace_shadowing(), test_process_group_killed_on_sigterm(), test_streaming_incremental_utf8_and_partial_line_buffering(), test_streaming_mode_records_actual_saved_chars() (+1 more)

### Community 18 - "LocalToolInspection"
Cohesion: 0.50
Nodes (4): LocalToolInspection, FailoverRequired, Repaired, ValidUnchanged

### Community 19 - "issue_17_telemetry_config_test.rs"
Cohesion: 0.19
Nodes (4): repo_root(), test_config_hygiene_no_dead_keys_no_lan_secrets_and_dockerfile_locked(), test_grafana_dashboard_valid_json_and_offload_query(), test_issue_32_env_overrides_routing_condenser_and_grafana_4_slots()

### Community 20 - "hardware.rs"
Cohesion: 0.14
Nodes (22): CloudConfig, default_adaptive_squeeze_ratio(), default_adaptive_squeezing(), default_bind(), default_circuit_breaker_threshold(), default_cloud_base(), default_cloud_model(), default_cloud_name() (+14 more)

### Community 21 - "issue_16_condenser_perf_test.rs"
Cohesion: 0.10
Nodes (8): ChildGuard, count_allocations(), CountingAlloc, GLOBAL_ALLOC, test_ac3_x_ferry_condense_false_and_cloud_target_exemptions(), test_ac3_x_ferry_condense_false_bypasses_adaptive_squeeze(), test_ac4_estimated_json_len_zero_allocation_and_exact_parity(), test_ac4_suppress_base64_and_hex_blobs_exact_behavior_and_minimal_allocations()

### Community 31 - "truncate_tool_content_smart"
Cohesion: 0.23
Nodes (12): head_tail_chars(), test_multi_region_diagnostic_signal_preservation(), test_single_line_diagnostic_signal_preservation(), DIAGNOSTIC_SIGNALS, find_diagnostic_signal_index(), find_diagnostic_signal_regions(), safe_ceil_char_boundary(), safe_floor_char_boundary() (+4 more)

### Community 32 - "code.rs"
Cohesion: 0.24
Nodes (9): collect_discovered_entities(), ContentSegment, Code, Text, extract_anchors_internal(), extract_compact_tool_arguments(), extract_critical_anchors(), extract_tool_anchors() (+1 more)

### Community 33 - "directory.rs"
Cohesion: 0.25
Nodes (8): DIR_SHIELD_PATTERNS, is_directory_listing_line(), LOCKFILE_EXTENSIONS, LOCKFILE_NAMES, prune_git_diff_lockfiles(), shield_directory_traversals(), test_prune_git_diff_lockfiles(), test_shield_directory_traversals()

### Community 34 - ".forward_chat_with_cloud_fallback"
Cohesion: 0.32
Nodes (5): RouteDecision, normalize_wire_format(), ResponseMeta, test_cloud_transport_error_and_http_429_trigger_inflight_local_rescue_with_failover_headers(), UpstreamClient

### Community 35 - "Value"
Cohesion: 0.30
Nodes (12): collect_call_ids_from_msg(), collect_output_call_ids(), extract_tool_output_id(), find_preserved_tail_start(), is_compact_tool_call(), is_compact_tool_output(), is_rolling_checkpoint_message(), is_tool_output_message() (+4 more)

### Community 36 - "SseFrameTracker"
Cohesion: 0.19
Nodes (7): format_sse_error_frame(), is_pure_sse_comment_chunk(), looks_like_json_value_tail(), SseFrameTracker, test_transform_strata_chunk_comment(), transform_strata_chunk(), test_sse_error_frame_escapes_multiline_quotes_and_backslashes()

### Community 37 - "TargetProvider"
Cohesion: 0.67
Nodes (3): TargetProvider, Cloud, Local

### Community 42 - "SseScanState"
Cohesion: 0.33
Nodes (6): SseScanState, AfterFirstCr, AfterNewlineCr, AfterSingleNewline, EventBoundary, MidLine

### Community 48 - "issue_37_security_auth_cors_test.rs"
Cohesion: 0.11
Nodes (14): apply_cors_headers(), constant_time_eq(), CORS_ALLOW_HEADERS, CORS_ALLOW_METHODS, CORS_EXPOSE_HEADERS, extract_inbound_api_key(), is_authorized(), is_loopback_bind() (+6 more)

## Knowledge Gaps
- **20 isolated node(s):** `EventBoundary`, `AfterSingleNewline`, `AfterFirstCr`, `AfterNewlineCr`, `MidLine` (+15 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 289 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **14 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Config` connect `condenser.rs` to `.forward_chat_with_cloud_fallback`, `upstream.rs`, `Router`, `SessionAffinityTracker`, `main.rs`, `hardware.rs`?**
  _High betweenness centrality (0.103) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `SessionAffinityTracker` (e.g. with `main.rs` and `UpstreamClient`) actually correct?**
  _`SessionAffinityTracker` has 4 INFERRED edges - model-reasoned connections that need verification._
- **What connects `EventBoundary`, `AfterSingleNewline`, `AfterFirstCr` to the rest of the system?**
  _20 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `whitespace.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.12681159420289856 - nodes in this community are weakly interconnected._
- **Why does `MetricsTracker` connect `MetricsTracker` to `.forward_chat_with_cloud_fallback`, `upstream.rs`, `Router`, `SessionAffinityTracker`, `main.rs`?**
  _High betweenness centrality (0.094) - this node is a cross-community bridge._
- **Are the 8 inferred relationships involving `Config` (e.g. with `CondenserConfig` and `CloudConfig`) actually correct?**
  _`Config` has 8 INFERRED edges - model-reasoned connections that need verification._
- **Should `MetricsTracker` be split into smaller, more focused modules?**
  _Cohesion score 0.0506558118498417 - nodes in this community are weakly interconnected._