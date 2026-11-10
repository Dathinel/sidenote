"""Exercise annotation model selection and storage isolation through the offscreen app."""
import json
from pathlib import Path
import sys
import tempfile
import render_people_review as native

exe = Path(sys.argv[1]).resolve()
native.OUT = native.ROOT / "artifacts/dev12/annotation-models"
(native.OUT / "state").mkdir(parents=True, exist_ok=True)
state = Path(tempfile.mkdtemp(prefix="models-", dir=native.OUT / "state"))
(state / "local").mkdir()
ask_models = {"Claude Sonnet": "ask/keep-model"}
(state / "local/ui.json").write_text(json.dumps({"ask_models": ask_models}))
settings = state / "annotations/settings.json"
settings.parent.mkdir()
settings.write_text(json.dumps({"helper": "claude", "model": "claude-sonnet-5-5"}))

def scroll(t=.3):
    return {"type": "scroll", "key": "hex-scroll", "dy": -600, "dur": .2, "t": t}

def render(name, events):
    result = native.render(exe, name, "settings-helpers", events=events, state=state)
    tree = json.loads(Path(result["tree"]).read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], tree["warnings"]
    return tree["elements"]

def stored():
    return json.loads(settings.read_text())

menu = render("annotation-menu", [scroll(), native.tap("annotation-model-menu-trigger", .8)])
summary_before = json.loads((state / "config.json").read_text()).get("notes")
assert {"Sonnet 5.5", "Opus 5.5", "Haiku 5.5", "Other\u2026"} <= {n.get("label") for n in menu if n.get("role") == "option"}
render("annotation-chosen", [scroll(), native.tap("annotation-model-menu-trigger", .8), native.tap("option2", 1.2)])
assert stored() == {"helper": "claude", "model": "claude-haiku-5-5"}, stored()
other = render("annotation-other", [scroll(), native.tap("annotation-model-menu-trigger", .8), native.tap("option3", 1.2)])
assert any(n.get("key") == "annotation-model" for n in other)
render("annotation-custom", [scroll(), native.tap("annotation-model-menu-trigger", .8), native.tap("option3", 1.2), native.tap("annotation-model", 1.6), {"type": "key", "name": "a", "cmd": True, "ctrl": True, "t": 1.9}, {"type": "type", "text": "custom/synthetic-annotation", "t": 2.2}, native.tap("annotation-save-model", 5)])
assert stored() == {"helper": "claude", "model": "custom/synthetic-annotation"}, stored()
reload = render("annotation-reloaded", [scroll()])
assert any(n.get("key") == "annotation-model" for n in reload)
assert stored()["model"] == "custom/synthetic-annotation"
render("annotation-codex", [scroll(), native.tap("annotation-agent-Codex", .8), native.tap("annotation-model-menu-trigger", 1.2), native.tap("option0", 1.6)])
assert stored() == {"helper": "codex", "model": "gpt-6.1-sol"}, stored()
# Annotation choices must never become Ask or meeting-summary preferences.
ui = state / "local/ui.json"
assert json.loads(ui.read_text())["ask_models"] == ask_models
assert json.loads((state / "config.json").read_text()).get("notes") == summary_before
print("PASS: annotation offered models, Other, custom save/restart, Codex catalog and independent helper/model storage")
