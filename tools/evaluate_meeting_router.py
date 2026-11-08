"""Public-audio inference evidence. No test runner and no app or Hex settings writes.

Uses the 50 matched M0b clips per class and its existing Parakeet readings, just
as the meeting runner reuses Hex's reading. Audio and outputs stay outside git.
"""
import argparse
import json
import pathlib
import re
import subprocess
import sys
import time
import unicodedata
import wave

import psutil

parser = argparse.ArgumentParser()
parser.add_argument("--public-root", type=pathlib.Path, required=True)
parser.add_argument("--out", type=pathlib.Path, required=True)
parser.add_argument("--exe", type=pathlib.Path, required=True)
parser.add_argument("--runtime-dll", type=pathlib.Path, required=True)
parser.add_argument("--model-state-dir", type=pathlib.Path, required=True)
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(args.public_root / "packages"))
from indic_transliteration import sanscript  # noqa: E402
from indic_transliteration.sanscript import transliterate  # noqa: E402
from jiwer import process_words  # noqa: E402


def norm(text):
    # Same Latin ITRANS normalization as lid/score-routing.py.
    text = unicodedata.normalize("NFKC", text)
    text = re.sub(r"[\u0900-\u097f]+", lambda m: transliterate(
        m.group(), sanscript.DEVANAGARI, sanscript.ITRANS), text)
    text = "".join(str(unicodedata.digit(c)) if c.isdecimal() else c
                   for c in text).lower()
    return " ".join("".join(c if unicodedata.category(c)[0] in "LMN"
                           or c.isspace() else " " for c in text).split())


manifest = json.loads((args.public_root / "manifest.json").read_text(encoding="utf8"))
references = {}
durations = {}
for group in ("english", "hindi", "hinglish"):
    rows = sorted((r for r in manifest if r["set"] == group), key=lambda r: r["id"])[:50]
    baseline = {r["id"]: r["text"] for line in (
        args.public_root / "asr-outputs/parakeet" / (group + ".jsonl")
    ).read_text(encoding="utf8").splitlines() for r in [json.loads(line)]}
    if len(rows) != 50 or set(baseline) != {r["id"] for r in rows}:
        raise RuntimeError("The 50 matched public clip IDs must agree")
    chunks, count = [], 0
    with wave.open(str(args.out / (group + ".wav")), "wb") as output:
        output.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
        for row in rows:
            source_path = pathlib.Path(row["path"]).resolve()
            if not source_path.is_relative_to(args.public_root.resolve()):
                raise RuntimeError("Evaluation audio must stay inside the public corpus")
            with wave.open(str(source_path), "rb") as source:
                if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, 16000):
                    raise RuntimeError("Public input must be mono 16 kHz PCM16")
                start = count / 16000
                frames = source.getnframes()
                output.writeframes(source.readframes(frames))
                count += frames
            chunks.append(dict(id=row["id"], start=start, end=count / 16000,
                               track=group, english=baseline[row["id"]]))
            output.writeframes(b"\0\0" * 8000)
            count += 8000
    (args.out / (group + "-chunks.json")).write_text(
        json.dumps(chunks, ensure_ascii=False), encoding="utf8")
    references[group] = {r["id"]: r["reference"] for r in rows}
    durations[group] = sum(r["duration"] for r in rows)


def run(name, group, languages, mode="auto", reuse=True):
    destination = args.out / (name + ".json")
    command = [str(args.exe), "--transcribe-file", str(args.out / (group + ".wav")),
               "--languages", languages, "--language-mode", mode, "--out", str(destination),
               "--runtime-dll", str(args.runtime_dll), "--model-state-dir", str(args.model_state_dir),
               "--data-dir", str(args.out / "state")]
    if reuse:
        command += ["--chunks", str(args.out / (group + "-chunks.json"))]
    peak, cpu, build_load = 0, {}, False
    start = time.perf_counter()
    with (args.out / (name + ".stdout")).open("wb") as stdout, (args.out / (name + ".stderr")).open("wb") as stderr:
        child = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                 creationflags=0x08000000)
        while child.poll() is None:
            try:
                process = psutil.Process(child.pid)
                tree = [process] + process.children(recursive=True)
                peak = max(peak, sum(p.memory_info().rss for p in tree if p.is_running()))
                for p in tree:
                    t = p.cpu_times()
                    cpu[p.pid] = t.user + t.system
                build_load |= any(p.info["name"] and p.info["name"].lower() in
                                  ("cargo.exe", "rustc.exe") for p in psutil.process_iter(["name"]))
            except psutil.Error:
                pass
            time.sleep(0.1)
    if child.returncode:
        raise RuntimeError(name + " failed; inspect its isolated stderr file")
    # Re-fetch the actual CLI output and account for every source, not a sample.
    result = json.loads(destination.read_text(encoding="utf8"))
    errors = words = 0
    if reuse:
        ids = [o["chunk"]["id"] for s in result["segments"] for o in s["sources"]]
        if len(ids) != 50 or len(set(ids)) != 50 or set(ids) != set(references[group]):
            raise RuntimeError("CLI output must account for all 50 public clips exactly once")
        for segment in result["segments"]:
            ref = norm(" ".join(references[group][o["chunk"]["id"]] for o in segment["sources"]))
            score = process_words(ref, norm(segment["text_original"]))
            errors += score.substitutions + score.deletions + score.insertions
            words += score.hits + score.substitutions + score.deletions
    row = dict(name=name, group=group, clips=50, speech_seconds=durations[group],
               wall_seconds=time.perf_counter() - start, peak_mib=peak / 1048576,
               cpu_seconds=sum(cpu.values()), under_build_load=build_load,
               lid_load_seconds=result["lid_load_seconds"], hi_load_seconds=result["hi_load_seconds"],
               mixed_calls=result.get("mixed_calls", 0), mixed_seconds=result.get("mixed_seconds", 0),
               errors=errors, words=words, wer=100 * errors / words if words else None,
               routes={route: sum(len(s["sources"]) for s in result["segments"] if s["route"] == route)
                       for route in ("en", "hi", "mixed")})
    print(json.dumps(row), flush=True)
    return row


measurements = []
for group in ("english", "hindi", "hinglish"):
    measurements.append(run("routed-" + group, group, "en,hi,mixed" if group == "hinglish" else "en,hi"))
measurements.append(run("english-only", "english", "en"))
measurements.append(run("parakeet-baseline", "english", "en", reuse=False))
measurements.append(run("forced-mixed", "hinglish", "en,hi", "forced:mixed"))
measurements.append(run("uniform-hinglish", "hinglish", "en,hi"))
(args.out / "measurements.json").write_text(json.dumps(measurements, indent=2), encoding="utf8")
