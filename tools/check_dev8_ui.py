"""Assert dev.8 task, sidebar and naming behavior in isolated offscreen render trees."""
import ast
import json
import sys
import struct
from pathlib import Path

LEGACY = "Put all production files in one tracker folder on the shared drive: batch details, finished goods and responsibilities"
SHORT = "Put all production files in one"
LONG = "Prepare the revised workshop materials and check every printed sample before the next planning meeting"
DESCRIPTION = "Collect the batch details and responsibilities in the shared folder."


def check(root, scaled_roots=()):
    checked = 0
    for theme in ("dark", "light"):
        def load(scene):
            nonlocal checked
            data = json.loads((root / f"{scene}-{theme}.json").read_text(encoding="utf-8-sig"))
            checked += 1
            assert not data["warnings"], (scene, data["warnings"])
            elements = data["elements"]
            return elements, {e["key"]: e for e in elements if "key" in e}

        def text(elements):
            return [e["text"] for e in elements if "text" in e]

        elements, keyed = load("tasks-v2-owner")
        row = keyed["ttask-1800000000000-1-6"]["path"] + "/"
        row_elements = [e for e in elements if e["path"].startswith(row)]
        assert SHORT in text(row_elements) and LEGACY in text(row_elements)
        title = next(e for e in row_elements if e.get("text") == SHORT)
        description = next(e for e in row_elements if e.get("text") == LEGACY)
        assert title["rect"][3] <= 42 and description["rect"][3] <= 36
        for scene in ("tasks-v2-panel", "tasks-v2-panel-560"):
            _, keyed = load(scene)
            assert keyed["ptitle"]["text"] == "Review the field guide"
            assert keyed["ptitle"]["rect"][3] <= 56
            assert "Read the sample pages" in keyed["pdescription"]["text"]
        _, keyed = load("tasks-v2-long-panel")
        assert keyed["ptitle"]["text"] == LONG and keyed["ptitle"]["rect"][3] <= 56
        elements, _ = load("sb-live-finishing")
        assert "A summary can be written once Hex finishes the transcript. You can leave this page." in text(elements)

        elements, keyed = load("tasks-v2-legacy-panel")
        assert keyed["ptitle"]["text"] == SHORT
        assert keyed["pdescription"]["text"] == LEGACY
        assert "Tue 17 Nov (guessed)" in text(elements)
        assert "The meeting did not set a date. Sidenote guessed one." in text(elements)
        assert "task-guessed-date" in keyed
        assert "Long task titles are now shown short. The full text is in each task's description." in text(elements)

        elements, keyed = load("tasks-v2-titles-edited")
        assert keyed["ptitle"]["text"] == "Organise the shared tracker", keyed["ptitle"]["text"]
        assert keyed["pdescription"]["text"] == DESCRIPTION, keyed["pdescription"]["text"]
        row = keyed["ttask-1800000000000-1-6"]["path"] + "/"
        row_text = text([e for e in elements if e["path"].startswith(row)])
        assert "Organise the shared tracker" in row_text
        assert DESCRIPTION in row_text
        assert "Undo my changes" in text(elements)
        assert "Discard my changes" not in text(elements)

        elements, keyed = load("tasks-v2-full-titles")
        assert keyed["show-full-titles"]["text"] == "Show full titles: on"
        for index, expected in [(6, LEGACY), (4, LONG)]:
            row = keyed[f"ttask-1800000000000-1-{index}"]["path"] + "/"
            assert expected in text([e for e in elements if e["path"].startswith(row)])

        _, keyed = load("tasks-v2-long-panel-560")
        assert keyed["ptitle"]["rect"][3] <= 56, keyed["ptitle"]["rect"]
        assert keyed["ptitle"]["text"] == LONG

        _, keyed = load("sidebar-settings")
        assert "hex-pane" in keyed and "profile-button" in keyed
        _, keyed = load("prompt-editor-link")
        assert "notes-state-edit" in keyed

        _, keyed = load("sidebar-meetings")
        assert keyed["nav"]["rect"][2] == 248
        assert "profile-button" in keyed
        assert "hex-status" not in keyed and "view-hex" not in keyed
        _, keyed = load("sidebar-rail")
        assert keyed["nav"]["rect"][2] == 56
        assert "rail-recording" in keyed
        _, keyed = load("sb-home-collapsed")
        assert "nav" not in keyed and "sidebar-toggle" in keyed

        elements, _ = load("profile-menu")
        menu = text([e for e in elements if e["path"].startswith("kit-root/profile-menu-menu/")])
        for label in ["You", "Prompt library", "Help", "Settings", "Ctrl+,"]:
            assert label in menu, (label, menu)
        assert any(line.startswith("This PC:") for line in menu)

        elements, keyed = load("profile-editor")
        assert "instructions-input-0" in keyed
        assert "Meeting summary instructions" in text(elements)
        assert not any(e["path"].startswith("kit-root/profile-menu-menu/") for e in elements)

        elements, _ = load("sb-live-making")
        assert text(elements).count("Writing the summary") == 2
        assert any(line.startswith("The summary opens here when it is ready") for line in text(elements))
        elements, _ = load("sb-live-making-slow")
        assert "The summary is taking a while" in text(elements)
        elements, _ = load("sb-live-recording")
        assert "Your notes save as you type. Sidenote writes the summary after you stop." in text(elements)
        elements, _ = load("screens-toast")
        assert "Copied summary" in text(elements)

    for folder, scale in zip(scaled_roots, (1.25, 1.5) if scaled_roots else (), strict=True):
        index = json.loads((folder / "render-index.json").read_text(encoding="utf-8-sig"))
        assert len(index) == 4
        assert {row["Scene"] for row in index} == {"sidebar-meetings", "sidebar-settings"}
        assert {row["Theme"] for row in index} == {"dark", "light"}
        for row in index:
            png = Path(row["Path"])
            data = json.loads(png.with_suffix(".json").read_text(encoding="utf-8-sig"))
            assert not data["warnings"]
            assert data["scale"] == scale
            image = png.read_bytes()
            assert image[:8] == bytes.fromhex("89504e470d0a1a0a")
            assert struct.unpack(">II", image[16:24]) == (round(1320 * scale), round(800 * scale))
            checked += 1

    source = Path(__file__).resolve().parents[1]
    assert '.help("Stop recording and write the summary")' in (source / "src/ui/nav.rs").read_text()
    table = ast.parse((source / "tools/icon/gen_lucide.py").read_text())
    entries = next(node.value for node in table.body if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == "ICONS" for target in node.targets))
    catalog = ast.literal_eval(entries)
    expected = {"MEETINGS": "notebook-pen", "TASKS": "list-checks", "MEMORY": "library", "PROMPTS": "braces", "SUMMARY": "sparkles", "UPDATES": "download", "CONNECTED": "plug", "PEOPLE": "users", "DONE": "circle-check"}
    for entry, name in expected.items():
        assert catalog[entry] == name, (entry, catalog[entry])
    assert 'pub const STROKE_WIDTH: f32 = 2.25;' in (source / "src/ui/icons.rs").read_text()
    notices = (source / "assets/licenses/lucide-LICENSE.txt").read_text()
    assert "ISC License" in notices and "MIT License" in notices and "Feather" in notices
    assert "assets/licenses/lucide-LICENSE.txt" in (source / "tools/pack_release.ps1").read_text()
    print(f"dev8 UI: PASS ({checked} offscreen trees; title edits, split switch, 2-line clamp, sidebar states, profile menu, J.5 icon map, 7 naming items and packaged notices)")


if __name__ == "__main__":
    check(Path(sys.argv[1]), tuple(Path(arg) for arg in sys.argv[2:]))
