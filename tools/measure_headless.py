"""Measure the real UI renderer while headless, holding its GPU alive for WDDM sampling."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from measure_memory import sample

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("exe", type=Path)
parser.add_argument("--out", type=Path, required=True)
parser.add_argument("--scene", default="jump")
parser.add_argument("--runs", type=int, default=3)
args = parser.parse_args()
out = args.out.resolve()
out.mkdir(parents=True, exist_ok=True)
results = []
for n in range(args.runs):
    state = out / f"state-{n}"
    state.mkdir(exist_ok=True)
    ready = state / "ready.txt"
    ready.unlink(missing_ok=True)
    ready.with_suffix(".release").unlink(missing_ok=True)
    env = dict(os.environ, HEX_MEETINGS_STATE_DIR=str(state),
               HEX_MEETINGS_BENCH_HEADLESS_READY=str(ready), RUNE_GPU_LOG="1")
    with (out / f"run-{n}.log").open("w") as log:
        started = time.perf_counter()
        process = subprocess.Popen([str(args.exe.resolve()), "--snap", str(out / f"frame-{n}.png"),
                                    "--scene", args.scene, "--theme", "dark", "--size", "1320x800",
                                    "--scale", "1.25", "--at", "0.6"],
                                   env=env, stdout=log, stderr=log,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            deadline = time.monotonic() + 30
            while not ready.exists() and time.monotonic() < deadline:
                assert process.poll() is None, "Renderer exited before its sample point"
                time.sleep(.005)
            assert ready.exists(), "Headless renderer did not reach its sample point"
            rendered_ms = (time.perf_counter() - started) * 1000
            memory = sample(process.pid)
            result = dict(rendered_ms=rendered_ms, **memory)
            results.append(result)
            print(json.dumps(result), flush=True)
        finally:
            ready.with_suffix(".release").write_text("release")
            process.wait(timeout=20)
        assert process.returncode == 0, "Headless render failed"
    (out / "results.json").write_text(json.dumps(results, indent=2))
