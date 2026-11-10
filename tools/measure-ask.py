"""Synthetic Ask latency probe. 18 timed invocations; help checks run separately."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "artifacts" / "ask-models"
QUESTION = "In two lines, what is a stand-up meeting? Answer directly. Do not use tools."


def answer_text(agent, event):
    if agent == "claude":
        return event.get("event", {}).get("delta", {}).get("text", "") or (
            event.get("result", "") if event.get("type") == "result" else ""
        )
    item = event.get("item", {})
    return item.get("text", "") if item.get("type") == "agent_message" else ""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--last", type=int, default=18)
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    cwd = OUT / "synthetic"
    cwd.mkdir(exist_ok=True)
    home = Path.home()
    programs = {
        "claude": home / ".local/bin/claude.exe",
        "codex": home / "AppData/Roaming/npm/node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe",
    }
    results = []
    for run in range(1, 4):
        for effort in ["default", "low", "medium"]:
            for agent in ["claude", "codex"]:
                if len(results) >= args.last:
                    break
                flags = (
                    ["-p", "--tools", "", "--strict-mcp-config", "--no-session-persistence", "--output-format", "stream-json", "--include-partial-messages", "--verbose"]
                    if agent == "claude" else
                    ["exec", "--sandbox", "read-only", "--ephemeral", "--skip-git-repo-check", "--json"]
                )
                if effort != "default":
                    flags += ["--effort", effort] if agent == "claude" else ["-c", f"model_reasoning_effort={effort}"]
                if agent == "codex":
                    flags += ["-"]
                started = time.perf_counter()
                process = subprocess.Popen(
                    [str(programs[agent]), *flags], cwd=cwd, stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    creationflags=subprocess.CREATE_NO_WINDOW,
                )
                first_output = None
                first_text = None
                text = ""
                model = None
                events = []
                def read():
                    nonlocal first_output, first_text, text, model
                    for line in process.stdout:
                        at = time.perf_counter() - started
                        if first_output is None:
                            first_output = at
                        try:
                            event = json.loads(line)
                        except ValueError:
                            continue
                        model = event.get("model", model)
                        chunk = answer_text(agent, event)
                        if chunk and first_text is None:
                            first_text = at
                        if chunk:
                            text = chunk if event.get("type") == "result" else text + chunk
                        # Store only event kinds and answer text, never tool payloads or metadata.
                        events.append({"at_s": round(at, 3), "type": event.get("type"), "text": chunk})
                reader = threading.Thread(target=read)
                reader.start()
                process.stdin.write(QUESTION.encode())
                process.stdin.close()
                errors = threading.Thread(target=lambda: process.stderr.read())
                errors.start()
                try:
                    process.wait(timeout=150)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                reader.join()
                errors.join()
                result = {
                    "agent": agent, "effort": effort, "run": run,
                    "first_output_s": first_output, "first_text_s": first_text,
                    "total_s": time.perf_counter() - started, "exit_code": process.returncode,
                    "model": model, "answer": text,
                    "error": "CLI failed (raw stderr omitted)" if process.returncode else None,
                    "events": events,
                }
                results.append(result)
                (OUT / "latency.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
                print(json.dumps({k: v for k, v in result.items() if k not in ["events", "answer"]}), flush=True)


if __name__ == "__main__":
    main()
