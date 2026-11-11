"""Render live Ask with synthetic data, without a window or provider request."""
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
out = root / "artifacts/meeting-agent/round2"
out.mkdir(parents=True, exist_ok=True)
receipts = []


def keyed(nodes, key):
    return next(node for node in nodes if node.get("key") == key)


def bottom(node):
    return node["rect"][1] + node["rect"][3]


def render(state_name, width, theme, scene=None, events=None):
    name = f"live-ask-{state_name}-{width}-{theme}"
    state = Path(tempfile.mkdtemp(prefix="round2-render-", dir=root / "target"))
    for folder in ("appdata", "localappdata", "peers"):
        (state / folder).mkdir()
    shutil.copyfile(root / "tools/ui-fixtures/hex.json", state / "hex-fixture.json")
    args = [str(binary), "--meetings-dir", str(root / "tools/ui-fixtures/meetings"),
            "--data-dir", str(state), "--peers-dir", str(state / "peers"),
            "--scene", scene or f"meeting-agent-{state_name}", "--theme", theme,
            "--size", f"{width}x800", "--scale", "1", "--snap", str(out / f"{name}.png"),
            "--tree", str(out / f"{name}.json")]
    if events:
        script = out / f"{name}-script.json"
        script.write_text(json.dumps({"events": events + [{"t": 0.8, "type": "mark", "name": "end"}]}))
        args += ["--script", str(script), "--at", "mark:end"]
    env = dict(os.environ, APPDATA=str(state / "appdata"), LOCALAPPDATA=str(state / "localappdata"),
               SIDENOTE_STATE_DIR=str(state), SIDENOTE_DATA_DIR=str(state))
    result = subprocess.run(args, cwd=root, env=env, capture_output=True,
                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=90)
    (out / f"{name}.log").write_bytes(result.stderr)
    assert result.returncode == 0, (name, result.returncode, result.stderr[-1000:])
    tree = json.loads((out / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    assert struct.unpack(">II", (out / f"{name}.png").read_bytes()[16:24]) == (width, 800)
    receipts.append(name)
    return tree["elements"]


for width in (1280, 390):
    for theme in ("light", "dark"):
        frames = {}
        for state_name in ("idle", "warming", "answer"):
            nodes = render(state_name, width, theme,
                           scene="meeting-agent-live" if state_name == "idle" else None)
            frames[state_name] = nodes
            toggle = keyed(nodes, "live-chevron")
            pill = keyed(nodes, "ask-pill")
            transcript = keyed(nodes, "live-panel")
            for key in ("live-title", "live-notes", "live-chevron"):
                assert bottom(keyed(nodes, key)) <= transcript["rect"][1] + 1, (width, theme, state_name, key)
            assert bottom(pill) <= 800, (width, theme, state_name, pill["rect"])
            assert pill["rect"][0] >= 0 and pill["rect"][0] + pill["rect"][2] <= width, pill
            assert abs(pill["rect"][0] - transcript["rect"][0]) <= 1, (pill, transcript)
            assert abs(pill["rect"][2] - transcript["rect"][2]) <= 1, (pill, transcript)
            scroll = keyed(nodes, "live-tx-scroll")
            assert bottom(scroll) <= bottom(transcript) + 1, (scroll, transcript)
            assert any(n.get("text") == "GPT-6.1 Terra" for n in nodes)
            assert not any(n.get("text") == "gpt-6.1-terra" for n in nodes)
            assert keyed(nodes, "ask-scope")["label"] == "Ask scope: This meeting"
            if state_name != "idle":
                thread = keyed(nodes, "ask-thread")
                assert thread["rect"][1] >= bottom(toggle), (thread, toggle)
                assert any(n.get("text") == "Asked about this meeting" for n in nodes)
                assert any("What did we decide?" == n.get("text") for n in nodes)
            if state_name == "warming":
                assert sum(n.get("text") == "Starting Codex\u2026" for n in nodes) == 1
                assert not any("Thinking," in n.get("text", "") for n in nodes)
                assert not any(n.get("key") in ("ask-starting-codex", "ask-fallback-notice") for n in nodes)
            if state_name == "answer":
                assert any("Test a shorter onboarding flow" in n.get("text", "") for n in nodes)
            print(f"PASS {state_name} {width} {theme}", flush=True)
        for state_name in ("warming", "answer"):
            assert keyed(frames[state_name], "live-tx-scroll")["rect"][3] < keyed(frames["idle"], "live-tx-scroll")["rect"][3]
            assert keyed(frames[state_name], "ask-pill")["rect"][::2] == keyed(frames["idle"], "ask-pill")["rect"][::2]

scope = render("scope-menu", 1280, "light", scene="meeting-agent-live",
               events=[{"t": 0.3, "type": "tap", "key": "ask-scope"}])
labels = {n.get("label") for n in scope if n.get("role") == "menuitemcheckbox"}
assert {"This meeting", "All meetings"} <= labels, labels
all_scope = render("all-scope", 1280, "light")
assert keyed(all_scope, "ask-scope")["label"] == "Ask scope: All meetings"
assert any(n.get("text", "").startswith("ASKED ABOUT ALL YOUR MEETINGS") for n in all_scope)
(out / "render-receipts.json").write_text(json.dumps(receipts, indent=2))
print(f"PASS: {len(receipts)} synthetic offscreen frames, bounded panels, equal widths, one warming status and live scope.")
