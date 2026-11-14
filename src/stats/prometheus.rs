use super::tracker::StatsSnapshot;

pub fn render_prometheus(s: &StatsSnapshot) -> String {
    format!(
        "# HELP ferry_requests_dispatched_total Total number of LLM requests dispatched\n\
         # TYPE ferry_requests_dispatched_total counter\n\
         ferry_requests_dispatched_total {total}\n\
         # HELP ferry_provider_requests_total Total number of LLM requests routed per provider\n\
         # TYPE ferry_provider_requests_total counter\n\
         ferry_provider_requests_total{{provider=\"local\"}} {local}\n\
         ferry_provider_requests_total{{provider=\"cloud\"}} {cloud}\n\
         # HELP ferry_routing_events_total Total routing spillover and failover events\n\
         # TYPE ferry_routing_events_total counter\n\
         ferry_routing_events_total{{event=\"spillover\"}} {spill}\n\
         ferry_routing_events_total{{event=\"failover\"}} {fail}\n\
         # HELP ferry_active_slots Current active local hardware slots\n\
         # TYPE ferry_active_slots gauge\n\
         ferry_active_slots {slots}\n\
         # HELP ferry_active_local_tokens Current estimated tokens locked in active local slots\n\
         # TYPE ferry_active_local_tokens gauge\n\
         ferry_active_local_tokens {active_tokens}\n\
         # HELP ferry_adaptive_squeezed_total Total requests rescued for local execution via adaptive squeezing\n\
         # TYPE ferry_adaptive_squeezed_total counter\n\
         ferry_adaptive_squeezed_total {adaptive_squeezed}\n\
         # HELP ferry_strata_online Health status of the local hardware provider (1 = online, 0 = offline)\n\
         # TYPE ferry_strata_online gauge\n\
         ferry_strata_online {strata_online}\n\
         # HELP ferry_circuit_open Whether the circuit breaker is currently open (1 = open, 0 = closed)\n\
         # TYPE ferry_circuit_open gauge\n\
         ferry_circuit_open {circuit_open}\n\
         # HELP ferry_circuit_trips_total Total times the circuit breaker tripped\n\
         # TYPE ferry_circuit_trips_total counter\n\
         ferry_circuit_trips_total {circuit_trips}\n\
         # HELP ferry_tokens_estimated Total estimated tokens processed\n\
         # TYPE ferry_tokens_estimated counter\n\
         ferry_tokens_estimated {tokens}\n\
         # HELP ferry_provider_tokens_estimated_total Total estimated tokens processed per provider\n\
         # TYPE ferry_provider_tokens_estimated_total counter\n\
         ferry_provider_tokens_estimated_total{{provider=\"local\"}} {ltokens}\n\
         ferry_provider_tokens_estimated_total{{provider=\"cloud\"}} {ctokens}\n\
         # HELP ferry_request_duration_ms_total Total latency of completed LLM requests in milliseconds\n\
         # TYPE ferry_request_duration_ms_total counter\n\
         ferry_request_duration_ms_total{{provider=\"local\"}} {local_ms}\n\
         ferry_request_duration_ms_total{{provider=\"cloud\"}} {cloud_ms}\n\
         # HELP ferry_last_request_duration_ms Latency of most recent completed request in milliseconds\n\
         # TYPE ferry_last_request_duration_ms gauge\n\
         ferry_last_request_duration_ms {last_ms}\n\
         # HELP ferry_condensed_requests_total Total requests optimized by condensation\n\
         # TYPE ferry_condensed_requests_total counter\n\
         ferry_condensed_requests_total {condensed_reqs}\n\
         # HELP ferry_chars_saved_total Total characters pruned from prompts\n\
         # TYPE ferry_chars_saved_total counter\n\
         ferry_chars_saved_total {chars_saved}\n\
         # HELP ferry_tokens_saved_estimated_total Total tokens estimated saved\n\
         # TYPE ferry_tokens_saved_estimated_total counter\n\
         ferry_tokens_saved_estimated_total {tokens_saved}\n\
         # HELP ferry_condensation_cleaner_actions_total Total actions performed per cleaner filter\n\
         # TYPE ferry_condensation_cleaner_actions_total counter\n\
         ferry_condensation_cleaner_actions_total{{action=\"reasoning_dropped\"}} {reasoning_dropped}\n\
         ferry_condensation_cleaner_actions_total{{action=\"tool_truncated\"}} {tools_truncated}\n\
         ferry_condensation_cleaner_actions_total{{action=\"ansi_stripped\"}} {ansi_stripped}\n\
         ferry_condensation_cleaner_actions_total{{action=\"progress_cleaned\"}} {progress_cleaned}\n\
         ferry_condensation_cleaner_actions_total{{action=\"lines_deduped\"}} {lines_deduped}\n\
         ferry_condensation_cleaner_actions_total{{action=\"whitespace_collapsed\"}} {whitespace_collapsed}\n\
         ferry_condensation_cleaner_actions_total{{action=\"json_minified\"}} {json_minified}\n\
         ferry_condensation_cleaner_actions_total{{action=\"signals_extracted\"}} {signals_extracted}\n\
         ferry_condensation_cleaner_actions_total{{action=\"loops_collapsed\"}} {loops_collapsed}\n\
         ferry_condensation_cleaner_actions_total{{action=\"lockfile_diffs_pruned\"}} {lockfile_pruned}\n\
         ferry_condensation_cleaner_actions_total{{action=\"base64_blobs_suppressed\"}} {base64_suppressed}\n\
         ferry_condensation_cleaner_actions_total{{action=\"dir_traversals_shielded\"}} {dir_shielded}\n\
         # HELP ferry_loop_nudges_total Total times the stateless loop guard nudged repeated identical failing tool calls\n\
         # TYPE ferry_loop_nudges_total counter\n\
         ferry_loop_nudges_total {loop_nudges}\n\
         # HELP ferry_ledger_checks_passed_total Total compression operations verified by the constraint ledger\n\
         # TYPE ferry_ledger_checks_passed_total counter\n\
         ferry_ledger_checks_passed_total {ledger_passed}\n\
         # HELP ferry_ledger_vetoed_total Total compression operations vetoed by the constraint ledger\n\
         # TYPE ferry_ledger_vetoed_total counter\n\
         ferry_ledger_vetoed_total {ledger_vetoed}\n\
         # HELP ferry_last_prompt_tokens_original Estimated tokens before condensation on most recent request\n\
         # TYPE ferry_last_prompt_tokens_original gauge\n\
         ferry_last_prompt_tokens_original {last_orig}\n\
         # HELP ferry_last_prompt_tokens_condensed Estimated tokens after condensation on most recent request\n\
         # TYPE ferry_last_prompt_tokens_condensed gauge\n\
         ferry_last_prompt_tokens_condensed {last_cond}\n\
         # HELP ferry_last_condensation_savings_percent Percentage of prompt tokens pruned on most recent request\n\
         # TYPE ferry_last_condensation_savings_percent gauge\n\
         ferry_last_condensation_savings_percent {last_pct:.2}\n\
         # HELP ferry_last_condensation_chars_saved Characters pruned on most recent request\n\
         # TYPE ferry_last_condensation_chars_saved gauge\n\
         ferry_last_condensation_chars_saved {last_chars}\n\
         # HELP ferry_affinity_hits_total Total requests routed to their bound session slot (affinity hit)\n\
         # TYPE ferry_affinity_hits_total counter\n\
         ferry_affinity_hits_total {aff_hits}\n\
         # HELP ferry_affinity_misses_total Total requests that were allocated a new or alternative slot\n\
         # TYPE ferry_affinity_misses_total counter\n\
         ferry_affinity_misses_total {aff_misses}\n\
         # HELP ferry_rolling_compact_total Total times rolling context compaction triggered\n\
         # TYPE ferry_rolling_compact_total counter\n\
         ferry_rolling_compact_total {rolling_compacts}\n\
         # HELP ferry_rolling_compact_chars_saved_total Total characters pruned via rolling compaction\n\
         # TYPE ferry_rolling_compact_chars_saved_total counter\n\
         ferry_rolling_compact_chars_saved_total {rolling_chars}\n\
         # HELP ferry_queued_requests_total Total requests that waited in local contention queue\n\
         # TYPE ferry_queued_requests_total counter\n\
         ferry_queued_requests_total {queued}\n\
         # HELP ferry_queue_wait_ms_total Total milliseconds spent waiting in contention queue\n\
         # TYPE ferry_queue_wait_ms_total counter\n\
         ferry_queue_wait_ms_total {queue_wait}\n\
         # HELP ferry_last_queue_wait_ms Latency of most recent contention queue wait in ms\n\
         # TYPE ferry_last_queue_wait_ms gauge\n\
         ferry_last_queue_wait_ms {last_queue}\n\
         # HELP ferry_orchestrator_requests_total Total requests identified and prioritized as orchestrator/root turns\n\
         # TYPE ferry_orchestrator_requests_total counter\n\
         ferry_orchestrator_requests_total {orchestrator_total}\n\
         # HELP ferry_orchestrator_spillover_total Total orchestrator turns spilled to cloud due to local saturation\n\
         # TYPE ferry_orchestrator_spillover_total counter\n\
         ferry_orchestrator_spillover_total {orchestrator_spill}\n\
         # HELP ferry_pillar_requests_total Total requests processed per sentinel pipeline pillar\n\
         # TYPE ferry_pillar_requests_total counter\n\
         ferry_pillar_requests_total{{pillar=\"security\"}} {sec_reqs}\n\
         ferry_pillar_requests_total{{pillar=\"fuzzer\"}} {fuzz_reqs}\n\
         ferry_pillar_requests_total{{pillar=\"explorer\"}} {exp_reqs}\n",
        total = s.total_requests,
        local = s.local_requests,
        cloud = s.cloud_requests,
        spill = s.spillover_requests,
        fail = s.failover_requests,
        slots = s.active_local_slots,
        active_tokens = s.active_local_tokens,
        adaptive_squeezed = s.adaptive_squeezed_count,
        strata_online = if s.strata_online { 1 } else { 0 },
        circuit_open = if s.circuit_open { 1 } else { 0 },
        circuit_trips = s.circuit_trips_total,
        tokens = s.total_tokens_est,
        ltokens = s.local_tokens_est,
        ctokens = s.cloud_tokens_est,
        local_ms = s.local_duration_ms_total,
        cloud_ms = s.cloud_duration_ms_total,
        last_ms = s.last_duration_ms,
        condensed_reqs = s.condensed_requests,
        chars_saved = s.chars_saved,
        tokens_saved = s.tokens_saved_est,
        reasoning_dropped = s.reasoning_blocks_dropped,
        tools_truncated = s.tool_outputs_truncated,
        ansi_stripped = s.ansi_stripped_count,
        progress_cleaned = s.progress_cleaned_count,
        lines_deduped = s.lines_deduped_count,
        whitespace_collapsed = s.whitespace_collapsed_count,
        json_minified = s.json_minified_count,
        signals_extracted = s.signals_extracted_count,
        loops_collapsed = s.loops_collapsed_count,
        lockfile_pruned = s.lockfile_diffs_pruned_count,
        base64_suppressed = s.base64_blobs_suppressed_count,
        dir_shielded = s.dir_traversals_shielded_count,
        loop_nudges = s.loop_nudges_total,
        ledger_passed = s.ledger_checks_passed,
        ledger_vetoed = s.ledger_vetoed_count,
        last_orig = s.last_prompt_tokens_original,
        last_cond = s.last_prompt_tokens_condensed,
        last_pct = s.last_savings_percent,
        last_chars = s.last_chars_saved,
        aff_hits = s.affinity_hits,
        aff_misses = s.affinity_misses,
        rolling_compacts = s.rolling_compact_count,
        rolling_chars = s.rolling_compact_chars_saved,
        queued = s.queued_requests_total,
        queue_wait = s.queue_wait_ms_total,
        last_queue = s.last_queue_wait_ms,
        orchestrator_total = s.orchestrator_requests_total,
        orchestrator_spill = s.orchestrator_spillover_total,
        sec_reqs = s.security_requests_total,
        fuzz_reqs = s.fuzzer_requests_total,
        exp_reqs = s.explorer_requests_total,
    )
}
