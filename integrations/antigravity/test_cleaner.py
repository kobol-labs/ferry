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
