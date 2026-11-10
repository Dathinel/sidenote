"""Exercise the actual thinking menu offscreen, including preferences across app restarts."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else root / "artifacts/ask-models/preferences"
out.mkdir(parents=True, exist_ok=True)
state = Path(tempfile.mkdtemp(prefix="ask-preferences-", dir=root / "target"))
for name in ("appdata", "localappdata", "peers"):
    (state / name).mkdir()
shutil.copyfile(root / "tools/ui-fixtures/hex.json", state / "hex-fixture.json")
script = out / "choose-high.json"
script.write_text(json.dumps({"events": [
    {"t": 0.3, "type": "tap", "key": "ask-thinking-menu-item2"},
    {"t": 0.8, "type": "mark", "name": "end"},
]}), encoding="utf-8")
env = dict(os.environ, APPDATA=str(state / "appdata"), LOCALAPPDATA=str(state / "localappdata"),
           SIDENOTE_STATE_DIR=str(state))


def render(name, scene, level, choose=False):
    args = [str(binary), "--meetings-dir", str(root / "tools/ui-fixtures/meetings"),
            "--data-dir", str(state), "--peers-dir", str(state / "peers"),
            "--scene", scene, "--theme", "light", "--size", "1280x800", "--scale", "1",
            "--snap", str(out / f"{name}.png"), "--tree", str(out / f"{name}.json")]
    if choose:
        args += ["--script", str(script), "--at", "mark:end"]
    result = subprocess.run(args, cwd=root, env=env, capture_output=True,
                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=60)
    assert result.returncode == 0, (name, result.returncode)
    tree = json.loads((out / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    trigger = next(n for n in tree["elements"] if n.get("key") == "ask-thinking")
    assert trigger["label"] == f"Thinking: {level}", (name, trigger)


render("outside-high", "ask-thinking-picker", "High", choose=True)
render("outside-remembered", "ask-thinking-picker", "High")
render("meeting-default", "ask-in-meeting", "Low")
render("meeting-override", "ask-in-meeting", "High", choose=True)
render("meeting-default-after-restart", "ask-in-meeting", "Low")
prefs = json.loads((state / "local/ui.json").read_text(encoding="utf-8-sig"))
assert prefs["ask_chat_thinking"] == "high"
assert prefs["ask_meeting_thinking"] == "low"
print("PASS: actual offscreen menu selection; outside High remembered across restart; meeting override High leaves the saved meeting default Low.")
