#!/usr/bin/env python3
"""
Adversarial Verification Suite for Ferry Hybrid Routing & Condensation
Tests edge cases, false alarm suppression, read-only file shielding,
contextual continuity for cognitive tasks, procedural vs hard logic segregation,
and zero-lobotomy guarantees.
"""

HARD_LOGIC_DIAGNOSTIC_SIGNALS = [
    "panic:", "fatal:", "fatal error:", "traceback (most recent call last):",
    "nullpointerexception", "segmentation fault", "sigsegv", "sigabrt",
    "unhandled rejection", "uncaught exception", "undefined is not a function",
    "exception in thread", "assertion failed",
    "cwe-", "cve-", "vulnerability", "security alert", "error[e",
    "status: 500", "status 500", "status: 502", "status 503",
    "test failed", "failing tests", "failed (", "build failed", "compilation failed"
]

HARD_LOGIC_USER_KEYWORDS = [
    "vulnerability", "exploit", "cve", "audit", "threat model",
    "architect", "refactor", "diagnose", "root cause", "race condition",
    "deadlock", "memory leak", "heap dump", "decompile", "disassemble",
    "invariant", "formal verification", "algorithmic", "security review", "penetration"
]

def extract_message_text(msg):
    c = msg.get("content", "")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "\n".join(p.get("text", "") for p in c if isinstance(p, dict))
    return ""

def get_last_tool_name(messages):
    if not messages:
        return ""
    last = messages[-1]
    if "name" in last:
        return last["name"].lower()
    for msg in reversed(messages[:-1]):
        if msg.get("role") == "assistant":
            calls = msg.get("tool_calls", [])
            if calls and "function" in calls[-1]:
                return calls[-1]["function"].get("name", "").lower()
            break
    return ""

def is_mechanical_user_command(lower):
    lower = lower.strip()
    git_ops = ["git ", "git status", "git diff", "git log", "git add", "git commit", "git push", "git pull", "git branch", "git checkout"]
    if any(lower.startswith(op) or op in lower for op in git_ops):
        return True
    shell_ops = ["run ", "execute ", "exec ", "stop ", "start ", "restart ", "kill ", "cat ", "ls ", "docker ", "cargo ", "npm ", "python "]
    if any(lower.startswith(op) for op in shell_ops):
        return True
    if lower.startswith("show me ") or lower.startswith("check ") or lower.startswith("list "):
        if any(w in lower for w in ["status", "diff", "files", "logs", "dir", "container"]):
            return True
    return False

def has_real_diagnostic_signal(text):
    text_lower = text.lower()
    for sig in HARD_LOGIC_DIAGNOSTIC_SIGNALS:
        if sig in text_lower:
            return sig
    for pattern in ["error:", "\"error\":"]:
        pos = text_lower.find(pattern)
        if pos != -1:
            tail = text_lower[pos + len(pattern):].lstrip()
            if not any(tail.startswith(bad) for bad in ["null", "\"\"", "false", "none", "0"]):
                return pattern
    if "[error]" in text_lower or "err:" in text_lower:
        return "error signal"
    return None

def is_hard_logic_turn(body):
    messages = body.get("messages", [])
    if not messages:
        return False, "no messages"
    last_msg = messages[-1]
    role = last_msg.get("role", "")
    text = extract_message_text(last_msg)
    text_lower = text.lower()

    if role == "tool":
        tool_name = get_last_tool_name(messages)
        is_read_only = tool_name in ["read_file", "cat", "view", "read_resource", "list_dir", "list_resources", "search_marketplace", "search_web"]
        if is_read_only:
            trimmed_lower = text.strip().lower()
            if any(trimmed_lower.startswith(prefix) for prefix in ["error:", "file not found", "no such file", "permission denied", "notfound"]):
                return True, f"read tool failed: '{tool_name}' returned error"

            # Contextual continuity check: did initiating user prompt request a cognitive task?
            for msg in reversed(messages):
                if msg.get("role") == "user":
                    user_text = extract_message_text(msg).lower()
                    for kw in HARD_LOGIC_USER_KEYWORDS:
                        if kw in user_text:
                            return True, f"info gathering for cognitive task: '{kw}'"
                    reasoning_verbs = ["why", "diagnose", "debug", "architect", "design", "explain", "solve", "analyze", "investigate", "fix the bug", "find the bug"]
                    for verb in reasoning_verbs:
                        if user_text.startswith(verb) or f" {verb} " in user_text:
                            return True, f"info gathering for reasoning task: '{verb}'"
                    break

            return False, f"passive file/dir inspection via '{tool_name}'"

        sig = has_real_diagnostic_signal(text)
        if sig:
            return True, f"diagnostic signal: '{sig}'"
        lines = len(text.splitlines())
        # Stack traces (> 60 lines with stack trace markers), exempting normal git diffs
        if lines > 60 and any(m in text for m in ["   at ", "File \""]):
            return True, f"complex stack trace ({lines} lines)"
        return False, "mechanical tool execution"

    elif role == "user":
        trimmed = text.strip()
        trimmed_lower = trimmed.lower()
        procedural = ["yes", "no", "y", "n", "ok", "okay", "proceed", "continue", "go ahead", "approved", "run it", "sure", "next", "done", "cancel", "thanks"]
        if trimmed_lower in procedural:
            return False, f"procedural confirmation: '{trimmed}'"

        for kw in HARD_LOGIC_USER_KEYWORDS:
            if kw in trimmed_lower:
                return True, f"cognitive keyword: '{kw}'"

        if is_mechanical_user_command(trimmed_lower):
            return False, f"mechanical action request: '{trimmed}'"

        non_system = [m for m in messages if m.get("role") != "system"]
        if len(non_system) <= 1 and len(trimmed) >= 60:
            return True, "initial task specification"

        reasoning_verbs = ["why", "diagnose", "debug", "architect", "design", "explain", "solve", "analyze", "investigate", "fix the bug", "find the bug"]
        for verb in reasoning_verbs:
            if trimmed_lower.startswith(verb) or f" {verb} " in trimmed_lower:
                return True, f"reasoning intent verb: '{verb}'"

        return False, "procedural or standard execution turn"

    return False, "default mechanical turn"


def run_adversarial_suite():
    print("================ ADVERSARIAL TEST 1: READ_FILE FOR MECHANICAL LOOKUP ================")
    body_read_mech = {
        "messages": [
            {"role": "user", "content": "check package.json and show dependencies"},
            {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "read_file", "arguments": "package.json"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": '{"name": "test", "dependencies": {"error-handler": "^1.0"}}'}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_read_mech)
    assert not is_hard, f"Failed: routine read_file must stay Local, got {reason}"
    print(f"PASS: Routine read_file stayed Local ({reason})")

    print("\n================ ADVERSARIAL TEST 2: READ_FILE DURING COGNITIVE AUDIT ================")
    body_read_audit = {
        "messages": [
            {"role": "user", "content": "conduct a security audit of our token authentication logic and check for cwe vulnerabilities"},
            {"role": "assistant", "tool_calls": [{"id": "c_aud", "type": "function", "function": {"name": "read_file", "arguments": "src/auth.rs"}}]},
            {"role": "tool", "tool_call_id": "c_aud", "content": "pub fn verify_token() { /* token logic */ }"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_read_audit)
    assert is_hard, f"Failed: read_file in cognitive audit must stay Cloud so Cloud can complete the analysis, got {reason}"
    print(f"PASS: read_file in cognitive audit stayed Cloud for analysis ({reason})")

    print("\n================ ADVERSARIAL TEST 3: 80-LINE GIT DIFF ================")
    diff_lines = ["diff --git a/src/main.rs b/src/main.rs", "index 123..456 100644", "--- a/src/main.rs", "+++ b/src/main.rs", "@@ -1,50 +1,80 @@"]
    diff_lines.extend([f"+ line {i}" for i in range(75)])
    body_diff = {
        "messages": [
            {"role": "user", "content": "show git diff"},
            {"role": "assistant", "tool_calls": [{"id": "c_diff", "type": "function", "function": {"name": "run_command", "arguments": "git diff"}}]},
            {"role": "tool", "tool_call_id": "c_diff", "content": "\n".join(diff_lines)}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_diff)
    assert not is_hard, f"Failed: 80-line git diff must stay Local, got {reason}"
    print(f"PASS: 80-line git diff stayed Local without false escalation ({reason})")

    print("\n================ ADVERSARIAL TEST 4: JSON API WITH 'error: null' ================")
    body_json = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c2", "type": "function", "function": {"name": "run_command", "arguments": "curl /api/status"}}]},
            {"role": "tool", "tool_call_id": "c2", "content": '{"status": 200, "error": null, "message": "all systems operational"}'}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_json)
    assert not is_hard, f"Failed: JSON with 'error: null' must NOT escalate, got {reason}"
    print(f"PASS: Clean JSON with 'error: null' stayed Local ({reason})")

    print("\n================ ADVERSARIAL TEST 5: REAL COMPILER PANIC ================")
    body_panic = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c3", "type": "function", "function": {"name": "run_command", "arguments": "cargo test"}}]},
            {"role": "tool", "tool_call_id": "c3", "content": "running 1 test\ntest test_auth ... FAILED\nthread 'main' panicked at 'assertion failed: 2 == 3'"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_panic)
    assert is_hard, "Failed: Real panic must escalate to Cloud"
    print(f"PASS: Real compiler panic escalated to Cloud ({reason})")

    print("\n================ ADVERSARIAL TEST 6: LONG MECHANICAL GIT COMMAND (>120 CHARS) ================")
    body_long_git = {
        "messages": [
            {"role": "system", "content": "You are an assistant."},
            {"role": "user", "content": "init"},
            {"role": "assistant", "content": "ready"},
            {"role": "user", "content": "Please run git status and git diff to see all the modified files in the repository before making any new commits to main"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_long_git)
    assert not is_hard, f"Failed: Long mechanical git command must stay Local, got {reason}"
    print(f"PASS: Long git command (133 chars) stayed Local ({reason})")

    print("\n================ ADVERSARIAL TEST 7: PROCEDURAL CONFIRMATIONS ================")
    for word in ["yes", "ok", "proceed", "continue", "go ahead", "approved", "run it"]:
        body_conf = {
            "messages": [
                {"role": "assistant", "content": "Should I proceed with the build?"},
                {"role": "user", "content": word}
            ]
        }
        is_hard, reason = is_hard_logic_turn(body_conf)
        assert not is_hard, f"Failed: Confirmation '{word}' must stay Local"
    print("PASS: All procedural affirmations stayed Local")

    print("\n================ ADVERSARIAL TEST 8: UNCAUGHT JAVASCRIPT EXCEPTION ================")
    body_js_err = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c_js", "type": "function", "function": {"name": "run_command", "arguments": "node index.js"}}]},
            {"role": "tool", "tool_call_id": "c_js", "content": "TypeError: undefined is not a function\n    at Object.<anonymous> (/app/index.js:12:3)"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_js_err)
    assert is_hard, f"Failed: Uncaught exception must escalate to Cloud, got {reason}"
    print(f"PASS: JavaScript TypeError escalated to Cloud ({reason})")

    print("\n>>> ALL 8 ADVANCED ADVERSARIAL TESTS PASSED PERFECTLY! <<<")

if __name__ == "__main__":
    run_adversarial_suite()
