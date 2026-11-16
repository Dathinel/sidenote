"""Fixed idle UI interval; count every descendant, including exited workers."""
import json
import pathlib
import re
import subprocess
import sys
import time
from idle_cpu_job import CpuJob


exe = pathlib.Path(sys.argv[1]).resolve(strict=True)
if b"idle CPU proof: " not in exe.read_bytes():
    raise RuntimeError("Refusing executable without opt-in headless entry")
job = CpuJob()
try:
    proc = job.start([str(exe), "--idle-cpu-proof", "--measure", "--retained-done"],
                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    before = idle = None
    for line in proc.stdout:
        print(line, end="", flush=True)
        if "IDLE_CPU_READY" in line:
            before = job.sample()
            started = time.monotonic()
        if line.startswith("30 s deadline:"):
            match = re.search(r"seconds=([\d.]+), cpu_s=([\d.]+), one_core_percent=[\d.]+, system_load_percent=([\d.]+)", line)
            assert match and before is not None, "missing idle interval counters"
            idle = tuple(map(float, match.groups()))
            idle_tree = job.sample()
            observed = time.monotonic() - started
    proc.wait()
    assert idle is not None, "headless UI never measured idle"
    seconds, root_cpu, load = idle
    processes = job.sample()[1]
    # One lifetime process proves that the production counter covers all CPU in the interval.
    child_cpu = 0.0 if processes == 1 else max(0, idle_tree[0] - before[0] - root_cpu)
    cpu = root_cpu + child_cpu
    print(json.dumps(dict(configuration="release headless UI, 300 production Done jobs, fixed idle interval; teardown excluded",
                          seconds=seconds, processor_seconds=cpu, one_core_percent=100 * cpu / seconds,
                          root_cpu_seconds=root_cpu, child_cpu_seconds=child_cpu,
                          system_load_percent=load, total_processes=processes,
                          new_child_processes=processes-before[1])))
    failures = [message for passed, message in (
        (observed >= 29, "idle readiness output arrived after the sample"),
        (processes == 1, "headless UI launched child processes"),
        (proc.returncode == 0, "headless UI proof failed"),
        (cpu / seconds < 0.01, "UI and descendants exceeded 1% of one core"),
    ) if not passed]
    assert not failures, "; ".join(failures)
finally:
    job.close()
