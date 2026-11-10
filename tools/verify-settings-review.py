"""Check review-round layout and shortcut behaviour in the real offscreen app trees."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("artifacts/settings-round1")


def shot(name):
    tree = json.loads((root / f"{name}.json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    return tree["elements"]


def keyed(elements, key):
    return next(node for node in elements if node.get("key") == key)


for page in ("recording", "summaries", "helpers"):
    elements = shot(f"settings-{page}-1280-light")
    title = keyed(elements, "settings-title")["rect"]
    tabs = keyed(elements, "settings-tabs")["rect"]
    assert title[1] + title[3] <= tabs[1], f"{page}: tabs precede title"

narrow = shot("settings-recording-640-light")
for label in ("Hex and dictation", "Appearance and startup"):
    node = next(node for node in narrow if node.get("text") == label)
    assert node["rect"][3] <= 21.1, f"{label} wraps"
    assert keyed(narrow, f"settings-page-{label}").get("help") == label

rail = shot("settings-sidebar-folded-1280-light")
centres = [keyed(rail, key)["rect"][0] + keyed(rail, key)["rect"][2] / 2 for key in ("nav-search", "rail-recording")]
assert abs(centres[0] - centres[1]) < 0.1, "Recording icon is off the rail column"

home = shot("settings-sidebar-open-1280-light")
assert sum(node.get("text") == "/" for node in home) == 1, "Duplicate slash hints"
helpers = shot("settings-helpers-1280-light")
assert sum(node.get("text") == "In use" for node in helpers) == 1, "Duplicate In use labels"
for scene, field in (("sidebar", "finder-input"), ("settings", "hex-filter")):
    elements = shot(f"settings-review-{scene}-slash-light")
    container = keyed(elements, field)
    assert any(node.get("focused") and node.get("role") == "textbox" and node.get("path", "").startswith(container["path"] + "/") for node in elements), f"Slash did not focus {scene} search"
print("PASS: title/tab order, single-line labels and tooltips, rail alignment, single hints/status, and both slash shortcuts.")
