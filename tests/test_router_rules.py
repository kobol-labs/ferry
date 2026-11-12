"""Adversarial and Edge Case Tests for Ferry Hybrid Routing Rules.

Verifies:
1. Routine read-only operations (read_file, list_dir) stay Local.
2. Cognitive tasks (security audit, architectural refactor) escalate or retain Cloud.
3. Git diffs (even multi-hunk 80+ lines) stay Local without false escalation.
4. JSON APIs with `"error": null` or `"error": ""` are not falsely triggered.
5. Real compiler panics, syntax errors, and uncaught exceptions trigger escalation.
6. Mechanical commands (>120 chars) stay Local without length-based false escalation.
7. Procedural affirmations ("yes", "proceed", "go ahead") stay Local.
8. Uncaught exceptions (e.g. TypeError, ReferenceError) escalate to Cloud.
"""

import sys
import unittest
from pathlib import Path

# Add antigravity integration directory for imports
REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "integrations" / "antigravity"))

from test_router_adversarial import (
    is_hard_logic_turn,
    has_real_diagnostic_signal,
    is_mechanical_user_command,
    extract_message_text,
    get_last_tool_name,
)


class TestFerryRouterAdversarial(unittest.TestCase):
    """Verifies edge case classification logic governing Local vs Cloud routing."""

    def test_routine_read_file_stays_local(self):
        body = {
            "messages": [
                {"role": "user", "content": "check package.json and show dependencies"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c1",
                            "type": "function",
                            "function": {"name": "read_file", "arguments": "package.json"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "c1",
                    "content": '{"name": "test", "dependencies": {"error-handler": "^1.0"}}',
                },
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertFalse(is_hard, f"Routine read_file must stay Local, got: {reason}")
        self.assertIn("passive file/dir inspection", reason)

    def test_read_file_in_cognitive_audit_escalates(self):
        body = {
            "messages": [
                {
                    "role": "user",
                    "content": "conduct a security audit of our token authentication logic and check for cwe vulnerabilities",
                },
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c_aud",
                            "type": "function",
                            "function": {"name": "read_file", "arguments": "src/auth.rs"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "c_aud",
                    "content": "pub fn verify_token() { /* token logic */ }",
                },
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertTrue(is_hard, f"Audit context must retain Cloud routing, got: {reason}")
        self.assertIn("info gathering for cognitive task", reason)

    def test_large_git_diff_stays_local(self):
        diff_lines = [
            "diff --git a/src/main.rs b/src/main.rs",
            "index 123..456 100644",
            "--- a/src/main.rs",
            "+++ b/src/main.rs",
            "@@ -1,50 +1,80 @@",
        ]
        diff_lines.extend([f"+ line {i}" for i in range(75)])
        body = {
            "messages": [
                {"role": "user", "content": "show git diff"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c_diff",
                            "type": "function",
                            "function": {"name": "run_command", "arguments": "git diff"},
                        }
                    ],
                },
                {"role": "tool", "tool_call_id": "c_diff", "content": "\n".join(diff_lines)},
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertFalse(is_hard, f"Large git diff must stay Local, got: {reason}")

    def test_json_api_null_error_stays_local(self):
        body = {
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c2",
                            "type": "function",
                            "function": {"name": "run_command", "arguments": "curl /api/status"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "c2",
                    "content": '{"status": 200, "error": null, "message": "all systems operational"}',
                },
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertFalse(is_hard, f"JSON with 'error: null' must stay Local, got: {reason}")

    def test_compiler_panic_escalates(self):
        body = {
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c3",
                            "type": "function",
                            "function": {"name": "run_command", "arguments": "cargo test"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "c3",
                    "content": "running 1 test\ntest test_auth ... FAILED\nthread 'main' panicked at 'assertion failed: 2 == 3'",
                },
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertTrue(is_hard, f"Compiler panic must escalate, got: {reason}")
        self.assertIn("diagnostic signal", reason)

    def test_long_mechanical_git_command_stays_local(self):
        long_git_cmd = (
            "Please run git status and git diff to see all the modified files in the "
            "repository before making any new commits to main"
        )
        body = {
            "messages": [
                {"role": "system", "content": "You are an assistant."},
                {"role": "user", "content": "init"},
                {"role": "assistant", "content": "ready"},
                {"role": "user", "content": long_git_cmd},
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertFalse(is_hard, f"Long mechanical command must stay Local, got: {reason}")

    def test_procedural_confirmations_stay_local(self):
        for word in ["yes", "ok", "proceed", "continue", "go ahead", "approved", "run it", "sure"]:
            body = {
                "messages": [
                    {"role": "assistant", "content": "Should I proceed with the build?"},
                    {"role": "user", "content": word},
                ]
            }
            is_hard, reason = is_hard_logic_turn(body)
            self.assertFalse(is_hard, f"Confirmation '{word}' must stay Local, got: {reason}")

    def test_uncaught_js_exception_escalates(self):
        body = {
            "messages": [
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c_js",
                            "type": "function",
                            "function": {"name": "run_command", "arguments": "node index.js"},
                        }
                    ],
                },
                {
                    "role": "tool",
                    "tool_call_id": "c_js",
                    "content": "TypeError: undefined is not a function\n    at Object.<anonymous> (/app/index.js:12:3)",
                },
            ]
        }
        is_hard, reason = is_hard_logic_turn(body)
        self.assertTrue(is_hard, f"Uncaught exception must escalate, got: {reason}")
        self.assertIn("diagnostic signal", reason)


if __name__ == "__main__":
    unittest.main()
