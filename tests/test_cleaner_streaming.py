"""Regression tests for ferry_cleaner deduplication and streaming mode fixes (#17)."""

import importlib.util
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
ROOT_CLEANER = REPO_ROOT / "ferry_cleaner.py"
ANTIGRAVITY_DIR = REPO_ROOT / "integrations" / "antigravity"
ANTIGRAVITY_CLEANER = ANTIGRAVITY_DIR / "ferry_cleaner.py"
CLEANER_RULES = ANTIGRAVITY_DIR / "cleaner_rules.py"


def _load_module_from_path(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, str(path))
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_cleaner_deduplicated_not_symlink_and_under_500_loc():
    assert not ROOT_CLEANER.is_symlink(), "Root ferry_cleaner.py must be a thin wrapper file, not a symlink"
    assert CLEANER_RULES.exists(), "integrations/antigravity/cleaner_rules.py must exist"

    for py_path in [ROOT_CLEANER, ANTIGRAVITY_CLEANER, CLEANER_RULES]:
        line_count = len(py_path.read_text(encoding="utf-8").splitlines())
        assert line_count < 500, f"{py_path} has {line_count} lines (must be < 500 LOC)"

    root_lines = len(ROOT_CLEANER.read_text(encoding="utf-8").splitlines())
    assert root_lines < 60, f"Root ferry_cleaner.py wrapper should be thin (< 60 LOC), got {root_lines}"

    root_mod = _load_module_from_path("root_ferry_cleaner_check", ROOT_CLEANER)
    for symbol in [
        "clean_output",
        "condense_output",
        "strip_ansi",
        "deduplicate_consecutive_lines",
        "truncate_signal_aware",
        "record_stats",
        "run_command_wrapped",
        "handle_hook",
        "main",
    ]:
        assert hasattr(root_mod, symbol), f"Root ferry_cleaner.py must re-export `{symbol}`"


def test_hook_json_roundtrip_both_entrypoints():
    payload = json.dumps({
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "echo hello"},
        }
    })
    for script in [ROOT_CLEANER, ANTIGRAVITY_CLEANER]:
        res = subprocess.run(
            [sys.executable, str(script), "--hook"],
            input=payload,
            capture_output=True,
            text=True,
            check=True,
        )
        data = json.loads(res.stdout)
        assert data["decision"] == "allow"
        rewritten = data["overwrite"]["CommandLine"]
        assert str(script) in rewritten
        exec_res = subprocess.run(rewritten, shell=True, capture_output=True, text=True, check=True)
        assert exec_res.stdout.strip() == "hello"


def test_streaming_mode_records_actual_saved_chars(tmp_path):
    stats_file = tmp_path / "streaming_stats.json"
    env = os.environ.copy()
    env["FERRY_CLEANER_STATS_FILE"] = str(stats_file)

    # Command runs >= 2.1s to trigger real streaming_mode in run_command_wrapped
    child_py = (
        "import time, sys; "
        "[print('\\x1b[32mWaiting for worker socket connection...\\x1b[0m') for _ in range(30)]; "
        "sys.stdout.flush(); "
        "time.sleep(2.15); "
        "[print('\\x1b[32mWaiting for worker socket connection...\\x1b[0m') for _ in range(30)]; "
        "sys.stdout.flush()"
    )
    res = subprocess.run(
        [sys.executable, str(ANTIGRAVITY_CLEANER), "--", f"{sys.executable} -c \"{child_py}\""],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    )
    assert "\x1b[" not in res.stdout
    assert "Repeated" in res.stdout

    assert stats_file.exists(), "Streaming run must record stats to FERRY_CLEANER_STATS_FILE"
    history = json.loads(stats_file.read_text(encoding="utf-8"))
    assert len(history) == 1
    entry = history[0]
    assert entry["saved_chars"] > 0, f"Expected positive saved_chars in streaming mode, got {entry}"
    assert entry["cond_chars"] < entry["orig_chars"]
    assert entry["ansi_stripped"] is True
    assert entry["deduped"] is True


def test_streaming_incremental_utf8_and_partial_line_buffering(tmp_path):
    stats_file = tmp_path / "utf8_stats.json"
    env = os.environ.copy()
    env["FERRY_CLEANER_STATS_FILE"] = str(stats_file)

    # Write a 4-byte emoji (🚀 = f0 9f 9a 80) and 3-byte CJK/Euro split across separate os.write + sleep
    # after streaming_mode activates, plus a partial line without '\n' across flushes.
    writer_script = tmp_path / "split_utf8_writer.py"
    writer_script.write_text(
        "import os, time\n"
        "os.write(1, b'warmup line\\n')\n"
        "time.sleep(2.15)\n"
        "# Now in streaming_mode: write partial line + split 4-byte UTF-8 rocket + split 3-byte euro\n"
        "rocket = '🚀'.encode('utf-8')\n"
        "euro = '€'.encode('utf-8')\n"
        "os.write(1, b'prefix-' + rocket[:2])\n"
        "time.sleep(0.35)\n"
        "os.write(1, rocket[2:] + b'-mid-' + euro[:1])\n"
        "time.sleep(0.35)\n"
        "os.write(1, euro[1:] + b'-suffix\\n')\n",
        encoding="utf-8",
    )

    res = subprocess.run(
        [sys.executable, str(ANTIGRAVITY_CLEANER), "--", f"{sys.executable} '{writer_script}'"],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    )
    assert "\ufffd" not in res.stdout, f"Split multi-byte UTF-8 was corrupted: {res.stdout!r}"
    lines = [ln for ln in res.stdout.splitlines() if ln.strip()]
    assert lines == ["warmup line", "prefix-🚀-mid-€-suffix"], (
        f"Partial line was prematurely flushed or split across lines: {lines!r}"
    )


def test_process_group_killed_on_sigterm(tmp_path):
    pid_file = tmp_path / "grandchild.pid"
    spawner_script = tmp_path / "spawn_grandchild.py"
    spawner_script.write_text(
        "import os, subprocess, sys, time\n"
        f"child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
        f"open({str(pid_file)!r}, 'w').write(str(child.pid))\n"
        "sys.stdout.write('spawned\\n')\n"
        "sys.stdout.flush()\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )

    env = os.environ.copy()
    env["FERRY_CLEANER_STATS_FILE"] = str(tmp_path / "term_stats.json")

    proc = subprocess.Popen(
        [sys.executable, str(ANTIGRAVITY_CLEANER), "--", f"{sys.executable} '{spawner_script}'"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=env,
    )

    # Wait for grandchild PID file to appear
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline and not pid_file.exists():
        time.sleep(0.05)
    assert pid_file.exists(), "Grandchild process did not start in time"
    grandchild_pid = int(pid_file.read_text().strip())

    # Terminate ferry_cleaner wrapper
    proc.send_signal(signal.SIGTERM)
    proc.wait(timeout=5.0)

    # Poll briefly for OS reaping, then verify grandchild is dead
    dead = False
    for _ in range(30):
        try:
            os.kill(grandchild_pid, 0)
            time.sleep(0.05)
        except ProcessLookupError:
            dead = True
            break

    if not dead:
        try:
            os.kill(grandchild_pid, signal.SIGKILL)
        except Exception:
            pass
    assert dead, f"Grandchild PID {grandchild_pid} survived SIGTERM of ferry_cleaner!"


def test_no_pythonpath_or_cwd_namespace_shadowing(tmp_path):
    # Create competing `integrations/__init__.py` and `cleaner_rules.py` that explode if imported
    bad_pkg = tmp_path / "integrations"
    bad_pkg.mkdir()
    (bad_pkg / "__init__.py").write_text(
        "raise RuntimeError('Shadowed integrations package imported!')\n",
        encoding="utf-8",
    )
    (tmp_path / "cleaner_rules.py").write_text(
        "raise RuntimeError('Shadowed cleaner_rules module imported!')\n",
        encoding="utf-8",
    )

    env = os.environ.copy()
    env["PYTHONPATH"] = str(tmp_path)
    env["FERRY_CLEANER_STATS_FILE"] = str(tmp_path / "shadow_stats.json")

    hook_payload = json.dumps({
        "toolCall": {
            "name": "run_command",
            "args": {"CommandLine": "echo unshadowed"},
        }
    })

    for script in [ROOT_CLEANER, ANTIGRAVITY_CLEANER]:
        hook_res = subprocess.run(
            [sys.executable, str(script), "--hook"],
            input=hook_payload,
            capture_output=True,
            text=True,
            cwd=str(tmp_path),
            env=env,
            check=True,
        )
        data = json.loads(hook_res.stdout)
        assert data["decision"] == "allow"
        assert str(script) in data["overwrite"]["CommandLine"]

        cmd_res = subprocess.run(
            [sys.executable, str(script), "--", "echo unshadowed_exec"],
            capture_output=True,
            text=True,
            cwd=str(tmp_path),
            env=env,
            check=True,
        )
        assert cmd_res.stdout.strip() == "unshadowed_exec"


def test_early_stdout_stderr_close_does_not_sigterm_child(tmp_path):
    env = os.environ.copy()
    env["FERRY_CLEANER_STATS_FILE"] = str(tmp_path / "early_close_stats.json")

    early_close_cmd = (
        f"{sys.executable} -c "
        "\"import os, time; os.close(1); os.close(2); time.sleep(0.2)\""
    )
    res = subprocess.run(
        [sys.executable, str(ANTIGRAVITY_CLEANER), "--", early_close_cmd],
        capture_output=True,
        text=True,
        env=env,
    )
    assert res.returncode == 0, (
        f"Expected exit code 0 when child closes stdout/stderr before exiting, got {res.returncode}"
    )
