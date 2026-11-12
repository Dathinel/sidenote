"""Owner-approved benchmark only. No tests. Public FLEURS windows from the runtime proof.

No downloads, app configuration, listener, Hex calls or private audio. Output must be new.
"""
import argparse
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time
import wave


def working_set(pid):
    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                "peak", "working", "peak_paged", "paged", "peak_nonpaged",
                "nonpaged", "pagefile", "peak_pagefile")]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    handle = kernel.OpenProcess(0x410, False, pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            raise ctypes.WinError(ctypes.get_last_error())
        return counters.working
    finally:
        kernel.CloseHandle(handle)


def read_clip(path, offset):
    with wave.open(str(path), "rb") as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getframerate(), wav.getnframes()) != (1, 2, 16000, 4800000):
            raise ValueError("Expected the proof's 300-second mono 16 kHz PCM16 windows")
        wav.setpos(offset * 16000)
        pcm = wav.readframes(5 * 16000)
        if len(pcm) != 160000:
            raise ValueError("Truncated public window")
        return pcm


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("exe", "data", "model", "dll", "output"):
        parser.add_argument("--" + flag, type=Path, required=True)
    parser.add_argument("--approved-run", action="store_true")
    args = parser.parse_args()
    if not args.approved_run or sys.platform != "win32":
        parser.error("Requires an owner-approved Windows machine with the pinned ONNX DLL; never run on the laptop")
    args.output.mkdir(parents=True, exist_ok=False)
    root = args.output.resolve()
    sources = json.loads((args.data / "sources.json").read_text(encoding="utf-8-sig"))
    if any(len(sources[lang]) != 120 or any(not s.startswith("dev/") or not s.endswith(".wav") for s in sources[lang]) for lang in ("en", "hi", "te")):
        raise ValueError("Use the public proof's sources.json with 120 FLEURS dev members per language")
    jobs = []
    for lang in ("en", "hi", "te", "mixed"):
        for index in range(10):
            name = f"{lang}-{index:02}"
            original = (args.data / "windows" / (name + ".wav")).resolve()
            chunks = []
            if lang == "mixed":
                # Artificial within-chunk Hindi/English switches, not natural code-switching.
                pcm = b"".join(read_clip(original, i * 25 + 10) for i in range(12))
                path = root / (name + ".wav")
                with wave.open(str(path), "wb") as wav:
                    wav.setnchannels(1)
                    wav.setsampwidth(2)
                    wav.setframerate(16000)
                    wav.writeframes(pcm)
                spans = [(i * 10, i * 10 + 10) for i in range(6)]
            else:
                read_clip(original, 10)
                path = original
                spans = [(i * 25 + 10, i * 25 + 15) for i in range(12)]
            for i, (start, end) in enumerate(spans):
                chunks.append({"id": str(i), "start": start, "end": end, "track": "public", "speaker": "fleurs", "turn": str(i)})
            jobs.append({"id": name, "wav": str(path), "chunks": chunks})
    request = root / "request.json"
    config = root / "config.json"
    request.write_text(json.dumps({"meeting_languages": ["en", "hi", "te"], "jobs": jobs}), encoding="utf-8")
    config.write_text(json.dumps({"labs": {"language_detection": True,
        "language_detection_settings": {"model": str(args.model.resolve()),
            "runtime_dll": str(args.dll.resolve())}}, "pipeline": {"language_mode": "auto"}}), encoding="utf-8")
    command = [str(args.exe.resolve()), "--language-detect", str(request), "--lid-config", str(config),
               "--lid-output", str(root / "sidecars"), "--lid-benchmark-handshake"]
    started = time.monotonic()
    with (root / "stderr.txt").open("w", encoding="utf-8") as errors:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True)
        events = queue.Queue()
        threading.Thread(target=lambda: events.put(process.stdout.readline()), daemon=True).start()
        try:
            event = events.get(timeout=120)
            if not event or json.loads(event).get("event") != "model_loaded":
                raise RuntimeError("ONNX unavailable or load failed; inspect sidecars and stderr; no benchmark result")
            baseline = peak = working_set(process.pid)
            inference_started = time.monotonic()
            process.stdin.write("continue\n")
            process.stdin.flush()
            while process.poll() is None:
                try:
                    peak = max(peak, working_set(process.pid))
                except OSError:
                    if process.poll() is None:
                        raise
                if time.monotonic() - started > 3600:
                    raise TimeoutError("Benchmark exceeded one hour")
                time.sleep(0.025)
            if process.returncode:
                raise RuntimeError("Stage failed; inspect stderr.txt")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdin.close()
            process.stdout.close()
    finished = time.monotonic()
    counts = {lang: {"correct": 0, "total": 0, "uncertain": 0, "outside_meeting_languages": 0} for lang in ("en", "hi", "te")}
    mixed = {"both_observed": 0, "extra_language": 0, "total": 0}
    for job in jobs:
        result = json.loads((root / "sidecars" / job["id"] / "language-detect.v1.json").read_text(encoding="utf-8"))
        lang = job["id"].split("-")[0]
        for guess in result["chunks"]:
            if guess["confidence"] is None:
                raise RuntimeError("Unchecked chunk: benchmark is incomplete")
            if lang == "mixed":
                observed = set(guess["observed_languages"])
                mixed["both_observed"] += {"en", "hi"} <= observed
                mixed["extra_language"] += bool(observed - {"en", "hi"})
                mixed["total"] += 1
            else:
                counts[lang]["correct"] += guess["language"] == lang
                counts[lang]["total"] += 1
                counts[lang]["uncertain"] += guess["status"] == "uncertain"
                counts[lang]["outside_meeting_languages"] += guess["outside_meeting_languages"]
    for values in counts.values():
        values["accuracy"] = values["correct"] / values["total"]
    report = {"per_language": counts, "artificial_mixed_chunks": mixed,
              "working_set_post_load_bytes": baseline, "sampled_peak_after_load_bytes": peak,
              "incremental_peak_bytes": peak - baseline, "under_100_mb_extra": peak - baseline < 100_000_000,
              "memory_sample_interval_ms": 25, "wall_seconds": finished - started,
              "startup_to_post_load_seconds": inference_started - started,
              "post_load_stage_seconds": finished - inference_started,
              "wall_seconds_per_audio_minute": (finished - inference_started) / 40,
              "audio_seconds": 2400, "thresholds_calibrated": False,
              "accuracy_semantics": "Top declared-language label, with uncertainty/out-of-set counts separate; not routing qualification",
              "limitations": "Same public pilot, no held-out/natural mixed speech; sampled working set can miss peaks"}
    (root / "benchmark.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
