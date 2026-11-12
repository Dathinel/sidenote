#!/usr/bin/env python3
"""Synthetic process qualification for the service; does not run any test suite."""
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import tempfile
import time


def environment(root):
    env = {
        key: value for key, value in os.environ.items()
        if key not in ("DISPLAY", "WAYLAND_DISPLAY")
        and not key.startswith(("SIDENOTE_", "HEX_MEETINGS_"))
    }
    for name, leaf in (("HOME", "home"), ("XDG_DATA_HOME", "data"),
                       ("XDG_STATE_HOME", "state"), ("XDG_CONFIG_HOME", "config"),
                       ("XDG_CACHE_HOME", "cache")):
        env[name] = str(root / leaf)
    return env


def qualify(binary, root, role, restart=False, stop_signal=signal.SIGTERM):
    env = environment(root)
    args = [str(binary), "--data-dir", str(root / role)]
    if not restart:
        args += ["--server", "--device-role", role]
    started = time.monotonic()
    process = subprocess.Popen(args, env=env, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 15
        startup = b""
        status = ""
        while time.monotonic() < deadline and not status:
            if not select.select([process.stderr], [], [], max(0, deadline - time.monotonic()))[0]:
                break
            chunk = os.read(process.stderr.fileno(), 8192)
            if not chunk:
                break
            startup += chunk
            status = next((line for line in startup.decode().splitlines()
                           if "status=running" in line), "")
        if not status:
            raise RuntimeError("No startup status within 15 seconds: " + startup.decode())
        if "status=running" not in status or f"role={role}" not in status or "ui=false" not in status:
            raise RuntimeError(f"Unexpected startup: {status}")
        if role == "hub" and "processing=disabled" not in status:
            raise RuntimeError("Hub started processing")
        time.sleep(2)
        proc = Path(f"/proc/{process.pid}")
        maps = (proc / "maps").read_text()
        gpu = re.compile(r"lib(vulkan|GL\b|GLX|EGL|wayland|X11|xcb|nvidia|cuda|drm|gbm)|wgpu|winit|librune", re.I)
        if gpu.search(maps):
            raise RuntimeError("Display or GPU library loaded")
        devices = [os.readlink(fd) for fd in (proc / "fd").iterdir() if fd.exists()]
        if any(re.search(r"/dev/(dri|nvidia|fb)", device) for device in devices):
            raise RuntimeError("GPU device opened")
        rss_kib = int(re.search(r"^VmRSS:\s+(\d+)", (proc / "status").read_text(), re.M)[1])
        if rss_kib > 100 * 1024:
            raise RuntimeError("Idle service exceeds 100 MiB")
        libraries = sorted({line.split()[-1] for line in maps.splitlines()
                            if ".so" in line and "/" in line})
        settings = json.loads((root / role / "device.json").read_text())
        if settings["role"] != role:
            raise RuntimeError("Role was not persisted")
        duplicate = subprocess.run(args, env=env, capture_output=True, text=True, timeout=5)
        if duplicate.returncode != 2 or "SERVICE_LOCK_FAILED" not in duplicate.stderr:
            raise RuntimeError("Duplicate supervisor was accepted")
        process.send_signal(stop_signal)
        _, tail = process.communicate(timeout=10)
        if process.returncode != 0 or "status=stopped" not in tail:
            raise RuntimeError("Signal did not shut down cleanly")
        print(json.dumps({"role": role, "restart": restart, "status": status,
                          "exit": process.returncode, "signal": signal.Signals(stop_signal).name,
                          "idle_rss_mib": round(rss_kib / 1024, 2),
                          "idle_rss_mb": round(rss_kib * 1024 / 1_000_000, 2),
                          "elapsed_seconds": round(time.monotonic() - started, 2),
                          "display_gpu_libraries": 0, "gpu_devices": 0, "libraries": libraries}))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def qualify_mcp(binary, root):
    process = subprocess.Popen([str(binary), "--mcp", "--data-dir", str(root / "hub")],
                               env=environment(root), stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def send(message):
        process.stdin.write(json.dumps(message) + "\n")
        process.stdin.flush()

    def receive(request_id):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if not select.select([process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                break
            line = process.stdout.readline()
            if not line:
                break
            message = json.loads(line)
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError("MCP request failed")
                return message["result"]
        raise RuntimeError("MCP response timed out")

    try:
        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "synthetic-service-proof", "version": "1"}}})
        receive(1)
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = receive(2)["tools"]
        if not tools:
            raise RuntimeError("MCP did not expose tools")
        process.stdin.close()
        process.stdin = None
        process.communicate(timeout=10)
        if process.returncode != 0:
            raise RuntimeError("MCP did not exit cleanly")
        print(json.dumps({"mcp": "stdio", "initialize": "passed", "tools": len(tools), "exit": 0}))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="sidenote-service-proof-") as folder:
        root = Path(folder)
        for device_role in ("full", "server", "hub"):
            qualify(binary, root, device_role)
            if device_role != "full":
                qualify(binary, root, device_role, restart=True,
                        stop_signal=signal.SIGINT if device_role == "server" else signal.SIGTERM)
        qualify_mcp(binary, root)
