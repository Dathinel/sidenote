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


def fixture(root):
    meetings = prepare(root)
    config = {"version": 1, "features": {"memory": True}, "notes": {"helper": "codex"}}
    (root / "config.json").write_text(json.dumps(config), encoding="utf-8")
    p = json.loads((root / "memory/records.json").read_text(encoding="utf-8"))["proposals"][0]
    return meetings, p["id"]


def run(exe, output, case, theme="dark"):
    with tempfile.TemporaryDirectory(prefix="sidenote-memory-proof-") as temporary:
        root = Path(temporary)
        meetings, proposal = fixture(root)
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
            SIDENOTE_MEMORY_TEST_MODE="hold",
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
        if case == "cancel-close":
            events += [mark(1.9, "close")]
        events.append(mark(3.5 if case != "cancel-close" else 2, "end"))
        script = root / "script.json"
        script.write_text(json.dumps(events), encoding="utf-8")
        command = [
            str(exe), "--data-dir", temporary, "--meetings-dir", str(meetings),
            "--peers-dir", str(root / "peers"), "--scene", "memory", "--size", "1334x900",
            "--theme", theme, "--scale", "1", "--script", str(script), "--at", "mark:end",
            "--realtime", "--tree", str(output / f"{case}.json"),
        ]
        with subprocess.Popen(
            command, env=env, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW,
        ) as app:
            watchdog = Timer(120, lambda: app.kill() if app.poll() is None else None)
            watchdog.start()
            try:
                for line in app.stdout:
                    print(line.rstrip(), flush=True)
                    if line.startswith("rune-mark before "):
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
        pid, scratch = started(marker)
        assert not alive(pid), "closing the app left the helper running"
        assert not scratch.exists(), "closing the app left the helper scratch folder"
        print(f"PASS {case}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("case", choices=["cancel-reject", "cancel-reject-export-fails", "cancel-close"])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    run(args.exe.resolve(), args.output.resolve(), args.case)


if __name__ == "__main__":
    main()
