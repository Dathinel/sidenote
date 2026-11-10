"""Local-only wrangler + release Rust clients. Raw ciphertext remains disposable.

Run from relay/: python harness/run.py (after bun install and Rust release build).
No keys, codes, auth messages, or document bodies are printed by this runner.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from scan import scan

RELAY = Path(__file__).resolve().parents[1]
ROOT = RELAY.parent
EVIDENCE = RELAY / "evidence"
LOCAL = RELAY / ".local-evidence"
EXE = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "x86_64-pc-windows-gnu/release/examples/local_e2e.exe"


def main() -> None:
    quick = "--quick" in sys.argv
    EVIDENCE.mkdir(exist_ok=True)
    LOCAL.mkdir(exist_ok=True)
    env = os.environ.copy()
    env.update(WRANGLER_SEND_METRICS="false", WRANGLER_CHECK_UPDATE="false", CI="true", BROWSER="none")
    creationflags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    bun = shutil.which("bun")
    if not bun or not EXE.is_file():
        raise RuntimeError("Install bun dependencies and build the Rust release example first")
    raw_path = LOCAL / "ciphertext.log"
    state = LOCAL / f"state-{time.time_ns()}"
    command = [bun, "x", "wrangler", "dev", "--config", "harness/wrangler.toml", "--local",
               "--ip", "127.0.0.1", "--port", "8787", "--inspector-ip", "127.0.0.1",
               "--inspector-port", "9237", "--show-interactive-dev-session", "false", "--persist-to", str(state)]
    with raw_path.open("w", encoding="utf-8") as raw:
        relay = subprocess.Popen(command, cwd=RELAY, env=env, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                                 creationflags=creationflags)

        capture_errors = []
        def capture() -> None:
            try:
                if relay.stdout is None:
                    raise RuntimeError("relay output pipe missing")
                for line in relay.stdout:
                    raw.write(line)
                raw.flush()
            except Exception as error:
                capture_errors.append(error)

        reader = threading.Thread(target=capture)
        reader.start()
        try:
            deadline = time.monotonic() + 45
            while True:
                if relay.poll() is not None:
                    raise RuntimeError("local wrangler exited; inspect disposable log")
                try:
                    with urllib.request.urlopen("http://127.0.0.1:8787/health", timeout=1) as response:
                        if response.read() != b"sidenote-relay-v1":
                            raise RuntimeError("unexpected relay health response")
                    break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError("local wrangler startup timeout")
                    time.sleep(0.2)
            print("Local wrangler ready; exercising two release clients", flush=True)
            with urllib.request.urlopen("http://127.0.0.1:8787/__proof/negative", timeout=10) as response:
                if response.read() != b"negative-control-written":
                    raise RuntimeError("negative control was not written")
            # Inherit the existing console handles. CREATE_NO_WINDOW hangs this GNU
            # executable before main on Windows; no new console/window is requested.
            subprocess.run([str(EXE), str(EVIDENCE / ("round1-quick.json" if quick else "round1-local.json"))] + (["--quick"] if quick else []), cwd=RELAY, env=env,
                           check=True, timeout=600)
        finally:
            # Only this runner's process tree is terminated; no other wrangler session is touched.
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(relay.pid), "/T", "/F"], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, creationflags=creationflags, check=False)
            else:
                relay.terminate()
            relay.wait(timeout=15)
            reader.join()
            if reader.is_alive():
                raise RuntimeError("capture reader did not stop")
            if capture_errors:
                raise RuntimeError("capture reader failed") from capture_errors[0]

    proof = scan(raw_path.read_text(encoding="utf-8").splitlines())
    frames = proof["captures_by_kind"]["in:envelope-3"]
    if frames <= (0 if quick else 1000):
        raise RuntimeError("ciphertext observer did not cover the edit stream")
    report_path = EVIDENCE / ("round1-quick.json" if quick else "round1-local.json")
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["relay_proof"] = proof
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"result": "passed", "ciphertext_frames": frames, "bytes_scanned": proof["bytes_scanned"],
                      "plaintext_marker_occurrences": 0, "latency": {k: v for k, v in report["latency"].items() if k != "samples_ms"},
                      "idle": report["idle"], "offline": report["offline"]}), flush=True)


if __name__ == "__main__":
    main()
