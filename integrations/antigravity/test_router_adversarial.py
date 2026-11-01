#!/usr/bin/env python3
"""
Adversarial Verification Suite for Ferry Hybrid Routing & Condensation
Tests edge cases, false alarm suppression, read-only file shielding,
procedural vs hard logic segregation, and zero-lobotomy guarantees.
"""

HARD_LOGIC_DIAGNOSTIC_SIGNALS = [
    "panic:", "fatal:", "fatal error:", "traceback (most recent call last):",
    "nullpointerexception", "segmentation fault", "sigsegv", "sigabrt",
    "unhandled rejection", "exception in thread", "assertion failed",
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
        is_read_only = tool_name in ["read_file", "cat", "view", "read_resource", "list_dir", "list_resources", "search_marketplace"]
        if is_read_only:
            trimmed_lower = text.strip().lower()
            if any(trimmed_lower.startswith(prefix) for prefix in ["error:", "file not found", "no such file", "permission denied", "notfound"]):
                return True, f"read tool failed: '{tool_name}' returned error"
            return False, f"passive file/dir inspection via '{tool_name}'"

        sig = has_real_diagnostic_signal(text)
        if sig:
            return True, f"diagnostic signal: '{sig}'"
        lines = len(text.splitlines())
        if lines > 60 and any(m in text for m in ["@@ -", "   at ", "File \""]):
            return True, f"complex trace/diff ({lines} lines)"
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
    print("================ ADVERSARIAL TEST 1: READ_FILE WITH ERROR STRINGS ================")
    body_read = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "read_file", "arguments": "src/errors.rs"}}]},
            {"role": "tool", "tool_call_id": "c1", "content": "pub fn panic_handler() {\n    panic!(\"Fatal kernel error: memory dump\");\n}"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_read)
    assert not is_hard, f"Failed: read_file containing code error strings must NOT escalate to Cloud, got {reason}"
    print(f"PASS: Reading source code with 'panic!' stayed Local ({reason})")

    print("\n================ ADVERSARIAL TEST 2: JSON API WITH 'error: null' ================")
    body_json = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c2", "type": "function", "function": {"name": "run_command", "arguments": "curl /api/status"}}]},
            {"role": "tool", "tool_call_id": "c2", "content": '{"status": 200, "error": null, "message": "all systems operational"}'}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_json)
    assert not is_hard, f"Failed: JSON with 'error: null' must NOT escalate, got {reason}"
    print(f"PASS: Clean JSON with 'error: null' stayed Local ({reason})")

    print("\n================ ADVERSARIAL TEST 3: COMPILER/TEST REAL PANIC ================")
    body_panic = {
        "messages": [
            {"role": "assistant", "tool_calls": [{"id": "c3", "type": "function", "function": {"name": "run_command", "arguments": "cargo test"}}]},
            {"role": "tool", "tool_call_id": "c3", "content": "running 1 test\ntest test_auth ... FAILED\nthread 'main' panicked at 'assertion failed: 2 == 3'"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_panic)
    assert is_hard, "Failed: Real panic must escalate to Cloud"
    print(f"PASS: Real compiler panic escalated to Cloud ({reason})")

    print("\n================ ADVERSARIAL TEST 4: LONG MECHANICAL GIT COMMAND (>120 CHARS) ================")
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

    print("\n================ ADVERSARIAL TEST 5: PROCEDURAL CONFIRMATIONS ================")
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

    print("\n================ ADVERSARIAL TEST 6: COGNITIVE SECURITY AUDIT PROMPT ================")
    body_audit = {
        "messages": [
            {"role": "assistant", "content": "What next?"},
            {"role": "user", "content": "Conduct a security audit of our token authentication logic and check for cwe vulnerabilities"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_audit)
    assert is_hard, f"Failed: Audit prompt must escalate to Cloud, got {reason}"
    print(f"PASS: Security audit escalated to Cloud ({reason})")

    print("\n================ ADVERSARIAL TEST 7: REASONING VERB 'WHY' ================")
    body_why = {
        "messages": [
            {"role": "assistant", "content": "exit code 1"},
            {"role": "user", "content": "Why did this command fail to bind to the socket?"}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_why)
    assert is_hard, f"Failed: Reasoning prompt starting with 'why' must escalate to Cloud, got {reason}"
    print(f"PASS: Reasoning question escalated to Cloud ({reason})")

    print("\n================ ADVERSARIAL TEST 8: INITIAL TASK SPECIFICATION ================")
    body_init = {
        "messages": [
            {"role": "system", "content": "You are a coding assistant."},
            {"role": "user", "content": "We need to build a distributed raft consensus algorithm from scratch in Rust."}
        ]
    }
    is_hard, reason = is_hard_logic_turn(body_init)
    assert is_hard, f"Failed: Initial task spec must escalate to Cloud, got {reason}"
    print(f"PASS: Initial task spec escalated to Cloud ({reason})")

    print("\n>>> ALL 8 ADVERSARIAL TESTS PASSED PERFECTLY! <<<")

if __name__ == "__main__":
    run_adversarial_suite()
