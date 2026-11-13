#!/usr/bin/env python3
"""Synthetic worker-tree proof. The trusted driver stays outside each bounded owner unit."""
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def live(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def wait(check, seconds=10):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        value = check()
        if value:
            return value
        time.sleep(0.03)
    raise AssertionError("condition deadline")


def workload(root, allocate):
    assert os.environ["R1_PUBLIC_ENV_CHECK"] == "public-fixture"
    if "CREDENTIALS_DIRECTORY" in os.environ:
        credential = Path(os.environ["CREDENTIALS_DIRECTORY"]) / "sidenote-key"
        assert credential.stat().st_size == 32
        assert credential.stat().st_mode & 0o077 == 0
    child = os.fork()
    if child == 0:
        os.setsid()  # Escape the old process-group-only boundary.
        (root / "grandchild.pid").write_text(Path("/proc/self/stat").read_text().split()[0])
        if allocate:
            wait(lambda: (root / "go").exists())
            held = bytearray(300 * 1024 * 1024)
            assert held[0] == 0
        time.sleep(90)
        os._exit(0)
    grandchild = int(wait(lambda: (root / "grandchild.pid").read_text() if (root / "grandchild.pid").exists() else None))
    group = Path("/proc/self/cgroup").read_text().strip().split("::", 1)[1]
    (root / "pids.json").write_text(json.dumps({
        "worker": int(Path("/proc/self/stat").read_text().split()[0]), "grandchild": grandchild, "group": group,
    }))
    if allocate:
        wait(lambda: (root / "go").exists())
        held = bytearray(300 * 1024 * 1024)
        assert held[0] == 0
    time.sleep(90)


def driver(binary_dir, root):
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    assert ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) == 0  # Subreap adopted launchers.
    example = binary_dir / "examples/linux_platform_proof"
    env = os.environ.copy()
    env["R1_PUBLIC_ENV_CHECK"] = "public-fixture"
    for mode in ("worker-death", "launcher-death", "owner-death", "aggregate-memory"):
        case = root / mode
        case.mkdir(mode=0o700)
        unit = "sidenote-review-" + mode + "-" + str(os.getpid())
        process = subprocess.Popen([
            "systemd-run", "--user", "--unit=" + unit, "--wait", "--pipe", "--collect",
            "--service-type=exec", "--property=MemoryMax=512M", "--property=TasksMax=32",
            "--property=CPUQuota=100%", "--property=OOMPolicy=stop",
            "--setenv=R1_PUBLIC_ENV_CHECK=public-fixture",
            str(example), "tree", "/usr/bin/python3", str(Path(__file__).resolve()),
            "workload", str(case), "allocate" if mode == "aggregate-memory" else "wait",
        ], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            line = process.stdout.readline().strip()
            assert line.startswith("launcher_pid="), (line, process.stderr.read())
            launcher = int(line.split("=")[1])
            wait(lambda: (case / "pids.json").exists())
            info = json.loads((case / "pids.json").read_text())
            worker, grandchild = info["worker"], info["grandchild"]
            assert live(worker) and live(grandchild)
            group = Path("/sys/fs/cgroup") / info["group"].lstrip("/")
            assert (group / "memory.max").read_text().strip() == "536870912"
            assert (group / "pids.max").read_text().strip() == "32"
            owner = int(subprocess.check_output(["systemctl","--user","show",unit,"-p","MainPID","--value"]))
            assert os.getsid(grandchild) == grandchild
            print(f"{mode}: tree memory=536870912 tasks=32 cpu={(group / 'cpu.max').read_text().strip()}", flush=True)
            if mode == "aggregate-memory":
                (case / "go").touch()
            else:
                victim = {"worker-death": worker, "launcher-death": launcher,
                          "owner-death": owner}[mode]
                os.kill(victim, signal.SIGKILL)
            wait(lambda: not live(worker) and not live(grandchild))
            process.wait(timeout=5)
            print(f"{mode}: escaped grandchild dead, owner/front exit contained: PASS", flush=True)
        finally:
            subprocess.run(["systemctl","--user","stop",unit], capture_output=True, timeout=10)
            if process.poll() is None:
                process.kill()
                process.wait()
            while True:
                try:
                    pid, _ = os.waitpid(-1, os.WNOHANG)
                    if not pid:
                        break
                except ChildProcessError:
                    break


if __name__ == "__main__":
    if sys.argv[1] == "workload":
        workload(Path(sys.argv[2]), sys.argv[3] == "allocate")
    else:
        driver(Path(sys.argv[1]), Path(sys.argv[2]))
