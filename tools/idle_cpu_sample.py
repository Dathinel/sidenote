"""Isolated release recorder, no meeting, no UI, no owner Hex discovery."""
import argparse
import ctypes
import http.server
import json
import os
import pathlib
import secrets
import subprocess
import tempfile
import threading
import time
import psutil
from idle_cpu_job import CpuJob

def system_times():
    idle, kernel, user = (ctypes.c_ulonglong() for _ in range(3))
    if not ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)):
        raise ctypes.WinError()
    return idle.value, kernel.value + user.value

parser = argparse.ArgumentParser()
parser.add_argument("exe")
parser.add_argument("--mock", action="store_true")
parser.add_argument("--meetings", type=int, default=0)
parser.add_argument("--detail", action="store_true")
parser.add_argument("--done-jobs", action="store_true")
args = parser.parse_args()
args.exe = str(pathlib.Path(args.exe).resolve(strict=True))
proof_root = pathlib.Path(tempfile.gettempdir()) / "sidenote-idle-cpu-proof"
proof_root.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="trial-", dir=proof_root) as temp:
    root = pathlib.Path(temp)
    for name in ("data", "hex/meetings", "peers", "appdata"):
        (root / name).mkdir(parents=True)
    # Skip legacy migration, which would inspect the owner's script path.
    (root / "data/config.json").write_text(json.dumps({"version": 1, "features": {}, "notes": {"engine": "app", "helper": None}}))
    for index in range(args.meetings):
        folder = root / "hex/meetings" / f"1800000000000-{index}"
        folder.mkdir()
        (folder / "manifest.json").write_text(json.dumps({"status": "complete", "duration_ms": 30000}))
        (folder / "quick-title.json").write_text(json.dumps({"title": f"Synthetic meeting {index}"}))
        if not args.done_jobs:
            (folder / "english-summary.json").write_text(json.dumps({"title": f"Synthetic meeting {index}", "context": ["Synthetic context. " * 100]}))
        (folder / "transcript.ndjson").write_text(json.dumps({"start_ms": 0, "text": " ".join(["Synthetic"] * 25)}) + "\n")
    server = None
    calls = [0]
    if args.mock:
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_GET(self):
                self.send_response(404)
                self.end_headers()
            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", "0")))
                calls[0] += 1
                body = json.dumps({"version": 4, "status": "status", "app_version": "synthetic", "meeting": {"state": "idle"}, "capture": {"state": "idle"}, "model": {}, "capabilities": {}}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        (root / "hex/local-api.json").write_text(json.dumps({"port": server.server_port, "token": secrets.token_hex(16)}))
        threading.Thread(target=server.serve_forever, daemon=True).start()
    env = dict(os.environ, SIDENOTE_DATA_DIR=str(root / "data"), SIDENOTE_HEX_DIR=str(root / "hex/meetings"), SIDENOTE_PEERS_DIR=str(root / "peers"), APPDATA=str(root / "appdata"))
    paths = ["--data-dir", str(root / "data"), "--meetings-dir", str(root / "hex/meetings"), "--peers-dir", str(root / "peers")]
    if args.done_jobs:
        seed = subprocess.run([args.exe, "--idle-cpu-seed-notes", *paths], env=env,
                              creationflags=subprocess.CREATE_NO_WINDOW, capture_output=True)
        print(seed.stdout.decode(errors="replace"), end="", flush=True)
        assert seed.returncode == 0, "production Done fixture failed"
        jobs = list((root / "data/local/notes-jobs").glob("*.json"))
        assert len(jobs) == args.meetings > 0
        assert all(json.loads(path.read_text())["state"] == "done" for path in jobs)
        assert len(list((root / "hex/meetings").glob("*/english-summary.json"))) == args.meetings
    accounting = CpuJob()
    proc = accounting.start([args.exe, "--listen", *paths], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(5)
        if args.meetings:
            deadline = time.monotonic() + 120
            cursor = root / "data/local/recording-source.json"
            while time.monotonic() < deadline:
                try:
                    if len(json.loads(cursor.read_text())["delivered"]) == args.meetings:
                        break
                except (OSError, ValueError, KeyError):
                    pass
                time.sleep(0.1)
            else:
                raise RuntimeError("Synthetic completion catch-up did not finish; not an idle sample")
        process = psutil.Process(proc.pid)
        retained_done = sum(json.loads(path.read_text())["state"] == "done"
                            for path in (root / "data/local/notes-jobs").glob("*.json"))
        if args.done_jobs:
            assert retained_done == args.meetings, "retained production Done jobs changed during catch-up"
        before = process.cpu_times()
        tree_before = accounting.sample()
        io_before = process.io_counters()
        threads_before = {t.id: t.user_time + t.system_time for t in process.threads()}
        system_before = system_times()
        begin = time.monotonic()
        calls_before = calls[0]
        if args.detail:
            previous = before.user + before.system
            for second in range(1, 31):
                time.sleep(max(0, begin + second - time.monotonic()))
                now = process.cpu_times()
                current = now.user + now.system
                print(json.dumps(dict(second=second, processor_seconds=current-previous)), flush=True)
                previous = current
        else:
            time.sleep(30)
        after = process.cpu_times()
        tree_after = accounting.sample()
        system_after = system_times()
        elapsed = time.monotonic() - begin
        cpu = after.user + after.system - before.user - before.system
        tree_cpu = tree_after[0] - tree_before[0]
        total = system_after[1] - system_before[1]
        load = 100 * (1 - (system_after[0] - system_before[0]) / total)
        if args.detail:
            print(json.dumps(dict(user_cpu=after.user-before.user, kernel_cpu=after.system-before.system,
                read_bytes=process.io_counters().read_bytes-io_before.read_bytes,
                threads=[dict(id=t.id, cpu=t.user_time+t.system_time-threads_before.get(t.id,0)) for t in process.threads()])))
        print(json.dumps(dict(configuration="release, isolated dirs, no recording, event endpoint 404", synthetic_meetings=args.meetings, seeded_production_done_jobs=args.meetings if args.done_jobs else 0, retained_done_jobs=retained_done, mock=args.mock, seconds=elapsed, recorder_cpu_seconds=cpu, child_cpu_seconds=tree_cpu-cpu, processor_seconds=tree_cpu, one_core_percent=100 * tree_cpu / elapsed, system_load_percent=load, status_calls=calls[0] - calls_before, new_child_processes=tree_after[1]-tree_before[1], total_processes=tree_after[1])))
        assert tree_cpu / elapsed < 0.01, "recorder and children exceeded 1% of one core"
        assert tree_after[1] == 1, "idle recorder launched child processes"
    finally:
        accounting.close()
        proc.wait()
        if server:
            server.shutdown()
