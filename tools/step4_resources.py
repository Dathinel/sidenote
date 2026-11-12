"""Box-only process-tree measurements. No model, GPU or production corpus."""
import os
from pathlib import Path
import time


def tree(pid):
    found = {pid}
    pending = [pid]
    while pending:
        parent = pending.pop()
        for task in Path(f"/proc/{parent}/task").glob("*/children"):
            try:
                children = [int(p) for p in task.read_text().split()]
            except FileNotFoundError:
                continue
            for child in children:
                if child not in found:
                    found.add(child)
                    pending.append(child)
    return found


def sample(pid):
    rss = ticks = 0
    for child in tree(pid):
        proc = Path(f"/proc/{child}")
        try:
            fields = proc.joinpath("stat").read_text().rsplit(")", 1)[1].split()
            ticks += int(fields[11]) + int(fields[12])
            rss += int(proc.joinpath("statm").read_text().split()[1]) * os.sysconf("SC_PAGE_SIZE") // 1024
            maps = proc.joinpath("maps").read_text().lower()
            assert not any(name in maps for name in ("libx11", "libwayland", "libglx", "libegl", "libvulkan", "libnvidia", "onnxruntime")), "unexpected display/GPU/inference mapping"
            for descriptor in proc.joinpath("fd").iterdir():
                try:
                    target = os.readlink(descriptor)
                    assert not target.startswith(("/dev/dri", "/dev/nvidia")), "GPU device open"
                except FileNotFoundError:
                    pass
        except FileNotFoundError:
            continue
    return rss, ticks


def measure(pid):
    time.sleep(2)
    start = time.monotonic()
    first_rss, first_ticks = sample(pid)
    peak = first_rss
    for _ in range(10):
        time.sleep(1)
        rss, ticks = sample(pid)
        peak = max(peak, rss)
    elapsed = time.monotonic() - start
    cpu = (ticks - first_ticks) / os.sysconf("SC_CLK_TCK") / elapsed * 100
    return {"rss_kib": rss, "peak_idle_rss_kib": peak, "cpu_one_core_percent": round(cpu, 3),
            "window_seconds": round(elapsed, 2), "processes": len(tree(pid))}


def enforce(cold, after, restarted):
    if os.environ.get("STEP4_BASELINE") == "1":
        return
    for result in (cold, after, restarted):
        assert result["peak_idle_rss_kib"] <= 98304, "96 MiB process-tree RSS budget"
        assert result["cpu_one_core_percent"] <= 2.0, "2% of one core idle CPU budget"
    assert after["rss_kib"] - cold["rss_kib"] <= 16384, "16 MiB retained-growth budget"
