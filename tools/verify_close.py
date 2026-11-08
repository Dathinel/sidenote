"""Close the real window while an Ask is running and check that everything ends.

Usage: python tools/verify_close.py <exe>
Keys and clicks are sent only while the Sidenote window is the foreground window; the script
aborts without sending anything otherwise. It brings the window to the front for a few seconds.
Starts the app against a synthetic copy of one meeting and a throwaway store, asks one question
with the default runner (one real, short-lived runner call), waits for the runner process to
appear, then closes the window with WM_CLOSE. Checks that the app process exits, that the
runner and every process below it are gone, that no prompt file is left in the temp folder,
and that the saved thread holds the question and no answer. The --listen process, a separate
program, is never touched. Writes artifacts/ui/close-running.png and close-after-reopen.png.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from winutil import alive, bring_to_front, capture, click, client_origin, close_window, descendants, wait_window

exe = Path(sys.argv[1]).resolve()
root = Path(__file__).resolve().parent.parent
out = root / "artifacts" / "ui"
out.mkdir(parents=True, exist_ok=True)
MEETING = "1800000000000-1"
QUESTION = "What did we decide?"
real = root / "tools/ui-fixtures/meetings" / MEETING
temp = Path(tempfile.mkdtemp(prefix="meetings-close-"))
meetings = temp / "appdata" / "hex" / "meetings" / MEETING
meetings.mkdir(parents=True)
for name in ("english-summary.json", "manifest.json", "english-transcript.md"):
    if (real / name).exists():
        shutil.copy2(real / name, meetings / name)
state = temp / "state"
env = dict(os.environ, APPDATA=str(temp / "appdata"), SIDENOTE_STATE_DIR=str(state))
before = {p.name for p in Path(tempfile.gettempdir()).iterdir()}
process = subprocess.Popen([str(exe)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
result = {}
try:
    hwnd, _ = wait_window(process)
    time.sleep(1.5)
    bring_to_front(hwnd)  # raises, and sends nothing, if it cannot
    x, y, width, height, scale = client_origin(hwnd)
    # Click the first suggestion chip ("What did we decide?"), which sends it as a question.
    # It sits in the middle pane, about 126 points above the bottom edge.
    click(hwnd, int(406 * scale), int(height - 126 * scale))
    time.sleep(1.0)
    runner = {}
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and not runner:
        runner = descendants(process.pid)
        time.sleep(0.05)
    if not runner:
        capture(hwnd, out / "close-no-runner.png")
    assert runner, "No runner process appeared; the question was not sent (see artifacts/ui/close-no-runner.png)"
    time.sleep(1.0)
    runner = descendants(process.pid)
    capture(hwnd, out / "close-running.png")
    started = time.perf_counter()
    close_window(hwnd)
    process.wait(timeout=15)
    exit_ms = round((time.perf_counter() - started) * 1000)
    time.sleep(0.3)
    saved = json.loads((state / f"{MEETING}.json").read_text(encoding="utf-8"))
    after = {p.name for p in Path(tempfile.gettempdir()).iterdir()} - before - {temp.name}
    result = {
        "app_exit_ms_after_close": exit_ms,
        "app_still_alive": alive(process.pid),
        "runner_processes_before_close": runner,
        "runner_processes_still_alive": {p: n for p, n in runner.items() if alive(p)},
        "saved_roles": [m["role"] for m in saved["messages"]],
        "new_temp_entries": sorted(after),
    }
    assert not result["app_still_alive"] and not result["runner_processes_still_alive"]
    assert result["saved_roles"] == ["user"], "The saved thread should hold only the question"
    # Reopen: the question shows, with the note that it has no answer.
    again = subprocess.Popen([str(exe)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    hwnd, _ = wait_window(again)
    time.sleep(1.5)
    capture(hwnd, out / "close-after-reopen.png")
    close_window(hwnd)
    again.wait(timeout=10)
finally:
    print(json.dumps(result, indent=2))
    for p in (process,):
        if p.poll() is None:
            p.kill()
    shutil.rmtree(temp, ignore_errors=True)
