"""Exercise the real listener against a private loopback server and measure private bytes."""
import ctypes
from ctypes import wintypes
import http.server
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import threading
import time


class Counters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
            "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]


def private_bytes(process):
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    get_info = ctypes.WinDLL("psapi").GetProcessMemoryInfo
    get_info.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    get_info.restype = wintypes.BOOL
    if not get_info(int(process._handle), ctypes.byref(counters), counters.cb):
        raise ctypes.WinError()
    return counters.PrivateUsage


def verify(exe, status):
    with tempfile.TemporaryDirectory(prefix="meetings-listener-") as temp:
        root = Path(temp)
        token = secrets.token_urlsafe(24)
        watch = root / "user/.config/hex/scripts/hex-meeting-watch.ps1"
        watch.parent.mkdir(parents=True)
        marker = root / "pipeline.txt"
        watch.write_text("Add-Content -LiteralPath '" + str(marker).replace("'", "''") + "' -Value 'run'\n", encoding="utf-8")

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                assert self.path == "/meetings/events"
                assert secrets.compare_digest(self.headers.get("Authorization", ""), "Bearer " + token)
                self.send_response(status)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                if status != 200:
                    self.wfile.write(b"0\r\n\r\n")
                    return
                pieces = [b': connected\n\n', b'data: {"type":"meeting.', b'finished","id":"1-2"}\n\n', b'data: {"type":"other"}\n\n']
                try:
                    for piece in pieces:
                        self.wfile.write(f"{len(piece):x}\r\n".encode() + piece + b"\r\n")
                        self.wfile.flush()
                    for _ in range(30):
                        piece = b": keepalive\n\n"
                        self.wfile.write(f"{len(piece):x}\r\n".encode() + piece + b"\r\n")
                        self.wfile.flush()
                        time.sleep(0.5)
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        discovery = root / "appdata/hex/local-api.json"
        discovery.parent.mkdir(parents=True)
        discovery.write_text(json.dumps({"port": server.server_port, "token": token}), encoding="utf-8")
        local = root / "local"
        local.mkdir()
        env = dict(os.environ, APPDATA=str(root / "appdata"), LOCALAPPDATA=str(local), USERPROFILE=str(root / "user"))
        process = subprocess.Popen([str(exe), "--listen"], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                assert process.poll() is None, "Listener exited"
                log_path = local / "hex-meeting-watch.log"
                log = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
                count = len(marker.read_text().splitlines()) if marker.exists() else 0
                if (status == 200 and count == 2) or (status != 200 and "retrying every 30 s" in log):
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("Listener did not reach the expected state")
            assert token not in log
            if status == 200:
                assert count == 2
                assert "listener: connected\n" in log
                assert "listener: meeting finished 1-2\n" in log
                time.sleep(1)
            else:
                assert count == 0
                expected = {404: "Labs > Meeting finished event is off", 409: "another app is already listening"}[status]
                assert expected + "; retrying every 30 s" in log
            return {"status": status, "pipeline_runs": count, "private_bytes": private_bytes(process)}
        finally:
            process.kill()
            process.wait()
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    exe = Path(sys.argv[1]).resolve()
    results = [verify(exe, status) for status in (200, 404, 409)]
    print(json.dumps(results, indent=2))
