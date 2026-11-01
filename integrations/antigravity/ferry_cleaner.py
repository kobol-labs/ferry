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
    "ghsa-",
    "assert",
    "traceback",
    "syntaxerror",
    "typeerror",
    "referenceerror",
    "status 500",
    "status 502",
    "status 503",
    "status 504",
    "status 400",
    "status 401",
    "status 403",
    "status 404",
    "status 422",
    "status 429",
    "bad request",
    "not found",
    "internal server error",
    "bad gateway",
    "service unavailable",
    "gateway timeout",
    "unauthorized",
    "forbidden",
    "access denied",
    "permission denied",
    "connection refused",
    "timed out",
    "timeout",
    "no such file",
    "command not found",
    "cannot find module",
    "module not found",
    "invalid",
    "unrecognized",
    "malformed",
    "corrupt",
    "warning:",
    "warn:",
    "critical:",
    "rejected",
    "expired",
    "sigsegv",
    "segmentation fault",
    "sql syntax",
    "sqlite3.",
    "syntax error near",
    "unterminated quotation",
    "sqli",
    "xss",
    "csrf",
    "idor",
    "ssrf",
    "token_version",
    "jwt",
    "bearer",
    "leak",
    "bypass",
    "unhandled",
    "uncaught",
    "nullpointer",
    "stack overflow",
]

# Tuning constants
DEFAULT_MAX_CHARS = 12000
DEFAULT_HEAD_LINES = 40
DEFAULT_TAIL_LINES = 30

ANSI_ESCAPE_RE = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07')

LOCKFILE_NAMES = {
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "mix.lock",
    "flake.lock",
}

LOCKFILE_EXTENSIONS = (
    ".min.js",
    ".min.css",
    ".map",
    ".bundle.js",
    ".bundle.css",
)

BASE64_DATA_URI_RE = re.compile(
    r'(data:[a-zA-Z0-9.+_-]+/[a-zA-Z0-9.+_-]+;base64,)[A-Za-z0-9+/=]{80,}'
)
BASE64_RAW_RE = re.compile(r'(?<=[\s"\'`=:,(\[])([A-Za-z0-9+/=]{120,})(?=[\s"\'`:,;)\]]|$)')

DIR_SHIELD_PATTERNS = [
    ("node_modules/", "node_modules/"),
    (".git/", ".git/"),
    ("target/debug/", "target/debug/"),
    ("target/release/", "target/release/"),
    ("__pycache__/", "__pycache__/"),
    (".venv/", ".venv/"),
    ("/venv/", "venv/"),
    ("vendor/bundle/", "vendor/bundle/"),
    (".cache/", ".cache/"),
]


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


COMMON_CODE_EXTENSIONS = (
    '.rs', '.py', '.js', '.ts', '.go', '.json', '.toml', '.yaml', '.yml',
    '.md', '.txt', '.html', '.css', '.sh', '.c', '.cpp', '.h', '.sql'
)


def is_base64_or_hex_payload(token: str) -> bool:
    """Verifies a token is truly a base64/hex payload and NOT a file path, URL, or code token."""
    if len(token) < 120:
        return False
    # Never suppress JWT tokens, API keys, or authentication headers
    if token.startswith(('eyJ', 'sk-', 'ghp_', 'gho_', 'glpat-', 'xoxb-')):
        return False
    # Never suppress file paths, URLs, or command flags
    if token.startswith('/') or token.startswith('./') or token.startswith('../') or token.startswith('http://') or token.startswith('https://'):
        return False
    if any(token.endswith(ext) for ext in COMMON_CODE_EXTENSIONS):
        return False
    # Directory paths have multiple forward slashes
    if token.count('/') >= 2:
        return False
    return True


def is_directory_listing_line(line: str, category: str) -> bool:
    """Checks whether a line is a real directory path listing rather than code mentioning a directory."""
    trimmed = line.strip()
    if category not in trimmed:
        return False
    # If line has code syntax or comment characters, it's code/comment, not a directory listing
    if ';' in trimmed or '{' in trimmed or '}' in trimmed or '=' in trimmed:
        return False
    if trimmed.startswith(('import ', 'from ', 'def ', 'class ', 'const ', 'let ', 'var ', '//', '/*', '*', '#')):
        return False
    return True


def prune_git_diff_lockfiles(text: str) -> str:
    """Prunes verbose diff bodies of lockfiles and generated asset bundles in git diff output."""
    if "diff --git " not in text:
        return text

    chunks = text.split("diff --git ")
    out = [chunks[0]]

    for chunk in chunks[1:]:
        lines = chunk.splitlines()
        header_line = lines[0] if lines else ""

        is_lockfile = any(f"/{name}" in header_line or header_line.endswith(name) for name in LOCKFILE_NAMES) or \
                      any(ext in header_line for ext in LOCKFILE_EXTENSIONS)

        if is_lockfile:
            adds = sum(1 for l in lines if l.startswith('+') and not l.startswith('+++'))
            dels = sum(1 for l in lines if l.startswith('-') and not l.startswith('---'))

            if (adds + dels) > 10:
                header_lines = []
                for line in lines:
                    header_lines.append(line)
                    if line.startswith("+++") or line.startswith("@@"):
                        break

                out.append('\n'.join(header_lines) + f"\n[... Ferry: lockfile/asset diff suppressed (+{adds} lines, -{dels} lines) ...]\n")
                continue

        out.append(chunk)

    return "diff --git ".join(out)


def suppress_base64_blobs(text: str) -> str:
    """Replaces large inline base64 payloads and continuous hex blobs with size banners."""
    if len(text) < 120:
        return text

    def _data_uri_sub(m):
        prefix = m.group(1)
        payload_len = len(m.group(0)) - len(prefix)
        return f"{prefix}[... Ferry: base64 payload ({payload_len} chars) suppressed ...]"

    text = BASE64_DATA_URI_RE.sub(_data_uri_sub, text)

    def _raw_base64_sub(m):
        raw = m.group(1)
        if not is_base64_or_hex_payload(raw):
            return m.group(0)
        return f"[... Ferry: base64/hex blob ({len(raw)} chars) suppressed ...]"

    text = BASE64_RAW_RE.sub(_raw_base64_sub, text)
    return text


def shield_directory_traversals(text: str) -> str:
    """Folds long runs of file traversals into dependency or cache directories."""
    lines = text.splitlines()
    if len(lines) < 6:
        return text

    out: List[str] = []
    idx = 0
    while idx < len(lines):
        line = lines[idx]
        matched_cat = None
        for pattern, cat in DIR_SHIELD_PATTERNS:
            if is_directory_listing_line(line, cat):
                matched_cat = cat
                break

        if matched_cat is not None:
            start_run = idx
            while idx < len(lines):
                if is_directory_listing_line(lines[idx], matched_cat):
                    idx += 1
                else:
                    break
            run_count = idx - start_run
            if run_count >= 4:
                out.append(f"[... Ferry: {run_count} directory entries in {matched_cat} suppressed ...]")
            else:
                out.extend(lines[start_run:idx])
        else:
            out.append(line)
            idx += 1

    return '\n'.join(out)


def try_minify_json(text: str) -> str:
    """Attempts to minify pretty-printed JSON if the whole string is JSON."""
    trimmed = text.strip()
    if (trimmed.startswith('{') and trimmed.endswith('}')) or (trimmed.startswith('[') and trimmed.endswith(']')):
        try:
            parsed = json.loads(trimmed)
            minified = json.dumps(parsed, separators=(',', ':'), ensure_ascii=False)
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

    # Group signals with 3 lines before and 4 lines after of context around them
    selected_mid_indices = set()
    for idx in signal_line_indices:
        for offset in range(-3, 5):
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
            "lockfile_pruned": "[... Ferry: lockfile/asset diff suppressed" in cleaned_text,
            "base64_suppressed": "[... Ferry: base64" in cleaned_text,
            "dir_shielded": "directory entries in " in cleaned_text,
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

    # 5. Prune git diff lockfile and generated bundle noise
    text = prune_git_diff_lockfiles(text)

    # 6. Suppress massive Base64 payloads and hex blobs
    text = suppress_base64_blobs(text)

    # 7. Shield directory traversal bombs (node_modules, .git, target, etc.)
    text = shield_directory_traversals(text)

    # 8. Try minifying JSON if applicable
    text = try_minify_json(text)

    # 9. Signal-aware smart truncation if size exceeds limit
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
        errors="replace",
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
        if len(sys.argv) == 3:
            cmd_to_run = sys.argv[2]
        else:
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
