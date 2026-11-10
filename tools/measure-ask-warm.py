"""Round-one synthetic probe: two persistent sessions and three cold Haiku processes."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "artifacts/ask-models-round1"
QUESTION = "In two lines, what is a stand-up meeting? Answer directly. Do not use tools."
RESULTS = OUT / "latency.json"


class Claude:
    def __init__(self, model):
        self.start = time.perf_counter()
        flags = ["-p", "--input-format", "stream-json", "--output-format", "stream-json",
                 "--include-partial-messages", "--verbose", "--effort", "low",
                 "--tools", "", "--strict-mcp-config", "--no-session-persistence"]
        if model:
            flags += ["--model", model]
        self.p = subprocess.Popen([str(Path.home() / ".local/bin/claude.exe"), *flags],
                                  cwd=OUT / "synthetic", stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  creationflags=subprocess.CREATE_NO_WINDOW)
        self.q = queue.Queue()
        def read():
            for line in self.p.stdout:
                try:
                    self.q.put(json.loads(line))
                except ValueError:
                    pass
            self.q.put({"type": "closed"})
        threading.Thread(target=read, daemon=True).start()
        threading.Thread(target=lambda: self.p.stderr.read(), daemon=True).start()
        self.send({"type": "control_request", "request_id": "init", "request": {"subtype": "initialize"}})
        while True:
            event = self.q.get(timeout=40)
            if event.get("type") == "closed":
                raise RuntimeError("Claude closed before initialization")
            if event.get("type") == "control_response":
                response = event.get("response", {})
                if response.get("request_id") == "init":
                    models = response.get("response", {}).get("models", [])
                    (OUT / "claude-models.json").write_text(json.dumps(models, indent=2), encoding="utf-8")
                    self.ready_s = time.perf_counter() - self.start
                    break

    def send(self, event):
        self.p.stdin.write((json.dumps(event) + "\n").encode())
        self.p.stdin.flush()

    def turn(self, cold=False):
        start = self.start if cold else time.perf_counter()
        first = None
        answer = ""
        model = None
        self.send({"type": "user", "message": {"role": "user", "content": QUESTION}})
        while True:
            event = self.q.get(timeout=150)
            if event.get("type") == "closed":
                return {"first_text_s": first, "result_s": time.perf_counter()-start,
                        "answer": answer, "error": "CLI closed", "model": model}
            model = event.get("model", model)
            delta = event.get("event", {}).get("delta", {}).get("text", "")
            if delta:
                if first is None:
                    first = time.perf_counter() - start
                answer += delta
            if event.get("type") == "result":
                answer = event.get("result", answer)
                if answer and first is None:
                    first = time.perf_counter() - start
                return {"first_text_s": first, "result_s": time.perf_counter()-start,
                        "answer": answer, "error": "Provider rejected request" if event.get("is_error") else None,
                        "model": model}

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("case", choices=["warm-default", "cold-haiku", "warm-haiku"])
    args = parser.parse_args()
    (OUT / "synthetic").mkdir(parents=True, exist_ok=True)
    results = json.loads(RESULTS.read_text()) if RESULTS.exists() else []
    model = None if args.case == "warm-default" else "claude-haiku-5-5"
    session = None
    try:
        if args.case.startswith("warm"):
            session = Claude(model)
            priming = session.turn()
            (OUT / f"{args.case}-priming.json").write_text(json.dumps(priming, indent=2), encoding="utf-8")
        for run in range(1, 4):
            if session is None:
                session = Claude(model)
            row = session.turn(cold=args.case.startswith("cold"))
            row.update(case=args.case, run=run, ready_s=session.ready_s)
            row["total_s"] = row["result_s"]
            if args.case.startswith("cold"):
                session.close()
                row["total_s"] = time.perf_counter() - session.start
                session = None
            results.append(row)
            RESULTS.write_text(json.dumps(results, indent=2), encoding="utf-8")
            print(json.dumps({k:v for k,v in row.items() if k != "answer"}), flush=True)
    finally:
        if session is not None:
            session.close()


if __name__ == "__main__":
    main()
