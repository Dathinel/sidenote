#!/usr/bin/env python3
# Maintainer tool: used for releases and proofs, not needed to contribute.
"""Run only on an isolated Linux box. Synthetic data and disposable tokens only."""
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

from prove_linux_step3 import Pipe, environment, private_json, seed, wait

CANARY = "step4-synthetic-content-canary"


def http(port, token, method="tools/list", params=None, headers=None, verb="POST"):
    connection = HTTPConnection("127.0.0.1", port, timeout=10)
    fields = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
              "MCP-Protocol-Version": "2025-03-26"}
    if token is not None:
        fields["Authorization"] = "Bearer " + token
    fields.update(headers or {})
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}})
    connection.request(verb, "/mcp", body, fields)
    response = connection.getresponse()
    content = response.read()
    status = response.status
    connection.close()
    return status, content


def rpc(pipe, method, params=None, ident=1):
    pipe.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": ident,
                                       "method": method, "params": params or {}}) + "\n")
    pipe.process.stdin.flush()
    result = json.loads(pipe.lines.get(timeout=15))
    assert result.get("id") == ident, "MCP response correlation"
    assert "error" not in result, "MCP request failed"
    return result["result"]


def initialized(pipe):
    result = rpc(pipe, "initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                     "clientInfo": {"name": "step4-synthetic", "version": "1"}})
    pipe.process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
    pipe.process.stdin.flush()
    return result


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=12)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            raise AssertionError("service shutdown deadline")


def qualify(binary, role):
    with tempfile.TemporaryDirectory(prefix="sidenote-step4-", dir=os.environ.get("SIDENOTE_PROOF_TMP") or None) as temp:
        base = Path(temp)
        root = base / "state"
        seed(root, "synthetic", role)
        env = environment(base, os.environ["CREDENTIALS_DIRECTORY"])
        paths = ["--data-dir", str(root), "--meetings-dir", str(base / "meetings"),
                 "--peers-dir", str(base / "peers")]
        private_json(root / "config.json", {"notes": {"engine": "off"}, "features": {"memory": True}})
        private_json(base / "meetings/1700000000000-1/summary.json",
                     {"title": CANARY, "summary": CANARY})
        private_json(base / "meetings/1700000000000-1/manifest.json", {"status": "complete"})
        log = base / "service.log"
        def start():
            with log.open("ab") as output:
                return subprocess.Popen([binary, *paths, "--server", "--mcp-port", "0"],
                                        env=env, stdout=output, stderr=output, start_new_session=True)
        process = start()
        pipe = None
        try:
            endpoint = root / "local/mcp/endpoint.json"
            wait(lambda: endpoint.exists() and root.joinpath("local/sync/control.sock").exists())
            port = int(json.loads(endpoint.read_text())["address"].split(":")[1])
            listeners = [line.split()[1] for line in Path("/proc/net/tcp").read_text().splitlines()[1:]
                         if line.split()[1].endswith(f":{port:04X}") and line.split()[3] == "0A"]
            assert listeners == [f"0100007F:{port:04X}"], "numeric loopback listener only"
            def cli(*args, allowed=(0,)):
                result = subprocess.run([binary, *paths, *args], env=env, capture_output=True, timeout=12)
                assert result.returncode in allowed, "operator command failed"
                return result.stdout
            def status():
                return json.loads(cli("--ops", "status", allowed=(0, 1)))
            wait(lambda: status()["ready"])
            pipe = Pipe([binary, *paths, "--mcp"], env)
            initialized(pipe)
            local = rpc(pipe, "tools/list")["tools"]
            assert any(t["name"] == "list_meetings" for t in local)
            result = rpc(pipe, "tools/call", {"name": "list_meetings", "arguments": {}})
            assert not result.get("isError", False), "local stdio tool compatibility"
            items = result["structuredContent"]["data"]["items"]
            assert len(items) == 1, "synthetic meeting actually returned"
            uri = items[0]["uri"]
            assert rpc(pipe, "resources/read", {"uri": uri})["contents"], "stdio resource compatibility"
            print("PASS local stdio initialize, catalog and tool through owner")
            token_path = base / "client-token"
            cli("--mcp-access", "grant", "phone", str(token_path))
            assert token_path.stat().st_mode & 0o777 == 0o600
            token = token_path.read_text()
            assert token not in root.joinpath("local/mcp/clients.json").read_text()
            assert http(port, None)[0] == 401
            assert http(port, "phone." + "0" * 64)[0] == 401
            assert http(port, token, headers={"Origin": "https://evil.invalid"})[0] == 403
            assert http(port, token, headers={"Host": "evil.invalid"})[0] == 403
            code, body = http(port, token, "initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                                        "clientInfo": {"name": "synthetic", "version": "1"}})
            assert code == 200 and "result" in json.loads(body), "HTTP initialization"
            code, body = http(port, token)
            assert code == 200, "HTTP catalog status"
            tools = json.loads(body)["result"]["tools"]
            assert all(t["name"] != "propose_memory" for t in tools), "network scope catalog"
            code, body = http(port, token, "tools/call", {"name": "propose_memory",
                               "arguments": {"text": CANARY, "reason": CANARY}})
            denied = json.loads(body)
            assert code == 200 and ("error" in denied or denied.get("result", {}).get("isError")), "write denial"
            assert CANARY.encode() not in body, "redacted tool denial"
            assert http(port, token, headers={"MCP-Protocol-Version": "invalid"})[0] == 400
            assert http(port, token, verb="GET")[0] == 405
            assert http(port, token, "tools/call", {"name":"list_meetings", "arguments":{"query":CANARY * 3000}})[0] == 413
            code, body = http(port, token, "resources/read", {"uri":uri})
            assert code == 200 and json.loads(body)["result"]["contents"], "HTTP resource compatibility"
            print("PASS HTTP initialization/catalog, missing/wrong-token denial, host/origin/version and write denial")
            cli("--mcp-access", "revoke", "phone")
            assert http(port, token)[0] == 401, "revocation after initialized client"
            replacement = base / "rotated-token"
            cli("--mcp-access", "grant", "phone", str(replacement))
            assert http(port, token)[0] == 401, "old token after regrant"
            token = replacement.read_text()
            assert http(port, token)[0] == 200
            print("PASS per-client revocation and rotation; no reusable authenticated HTTP session")
            notes = root / "local/notes-jobs"
            private_json(notes / "synthetic.json", {"state": "backend_error", "message": CANARY})
            health = status()
            assert not health["ready"] and health["queues"]["notes"]["failed"] == 1
            notes.joinpath("synthetic.json").write_text("{")
            assert status()["queues"]["notes"]["unreadable"] == 1
            notes.joinpath("synthetic.json").unlink()
            exported = cli("--ops", "status", allowed=(0, 1)) + cli("--ops", "logs")
            assert CANARY.encode() not in exported and token.encode() not in exported
            assert CANARY not in log.read_text() and token not in log.read_text()
            print("PASS live queue failure/corruption, freshness and redacted status/logs")
            pipe.close()
            pipe = None
            from step4_resources import measure
            baseline = measure(process.pid)
            for _ in range(60):
                code, body = http(port, token, "tools/call", {"name": "list_meetings", "arguments": {}})
                assert code == 200 and not json.loads(body).get("result", {}).get("isError"), "repeated HTTP work"
            after = measure(process.pid)
            stop(process)
            assert process.returncode == 0, "clean server stop"
            assert not endpoint.exists() and not root.joinpath("local/sync/control.sock").exists()
            failed = subprocess.run([binary, *paths, "--ops", "status"], env=env, capture_output=True, timeout=10)
            assert failed.returncode == 2 and failed.stderr == b"OPS_OWNER_UNAVAILABLE\n"
            unavailable = Pipe([binary, *paths, "--mcp"], env)
            try:
                initialized(unavailable)
                unavailable.process.stdin.write('{"jsonrpc":"2.0","id":2,"method":"tools/list"}\n')
                unavailable.process.stdin.flush()
                error = json.loads(unavailable.lines.get(timeout=10))
                assert error["error"]["message"] == "MCP_OWNER_UNAVAILABLE"
            finally:
                unavailable.close()
            process = start()
            wait(lambda: endpoint.exists() and root.joinpath("local/sync/control.sock").exists())
            new_port = int(json.loads(endpoint.read_text())["address"].split(":")[1])
            assert http(new_port, token)[0] == 200, "token survives service restart"
            restarted = measure(process.pid)
            from step4_resources import enforce
            enforce(baseline, after, restarted)
            print(json.dumps({"role": role, "cold": baseline, "after_work": after, "restart": restarted}))
            print("PASS owner outage/restart, token persistence and clean endpoint shutdown; " +
                  ("resource baseline only" if os.environ.get("STEP4_BASELINE") == "1" else "resource budget enforced"))
        finally:
            if pipe:
                pipe.close()
            stop(process)


if __name__ == "__main__":
    os.umask(0o077)
    qualify(str(Path(sys.argv[1]).resolve()), sys.argv[2] if len(sys.argv) > 2 else "hub")
