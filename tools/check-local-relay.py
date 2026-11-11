"""Exercise the self-host guide on numeric loopback without an account or deployment."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request


def main():
    root = Path(__file__).resolve().parents[1]
    relay = root / "relay"
    output = root / "artifacts" / "relay4-local"
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(CI="true", BROWSER="none", WRANGLER_SEND_METRICS="false", WRANGLER_CHECK_UPDATE="false")
    hidden = {"creationflags": 0x08000000} if os.name == "nt" else {}
    commands = [
        ["node", "--version"],
        ["bun", "--version"],
        ["bun", "install", "--frozen-lockfile"],
        ["bun", "run", "typecheck"],
        ["bun", "test", "tests"],
        ["node", "node_modules/wrangler/bin/wrangler.js", "--version"],
    ]
    results = []
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def record(command, code, log):
        results.append({"command": subprocess.list2cmdline(command), "exit": code, "log": str(log)})
        (output / "commands.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        print(f"{'PASS' if code == 0 else 'FAIL'}: {subprocess.list2cmdline(command)}", flush=True)
        if code:
            raise RuntimeError("Local guide command failed; see captured output")

    for index, command in enumerate(commands, 1):
        log = output / f"command-{index}.txt"
        with log.open("w", encoding="utf-8") as stream:
            result = subprocess.run(command, cwd=relay, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=180, **hidden)
        record(command, result.returncode, log)

    command = ["node", "node_modules/wrangler/bin/wrangler.js", "dev", "--local", "--ip", "127.0.0.1", "--port", "8787", "--inspector-ip", "127.0.0.1", "--inspector-port", "0", "--show-interactive-dev-session", "false", "--log-level", "error", "--persist-to", "../artifacts/relay4-local/state"]
    log = output / "wrangler-dev.txt"
    # Refuse to contact an unrelated service already using the documented port.
    import socket
    with socket.socket() as available:
        available.bind(("127.0.0.1", 8787))
    with log.open("w", encoding="utf-8") as stream:
        process = subprocess.Popen(command, cwd=relay, env=env, stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT, **hidden)
        try:
            deadline = time.monotonic() + 45
            while True:
                if process.poll() is not None:
                    raise RuntimeError("Local Worker exited before health check")
                try:
                    with opener.open("http://127.0.0.1:8787/health", timeout=1) as response:
                        if response.read(65) == b"sidenote-relay-v1":
                            break
                except (urllib.error.URLError, TimeoutError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Local Worker did not become healthy")
                time.sleep(0.1)
            record(command, 0, log)
            for index, args in enumerate(([], ["--head"]), 7):
                curl = ["curl.exe", "--fail", "--silent", "--show-error", "--noproxy", "127.0.0.1", *args, "http://127.0.0.1:8787/health"]
                captured = output / f"command-{index}.txt"
                with captured.open("w", encoding="utf-8") as target:
                    result = subprocess.run(curl, cwd=relay, env=env, stdout=target, stderr=subprocess.STDOUT, timeout=10, **hidden)
                record(curl, result.returncode, captured)
            assert (output / "command-7.txt").read_text().strip() == "sidenote-relay-v1"
            assert "no-store" in (output / "command-8.txt").read_text().lower()
        finally:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, **hidden)
            else:
                process.terminate()
            process.wait(timeout=15)
    print("PASS: documented local commands; no login or deployment", flush=True)


if __name__ == "__main__":
    main()
