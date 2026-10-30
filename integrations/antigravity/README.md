# Ferry Antigravity Integration

Lightweight, client-side context reducer and lifecycle hook for Google Antigravity.

## Purpose

While **Ferry core** operates as an OpenAI/vLLM HTTP reverse proxy condensing context between clients and inference engines, this integration operates **at the tool execution boundary** inside Antigravity via its native `PreToolUse` lifecycle hook (`hooks.json`).

It prevents bloated terminal commands, test suites, builds, and repetitive polling loops from ever entering the conversation transcript in the first place, keeping the primary LLM context permanently lean and cutting prompt prefill latency.

## Architecture & Alignment with Ferry Core

This script maintains strict behavioral parity with `src/condenser.rs`:

| Feature | Ferry Rust Core (`src/condenser.rs`) | Antigravity Integration (`ferry_cleaner.py`) |
| :--- | :--- | :--- |
| **ANSI Stripping** | `strip_ansi_codes` | `strip_ansi` (strips `\x1b[...]` and OSC sequences) |
| **Progress Overwrites** | `strip_progress_artifacts` | `strip_progress_artifacts` (collapses `\r` overwrites) |
| **Whitespace Collapse** | `collapse_excess_whitespace` | `collapse_excess_whitespace` (max 2 consecutive newlines) |
| **Repetition Folding** | `fold_repeated_tool_failures` | `deduplicate_consecutive_lines` (folds 3+ identical consecutive lines) |
| **JSON Minification** | `minify_json_outputs` | `try_minify_json` (compacts pretty-printed JSON) |
| **Signal-Aware Truncation** | `truncate_tool_content_smart` | `truncate_signal_aware` (head 20, tail 15, scans `DIAGNOSTIC_SIGNALS`) |
| **Diagnostic Signals** | 21 canonical signals (`DIAGNOSTIC_SIGNALS`) | Exact 21 signals mirrored in `DIAGNOSTIC_SIGNALS` |
| **Exit Code Safety** | Handled by upstream caller | Preserves exact child exit code (`sys.exit(proc.returncode)`) |

> **Alignment Note**: When tuning diagnostic keywords or truncation rules in `src/condenser.rs`, update `integrations/antigravity/ferry_cleaner.py` to keep the reduction engines synchronized.

## Installation & Configuration

To enable globally in Antigravity, add to `~/.gemini/config/hooks.json`:

```json
{
  "ferry-cleaner": {
    "PreToolUse": [
      {
        "matcher": "run_command",
        "hooks": [
          {
            "type": "command",
            "command": "python3 /Users/steve/local/ferry/integrations/antigravity/ferry_cleaner.py --hook",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

## Running Tests

```bash
python3 /Users/steve/local/ferry/integrations/antigravity/test_cleaner.py
```

## Reduction Telemetry

Every execution logs its input, output, tokens saved, and cleaning triggers to:
```bash
cat /tmp/ferry_cleaner_stats.json
```
