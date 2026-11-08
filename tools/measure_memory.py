"""Release memory/startup probe. Never sends keyboard or mouse input.

The optional app-owned command file opens and scrolls the longest transcript.
All output and throwaway state stay under the supplied worktree output folder.
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

from winutil import capture, close_window, wait_window, user32


def sample(pid):
    # These are the same process and WDDM counter providers used by Task Manager.
    script = r"""
$p = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter 'IDProcess = PID_VALUE'
$g = @(Get-CimInstance Win32_PerfRawData_GPUPerformanceCounters_GPUProcessMemory | Where-Object { $_.Name -like 'pid_PID_VALUE_*' })
if (-not $p -or -not $g) { throw 'Missing process or GPU counters' }
@{private_bytes=[long]$p.PrivateBytes; private_working_set=[long]$p.WorkingSetPrivate; working_set=[long]$p.WorkingSet;
gpu_dedicated=[long](($g | Measure-Object DedicatedUsage -Sum).Sum);
gpu_shared=[long](($g | Measure-Object SharedUsage -Sum).Sum);
gpu_committed=[long](($g | Measure-Object TotalCommitted -Sum).Sum)} | ConvertTo-Json -Compress
""".replace("PID_VALUE", str(pid))
    result = subprocess.run(["powershell", "-NoProfile", "-Command", script],
                            capture_output=True, text=True, check=True,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--scroll", action="store_true")
    args = parser.parse_args()
    binary = args.exe.read_bytes()
    assert all(name in binary for name in (b"SIDENOTE_BENCH_OFFSCREEN", b"SIDENOTE_BENCH_NO_WARM")), "Executable lacks the isolated off-screen mode"
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    results = []
    for n in range(args.runs):
        state = out / f"state-{n}"
        state.mkdir(exist_ok=True)
        trace, command = state / "first-frame.txt", state / "command.txt"
        ready = state / "window-ready.txt"
        ready.unlink(missing_ok=True)
        trace.unlink(missing_ok=True)
        command.unlink(missing_ok=True)
        command.with_suffix(".done").unlink(missing_ok=True)
        env = dict(os.environ, SIDENOTE_STATE_DIR=str(state),
                   SIDENOTE_TRACE_FIRST_FRAME=str(trace), RUNE_GPU_LOG="1",
                   SIDENOTE_BENCH_OFFSCREEN="1", SIDENOTE_BENCH_WINDOW_READY=str(ready))
        if args.scroll:
            env["SIDENOTE_BENCH_COMMAND"] = str(command)
        with (out / f"run-{n}.log").open("w") as log:
            epoch = time.time_ns() / 1e6
            started = time.perf_counter()
            process = subprocess.Popen([str(args.exe.resolve())], env=env, stdout=log,
                                       stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
            hwnd = None
            try:
                hwnd, shown = wait_window(process)
                assert ready.exists(), "App did not acknowledge the safe window mode"
                assert user32.GetForegroundWindow() != hwnd, "Benchmark window took focus"
                shown_ms = (shown - started) * 1000
                deadline = time.monotonic() + 30
                while not trace.exists() and time.monotonic() < deadline:
                    time.sleep(.002)
                assert trace.exists(), "First frame was not traced"
                first_ms = int(trace.read_text()) / 1000 - epoch
                time.sleep(2)
                result = {"pid": process.pid, "shown_ms": shown_ms, "first_frame_ms": first_ms,
                          "idle": sample(process.pid)}
                capture(hwnd, out / f"idle-{n}.png")
                if args.scroll:
                    command.write_text("scroll", encoding="utf-8")
                    deadline = time.monotonic() + 60
                    while not command.with_suffix(".done").exists() and time.monotonic() < deadline:
                        assert process.poll() is None
                        time.sleep(.05)
                    assert command.with_suffix(".done").exists(), "Scroll benchmark did not finish"
                    result["transcript"] = json.loads(command.with_suffix(".done").read_text())
                    time.sleep(2)
                    result["scrolled"] = sample(process.pid)
                    capture(hwnd, out / f"scrolled-{n}.png")
                # Minimize this PID's HWND directly. No desktop input is generated.
                user32.ShowWindow(hwnd, 7)
                assert user32.IsIconic(hwnd), "Window did not minimize"
                time.sleep(3)
                result["minimized"] = sample(process.pid)
                results.append(result)
            finally:
                if hwnd:
                    close_window(hwnd)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            (out / "results.json").write_text(json.dumps(results, indent=2))
            print(json.dumps(result), flush=True)
    medians = {phase: {key: statistics.median(r[phase][key] for r in results)
                       for key in results[0][phase]}
               for phase in ("idle", "scrolled", "minimized", "restored") if phase in results[0]}
    medians.update({key: statistics.median(r[key] for r in results)
                    for key in ("shown_ms", "first_frame_ms")})
    (out / "medians.json").write_text(json.dumps(medians, indent=2))
    print(json.dumps(medians, indent=2))


if __name__ == "__main__":
    main()
