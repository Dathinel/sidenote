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


def pairing(host, guest, decline=False):
    host.rpc("host")
    offered = wait(lambda: (p if (p := host.rpc("pair-state"))["phase"] == "code" else None))
    guest.rpc("join", code=offered["code"])
    del offered
    a = wait(lambda: (p if (p := host.rpc("pair-state"))["phase"] == "confirm" else None))
    b = wait(lambda: (p if (p := guest.rpc("pair-state"))["phase"] == "confirm" else None))
    assert a["phrase"] == b["phrase"], "SAS mismatch"
    mismatch = host.rpc_result("confirm", attempt=a["attempt"], phrase="synthetic mismatch", accept=True)
    stale = host.rpc_result("confirm", attempt=a["attempt"] + 1, phrase=a["phrase"], accept=True)
    assert not mismatch["ok"] and not stale["ok"], "mismatched/stale confirmation accepted"
    host.rpc("confirm", attempt=a["attempt"], phrase=a["phrase"], accept=not decline)
    guest.rpc("confirm", attempt=b["attempt"], phrase=b["phrase"], accept=not decline)
    del a, b
    expected = ("failed", "timed-out") if decline else ("paired",)
    wait(lambda: host.rpc("pair-state")["phase"] in expected)
    wait(lambda: guest.rpc("pair-state")["phase"] in expected)


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
        endpoint = server_root / "local/sync/control.sock"
        assert endpoint.stat().st_mode & 0o777 == 0o600
        assert endpoint.parent.stat().st_mode & 0o777 == 0o700
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(3)
            sock.connect(str(endpoint))
            sock.sendall(struct.pack(">I", 65537))
            assert sock.recv(1) == b"", "oversized frame accepted"
        duplicate = subprocess.run(server_args(), env=server_env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
        assert duplicate.returncode == 2, "duplicate owner accepted"
        print("PASS private endpoint, oversized request rejection, duplicate ownership", flush=True)

        if not opt.peer_command_json:
            full.rpc("check", meeting="100-2", task="Synthetic tick", checked=True)
            for marker in (full_root / "state").glob("*/area-format.json"):
                copied = server_root / marker.relative_to(full_root)
                copied.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                copied.write_bytes(marker.read_bytes())
                copied.chmod(0o600)
            source = next(p for p in (full_root / "state").rglob("*.json")
                          if len(p.relative_to(full_root / "state").parts) == 3
                          and json.loads(p.read_text()).get("version") == 2
                          and json.loads(p.read_text()).get("store") == "100-2.json")
            target = server_root / source.relative_to(full_root)
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            target.write_bytes(b"{")
            target.chmod(0o600)
            wait(lambda: control.rpc("status")["errors"] > 0)
            original = source.read_bytes()
            target.write_bytes(original)
            wait(lambda: "Synthetic tick" in control.rpc("checks", meeting="100-2")["checked"])
            wait(lambda: control.rpc("status")["errors"] == 0)
            assert target.read_bytes() == original, "ingestion rewrote transported journal"
            print("PASS journal partial-arrival retry and unchanged input", flush=True)

        folder = case / "owner/peers/synthetic-source/100-3"
        folder.mkdir(mode=0o700, parents=True)
        private_json(folder / "manifest.json", {"status": "pending"})
        (folder / "english-summary.json").write_bytes(b"{")
        (folder / "english-summary.json").chmod(0o600)
        time.sleep(1.2)
        assert not control.rpc("status")["meetings"], "incomplete meeting ingested"
        private_json(folder / "english-summary.json", {"schema": 1, "title": "Synthetic peer meeting"})
        private_json(folder / "manifest.json", {"status": "complete"})
        incoming = files_hash(folder)
        wait(lambda: control.rpc("status")["meetings"] == 1)
        generation = control.rpc("status")["generation"]
        time.sleep(2)
        assert control.rpc("status")["generation"] == generation, "repeated input duplicated ingestion"
        assert files_hash(folder) == incoming, "peer source changed"
        revision = control.rpc("inventory")[0]["revision"]
        private_json(folder / "english-summary.json", {"schema": 1, "title": "Synthetic revised meeting"})
        wait(lambda: control.rpc("inventory")[0]["revision"] != revision)
        assert control.rpc("status")["meetings"] == 1
        incoming = files_hash(folder)
        print("PASS unattended meeting ingestion, retry, content revision, deduplication and input preservation", flush=True)

        # A failed replacement must retain its last successful revision across restart.
        good = (folder / "english-summary.json").read_bytes()
        prior = control.rpc("inventory")[0]
        generation = control.rpc("status")["generation"]
        (folder / "english-summary.json").write_bytes(b"{")
        wait(lambda: control.rpc("inventory")[0]["pending"])
        assert control.rpc("inventory")[0]["revision"] == prior["revision"]
        assert control.rpc("status")["generation"] == generation
        control.close()
        os.killpg(server.pid, signal.SIGKILL)
        server.wait(timeout=10)
        start_owner()
        assert control.rpc("inventory")[0]["pending"]
        assert control.rpc("inventory")[0]["revision"] == prior["revision"]
        (folder / "english-summary.json").write_bytes(good)
        wait(lambda: not control.rpc("inventory")[0]["pending"])
        assert control.rpc("status")["generation"] == generation
        print("PASS last-good replacement revision survives restart and identical recovery", flush=True)

        second = case / "owner/peers/second-source/100-3"
        second.mkdir(mode=0o700, parents=True)
        private_json(second / "manifest.json", {"status": "complete"})
        private_json(second / "english-summary.json", {"schema": 1, "title": "Other source"})
        incomplete = case / "owner/meetings/100-3"
        incomplete.mkdir(mode=0o700, parents=True)
        (incomplete / "english-summary.json").write_bytes(b"{")
        wait(lambda: control.rpc("status")["meeting_conflicts"] == 1)
        rows = control.rpc("inventory")
        assert len(rows) == 2 and all(row["conflict"] for row in rows)
        old = next(row["revision"] for row in rows if row["device"] == "second-source")
        private_json(second / "english-summary.json", {"schema": 1, "title": "Later divergent source"})
        wait(lambda: next(row["revision"] for row in control.rpc("inventory") if row["device"] == "second-source") != old)
        (second / "english-summary.json").write_bytes(good)
        wait(lambda: control.rpc("status")["meeting_conflicts"] == 0)
        assert control.rpc("status")["meeting_sources"] == 2
        assert control.rpc("status")["meetings"] == 1
        assert incomplete.resolve().is_relative_to(case.resolve())
        shutil.rmtree(incomplete)
        print("PASS divergent sources remain visible; identical copies coalesce; local partial cannot hide peers", flush=True)

        for peer in (full, control):
            peer.rpc("configure", enabled=True, address=address)
        pairing(full, control, decline=True)
        wait(lambda: full.rpc("status")["peers"] == 0 and control.rpc("status")["peers"] == 0)
        print("PASS declined SAS grants no membership", flush=True)
        pairing(full, control)
        wait(lambda: full.rpc("status")["peers"] == 1 and control.rpc("status")["peers"] == 1)
        identity = control.rpc("status")["identity"]
        assert identity, "missing public identity fingerprint"
        assert identity != full.rpc("status")["identity"], "devices share an identity"
        full.rpc("add", task={"id": "synthetic-task", "task": "Synthetic task", "owner": "Initial"})
        wait(lambda: len(control.rpc("list")) == 1)
        control.rpc("edit", id="synthetic-task", fields={"due": "2030-01-02"})
        full.rpc("edit", id="synthetic-task", fields={"project": "Synthetic project"})
        wait(lambda: full.rpc("list") == control.rpc("list") and full.rpc("list")[0].get("due") == "2030-01-02")
        print("PASS two-sided confirmation and bidirectional automatic convergence", flush=True)

        for peer in (full, control):
            peer.rpc("offline")
        full.rpc("edit", id="synthetic-task", fields={"owner": "Full choice"})
        control.rpc("edit", id="synthetic-task", fields={"owner": "Server choice"})
        for peer in (full, control):
            peer.rpc("online")
        conflicts = wait(lambda: full.rpc("conflicts"))
        wait(lambda: control.rpc("conflicts"))
        choices = full.rpc("choices", conflict=conflicts[0])
        chosen = next(i for i, c in enumerate(choices) if c["value"] == "Full choice")
        full.rpc("resolve-choice", conflict=conflicts[0], choice=chosen)
        wait(lambda: not control.rpc("conflicts") and full.rpc("list") == control.rpc("list"))
        assert control.rpc("list")[0]["owner"] == "Full choice"
        print("PASS offline same-field conflict preserved and explicit resolution converged", flush=True)

        generation = control.rpc("status")["generation"]
        relay.close()
        full.rpc("edit", id="synthetic-task", fields={"task": "Synthetic edit during relay outage"})
        control.rpc("edit", id="synthetic-task", fields={"due": "2030-02-03"})
        relay, _ = start_relay(port)
        wait(lambda: control.rpc("status")["generation"] > generation)
        wait(lambda: full.rpc("list") == control.rpc("list"))
        assert control.rpc("list")[0]["task"] == "Synthetic edit during relay outage"
        print("PASS relay outage/reconnect and continuous ingestion without a window", flush=True)

        control.close()
        os.killpg(server.pid, signal.SIGKILL)
        server.wait(timeout=10)
        full.rpc("edit", id="synthetic-task", fields={"project": "Synthetic restart edit"})
        start_owner()
        wait(lambda: control.rpc("status")["identity"] == identity)
        wait(lambda: full.rpc("list") == control.rpc("list"))
        assert control.rpc("status")["meetings"] == 1
        assert len(control.rpc("list")) == 1
        assert files_hash(folder) == incoming
        print("PASS crash restart preserves identity/index and receives offline peer edits", flush=True)
        rss = next(line.split()[1] for line in Path(f"/proc/{server.pid}/status").read_text().splitlines() if line.startswith("VmRSS:"))
        maps = Path(f"/proc/{server.pid}/maps").read_text().lower()
        assert not any(name in maps for name in ("libx11", "libwayland", "libglx", "libegl", "libvulkan", "libnvidia")), "display/GPU library loaded"
        control.close()
        server.terminate()
        assert server.wait(timeout=15) == 0, "owner did not stop cleanly"
        assert not endpoint.exists(), "owner endpoint survived clean stop"
        server = None
        print("PASS clean shutdown releases endpoint; no display/GPU libraries loaded", flush=True)
        print(json.dumps({"role": opt.role, "peer": "adapter" if opt.peer_command_json else "Linux Full",
                          "idle_rss_kib": int(rss), "windows_runtime": "pending", "result": "passed"}), flush=True)
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
