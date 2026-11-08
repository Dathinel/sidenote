"""Launch to first presented frame, and close to process exit, in release.

Usage: python tools/measure_launch.py <exe> [--runs N]
The app writes the Unix time of its first presented frame to the file named by
SIDENOTE_TRACE_FIRST_FRAME; this script subtracts its own launch time. "Window shown" is when
Windows reports the window visible and not cloaked. Each
run starts a fresh process that reads the real meetings folder (read only) with a throwaway tick
and chat store, closes it with WM_CLOSE, and checks that the process is gone and had no child
processes. Launching here is a plain process start; a shortcut goes through Explorer, which adds
about 0.35 s before the process exists. Other Sidenote or legacy hex-meetings processes, such as --listen, are never
touched.
"""
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time

from winutil import alive, close_window, descendants, wait_window

exe = Path(sys.argv[1]).resolve()
binary = exe.read_bytes()
assert all(name in binary for name in (b"SIDENOTE_BENCH_OFFSCREEN", b"SIDENOTE_BENCH_NO_WARM")), "Executable lacks the isolated off-screen mode"
runs = int(sys.argv[sys.argv.index("--runs") + 1]) if "--runs" in sys.argv else 7
state = Path(__file__).resolve().parents[1] / "artifacts/memory/launch" / str(time.time_ns())
state.mkdir(parents=True)
trace = state / "first-frame.txt"
env = dict(os.environ, SIDENOTE_STATE_DIR=str(state), SIDENOTE_TRACE_FIRST_FRAME=str(trace),
           SIDENOTE_BENCH_OFFSCREEN="1", SIDENOTE_BENCH_WINDOW_READY=str(state / "ready.txt"))
first_paint, close_exit, drawn = [], [], []
process, hwnd = None, None
try:
    for n in range(runs + 1):  # the first run warms the disk cache and is not counted
        trace.unlink(missing_ok=True)
        started_epoch = time.time()
        started = time.perf_counter()
        hwnd = None
        process = subprocess.Popen([str(exe)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
        hwnd, seen = wait_window(process)
        elapsed = (seen - started) * 1000
        wait = time.monotonic() + 10
        while not trace.exists() and time.monotonic() < wait:
            time.sleep(0.002)
        time.sleep(0.8)
        left_over = descendants(process.pid)
        closing = time.perf_counter()
        close_window(hwnd)
        process.wait(timeout=10)
        gone = (time.perf_counter() - closing) * 1000
        assert not alive(process.pid), "The process is still running"
        assert not left_over, f"Child processes at idle: {left_over}"
        if trace.exists() and n:
            drawn.append((int(trace.read_text()) / 1e6 - started_epoch) * 1000)
        if n:
            first_paint.append(elapsed)
            close_exit.append(gone)
finally:
    if process is not None and process.poll() is None:
        if hwnd:
            close_window(hwnd)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    trace.unlink(missing_ok=True)
print(json.dumps({
    "runs": runs,
    "launch_to_window_shown_ms": [round(v) for v in first_paint],
    "launch_to_window_shown_median_ms": round(statistics.median(first_paint)),
    "close_to_exit_ms": [round(v) for v in close_exit],
    "close_to_exit_median_ms": round(statistics.median(close_exit)),
    **({"launch_to_first_frame_ms": [round(v) for v in drawn], "launch_to_first_frame_median_ms": round(statistics.median(drawn))} if drawn else {}),
}, indent=2))
