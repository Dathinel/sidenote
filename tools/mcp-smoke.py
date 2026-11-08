"""The one allowed protocol smoke check. Uses only temporary fixture folders."""
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading


def main():
    root = Path(__file__).resolve().parents[1]
    binary = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else (
        root / "target/x86_64-pc-windows-gnu/debug/sidenote.exe"
    )
    transcript = []
    with tempfile.TemporaryDirectory(prefix="mcp-smoke-", dir=root / "target") as folder:
        fixture = Path(folder)
        state, meetings, peers = (fixture / name for name in ("state", "meetings", "peers"))
        state.mkdir()

        def write(path, value):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")

        def meeting(parent, ident, title, text, timestamp):
            target = parent / ident
            write(target / "english-summary.json", {
                "title": title, "people": ["You", "Taylor"], "context": ["Release review"],
                "topics": [{"title": "Decision", "points": [{
                    "type": "decided", "text": "Proceed after sign-off", "at": ["00:00:01"]
                }]}], "todo": [{"task": "Check the release gate", "owner": "You"}]
            })
            write(target / "manifest.json", {"created_at_ms": timestamp, "duration_ms": 60000})
            write(target / "tasks.json", {"meeting_project": "Legacy launch"})
            (target / "english-transcript.md").write_text(
                "[00:00:01] Taylor (English): " + text, encoding="utf-8"
            )

        meeting(meetings, "100-1", "Launch planning", "Review the scope.", 1794009600000)
        meeting(peers / "desktop", "100-2", "Release review", "The launch gate is signed off.", 1794096000000)
        meeting(peers / "desktop", "100-1", "Duplicate peer", "launch duplicate", 1794182400000)
        write(state / "projects.json", {
            "projects": ["Launch"], "aliases": {"Legacy launch": "Launch"}
        })
        write(state / "100-1.json", {"checked": ["Check the release gate"], "messages": []})

        def snapshot():
            return {str(p.relative_to(fixture)): (
                p.stat().st_mtime_ns, hashlib.sha256(p.read_bytes()).hexdigest()
            ) for p in fixture.rglob("*") if p.is_file()}

        before = snapshot()
        environment = os.environ.copy()
        for name, value in {
            "APPDATA": fixture / "appdata", "LOCALAPPDATA": fixture / "localappdata",
            "SIDENOTE_DATA_DIR": state, "SIDENOTE_STATE_DIR": state,
            "SIDENOTE_HEX_DIR": meetings, "SIDENOTE_PEERS_DIR": peers,
            "COMPUTERNAME": "fixture-laptop",
        }.items():
            environment[name] = str(value)
        process = subprocess.Popen(
            [str(binary), "--mcp", "--data-dir", str(state), "--meetings-dir", str(meetings),
             "--peers-dir", str(peers)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, text=True, encoding="utf-8", env=environment,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        replies = queue.Queue()
        diagnostics = []

        def stdout_reader():
            for line in process.stdout:
                replies.put(line)
            replies.put(None)

        def stderr_reader():
            diagnostics.append(process.stderr.read())

        out_thread = threading.Thread(target=stdout_reader)
        err_thread = threading.Thread(target=stderr_reader)
        out_thread.start()
        err_thread.start()

        def send(message, reply=True):
            transcript.append({"sent": message})
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            if not reply:
                return None
            line = replies.get(timeout=30)
            assert line is not None, "Server closed stdout before replying"
            response = json.loads(line)
            transcript.append({"received": response})
            assert response.get("id") == message["id"], "Unexpected response id"
            assert "error" not in response, "Protocol request failed"
            return response["result"]

        try:
            initialized = send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "meetings-fixture-smoke", "version": "1"}
            }})
            assert initialized["protocolVersion"] == "2025-11-25"
            assert "instructions" in initialized
            send({"jsonrpc": "2.0", "method": "notifications/initialized"}, reply=False)
            listed = send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            expected = {"search_meetings", "list_meetings", "get_meeting", "list_tasks",
                        "get_memory", "list_projects", "get_daily_check", "propose_memory"}
            assert {t["name"] for t in listed["tools"]} == expected
            for tool in listed["tools"]:
                assert tool["title"] and tool["inputSchema"] and tool["outputSchema"]
                annotations = tool["annotations"]
                assert annotations["readOnlyHint"] == (tool["name"] != "propose_memory")
                assert annotations["idempotentHint"] is True
                assert annotations["destructiveHint"] is False
                assert annotations["openWorldHint"] is False
            searched = send({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
                "name": "search_meetings", "arguments": {"query": "launch", "project": "Launch", "limit": 50}
            }})
            assert searched.get("isError") is False
            result = searched["structuredContent"]
            items = result["data"]["items"]
            assert [m["id"] for m in items] == ["100-2", "100-1"]
            assert [m["device"] for m in items] == ["desktop", "fixture-laptop"]
            assert items[0]["snippet"] == "The launch gate is signed off."
            assert all(m["project"] == "Launch" and m["date"] for m in items)
            assert any(c["type"] == "resource_link" for c in searched["content"])
            assert result["warnings"] == [] and result["truncated"] is False
            assert result["data"]["hasMore"] is False
            assert before == snapshot(), "A read changed fixture files or created state"
            process.stdin.close()
            assert process.wait(timeout=30) == 0
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
            out_thread.join(timeout=10)
            err_thread.join(timeout=10)
        assert not any(diagnostics), "Unexpected stderr diagnostics"
        assert replies.get(timeout=5) is None, "Unexpected non-protocol stdout"
        artifact = root / "artifacts/mcp-smoke.json"
        artifact.parent.mkdir(exist_ok=True)
        artifact.write_text(json.dumps(transcript, indent=2, ensure_ascii=False), encoding="utf-8")
        print("initialize -> " + json.dumps(initialized, ensure_ascii=False))
        print("tools/list -> " + json.dumps({"tools": sorted(expected), "schemas_and_annotations": "8 checked"}))
        print("search_meetings -> " + json.dumps(result, ensure_ascii=False))
        print("exit=0; stderr empty; stdout JSON-RPC only; fixture files unchanged")
        print("Full transcript: " + str(artifact))


if __name__ == "__main__":
    main()
