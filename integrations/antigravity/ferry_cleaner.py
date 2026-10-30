#!/usr/bin/env python3
"""
Ferry Command Cleaner & Context Reducer for Antigravity.

Port of Ferry's Rust condenser logic (from ferry/src/condenser.rs):
1. Strips ANSI escape sequences and terminal noise.
2. Strips carriage-return (\r) progress overwrite artifacts.
3. Collapses excess whitespace.
4. Deduplicates consecutive repeating lines.
5. Minifies standalone pretty-printed JSON payloads.
6. Performs signal-aware head/tail truncation on large outputs,
   preserving critical error/diagnostic signals in the middle.
7. Preserves the exact exit code of executed commands.
8. Operates either as a command runner, a stdin pipe, or an Antigravity PreToolUse hook.
"""

import sys
import os
import re
import json
import subprocess
from typing import List, Tuple, Optional

# Diagnostic failure signals (from Ferry's condenser.rs DIAGNOSTIC_SIGNALS)
DIAGNOSTIC_SIGNALS = [
    "error:",
    "error[",
    "failed",
    "failure",
    "exception",
    "panic:",
    "panicked",
    "fatal:",
    "vulnerability",
    "vulnerabilities",
    "cwe-",
    "cve-",
    "assert",
    "traceback",
    "syntaxerror",
    "typeerror",
    "referenceerror",
    "status 500",
    "internal server error",
    "connection refused",
    "timed out",
    "timeout",
]

# Tuning constants
DEFAULT_MAX_CHARS = 3000
DEFAULT_HEAD_LINES = 20
DEFAULT_TAIL_LINES = 15

ANSI_ESCAPE_RE = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07')


def strip_ansi(text: str) -> str:
    """Removes terminal ANSI color and cursor codes."""
    return ANSI_ESCAPE_RE.sub('', text)


def strip_progress_artifacts(text: str) -> str:
    """Cleans up carriage return progress bar overwrites (e.g. download % lines)."""
    if '\r' not in text:
        return text
    lines = []
    for line in text.split('\n'):
        if '\r' in line:
            # The last segment after \r is what remained on screen
            parts = line.split('\r')
            line = parts[-1] if parts[-1].strip() else parts[0]
        lines.append(line)
    return '\n'.join(lines)


def collapse_excess_whitespace(text: str) -> str:
    """Collapses 3+ consecutive newlines into 2."""
    return re.sub(r'\n{3,}', '\n\n', text)


def deduplicate_consecutive_lines(text: str) -> str:
    """Folds consecutive identical lines into a single line with repetition count."""
    lines = text.splitlines()
    if len(lines) < 3:
        return text

    out: List[str] = []
    repeat_count = 1
    prev_line = None

    for line in lines:
        if prev_line is not None and line == prev_line and line.strip():
            repeat_count += 1
        else:
            if repeat_count > 1:
                if repeat_count >= 3:
                    out.append(f"[... Repeated {repeat_count - 1} more times: {prev_line.strip()} ...]")
                else:
                    out.append(prev_line)
                repeat_count = 1
            out.append(line)
            prev_line = line

    if repeat_count > 1:
        if repeat_count >= 3:
            out.append(f"[... Repeated {repeat_count - 1} more times: {prev_line.strip()} ...]")
        else:
            out.append(prev_line)

    return '\n'.join(out)


def try_minify_json(text: str) -> str:
    """Attempts to minify pretty-printed JSON if the whole string is JSON."""
    trimmed = text.strip()
    if (trimmed.startswith('{') and trimmed.endswith('}')) or (trimmed.startswith('[') and trimmed.endswith(']')):
        try:
            parsed = json.loads(trimmed)
            minified = json.dumps(parsed, separators=(',', ':'))
            if len(minified) < len(trimmed):
                return minified
        except Exception:
            pass
    return text


def truncate_signal_aware(text: str, head_lines: int = DEFAULT_HEAD_LINES, tail_lines: int = DEFAULT_TAIL_LINES) -> str:
    """Truncates output while preserving head, tail, and any diagnostic error regions in between."""
    lines = text.splitlines()
    if len(lines) <= (head_lines + tail_lines + 5):
        return text

    head = lines[:head_lines]
    tail = lines[-tail_lines:]
    mid = lines[head_lines:-tail_lines]

    # Search middle section for diagnostic signals
    signal_line_indices = []
    for i, line in enumerate(mid):
        line_lower = line.lower()
        if any(sig in line_lower for sig in DIAGNOSTIC_SIGNALS):
            signal_line_indices.append(i)

    if not signal_line_indices:
        # No error signals found in middle; clean truncate
        dropped_lines = len(mid)
        dropped_chars = sum(len(l) for l in mid)
        banner = f"\n[... Ferry Cleaner: Truncated {dropped_lines} lines (~{dropped_chars} chars) ...]\n"
        return '\n'.join(head) + banner + '\n'.join(tail)

    # Group signals with 1 line of context around them
    selected_mid_indices = set()
    for idx in signal_line_indices:
        for offset in range(-1, 2):
            target = idx + offset
            if 0 <= target < len(mid):
                selected_mid_indices.add(target)

    # Sort and produce middle snippet
    sorted_indices = sorted(selected_mid_indices)
    extracted_lines = []
    last_idx = None
    for idx in sorted_indices:
        if last_idx is not None and idx > last_idx + 1:
            extracted_lines.append(f"[... Truncated {idx - last_idx - 1} uninformative lines ...]")
        extracted_lines.append(mid[idx])
        last_idx = idx

    banner_start = f"\n[... Ferry Cleaner: Truncated {len(mid) - len(sorted_indices)} uninformative lines; preserved {len(signal_line_indices)} diagnostic signal(s) ...]\n"
    mid_content = '\n'.join(extracted_lines)
    return '\n'.join(head) + banner_start + mid_content + "\n\n" + '\n'.join(tail)


STATS_FILE = "/tmp/ferry_cleaner_stats.json"


def record_stats(command_str: str, raw_text: str, cleaned_text: str):
    """Appends condensation run statistics to STATS_FILE."""
    try:
        orig_chars = len(raw_text)
        cond_chars = len(cleaned_text)
        saved_chars = max(0, orig_chars - cond_chars)
        pct_saved = (saved_chars / orig_chars * 100.0) if orig_chars > 0 else 0.0
        est_tokens_saved = saved_chars // 4

        entry = {
            "timestamp": str(subprocess.check_output(["date", "+%Y-%m-%dT%H:%M:%S%z"]).decode().strip()),
            "command": command_str[:120],
            "orig_chars": orig_chars,
            "cond_chars": cond_chars,
            "saved_chars": saved_chars,
            "est_tokens_saved": est_tokens_saved,
            "pct_saved": round(pct_saved, 1),
            "ansi_stripped": bool(ANSI_ESCAPE_RE.search(raw_text)),
            "deduped": "[... Repeated" in cleaned_text,
            "truncated": "[... Ferry Cleaner: Truncated" in cleaned_text,
        }

        history = []
        if os.path.exists(STATS_FILE):
            try:
                with open(STATS_FILE, "r") as f:
                    history = json.load(f)
            except Exception:
                history = []

        history.append(entry)
        with open(STATS_FILE, "w") as f:
            json.dump(history, f, indent=2)
    except Exception:
        pass


def condense_output(raw_text: str, max_chars: int = DEFAULT_MAX_CHARS) -> str:
    """Full condensation pipeline."""
    if not raw_text:
        return ""

    # 1. Strip ANSI noise
    text = strip_ansi(raw_text)

    # 2. Strip progress overwrites
    text = strip_progress_artifacts(text)

    # 3. Collapse excess whitespace
    text = collapse_excess_whitespace(text)

    # 4. Deduplicate consecutive repeated lines
    text = deduplicate_consecutive_lines(text)

    # 5. Try minifying JSON if applicable
    text = try_minify_json(text)

    # 6. Signal-aware smart truncation if size exceeds limit
    if len(text) > max_chars:
        text = truncate_signal_aware(text)

    return text


def run_command_wrapped(command_str: str) -> int:
    """Executes a command string through shell, cleans its output, and preserves the exit code."""
    proc = subprocess.Popen(
        command_str,
        shell=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        executable="/bin/zsh" if os.path.exists("/bin/zsh") else "/bin/sh",
    )
    stdout_raw, _ = proc.communicate()
    cleaned = condense_output(stdout_raw)
    record_stats(command_str, stdout_raw, cleaned)
    if cleaned:
        sys.stdout.write(cleaned + "\n")
        sys.stdout.flush()
    return proc.returncode


def handle_hook():
    """Handles Antigravity PreToolUse hook invocation over stdin/stdout."""
    try:
        raw_input = sys.stdin.read()
        if not raw_input.strip():
            sys.stdout.write(json.dumps({"decision": "allow"}))
            return

        payload = json.loads(raw_input)
        tool_call = payload.get("toolCall", {})
        tool_name = tool_call.get("name")
        args = tool_call.get("args", {})
        cmd = args.get("CommandLine", "")

        this_script = os.path.abspath(__file__)

        # Only wrap run_command if not already wrapped
        if tool_name == "run_command" and cmd and "ferry_cleaner.py" not in cmd:
            # Escape quotes safely or pass via wrap
            escaped_cmd = cmd.replace("'", "'\"'\"'")
            new_cmd = f"python3 '{this_script}' -- '{escaped_cmd}'"
            response = {
                "decision": "allow",
                "overwrite": {
                    "CommandLine": new_cmd
                }
            }
            sys.stdout.write(json.dumps(response))
            return

        sys.stdout.write(json.dumps({"decision": "allow"}))
    except Exception as e:
        # Fallback gracefully so we never block user workflow
        sys.stdout.write(json.dumps({"decision": "allow"}))


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--hook":
        handle_hook()
        return

    if len(sys.argv) > 1 and sys.argv[1] == "--":
        # Run command passed as arguments
        cmd_to_run = " ".join(sys.argv[2:])
        exit_code = run_command_wrapped(cmd_to_run)
        sys.exit(exit_code)

    # Otherwise read from stdin (filter mode)
    if not sys.stdin.isatty():
        raw_text = sys.stdin.read()
        cleaned = condense_output(raw_text)
        sys.stdout.write(cleaned + "\n")
        return

    # Help
    print("Ferry Cleaner for Antigravity")
    print("Usage:")
    print("  python3 ferry_cleaner.py -- <command to run>")
    print("  cat large.log | python3 ferry_cleaner.py")
    print("  python3 ferry_cleaner.py --hook (Antigravity PreToolUse mode)")


if __name__ == "__main__":
    main()
