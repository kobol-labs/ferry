"""Pure text condensation and noise-stripping rules for Ferry Command Cleaner."""

import json
import re
from typing import List, Tuple

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
    "traceback",
    "syntaxerror",
    "typeerror",
    "referenceerror",
    "status 500",
    "status 502",
    "status 503",
    "status 504",
    "internal server error",
    "bad gateway",
    "service unavailable",
    "gateway timeout",
    "permission denied",
    "connection refused",
    "timed out",
    "no such file",
    "command not found",
    "cannot find module",
    "module not found",
    "warning:",
    "warn:",
    "critical:",
    "sigsegv",
    "segmentation fault",
    "sql syntax",
    "sqlite3.",
    "syntax error near",
    "unterminated quotation",
    "unhandled",
    "uncaught",
    "nullpointer",
    "stack overflow",
]

DEFAULT_MAX_CHARS = 12000
DEFAULT_HEAD_LINES = 40
DEFAULT_TAIL_LINES = 30

ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07")

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
    r"(data:[a-zA-Z0-9.+_-]+/[a-zA-Z0-9.+_-]+;base64,)[A-Za-z0-9+/=]{80,}"
)
BASE64_RAW_RE = re.compile(
    r"(?<=[\s\"'`=:,(\[])([A-Za-z0-9+/=]{120,})(?=[\s\"'`:,;)\]]|$)"
)

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

COMMON_CODE_EXTENSIONS = (
    ".rs", ".py", ".js", ".ts", ".go", ".json", ".toml", ".yaml", ".yml",
    ".md", ".txt", ".html", ".css", ".sh", ".c", ".cpp", ".h", ".sql",
)


def strip_ansi(text: str) -> str:
    """Removes terminal ANSI color and cursor codes."""
    return ANSI_ESCAPE_RE.sub("", text)


def strip_progress_artifacts(text: str) -> str:
    """Cleans up carriage return progress bar overwrites (e.g. download % lines)."""
    if "\r" not in text:
        return text
    lines = []
    for line in text.split("\n"):
        if "\r" in line:
            parts = line.split("\r")
            line = parts[-1] if parts[-1].strip() else parts[0]
        lines.append(line)
    return "\n".join(lines)


def collapse_excess_whitespace(text: str) -> str:
    """Collapses 3+ consecutive newlines into 2."""
    return re.sub(r"\n{3,}", "\n\n", text)


def deduplicate_consecutive_lines(text: str) -> str:
    """Folds consecutive identical lines into a single line with repetition count."""
    lines = text.splitlines()
    if len(lines) < 3:
        return text

    def can_dedup(line: str) -> bool:
        trimmed = line.strip()
        return len(trimmed) >= 12 and any(c.isalnum() for c in trimmed)

    out: List[str] = []
    repeat_count = 1
    prev_line = None

    for line in lines:
        if prev_line is not None and line == prev_line and line.strip():
            repeat_count += 1
        else:
            if repeat_count > 1:
                if repeat_count >= 3 and prev_line is not None and can_dedup(prev_line):
                    out.append(f"[... Repeated {repeat_count - 1} more times: {prev_line.strip()} ...]")
                elif prev_line is not None:
                    out.extend([prev_line] * (repeat_count - 1))
                repeat_count = 1
            out.append(line)
            prev_line = line

    if repeat_count > 1 and prev_line is not None:
        if repeat_count >= 3 and can_dedup(prev_line):
            out.append(f"[... Repeated {repeat_count - 1} more times: {prev_line.strip()} ...]")
        else:
            out.extend([prev_line] * (repeat_count - 1))

    return "\n".join(out)


def is_base64_or_hex_payload(token: str) -> bool:
    """Verifies a token is truly a base64/hex payload and NOT a file path, URL, or code token."""
    if len(token) < 120:
        return False
    if token.startswith(("eyJ", "sk-", "ghp_", "gho_", "glpat-", "xoxb-")):
        return False
    if token.startswith(("/", "./", "../", "http://", "https://")):
        return False
    if any(token.endswith(ext) for ext in COMMON_CODE_EXTENSIONS):
        return False
    if token.count("/") >= 2:
        return False
    return True


def is_directory_listing_line(line: str, category: str) -> bool:
    """Checks whether a line is a real directory path listing rather than code mentioning a directory."""
    trimmed = line.strip()
    if category not in trimmed:
        return False
    if ";" in trimmed or "{" in trimmed or "}" in trimmed or "=" in trimmed:
        return False
    if trimmed.startswith(("import ", "from ", "def ", "class ", "const ", "let ", "var ", "//", "/*", "*", "#")):
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

        is_lockfile = any(f"/{name}" in header_line or header_line.endswith(name) for name in LOCKFILE_NAMES) or any(
            ext in header_line for ext in LOCKFILE_EXTENSIONS
        )

        if is_lockfile:
            adds = sum(1 for ln in lines if ln.startswith("+") and not ln.startswith("+++"))
            dels = sum(1 for ln in lines if ln.startswith("-") and not ln.startswith("---"))

            if (adds + dels) > 10:
                header_lines = []
                for line in lines:
                    header_lines.append(line)
                    if line.startswith("+++") or line.startswith("@@"):
                        break

                out.append(
                    "\n".join(header_lines)
                    + f"\n[... Ferry: lockfile/asset diff suppressed (+{adds} lines, -{dels} lines) ...]\n"
                )
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

    return BASE64_RAW_RE.sub(_raw_base64_sub, text)


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
        for _pattern, cat in DIR_SHIELD_PATTERNS:
            if is_directory_listing_line(line, cat):
                matched_cat = cat
                break

        if matched_cat is not None:
            start_run = idx
            while idx < len(lines) and is_directory_listing_line(lines[idx], matched_cat):
                idx += 1
            run_count = idx - start_run
            if run_count >= 4:
                out.append(f"[... Ferry: {run_count} directory entries in {matched_cat} suppressed ...]")
            else:
                out.extend(lines[start_run:idx])
        else:
            out.append(line)
            idx += 1

    return "\n".join(out)


def try_minify_json(text: str) -> str:
    """Minifies large pretty-printed JSON (>3000 chars) while keeping small JSON readable."""
    trimmed = text.strip()
    if len(trimmed) >= 3000 and (
        (trimmed.startswith("{") and trimmed.endswith("}"))
        or (trimmed.startswith("[") and trimmed.endswith("]"))
    ):
        try:
            parsed = json.loads(trimmed)
            minified = json.dumps(parsed, separators=(",", ":"), ensure_ascii=False)
            if len(minified) < len(trimmed):
                return minified
        except Exception:
            pass
    return text


def truncate_signal_aware(
    text: str, head_lines: int = DEFAULT_HEAD_LINES, tail_lines: int = DEFAULT_TAIL_LINES
) -> str:
    """Truncates output while preserving head, tail, and any diagnostic error regions in between."""
    lines = text.splitlines()
    if len(lines) <= (head_lines + tail_lines + 5):
        return text

    head = lines[:head_lines]
    tail = lines[-tail_lines:]
    mid = lines[head_lines:-tail_lines]

    signal_line_indices = [
        i for i, line in enumerate(mid) if any(sig in line.lower() for sig in DIAGNOSTIC_SIGNALS)
    ]

    if not signal_line_indices:
        dropped_lines = len(mid)
        dropped_chars = sum(len(ln) for ln in mid)
        banner = f"\n[... Ferry Cleaner: Truncated {dropped_lines} lines (~{dropped_chars} chars) ...]\n"
        return "\n".join(head) + banner + "\n".join(tail)

    selected_mid_indices = set()
    for idx in signal_line_indices:
        for offset in range(-3, 5):
            target = idx + offset
            if 0 <= target < len(mid):
                selected_mid_indices.add(target)

    sorted_indices = sorted(selected_mid_indices)
    coalesced_indices: List[int] = []
    for idx in sorted_indices:
        if coalesced_indices and 1 < (idx - coalesced_indices[-1]) <= 5:
            for fill_idx in range(coalesced_indices[-1] + 1, idx):
                coalesced_indices.append(fill_idx)
        coalesced_indices.append(idx)

    extracted_lines = []
    last_idx = None
    for idx in coalesced_indices:
        if last_idx is not None and idx > last_idx + 1:
            extracted_lines.append(f"[... Truncated {idx - last_idx - 1} uninformative lines ...]")
        extracted_lines.append(mid[idx])
        last_idx = idx

    banner_start = (
        f"\n[... Ferry Cleaner: Truncated {len(mid) - len(coalesced_indices)} uninformative lines; "
        f"preserved {len(signal_line_indices)} diagnostic signal(s) ...]\n"
    )
    return "\n".join(head) + banner_start + "\n".join(extracted_lines) + "\n\n" + "\n".join(tail)


def segment_markdown_code_blocks(text: str) -> List[Tuple[bool, str]]:
    """Splits text into (is_code_block, chunk) segments to protect fenced code blocks."""
    segments: List[Tuple[bool, str]] = []
    current_lines: List[str] = []
    in_code = False
    fence_len = 0

    for line in text.splitlines():
        trimmed = line.strip()
        if trimmed.startswith("```"):
            backticks = len(trimmed) - len(trimmed.lstrip("`"))
            if in_code:
                if backticks >= fence_len:
                    current_lines.append(line)
                    segments.append((True, "\n".join(current_lines)))
                    current_lines = []
                    in_code = False
                    fence_len = 0
                else:
                    current_lines.append(line)
            else:
                if current_lines:
                    segments.append((False, "\n".join(current_lines)))
                    current_lines = []
                current_lines.append(line)
                in_code = True
                fence_len = backticks
        else:
            current_lines.append(line)

    if current_lines:
        segments.append((in_code, "\n".join(current_lines)))

    return segments


def _clean_text_chunk(text: str) -> str:
    text = strip_ansi(text)
    text = strip_progress_artifacts(text)
    text = collapse_excess_whitespace(text)
    text = deduplicate_consecutive_lines(text)
    text = prune_git_diff_lockfiles(text)
    text = suppress_base64_blobs(text)
    text = shield_directory_traversals(text)
    text = try_minify_json(text)
    return text


def condense_output(raw_text: str, max_chars: int = DEFAULT_MAX_CHARS) -> str:
    """Full condensation pipeline with markdown code-fence protection."""
    if not raw_text:
        return ""

    if "```" in raw_text:
        cleaned_parts = [
            strip_ansi(chunk) if is_code else _clean_text_chunk(chunk)
            for is_code, chunk in segment_markdown_code_blocks(raw_text)
        ]
        text = "\n".join(cleaned_parts)
    else:
        text = _clean_text_chunk(raw_text)

    if len(text) > max_chars:
        text = truncate_signal_aware(text)

    return text


clean_output = condense_output
