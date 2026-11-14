#!/usr/bin/env python3
"""
Ferry Command Cleaner & Context Reducer for Antigravity.

Executes shell commands, stdin filters, or PreToolUse hook rewrites while condensing
noisy terminal output and preserving diagnostic signals and exact exit codes.
"""

import codecs
import importlib.util
import json
import os
import selectors
import shlex
import signal
import subprocess
import sys
import threading
import time
from datetime import datetime
from typing import List, Optional


def _load_cleaner_rules():
    rules_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cleaner_rules.py")
    spec = importlib.util.spec_from_file_location("_ferry_cleaner_rules", rules_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"Unable to load cleaner_rules from {rules_path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_rules = _load_cleaner_rules()
ANSI_ESCAPE_RE = _rules.ANSI_ESCAPE_RE
BASE64_DATA_URI_RE = _rules.BASE64_DATA_URI_RE
BASE64_RAW_RE = _rules.BASE64_RAW_RE
COMMON_CODE_EXTENSIONS = _rules.COMMON_CODE_EXTENSIONS
DEFAULT_HEAD_LINES = _rules.DEFAULT_HEAD_LINES
DEFAULT_MAX_CHARS = _rules.DEFAULT_MAX_CHARS
DEFAULT_TAIL_LINES = _rules.DEFAULT_TAIL_LINES
DIAGNOSTIC_SIGNALS = _rules.DIAGNOSTIC_SIGNALS
DIR_SHIELD_PATTERNS = _rules.DIR_SHIELD_PATTERNS
LOCKFILE_EXTENSIONS = _rules.LOCKFILE_EXTENSIONS
LOCKFILE_NAMES = _rules.LOCKFILE_NAMES
_clean_text_chunk = _rules._clean_text_chunk
clean_output = _rules.clean_output
collapse_excess_whitespace = _rules.collapse_excess_whitespace
condense_output = _rules.condense_output
deduplicate_consecutive_lines = _rules.deduplicate_consecutive_lines
is_base64_or_hex_payload = _rules.is_base64_or_hex_payload
is_directory_listing_line = _rules.is_directory_listing_line
prune_git_diff_lockfiles = _rules.prune_git_diff_lockfiles
segment_markdown_code_blocks = _rules.segment_markdown_code_blocks
shield_directory_traversals = _rules.shield_directory_traversals
strip_ansi = _rules.strip_ansi
strip_progress_artifacts = _rules.strip_progress_artifacts
suppress_base64_blobs = _rules.suppress_base64_blobs
truncate_signal_aware = _rules.truncate_signal_aware
try_minify_json = _rules.try_minify_json

STATS_FILE = "/tmp/ferry_cleaner_stats.json"
MAX_STATS_HISTORY = 500


def record_stats(command_str: str, raw_text: str, cleaned_text: str) -> None:
    """Appends condensation run statistics to STATS_FILE (or FERRY_CLEANER_STATS_FILE)."""
    try:
        stats_path = os.environ.get("FERRY_CLEANER_STATS_FILE") or STATS_FILE
        orig_chars = len(raw_text)
        cond_chars = len(cleaned_text)
        saved_chars = max(0, orig_chars - cond_chars)
        pct_saved = (saved_chars / orig_chars * 100.0) if orig_chars > 0 else 0.0
        est_tokens_saved = saved_chars // 4

        entry = {
            "timestamp": datetime.now().astimezone().strftime("%Y-%m-%dT%H:%M:%S%z"),
            "command": command_str[:120],
            "orig_chars": orig_chars,
            "cond_chars": cond_chars,
            "saved_chars": saved_chars,
            "est_tokens_saved": est_tokens_saved,
            "pct_saved": round(pct_saved, 1),
            "ansi_stripped": bool(ANSI_ESCAPE_RE.search(raw_text)),
            "deduped": "[... Repeated" in cleaned_text,
            "truncated": "[... Ferry Cleaner: Truncated" in cleaned_text,
            "lockfile_pruned": "[... Ferry: lockfile/asset diff suppressed" in cleaned_text,
            "base64_suppressed": "[... Ferry: base64" in cleaned_text,
            "dir_shielded": "directory entries in " in cleaned_text,
        }

        history = []
        if os.path.exists(stats_path):
            try:
                with open(stats_path, "r", encoding="utf-8") as f:
                    loaded = json.load(f)
                    if isinstance(loaded, list):
                        history = loaded
            except Exception:
                history = []

        if len(history) >= MAX_STATS_HISTORY:
            history = history[-(MAX_STATS_HISTORY - 1) :]
        history.append(entry)
        with open(stats_path, "w", encoding="utf-8") as f:
            json.dump(history, f, indent=2)
    except Exception:
        pass


def terminate_process_group(proc: subprocess.Popen, sig: int = signal.SIGTERM) -> None:
    """Signals the entire child process group so grandchildren never outlive the wrapper."""
    pgid = proc.pid
    try:
        os.killpg(pgid, sig)
    except ProcessLookupError:
        return
    except Exception:
        try:
            proc.terminate()
        except Exception:
            pass
        return

    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            try:
                os.killpg(pgid, 0)
            except ProcessLookupError:
                return
            except PermissionError:
                break
        time.sleep(0.02)

    try:
        os.killpg(pgid, signal.SIGKILL)
    except Exception:
        pass
    try:
        proc.wait(timeout=0.5)
    except Exception:
        pass


def run_command_wrapped(command_str: str, streaming_threshold_secs: float = 2.0) -> int:
    """Executes a command string in its own process group, cleans output, and preserves exit code."""
    proc = subprocess.Popen(
        command_str,
        shell=True,
        stdin=sys.stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        executable="/bin/zsh" if os.path.exists("/bin/zsh") else "/bin/sh",
        start_new_session=True,
    )

    start_time = time.monotonic()
    decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
    raw_chunks: List[str] = []
    emitted_chunks: List[str] = []
    pending_buf = ""
    streaming_mode = False
    flushed_any = False

    def flush_pending(final: bool = False) -> None:
        nonlocal pending_buf, flushed_any
        if not pending_buf:
            return
        if final:
            batch_raw = pending_buf
            pending_buf = ""
        else:
            last_nl = pending_buf.rfind("\n")
            if last_nl == -1:
                return
            batch_raw = pending_buf[: last_nl + 1]
            pending_buf = pending_buf[last_nl + 1 :]

        cleaned_batch = condense_output(batch_raw, max_chars=DEFAULT_MAX_CHARS)
        if cleaned_batch:
            out_chunk = cleaned_batch + ("\n" if not cleaned_batch.endswith("\n") else "")
            sys.stdout.write(out_chunk)
            sys.stdout.flush()
            emitted_chunks.append(out_chunk)
            flushed_any = True

    on_main_thread = threading.current_thread() is threading.main_thread()

    def handle_term(signum, _frame):
        if on_main_thread:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGINT, signal.SIG_IGN)
        flush_pending(final=True)
        terminate_process_group(proc, signal.SIGTERM)
        sys.exit(128 + signum)

    prev_sigterm = signal.getsignal(signal.SIGTERM) if on_main_thread else None
    prev_sigint = signal.getsignal(signal.SIGINT) if on_main_thread else None
    if on_main_thread:
        signal.signal(signal.SIGTERM, handle_term)
        signal.signal(signal.SIGINT, handle_term)

    assert proc.stdout is not None
    sel = selectors.DefaultSelector()
    sel.register(proc.stdout, selectors.EVENT_READ)
    eof_reached = False

    try:
        while True:
            events = sel.select(timeout=0.25)
            for key, _ in events:
                data = os.read(key.fd, 8192)
                if not data:
                    tail = decoder.decode(b"", final=True)
                    if tail:
                        raw_chunks.append(tail)
                        pending_buf += tail
                    sel.unregister(proc.stdout)
                    eof_reached = True
                    break
                decoded = decoder.decode(data, final=False)
                if decoded:
                    raw_chunks.append(decoded)
                    pending_buf += decoded
                    if streaming_mode and ("\n" in pending_buf or len(pending_buf) >= 4096):
                        flush_pending(final=False)
            else:
                if proc.poll() is not None and not sel.get_map():
                    eof_reached = True
                    break
                if not streaming_mode and (time.monotonic() - start_time) >= streaming_threshold_secs:
                    streaming_mode = True
                    flush_pending(final=False)
                continue
            break
    except KeyboardInterrupt:
        flush_pending(final=True)
        terminate_process_group(proc, signal.SIGTERM)
        return 130
    except Exception:
        pass
    finally:
        try:
            sel.close()
        except Exception:
            pass
        if on_main_thread:
            if prev_sigterm is not None:
                signal.signal(signal.SIGTERM, prev_sigterm)
            if prev_sigint is not None:
                signal.signal(signal.SIGINT, prev_sigint)
        if not eof_reached and proc.poll() is None:
            terminate_process_group(proc, signal.SIGTERM)

    proc.wait()
    stdout_raw = "".join(raw_chunks)

    if not streaming_mode and not flushed_any:
        cleaned = condense_output(stdout_raw)
        record_stats(command_str, stdout_raw, cleaned)
        if cleaned:
            sys.stdout.write(cleaned + "\n")
            sys.stdout.flush()
    else:
        flush_pending(final=True)
        cleaned_stream = "".join(emitted_chunks)
        record_stats(command_str, stdout_raw, cleaned_stream)

    return proc.returncode


def handle_hook(script_path: Optional[str] = None) -> None:
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
        is_daemon = bool(args.get("IsDaemon", False))

        if (
            "NO_FERRY=1" in cmd
            or "FERRY_BYPASS=1" in cmd
            or os.environ.get("NO_FERRY") == "1"
            or is_daemon
            or "gh run watch" in cmd
            or "tail -f" in cmd
        ):
            sys.stdout.write(json.dumps({"decision": "allow"}))
            return

        this_script = os.path.abspath(script_path or __file__)

        if tool_name == "run_command" and cmd and "ferry_cleaner.py" not in cmd:
            escaped_cmd = cmd.replace("'", "'\"'\"'")
            new_cmd = f"python3 '{this_script}' -- '{escaped_cmd}'"
            response = {"decision": "allow", "overwrite": {"CommandLine": new_cmd}}
            sys.stdout.write(json.dumps(response))
            return

        sys.stdout.write(json.dumps({"decision": "allow"}))
    except Exception:
        sys.stdout.write(json.dumps({"decision": "allow"}))


def main(script_path: Optional[str] = None) -> None:
    if len(sys.argv) > 1 and sys.argv[1] == "--hook":
        handle_hook(script_path=script_path)
        return

    if len(sys.argv) > 1 and sys.argv[1] == "--":
        cmd_to_run = sys.argv[2] if len(sys.argv) == 3 else shlex.join(sys.argv[2:])
        exit_code = run_command_wrapped(cmd_to_run)
        sys.exit(exit_code)

    if not sys.stdin.isatty():
        raw_text = sys.stdin.read()
        cleaned = condense_output(raw_text)
        sys.stdout.write(cleaned + "\n")
        return

    print("Ferry Cleaner for Antigravity")
    print("Usage:")
    print("  python3 ferry_cleaner.py -- <command to run>")
    print("  cat large.log | python3 ferry_cleaner.py")
    print("  python3 ferry_cleaner.py --hook (Antigravity PreToolUse mode)")


if __name__ == "__main__":
    main()
