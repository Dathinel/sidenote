"""Exercise offered-model selection, Other entry, and the existing storage key offscreen."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
out.mkdir(parents=True, exist_ok=True)
state = Path(tempfile.mkdtemp(prefix="model-menu-", dir=root / "target"))
for name in ("appdata", "localappdata", "peers"):
    (state / name).mkdir()
shutil.copyfile(root / "tools/ui-fixtures/hex.json", state / "hex-fixture.json")
env = dict(os.environ, APPDATA=str(state / "appdata"), LOCALAPPDATA=str(state / "localappdata"), SIDENOTE_STATE_DIR=str(state))


def render(name, scene, events=None):
    args = [str(binary), "--meetings-dir", str(root / "tools/ui-fixtures/meetings"), "--data-dir", str(state),
            "--peers-dir", str(state / "peers"), "--scene", scene, "--theme", "light", "--size", "1280x800",
            "--scale", "1", "--snap", str(out / f"{name}.png"), "--tree", str(out / f"{name}.json")]
    if events:
        script = out / f"{name}-script.json"
        script.write_text(json.dumps({"events": events + [{"t": 4.5, "type": "mark", "name": "end"}]}), encoding="utf-8")
        args += ["--script", str(script), "--at", "mark:end"]
    result = subprocess.run(args, cwd=root, env=env, capture_output=True, timeout=60, creationflags=subprocess.CREATE_NO_WINDOW)
    assert result.returncode == 0, (name, result.returncode)
    tree = json.loads((out / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    assert not any("helper" in node.get("text", "").lower() for node in tree["elements"])
    return tree["elements"]


def prefs():
    return json.loads((state / "local/ui.json").read_text(encoding="utf-8-sig"))


menu = render("menu", "settings-model-menu")
assert {"Sonnet 5.5", "Opus 5.5", "Haiku 5.5", "Other…"} <= {n.get("label") for n in menu if n.get("role") == "option"}
assert not any(n.get("key") == "ask-settings-model-input" for n in menu)
chosen = render("chosen", "settings-model-menu", [{"t": 0.3, "type": "tap", "key": "option2"}])
assert prefs()["ask_models"]["Claude Sonnet"] == "claude-haiku-5-5"
assert not any(n.get("key") == "ask-settings-model-input" for n in chosen)
other = render("other", "settings-model-menu", [{"t": 0.3, "type": "tap", "key": "option3"}])
assert any(n.get("key") == "ask-settings-model-input" for n in other)
custom = render("custom", "settings-model-other", [
    {"t": 0.3, "type": "tap", "key": "ask-settings-model-input"},
    {"t": 0.5, "type": "key", "name": "A", "ctrl": True, "cmd": True},
    {"t": 0.7, "type": "type", "text": "custom/synthetic-model"},
    {"t": 3.0, "type": "tap", "key": "ask-settings-model-save"},
])
assert prefs()["ask_models"]["Claude Sonnet"] == "custom/synthetic-model"
reloaded = render("reloaded", "settings-helpers")
assert any(n.get("key") == "ask-settings-model-input" for n in reloaded)
assert prefs()["ask_models"]["Claude Sonnet"] == "custom/synthetic-model"
print("PASS: offered-model menu, explicit Other entry, saved selection and custom model across restart; ask_models storage key preserved.")
