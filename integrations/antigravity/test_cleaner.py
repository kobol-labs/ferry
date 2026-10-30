#!/usr/bin/env python3
import os
import subprocess
import json

FERRY = os.path.abspath(os.path.join(os.path.dirname(__file__), "ferry_cleaner.py"))

print("================ TEST 1: REPEATED LINES DEDUPLICATION ================")
p = subprocess.Popen([FERRY, "--hook"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
hook_input = {
    "toolCall": {
        "name": "run_command",
        "args": {
            "CommandLine": "python3 -c 'for i in range(8): print(\"Waiting for socket...\")'"
        }
    }
}
hook_output, _ = p.communicate(json.dumps(hook_input))
hook_resp = json.loads(hook_output)
wrapped_cmd = hook_resp["overwrite"]["CommandLine"]
print(f"Hook rewritten command: {wrapped_cmd}")

# Now execute the rewritten command
res = subprocess.run(wrapped_cmd, shell=True, capture_output=True, text=True)
print("Output:\n" + res.stdout.strip())

print("\n================ TEST 2: ANSI ESCAPE STRIPPING ================")
test_ansi_cmd = f"python3 '{FERRY}' -- 'printf \"\\033[32mSUCCESS:\\033[0m build completed in 2s\\n\"'"
res = subprocess.run(test_ansi_cmd, shell=True, capture_output=True, text=True)
print("Raw bytes of output:", repr(res.stdout))
print("Visible output:", res.stdout.strip())

print("\n================ TEST 3: SIGNAL-AWARE TRUNCATION ================")
# Generate 150 lines with an error at line 75 via python one-liner without newlines
gen_script = 'python3 -c "for i in range(150): (print(\'panic: connection refused to 192.168.9.99:8080\') if i == 75 else print(f\'Processing transaction chunk #{i}\'))"'
res = subprocess.run([FERRY, "--", gen_script], capture_output=True, text=True)
print("Truncated output:\n" + res.stdout.strip())

print("\n================ TEST 4: PRESERVE EXIT CODE ================")
exit_cmd = f"python3 '{FERRY}' -- 'exit 37'"
res = subprocess.run(exit_cmd, shell=True, capture_output=True, text=True)
print(f"Exit code: {res.returncode} (expected 37)")

print("\n================ TEST 5: JSON MINIFICATION ================")
pretty_json = """{
  "status": "success",
  "data": {
    "items": [1, 2, 3],
    "message": "all good"
  }
}"""
p = subprocess.Popen([FERRY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
minified, _ = p.communicate(pretty_json)
print("Original lines:", len(pretty_json.splitlines()))
print("Minified output:", minified.strip())

print("\n================ TEST 6: LOCKFILE DIFF PRUNING ================")
mock_git_diff = """diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
-fn old() {}
+fn new() {}
diff --git a/package-lock.json b/package-lock.json
index 3333333..4444444 100644
--- a/package-lock.json
+++ b/package-lock.json
@@ -10,30 +10,30 @@
+    "dependency-a": {
+      "version": "1.2.3",
+      "resolved": "https://registry.npmjs.org/dep-a",
+      "integrity": "sha512-abc123"
+    },
+    "dependency-b": {
+      "version": "4.5.6",
+      "resolved": "https://registry.npmjs.org/dep-b",
+      "integrity": "sha512-def456"
+    },
+    "dependency-c": {
+      "version": "7.8.9",
+      "resolved": "https://registry.npmjs.org/dep-c",
+      "integrity": "sha512-ghi789"
+    }
"""
p = subprocess.Popen([FERRY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
pruned_diff, _ = p.communicate(mock_git_diff)
print("Pruned Git Diff Output:\n" + pruned_diff.strip())

print("\n================ TEST 7: BASE64 BLOB SUPPRESSION ================")
mock_base64_log = 'User avatar upload response: {"image": "data:image/png;base64,' + ('A' * 250) + '", "status": "ok"}'
p = subprocess.Popen([FERRY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
suppressed_b64, _ = p.communicate(mock_base64_log)
print("Suppressed Base64 Output:\n" + suppressed_b64.strip())

print("\n================ TEST 8: DIRECTORY TRAVERSAL SHIELDING ================")
mock_find_output = "\n".join([
    "./src/index.ts",
    "./src/utils.ts",
    "./node_modules/lodash/package.json",
    "./node_modules/lodash/lodash.js",
    "./node_modules/lodash/core.js",
    "./node_modules/lodash/fp.js",
    "./node_modules/express/index.js",
    "./node_modules/express/lib/application.js",
    "./package.json",
    "./.git/HEAD",
    "./.git/config",
    "./.git/description",
    "./.git/hooks/pre-commit",
    "./.git/hooks/pre-push",
    "./README.md",
])
p = subprocess.Popen([FERRY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
shielded_find, _ = p.communicate(mock_find_output)
print("Shielded Directory Output:\n" + shielded_find.strip())
