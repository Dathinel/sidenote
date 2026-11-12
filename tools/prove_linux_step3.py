#!/usr/bin/env python3
"""Synthetic two-process proof. Private protocol responses are never logged.

--peer-command-json accepts an adapter command with {root} and {relay} placeholders.
The adapter must run the Full-role peer with isolated native paths and forward the
same NDJSON protocol. Convergence assertions are identical for a future Windows peer.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time


def wait(check, seconds=90):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError("proof condition deadline")


def private_json(path, value):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(value, stream)


class Pipe:
    def __init__(self, command, env):
        self.process = subprocess.Popen(command, env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                        text=True, start_new_session=os.name != "nt")
        self.lines = queue.Queue()
        def read():
            for line in self.process.stdout:
                self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=read, daemon=True).start()

    def rpc_result(self, op, **fields):
        self.process.stdin.write(json.dumps(dict(op=op, **fields)) + "\n")
        self.process.stdin.flush()
        line = self.lines.get(timeout=15)
        assert line is not None, "private control process exited"
        return json.loads(line)

    def rpc(self, op, **fields):
        result = self.rpc_result(op, **fields)
        assert result["ok"], "private control operation rejected: " + op
        value = result.get("value")
        if op in ("list", "conflicts", "inventory", "choices"):
            items = value["items"]
            while value["next"] is not None:
                value = self.rpc_result(op, **dict(fields, cursor=value["next"]))
                assert value["ok"], "paged read changed; retry proof"
                value = value["value"]
                items.extend(value["items"])
            return items
        return value

    def close(self):
        if self.process.poll() is None:
            if os.name == "nt":
                self.process.kill()
            else:
                os.killpg(self.process.pid, signal.SIGKILL)
        self.process.wait(timeout=10)


def environment(root, credential):
    env = os.environ.copy()
    for key in ("DISPLAY", "WAYLAND_DISPLAY"):
        env.pop(key, None)
    for key in ("HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        path = root / key.lower()
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        env[key] = str(path)
    env["CREDENTIALS_DIRECTORY"] = credential
    return env


def seed(root, name, role):
    root.mkdir(mode=0o700, parents=True)
    private_json(root / "device.json", {"id": os.urandom(16).hex(), "name": name,
                                        "role": role, "daily_check": False})


def files_hash(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob("*") if p.is_file()}


def main():
    def terminate(*_):
        raise SystemExit(2)
    signal.signal(signal.SIGTERM, terminate)
    args = argparse.ArgumentParser()
    args.add_argument("binary", type=Path)
    args.add_argument("--peer-command-json")
    args.add_argument("--role", choices=("server", "hub"), default="hub")
    opt = args.parse_args()
    binary = opt.binary.resolve()
    proof = Path(tempfile.gettempdir()) / "sidenote-sync-step3"
    proof.mkdir(mode=0o700, exist_ok=True)
    case = Path(tempfile.mkdtemp(prefix=opt.role + "-", dir=proof))
    full_root, server_root = case / "full/data", case / "owner/data"
    seed(full_root, "Synthetic Full peer", "full")
    seed(server_root, "Synthetic " + opt.role, opt.role)
    full_env = environment(case / "full", os.environ["STEP3_PEER_CREDENTIAL_DIRECTORY"])
    server_env = environment(case / "owner", os.environ["CREDENTIALS_DIRECTORY"])
    # The control client has no provider credential and cannot initialize a replica.
    client_env = server_env.copy()
    client_env.pop("CREDENTIALS_DIRECTORY", None)
    relay_script = Path(__file__).with_name("sync_step3_relay.cjs")
    children = []
    relay = full = control = server = None

    def start_relay(port=0):
        proc = Pipe(["node", str(relay_script), str(port)], os.environ.copy())
        children.append(proc)
        line = proc.lines.get(timeout=10)
        return proc, json.loads(line)["port"]

    def server_args():
        return [str(binary), "--server", "--device-role", opt.role, "--data-dir", str(server_root),
                "--meetings-dir", str(case / "owner/meetings"), "--peers-dir", str(case / "owner/peers")]

    def start_owner():
        nonlocal server, control
        server = subprocess.Popen(server_args(), env=server_env, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                  start_new_session=True)
        def ready_socket():
            assert server.poll() is None, "owner exited during startup"
            try:
                with socket.socket(socket.AF_UNIX) as probe:
                    probe.settimeout(0.1)
                    probe.connect(str(server_root / "local/sync/control.sock"))
                return True
            except OSError:
                return False
        wait(ready_socket, 15)
        control = Pipe([str(binary), "--sync-control", "--data-dir", str(server_root)], client_env)
        children.append(control)
        wait(lambda: control.rpc("status")["generation"] > 0, 15)

    try:
        relay, port = start_relay()
        address = "ws://127.0.0.1:" + str(port)
        if opt.peer_command_json:
            command = [s.replace("{root}", str(full_root)).replace("{relay}", address)
                       for s in json.loads(opt.peer_command_json)]
        else:
            command = [str(binary.parent / "examples/sync_peer"), "--device-role", "full",
                       "--data-dir", str(full_root), "--meetings-dir", str(case / "full/meetings"),
                       "--peers-dir", str(case / "full/peers")]
        full = Pipe(command, full_env)
        children.append(full)
        wait(lambda: full.rpc("status")["generation"] > 0, 15)
        start_owner()
        print("PASS isolated Full/owner foundation ready", flush=True)
    finally:
        for child in reversed(children):
            child.close()
        if server and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(server.pid, signal.SIGKILL)
                server.wait()


if __name__ == "__main__":
    main()
