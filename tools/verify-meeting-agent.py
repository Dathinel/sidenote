"""Render synthetic states offscreen and check meeting agent controls and old settings."""
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from prepare_people_fixture import prepare

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
out = root / "artifacts/meeting-agent"
out.mkdir(parents=True, exist_ok=True)
receipts = []
codex_model = next(r for r in json.loads((root / "default-runners.json").read_text())
                   if r["name"] == "Codex").get("model") or "Codex"
codex_model = {"gpt-6.1-terra": "GPT-6.1 Terra", "gpt-6.1-sol": "GPT-6.1 Sol"}.get(codex_model, codex_model)


def render(name, scene, theme="light", events=None, state=None):
    state = state or Path(tempfile.mkdtemp(prefix="meeting-agent-render-", dir=root / "target"))
    for folder in ("appdata", "localappdata", "peers"):
        (state / folder).mkdir(exist_ok=True)
    shutil.copyfile(root / "tools/ui-fixtures/hex.json", state / "hex-fixture.json")
    meetings = prepare(state) if scene.startswith("people-") else root / "tools/ui-fixtures/meetings"
    args = [str(binary), "--meetings-dir", str(meetings),
            "--data-dir", str(state), "--peers-dir", str(state / "peers"),
            "--scene", scene, "--theme", theme, "--size", "1280x800", "--scale", "1",
            "--snap", str(out / f"{name}.png"), "--tree", str(out / f"{name}.json")]
    if events:
        (out / "scripts").mkdir(exist_ok=True)
        script = out / "scripts" / f"{name}.json"
        script.write_text(json.dumps({"events": events + [{"t": 0.8, "type": "mark", "name": "end"}]}))
        args += ["--script", str(script), "--at", "mark:end"]
    elif scene == "settings-search":
        # The highlight lasts two wall-clock seconds; capture once the scroll settles.
        events = json.loads((root / "tools/ui-scripts/settings-search.json").read_text())
        events["events"][-1]["t"] = 1.3
        (out / "scripts").mkdir(exist_ok=True)
        script = out / "scripts/search-highlight.json"
        script.write_text(json.dumps(events))
        args += ["--script", str(script), "--at", "mark:end"]
    env = dict(os.environ, APPDATA=str(state / "appdata"), LOCALAPPDATA=str(state / "localappdata"),
               SIDENOTE_STATE_DIR=str(state), SIDENOTE_DATA_DIR=str(state))
    result = subprocess.run(args, cwd=root, env=env, capture_output=True,
                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=90)
    (out / f"{name}.log").write_bytes(result.stderr)
    assert result.returncode == 0, (name, result.returncode, result.stderr.decode(errors="replace")[-1000:])
    tree = json.loads((out / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    assert struct.unpack(">II", (out / f"{name}.png").read_bytes()[16:24]) == (1280, 800)
    receipts.append(name)
    return tree["elements"], state


def keyed(nodes, key):
    return next(node for node in nodes if node.get("key") == key)


for theme in ("light", "dark"):
    page, _ = render(f"models-and-agents-1280-{theme}", "meeting-agent-settings", theme)
    keyed(page, "ask-meeting-agent-row")
    for key in ("ask-agent-row", "ask-model-row", "ask-meeting-thinking-row", "ask-chat-thinking-row"):
        keyed(page, key)
    assert any("Used for questions while a meeting is live. Low thinking keeps answers fast." == n.get("text") for n in page)
    picker, _ = render(f"meeting-agent-picker-1280-{theme}", "meeting-agent-picker", theme)
    options = {n.get("label") for n in picker if n.get("role") == "menuitemcheckbox"}
    assert {"Codex · Default", "Claude Code · Default", "OpenCode · Default"} <= options, options
    assert all(n["rect"][2] >= 300 for n in picker if n.get("role") == "menuitemcheckbox")
    thinking, _ = render(f"meeting-thinking-picker-1280-{theme}", "meeting-agent-settings", theme,
                         events=[{"t": 0.3, "type": "tap", "key": "ask-meeting-thinking-trigger"}])
    assert {n.get("label") for n in thinking if n.get("role") == "option"} == {"Low", "Medium"}
    live, _ = render(f"live-ask-codex-low-1280-{theme}", "meeting-agent-live", theme)
    assert keyed(live, "ask-model")["label"] == "Model: Codex"
    assert keyed(live, "ask-thinking")["label"] == "Thinking: Low"
    assert any(n.get("text") == codex_model for n in live)
    assert sum(n.get("text") == "Low" for n in live) == 1
    assert not any(n.get("text") == "Codex · Low" for n in live)
    assert not any(n.get("key") == "ask-starting-codex" for n in live)
    normal, _ = render(f"normal-ask-codex-low-1280-{theme}", "meeting-agent-normal", theme)
    assert keyed(normal, "ask-model")["label"] == "Model: Codex"
    assert keyed(normal, "ask-thinking")["label"] == "Thinking: Low"
    assert any(n.get("text") == "Codex" for n in normal)
    assert sum(n.get("text") == "Low" for n in normal) == 1
    assert not any(n.get("text") == "Codex · Low" for n in normal)
    fallback, _ = render(f"fallback-notice-1280-{theme}", "meeting-agent-fallback", theme)
    assert any(n.get("text") == "Codex isn't available, using Claude Code." for n in fallback)
    assert any("What did we decide?" in n.get("text", "") for n in fallback)

warming, _ = render("live-ask-warming-1280-light", "meeting-agent-warming")
keyed(warming, "ask-progress")
assert sum(n.get("text") == "Starting Codex…" for n in warming) == 1
assert not any("Thinking," in n.get("text", "") for n in warming)
assert not any(n.get("key") == "ask-starting-codex" for n in warming)
assert keyed(warming, "ask-model")["label"] == "Model: Codex"
assert not any(n.get("key") == "ask-fallback-notice" for n in warming)

# Use the actual settings menu and restart to read the saved meeting preference.
picker, state = render("meeting-choice-before", "meeting-agent-picker")
option = next(n for n in picker if n.get("role") == "menuitemcheckbox" and n.get("label") == "Codex · GPT-6.1 Terra")
render("meeting-choice-saved", "meeting-agent-picker", events=[{"t": 0.3, "type": "tap", "key": option["key"]}], state=state)
prefs = json.loads((state / "local/ui.json").read_text(encoding="utf-8-sig"))
assert prefs["ask_meeting_agent"] == "Codex" and prefs["ask_meeting_model"] == "gpt-6.1-terra"
restarted, _ = render("meeting-choice-restarted", "meeting-agent-settings", state=state)
assert keyed(restarted, "ask-meeting-agent-trigger")["label"] == "Agent during meetings"
assert any(n.get("text") == "Codex · GPT-6.1 Terra" for n in restarted)

scenes = ("general recording summaries tasks prompts people hex helpers speech helper-advanced "
          "appearance sync updates sidebar-open sidebar-folded recording-expanded "
          "summaries-expanded tasks-expanded hex-expanded helper-advanced-expanded sync-expanded "
          "updates-expanded helpers-expanded").split()
with ThreadPoolExecutor(max_workers=2) as pool:
    list(pool.map(lambda scene: render(f"settings-{scene}-1280-light", f"settings-{scene}"), scenes))
render("settings-people-team-editor-1280-light", "people-team-editor",
       events=[{"t": 0.3, "type": "tap", "key": "team-advanced"}])
render("settings-search-1280-light", "settings-search")
subprocess.run([sys.executable, str(root / "tools/settings-reachability.py"), str(out)], check=True, cwd=root)
(out / "render-receipts.json").write_text(json.dumps(receipts, indent=2))
print(f"PASS: {len(receipts)} offscreen PNGs and trees; meeting row, installed model picker, live Codex Low, fallback notice, saved model restart and legacy settings.")
