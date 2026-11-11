"""Protocol smoke check using only temporary fixture folders."""
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
                "schema": 2, "title": title, "people": ["You", "Taylor"], "context": ["Release review"],
                "topics": [{"title": "Decision", "points": [{
                    "type": "decided", "text": "Proceed after sign-off", "at": ["00:00:01"]
                }]}], "todo": [
                    {"id":"gate", "task":"Check the release gate", "title":"Check release", "description":"Get approval", "owner":"You", "status":"review", "subtasks":[{"id":"proof", "title":"Read proof", "done":False}]},
                    {"id":"ship", "title":"Ship the build", "status":"waiting", "waiting_on":{"task_id":"gate"}},
                    {"id":"wait", "title":"Wait for reply", "status":"waiting", "waiting_on":{"text":"Legal"}},
                    {"id":"review", "title":"Review draft", "status":"review"},
                    {"id":"review2", "title":"Review draft", "status":"review"}
                ]
            })
            write(target / "manifest.json", {"created_at_ms": timestamp, "duration_ms": 60000})
            write(target / "tasks.json", {"meeting_project": "Legacy launch"})
            (target / "english-transcript.md").write_text(
                "[00:00:01] Taylor (English): " + text, encoding="utf-8"
            )

        meeting(meetings, "100-0", "Cleared meeting", "Old release.", 1600000000000)
        meeting(meetings, "100-1", "Launch planning", "Review the scope.", 1794009600000)
        meeting(peers / "desktop", "100-2", "Release review", "The launch gate is signed off.", 1794096000000)
        meeting(peers / "desktop", "100-1", "Duplicate peer", "launch duplicate", 1794182400000)
        for ident, duration, words in [("100-3", 29999, 25), ("100-4", 96000, 0)]:
            meeting(meetings, ident, "Abandoned launch recording", "Hidden synthetic source", 1794182400000)
            write(meetings / ident / "manifest.json", {"status":"complete", "created_at_ms":1794182400000, "duration_ms":duration})
            write(meetings / ident / "transcript.ndjson", {"text":" ".join(["synthetic"] * words)})
        write(state / "projects.json", {
            "projects": ["Launch"], "aliases": {"Legacy launch": "Launch"}
        })
        write(state / "100-1.json", {"checked": ["Check the release gate"], "messages": []})

        (meetings / "100-1/notes.md").write_text("Typed-only nebula caf\u00e9 notes", encoding="utf-8")
        write(state / "config.json", {"version":1,"features":{"tasks":True,"memory":True,"daily_check":True,"projects":True,"other_devices":True},"notes":{}})
        write(state / "teams.json", {"teams":[
            {"name":"Delivery", "members":["Taylor Smith"], "aliases":{"You":"Taylor Smith"}},
            {"name":"Legal", "members":["Lee"], "aliases":{}}
        ]})
        write(state / "tasks-settings.json", {"cleared_before":"2026-01-01"})
        write(state / "task-overrides.json", {
            "tasks":{"100-1":{"id:gate":{"task":"Check the release gate","subtask_ticks":{"sub:4:gate:5:proof":True}}}},
            "manual":[{"id":"hand","task":"Manual follow-up","owner":"You","added":"2026-11-10","status":"review"}]
        })

        # Config loading keeps this lock file even when it only reads existing settings.
        (state / "local").mkdir()
        (state / "local/config.lock").touch()

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
                        "get_memory", "list_projects", "list_teams", "get_daily_check", "propose_memory"}
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
            after = snapshot()
            changed = {name for name in before.keys() | after.keys() if before.get(name) != after.get(name)}
            assert before == after, "A read changed fixture files or created state: " + ", ".join(sorted(changed))
            next_id = 4
            def call(name, arguments):
                nonlocal next_id
                reply = send({"jsonrpc":"2.0","id":next_id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
                next_id += 1
                assert reply.get("isError") is False, reply
                return reply["structuredContent"]["data"]

            tasks = call("list_tasks", {"status":"all","limit":50})["items"]
            assert len(tasks) == 11, tasks
            gate = next(t for t in tasks if t["meeting_id"] == "100-1" and t["stable_id"] == "gate")
            assert gate["status"] == "done" and gate["status_label"] == "Done"
            assert gate["title"] == "Check release" and gate["description"] == "Get approval"
            assert gate["task"] == "Check the release gate" and gate["subtasks"][0]["done"] is True
            assert gate["team"] == "Delivery" and gate["owner"] == "Taylor Smith"
            ship = next(t for t in tasks if t["meeting_id"] == "100-1" and t["stable_id"] == "ship")
            assert ship["status"] == "todo" and ship["waiting_on"]["task_id"] == "gate" and ship["waiting_label"] is None
            peer_ship = next(t for t in tasks if t["meeting_id"] == "100-2" and t["stable_id"] == "ship")
            assert peer_ship["status"] == "waiting" and peer_ship["waiting_label"] == "Check release"
            manual = next(t for t in tasks if t["manual"])
            assert manual["stable_id"] == "hand" and manual["team"] == "Delivery" and manual["owner"] == "Taylor Smith"
            for status in ["todo","waiting","review","done","open","all"]:
                filtered = call("list_tasks", {"status":status,"limit":50})["items"]
                expected_tasks = [t for t in tasks if status == "all" or (status == "open" and t["status"] != "done") or t["status"] == status]
                assert filtered == expected_tasks, status
            first = call("list_tasks", {"status":"all","limit":1})
            second = call("list_tasks", {"status":"all","limit":50,"cursor":first["nextCursor"]})
            assert first["items"] + second["items"] == tasks
            paged, cursor = [], None
            for _ in range(len(tasks)):
                arguments = {"status":"all","limit":1}
                if cursor: arguments["cursor"] = cursor
                one = call("list_tasks", arguments)
                paged += one["items"]
                cursor = one["nextCursor"]
                if not one["hasMore"]: break
            assert paged == tasks and cursor is None, "Duplicate task titles must page by stable id"
            teams = call("list_teams", {"limit":1})
            assert teams["items"][0]["name"] == "Delivery" and teams["items"][0]["aliases"]["You"] == "Taylor Smith"
            assert call("list_teams", {"cursor":teams["nextCursor"]})["items"][0]["name"] == "Legal"
            meeting_data = call("get_meeting", {"id":"100-1","parts":["tasks","notes","summary"]})
            assert sorted(meeting_data["tasks"], key=lambda t:t["stable_id"]) == sorted([t for t in tasks if t["meeting_id"] == "100-1"], key=lambda t:t["stable_id"])
            assert "Original AI list" in meeting_data["summary"]["todo_label"]
            assert meeting_data["notes"]["text"] == "Typed-only nebula caf\u00e9 notes"
            page = call("get_meeting", {"id":"100-1","parts":["notes"],"limit":22})["notes"]
            rest = call("get_meeting", {"id":"100-1","parts":["notes"],"offset":page["nextOffset"]})["notes"]
            assert page["text"] + rest["text"] == meeting_data["notes"]["text"]
            assert [m["id"] for m in call("search_meetings", {"query":"nebula"})["items"]] == ["100-1"]
            assert "nebula" in call("search_meetings", {"query":"nebula"})["items"][0]["snippet"]
            assert len(call("search_meetings", {"query":"Taylor Smith"})["items"]) == 2
            assert call("get_meeting", {"id":"100-0","parts":["tasks"]})["tasks"] == []
            after = snapshot()
            changed = {name for name in before.keys() | after.keys() if before.get(name) != after.get(name)}
            assert before == after, "A read changed fixture files or created state: " + ", ".join(sorted(changed))
            write(state / "config.json", {"version":1,"features":{"tasks":False,"memory":True,"daily_check":True,"projects":True,"other_devices":True},"notes":{}})
            disabled_snapshot = snapshot()
            tools_off = send({"jsonrpc":"2.0","id":next_id,"method":"tools/list","params":{}})
            next_id += 1
            assert not {"list_tasks","list_teams"} & {t["name"] for t in tools_off["tools"]}
            for tool in ["list_tasks","list_teams"]:
                denied = send({"jsonrpc":"2.0","id":next_id,"method":"tools/call","params":{"name":tool,"arguments":{}}})
                next_id += 1
                assert denied["isError"] is True
            assert len(call("get_meeting", {"id":"100-1","parts":["tasks"]})["tasks"]) == 5
            assert disabled_snapshot == snapshot(), "Feature-disabled reads changed fixture files"
            print("Task fields, six status filters, teams and task pagination, typed notes and UTF-8 paging, effective search, cutoff, disabled feature behavior: passed")
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
        print("tools/list -> " + json.dumps({"tools": sorted(expected), "schemas_and_annotations": "9 checked"}))
        print("search_meetings -> " + json.dumps(result, ensure_ascii=False))
        print("exit=0; stderr empty; stdout JSON-RPC only; fixture files unchanged")
        print("Full transcript: " + str(artifact))


if __name__ == "__main__":
    main()
