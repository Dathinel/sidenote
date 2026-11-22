"""Exercise Memory actions through the native app's offscreen script driver."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile
from threading import Timer

from prepare_memory_fixture import prepare
from prepare_check_fixture import proposal_id, sources_fp

ROOT = Path(__file__).resolve().parents[1]
DEVKIT = Path("E:/Personal-Projects/tools/hex-meetings-gnu/w64devkit/bin")


def tap(t, key):
    return dict(t=t, type="tap", key=key)


def mark(t, name):
    return dict(t=t, type="mark", name=name)


def started(path):
    assert path.is_file(), "the fake helper never read stdin"
    pid, cwd = path.read_text(encoding="utf-8").splitlines()
    return int(pid), Path(cwd)


def alive(pid):
    api = ctypes.windll.kernel32
    api.OpenProcess.restype = ctypes.c_void_p
    handle = api.OpenProcess(0x100000, False, pid)
    if not handle:
        return False
    try:
        return api.WaitForSingleObject(ctypes.c_void_p(handle), 0) == 0x102
    finally:
        api.CloseHandle(ctypes.c_void_p(handle))


def fixture(root, case):
    meetings = prepare(root)
    config = {"version": 1, "features": {"memory": True}, "notes": {"helper": "codex"}}
    (root / "config.json").write_text(json.dumps(config), encoding="utf-8")
    records = root / "memory/records.json"
    data = json.loads(records.read_text(encoding="utf-8"))
    p = data["proposals"][0]
    if case == "retire-note":
        p["op"] = "retire"
        p["id"] = proposal_id(p)
        records.write_text(json.dumps(data), encoding="utf-8")
    elif case in {"needs-you", "confirm-legacy"}:
        p.update(status="accepted", decided_at=p["created_at"], comment="Use Friday afternoon.")
        original = next(r for r in data["records"] if r["id"] == p["target_id"])
        original["status"] = "superseded"
        record = dict(
            original, id=f"memory-{p['id']}", status="active", text=p["text"], basis="owner_confirmed",
            supersedes=original["id"], updated_at=p["decided_at"], sources=p["sources"],
            source_fingerprint=sources_fp(p["sources"]),
        )
        data["records"].append(record)
        data["events"].append(dict(
            record_id=record["id"], operation="accept_proposal",
            at=p["decided_at"], proposal_id=p["id"],
        ))
        records.write_text(json.dumps(data), encoding="utf-8")
    elif case == "citation-second-jump":
        p["sources"].append(dict(type="meeting", reference="[[1800000000001-1@15:00]]"))
        p["id"] = proposal_id(p)
        records.write_text(json.dumps(data), encoding="utf-8")
        transcript = meetings / "1800000000001-1/english-transcript.md"
        transcript.write_text(
            "\n".join(
                f"[00:{i // 6:02}:{i % 6 * 10:02}.000] Speaker{i % 2} (English): Synthetic transcript line {i}."
                for i in range(110)
            ),
            encoding="utf-8",
        )
    return meetings, p["id"]


def run(exe, output, case, theme="dark"):
    with tempfile.TemporaryDirectory(prefix="sidenote-memory-proof-") as temporary:
        root = Path(temporary)
        meetings, proposal = fixture(root, case)
        fake = root / "codex.exe"
        subprocess.run(
            [str(DEVKIT / "gcc.exe"), str(ROOT / "tools/ui-fixtures/memory-rewrite-helper.c"), "-o", str(fake)],
            creationflags=subprocess.CREATE_NO_WINDOW, check=True,
        )
        marker = root / "helper.txt"
        export = root / "local/memory/company.md"
        env = {k: v for k, v in os.environ.items() if not k.startswith("HEX_MEETING_")}
        for name in ("appdata", "localappdata", "home", "codex"):
            (root / name).mkdir()
        hold = case.startswith("cancel-") or case in {"rewrite-wait", "helper-timeout"}
        env.update(
            APPDATA=str(root / "appdata"),
            LOCALAPPDATA=str(root / "localappdata"),
            USERPROFILE=str(root / "home"),
            HOME=str(root / "home"),
            CODEX_HOME=str(root / "codex"),
            SIDENOTE_DATA_DIR=str(root),
            SIDENOTE_HEX_DIR=str(meetings),
            SIDENOTE_PEERS_DIR=str(root / "peers"),
            SIDENOTE_MEMORY_TEST_MARKER=str(marker),
            SIDENOTE_MEMORY_TEST_MODE="hold" if hold else "reply",
            TEMP=temporary,
            TMP=temporary,
            PATH=os.pathsep.join([temporary, str(DEVKIT), env["PATH"]]),
        )
        events = [
            tap(0.15, f"memory-comment-{proposal}"),
            tap(0.32, "memory-comment-input"),
            dict(t=0.48, paste="Use Friday afternoon."),
            tap(0.65, "memory-comment-save"),
            mark(1.4, "before"),
        ]
        if case.startswith("cancel-reject"):
            events += [tap(2, f"memory-reject-{proposal}"), mark(3.25, "after")]
        elif case == "cancel-accept-original":
            events += [tap(2, f"memory-accept-original-{proposal}"), mark(3.25, "after")]
        elif case == "accept-rewrite":
            events += [tap(2, f"memory-accept-{proposal}")]
        if case == "cancel-close":
            events += [mark(1.9, "close")]
        if case in {"needs-you", "confirm-legacy"}:
            events = []
            if case == "confirm-legacy":
                events.append(tap(0.4, f"memory-legacy-confirm-memory-{proposal}"))
        if case == "citation-second-jump":
            events = [tap(0.4, f"need-{proposal}-suggested-source-0-time-1")]
        end = {"cancel-close": 2, "helper-timeout": 92.5, "rewrite-wait": 1.9}.get(case, 3.5)
        events.append(mark(end, "end"))
        script = root / "script.json"
        script.write_text(json.dumps(events), encoding="utf-8")
        artifact = output / f"{case}-1334-{theme}"
        command = [
            str(exe), "--data-dir", temporary, "--meetings-dir", str(meetings),
            "--peers-dir", str(root / "peers"), "--scene", "memory", "--size", "1334x900",
            "--theme", theme, "--scale", "1", "--script", str(script), "--at", "mark:end",
            "--realtime", "--tree", str(artifact.with_suffix(".json")),
        ]
        if case in {"comment-card", "needs-you"}:
            command += ["--snap", str(artifact.with_suffix(".png"))]
        with subprocess.Popen(
            command, env=env, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW,
        ) as app:
            watchdog = Timer(120, lambda: app.kill() if app.poll() is None else None)
            watchdog.start()
            try:
                for line in app.stdout:
                    print(line.rstrip(), flush=True)
                    if line.startswith("rune-mark before ") and hold:
                        pid, scratch = started(marker)
                        assert alive(pid), "the helper already exited before the decision"
                        if case == "cancel-reject-export-fails":
                            export.unlink()
                            export.mkdir()
                    elif line.startswith("rune-mark after "):
                        assert not alive(pid), "the decision left the helper running"
                        assert not scratch.exists(), "the decision left the helper scratch folder"
                assert app.wait(timeout=5) == 0
            finally:
                watchdog.cancel()
                if app.poll() is None:
                    app.kill()
                    app.wait(timeout=5)
        if case in {"retire-note", "needs-you", "confirm-legacy", "citation-second-jump"}:
            assert not marker.exists(), "a read-only decision view or note started the helper"
        else:
            pid, scratch = started(marker)
            assert not alive(pid), "closing the app left the helper running"
            assert not scratch.exists(), "closing the app left the helper scratch folder"
        if case in {"comment-card", "rewrite-wait", "retire-note", "helper-timeout", "needs-you", "confirm-legacy"}:
            text = artifact.with_suffix(".json").read_text(encoding="utf-8-sig")
            expected = {
                "comment-card": ["New wording ready", "Your comment: Use Friday afternoon."],
                "rewrite-wait": ["Rewriting", "Accept original wording"],
                "retire-note": ["Your note: Use Friday afternoon."],
                "helper-timeout": ["Rewrite failed", "Try again"],
                "needs-you": [
                    "Your comment was not used",
                    f"memory-legacy-edit-memory-{proposal}",
                    f"memory-legacy-confirm-memory-{proposal}",
                ],
                "confirm-legacy": [],
            }[case]
            for line in expected:
                assert line in text, f"the card is missing {line!r}"
            if case == "confirm-legacy":
                assert "memory-comments-not-used" not in text, "confirmation left the legacy comment in Needs you"
        if case in {"accept-rewrite", "cancel-accept-original"}:
            saved = export.read_text(encoding="utf-8")
            assert ("The demo is on Friday afternoon." in saved) == (case == "accept-rewrite"), "acceptance saved different wording"
            if case == "cancel-accept-original":
                assert "The demo is on Friday." in saved, "the original wording was not saved"
        if case == "citation-second-jump":
            tree = json.loads(artifact.with_suffix(".json").read_text(encoding="utf-8-sig"))
            lines = {e.get("key"): e for e in tree["elements"]}
            second = lines.get("line-90")
            first = lines.get("line-0")
            assert second and 0 < second["rect"][1] < 860, "the second citation did not bring its line into view"
            assert not first or first["rect"][1] + first["rect"][3] < 0, "the jump stayed at the first citation"
        print(f"PASS {case}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("case", choices=[
        "cancel-reject", "cancel-reject-export-fails", "cancel-close", "cancel-accept-original",
        "comment-card", "rewrite-wait", "accept-rewrite", "retire-note", "helper-timeout",
        "needs-you", "confirm-legacy",
        "citation-second-jump",
    ])
    parser.add_argument("--theme", choices=["light", "dark"], default="dark")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    run(args.exe.resolve(), args.output.resolve(), args.case, args.theme)


if __name__ == "__main__":
    main()
