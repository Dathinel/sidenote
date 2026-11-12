#!/usr/bin/env python3
"""Private-bus hung-provider and unsafe-state regression proof; synthetic state only."""
import json
import os
from pathlib import Path
import select
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time


def line(process, deadline=5):
    assert select.select([process.stdout], [], [], deadline)[0], "provider readiness deadline"
    value = process.stdout.readline().strip()
    assert value, process.stderr.read()
    return value


def environment(root):
    env = os.environ.copy()
    env.pop("CREDENTIALS_DIRECTORY", None)
    for name in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        path = root / name.lower()
        path.mkdir(mode=0o700)
        env[name] = str(path)
    return env


binary = Path(sys.argv[1])
root = Path(tempfile.mkdtemp(prefix="startup-", dir=sys.argv[2]))
mock = subprocess.Popen(["/usr/bin/python3", str(Path(__file__).with_name("prove_linux_review_secret.py"))],
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
try:
    assert "ready" in line(mock)
    for cancel in (False, True):
        case = root / ("cancel" if cancel else "deadline")
        case.mkdir(mode=0o700)
        started = time.monotonic()
        process = subprocess.Popen([
            str(binary), "--server", "--device-role", "hub", "--data-dir", str(case / "state"),
        ], env=environment(case), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            assert "received" in line(mock)
            if cancel:
                process.send_signal(signal.SIGTERM)
            _, stderr = process.communicate(timeout=8)
            elapsed = time.monotonic() - started
            assert process.returncode != 0
            if cancel:
                assert "SECRET_INITIALIZATION_CANCELLED" in stderr, stderr
                assert elapsed < 3.5, elapsed
            else:
                assert "SECRET_SERVICE_UNAVAILABLE" in stderr, stderr
                assert 4.5 < elapsed < 7, elapsed
            db = sqlite3.connect(case / "state/local/relay/replica.sqlite")
            assert db.execute("select count(*) from protected").fetchone()[0] == 0
            db.close()
            print(json.dumps({"blocked_secret_service": True, "SIGTERM": cancel,
                              "exit": process.returncode, "seconds": round(elapsed, 3),
                              "protected_rows": 0, "error": stderr.strip().splitlines()[-1]}), flush=True)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
finally:
    mock.terminate()
    mock.wait(timeout=3)
