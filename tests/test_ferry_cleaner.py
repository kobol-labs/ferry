"""Unit and integration tests for Ferry Command Cleaner and Context Reducer.

Covers:
1. ANSI escape stripping & terminal noise suppression.
2. Signal-aware head/tail truncation with diagnostic signal preservation.
3. Repetitive line deduplication with code-brace protection.
4. JSON minification for large payloads while preserving small readable JSON.
5. Lockfile diff pruning while preserving application code diffs.
6. Base64 payload suppression while protecting JWTs and long file paths.
7. Directory traversal shielding for vendor/cache dirs while protecting code.
8. Non-UTF8/binary stream resilience and exit-code fidelity.
9. Hook command rewriting and NO_FERRY=1 bypass escape hatch.
"""

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

# Add repo root and antigravity integration to sys.path
REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "integrations" / "antigravity"))

import ferry_cleaner


class TestFerryCleanerDirect(unittest.TestCase):
    """Direct unit tests for internal cleaning, shielding, and formatting functions."""

    def test_strip_ansi(self):
        text = "\x1b[32mSUCCESS:\x1b[0m build completed in \x1b[1m2s\x1b[0m\n"
        cleaned = ferry_cleaner.strip_ansi(text)
        self.assertEqual(cleaned, "SUCCESS: build completed in 2s\n")

    def test_deduplicate_lines_with_brace_protection(self):
        # Repeated socket logs (>=12 chars, alphanumeric, >=3 repeats) should be collapsed
        text = "\n".join(["Waiting for socket..."] * 8)
        deduped = ferry_cleaner.deduplicate_consecutive_lines(text)
        self.assertIn("Waiting for socket...", deduped)
        self.assertIn("[... Repeated 7 more times: Waiting for socket... ...]", deduped)

        # Code closing braces must NOT be collapsed
        braces = "    }\n    }\n    }"
        deduped_braces = ferry_cleaner.deduplicate_consecutive_lines(braces)
        self.assertEqual(deduped_braces, braces)

    def test_preserve_small_json(self):
        small_json = json.dumps({"status": "ok", "items": [1, 2, 3]}, indent=2)
        result = ferry_cleaner.try_minify_json(small_json)
        # Small JSON (< 3000 chars) must remain pretty printed for agent legibility
        self.assertEqual(result, small_json)

    def test_minify_large_json(self):
        large_obj = {"entries": [{"id": i, "data": "item_" + str(i)} for i in range(120)]}
        pretty_large_json = json.dumps(large_obj, indent=2)
        self.assertGreater(len(pretty_large_json), 3000)
        minified = ferry_cleaner.try_minify_json(pretty_large_json)
        self.assertEqual(len(minified.splitlines()), 1)
        self.assertIn('"id":0', minified)

    def test_prune_lockfile_diffs(self):
        mixed_diff = (
            "diff --git a/src/main.rs b/src/main.rs\n"
            "--- a/src/main.rs\n"
            "+++ b/src/main.rs\n"
            "@@ -1,3 +1,3 @@\n"
            "-fn old() {}\n"
            "+fn new() {}\n"
            "diff --git a/package-lock.json b/package-lock.json\n"
            "--- a/package-lock.json\n"
            "+++ b/package-lock.json\n"
            "@@ -10,30 +10,30 @@\n"
            + "\n".join([f'+    "dep-{i}": {{ "version": "1.0.{i}" }},' for i in range(15)])
        )
        pruned = ferry_cleaner.prune_git_diff_lockfiles(mixed_diff)
        self.assertIn("fn new() {}", pruned)
        self.assertIn("Ferry: lockfile/asset diff suppressed", pruned)
        self.assertNotIn('"dep-14"', pruned)

    def test_never_prune_normal_code_diff(self):
        code_diff = (
            "diff --git a/src/auth.ts b/src/auth.ts\n"
            "--- a/src/auth.ts\n"
            "+++ b/src/auth.ts\n"
            "@@ -1,5 +1,5 @@\n"
            "+function authenticateUser() {\n"
            "+    return verifyToken();\n"
            "+}\n"
        )
        pruned = ferry_cleaner.prune_git_diff_lockfiles(code_diff)
        self.assertEqual(pruned, code_diff)

    def test_suppress_base64_blobs(self):
        b64_payload = "A" * 250
        text = f'User avatar: {{"image": "data:image/png;base64,{b64_payload}", "status": "ok"}}'
        suppressed = ferry_cleaner.suppress_base64_blobs(text)
        self.assertIn("base64 payload (250 chars) suppressed", suppressed)
        self.assertNotIn(b64_payload, suppressed)

    def test_jwt_token_never_suppressed_as_base64(self):
        jwt_token = (
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."
            "eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiYWRtaW4iOnRydWUsImlhdCI6MTUxNjIzOTAyMiwiZXhwIjoxNTE2MjQyNjIyLCJyb2xlIjoic3VwZXJfYWRtaW4iLCJhY2NvdW50X2lkIjoiYWNjX3ByaW1hcnkiLCJlbWFpbCI6ImFkbWluQGV4YW1wbGUuY29tIn0."
            "some_test_signature_here_that_is_long"
        )
        text = f"Authorization: Bearer {jwt_token}\nResponse 200 OK"
        suppressed = ferry_cleaner.suppress_base64_blobs(text)
        self.assertIn(jwt_token, suppressed)

    def test_long_file_paths_never_suppressed(self):
        long_path = "/Users/steve/local/ferry/somelongdirectorynamewithoutanyhyphens/andanotherlongdirectorynamedhere/nestedcomponentfile.rs"
        text = f"Inspecting file: {long_path}\nDone."
        suppressed = ferry_cleaner.suppress_base64_blobs(text)
        self.assertIn(long_path, suppressed)

    def test_shield_directory_traversals(self):
        listing = "\n".join([
            "./src/index.ts",
            "./src/utils.ts",
            "./node_modules/lodash/package.json",
            "./node_modules/lodash/lodash.js",
            "./node_modules/lodash/core.js",
            "./node_modules/lodash/fp.js",
            "./package.json",
            "./.git/HEAD",
            "./.git/config",
            "./.git/description",
            "./.git/hooks/pre-commit",
            "./README.md",
        ])
        shielded = ferry_cleaner.shield_directory_traversals(listing)
        self.assertIn("./src/index.ts", shielded)
        self.assertIn("./package.json", shielded)
        self.assertIn("./README.md", shielded)
        self.assertIn("directory entries in node_modules/ suppressed", shielded)
        self.assertIn("directory entries in .git/ suppressed", shielded)

    def test_code_mentioning_node_modules_never_shielded(self):
        code = (
            'const p1 = require("node_modules/pkg1");\n'
            'const p2 = require("node_modules/pkg2");\n'
            'const p3 = require("node_modules/pkg3");\n'
            'const p4 = require("node_modules/pkg4");\n'
            'const p5 = require("node_modules/pkg5");\n'
        )
        shielded = ferry_cleaner.shield_directory_traversals(code)
        self.assertNotIn("Ferry Shield", shielded)
        self.assertEqual(shielded, code)

    def test_signal_aware_truncation_preserves_error(self):
        lines = [f"Processing transaction chunk #{i}" for i in range(150)]
        lines[75] = "panic: connection refused to 192.168.9.99:8080"
        text = "\n".join(lines)

        truncated = ferry_cleaner.truncate_signal_aware(
            text, head_lines=20, tail_lines=20
        )
        self.assertIn("panic: connection refused to 192.168.9.99:8080", truncated)
        self.assertIn("Ferry Cleaner: Truncated", truncated)
        self.assertIn("diagnostic signal(s)", truncated)

    def test_markdown_code_blocks_protected_from_mangling(self):
        raw = (
            "Here is the result:\n"
            "```json\n"
            "{\n"
            '  "title": "sample"\n'
            "}\n"
            "```\n"
        )
        condensed = ferry_cleaner.condense_output(raw)
        self.assertIn("```json\n{\n  \"title\": \"sample\"\n}\n```", condensed)


class TestFerryCleanerSubprocess(unittest.TestCase):
    """End-to-end integration tests executing ferry_cleaner.py as a CLI process."""

    @classmethod
    def setUpClass(cls):
        cls.script_path = str(REPO_ROOT / "integrations" / "antigravity" / "ferry_cleaner.py")

    def test_exit_code_preservation(self):
        res = subprocess.run(
            ["python3", self.script_path, "--", "exit 42"],
            shell=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(res.returncode, 42)

    def test_stdin_pipeline_cleaning(self):
        payload = "Line 1\nWaiting for socket...\nWaiting for socket...\nWaiting for socket...\nLine 5\n"
        proc = subprocess.Popen(
            ["python3", self.script_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        out, _ = proc.communicate(payload)
        self.assertEqual(proc.returncode, 0)
        self.assertIn("Line 1", out)
        self.assertIn("Line 5", out)
        self.assertIn("Repeated 2 more times: Waiting for socket...", out)

    def test_hook_protocol_command_rewriting(self):
        hook_payload = {
            "toolCall": {
                "name": "run_command",
                "args": {"CommandLine": "ls -la"}
            }
        }
        proc = subprocess.Popen(
            ["python3", self.script_path, "--hook"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        out, _ = proc.communicate(json.dumps(hook_payload))
        self.assertEqual(proc.returncode, 0)
        resp = json.loads(out)
        self.assertIn("overwrite", resp)
        self.assertIn("CommandLine", resp["overwrite"])
        self.assertIn(f"python3 '{self.script_path}' -- 'ls -la'", resp["overwrite"]["CommandLine"])

    def test_hook_protocol_no_ferry_bypass(self):
        hook_payload = {
            "toolCall": {
                "name": "run_command",
                "args": {"CommandLine": "NO_FERRY=1 echo 'raw data'"}
            }
        }
        proc = subprocess.Popen(
            ["python3", self.script_path, "--hook"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        out, _ = proc.communicate(json.dumps(hook_payload))
        self.assertEqual(proc.returncode, 0)
        resp = json.loads(out)
        self.assertNotIn("overwrite", resp)

    def test_non_utf8_binary_stdout_resilience(self):
        res = subprocess.run(
            ["python3", self.script_path, "--", "python3 -c 'import sys; sys.stdout.buffer.write(bytes([128, 129, 130]))'"],
            shell=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(res.returncode, 0)


if __name__ == "__main__":
    unittest.main()
