"""Run the requested Diary gates from this worktree, with no visible application windows."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "artifacts/diary-build"
ENV = os.environ.copy()
ENV["PATH"] = r"E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;" + ENV["PATH"]
ENV["CARGO_TARGET_DIR"] = str(ROOT / "target")
# Keep generated caches and parallel link outputs within the available worktree disk space.
ENV["CARGO_INCREMENTAL"] = "0"
ENV["CARGO_PROFILE_DEV_DEBUG"] = "0"
ENV["CARGO_BUILD_JOBS"] = "1"
EXE = ROOT / "target/x86_64-pc-windows-gnu/debug/sidenote.exe"
GATES = {
    "fmt": ["cargo", "fmt", "--all", "--check"],
    "clippy": ["cargo", "clippy", "--workspace", "--locked", "--all-targets", "--", "-D", "warnings"],
    "clippy-asr": ["cargo", "clippy", "--workspace", "--locked", "--all-targets", "--features", "asr-runtime", "--", "-D", "warnings"],
    "test": ["cargo", "test", "--workspace", "--locked"],
    "test-asr": ["cargo", "test", "--workspace", "--locked", "--features", "asr-runtime"],
    "build": ["cargo", "build", "--locked"],
    "smoke": ["python", "tools/mcp-smoke.py", str(EXE)],
    "settings": ["python", "tools/settings-reachability.py", str(OUT)],
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("gates", nargs="*", choices=list(GATES), default=list(GATES))
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    manifest = OUT / "gates.json"
    rows = json.loads(manifest.read_text()) if manifest.exists() else {}
    for name in args.gates:
        command = GATES[name]
        started = time.monotonic()
        print("RUNNING " + name, flush=True)
        with (OUT / ("gate-" + name + ".log")).open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=ENV, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        rows = json.loads(manifest.read_text()) if manifest.exists() else {}
        rows[name] = dict(command=command, exit_code=result.returncode, seconds=round(time.monotonic()-started, 2))
        manifest.write_text(json.dumps(rows, indent=2))
        print(("PASS " if result.returncode == 0 else "FAIL ") + name + f" ({rows[name]['seconds']}s)", flush=True)
        if result.returncode: raise SystemExit(result.returncode)

if __name__ == "__main__": main()
