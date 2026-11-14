"""Run as root; inspect only this owned synthetic proof unit's namespace init."""
import os
from pathlib import Path
import subprocess
import sys

binary, unit, expected = sys.argv[1:]
command = ["systemd-run", "--unit=" + unit, "--wait", "--pipe", "--collect"]
for key, value in {"User": "boxes", "MemoryMax": "512M", "MemorySwapMax": "0",
                   "TasksMax": "32", "CPUQuota": "100%", "OOMPolicy": "continue",
                   "RuntimeMaxSec": "90", "Delegate": "memory pids cpu"}.items():
    command.append("--property=" + key + "=" + value)
for key, value in {"SIDENOTE_TEST_UNIT": unit, "SIDENOTE_DATA_DIR": os.environ["SIDENOTE_DATA_DIR"],
                   "TMPDIR": os.environ["TMPDIR"]}.items():
    command.append("--setenv=" + key + "=" + value)
run = subprocess.Popen(command + [binary], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
observed = None
try:
    for line in run.stdout:
        print(line, end="", flush=True)
        if line.strip() != "FD_READY":
            continue
        main = subprocess.check_output(["systemctl", "show", unit, "-p", "MainPID", "--value"], text=True).strip()
        pending = [main]
        while pending:
            pid = pending.pop()
            proc = Path("/proc") / pid
            pending += (proc / "task" / pid / "children").read_text().split()
            status = (proc / "status").read_text().splitlines()
            if next(s for s in status if s.startswith("NSpid:")).split()[-1] != "1":
                continue
            mounts = {s.split()[0] for s in (proc / "mountinfo").read_text().splitlines()}
            leaked = []
            for fd in (proc / "fd").iterdir():
                target = os.readlink(fd)
                info = (proc / "fdinfo" / fd.name).read_text().splitlines()
                mount = next(s for s in info if s.startswith("mnt_id:")).split()[-1]
                if int(fd.name) > 2 and target.startswith("/") and mount not in mounts:
                    leaked.append((fd.name, target))
            observed = len(leaked)
            print("PID1 outside-tree FDs:", leaked, flush=True)
    assert run.wait() == 0
    assert observed == int(expected), (observed, expected)
finally:
    if run.poll() is None:
        subprocess.run(["systemctl", "stop", unit], check=True)
        run.wait(timeout=10)
