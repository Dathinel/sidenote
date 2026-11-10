"""Verify the six dev.8 review fixes from offscreen render trees and fixture output."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def check(folder):
    index = json.loads((folder / "render-index.json").read_text(encoding="utf-8-sig"))
    data = {}
    for row in index:
        png = Path(row["Path"])
        assert png.is_file() and png.stat().st_size
        tree = json.loads(png.with_suffix(".json").read_text(encoding="utf-8-sig"))
        assert not tree["warnings"], (row["Scene"], tree["warnings"])
        data[row["Scene"], row["Theme"]] = tree["elements"]
    for theme in ("dark", "light"):
        def load(scene):
            elements = data[scene, theme]
            return elements, {e["key"]: e for e in elements if "key" in e}
        def below(elements, parent):
            return [e for e in elements if e["path"].startswith(parent["path"] + "/")]

        elements, keys = load("tasks-v2-owner")
        search = keys["task-search"]["rect"]
        for key in ("seg-who", "seg-done", "group-by"):
            rect = keys[key]["rect"]
            assert abs((rect[1] + rect[3]/2) - (search[1] + search[3]/2)) <= 1
        row = below(elements, keys["ttask-1800000000000-1-6"])
        title = next(e for e in row if e.get("text") == "Put all production files in one")
        description = next(e for e in row if e.get("text", "").startswith("Put all production files in one tracker folder"))
        assert len(title["text"].split()) <= 6 and len(title["text"]) <= 45
        assert 18 < description["rect"][3] <= 36, description["rect"]
        assert sum(e.get("key", "").startswith("subtick-") for e in elements) <= 2

        _, keys = load("tasks-v2-owner-narrow")
        search = keys["task-search"]["rect"]
        assert keys["seg-who"]["rect"][1] >= search[1] + search[3]
        _, keys = load("tasks-v2-panel")
        assert keys["ptitle"]["text"] == "Review the field guide"
        assert keys["pdescription"]["text"].startswith("Read the sample pages")
        _, keys = load("tasks-v2-long-panel-560")
        assert keys["ptitle"]["rect"][3] <= 56

        for scene in ("profile-menu", "profile-menu-narrow"):
            elements, keys = load(scene)
            panel, trigger = keys["profile-menu-menu"], keys["profile-button"]
            assert panel["rect"][2] == 228 == trigger["rect"][2]
            assert panel["rect"][0] == trigger["rect"][0]
            assert panel["rect"][1] + panel["rect"][3] <= trigger["rect"][1]
            name, device = keys["profile-menu-name"], keys["profile-menu-device"]
            assert not name["inert"] and not device["inert"]
            assert name["opacity"] == device["opacity"] == 1
            assert name["color"] != device["color"]
            for key in ("profile-menu-item2", "profile-menu-item3", "profile-menu-item4"):
                assert keys[key]["rect"][3] == 40
                icons = [e for e in below(elements, keys[key]) if e["role"] == "icon"]
                assert len(icons) == 1 and icons[0]["rect"][2:4] == [18, 18]
            assert any(e.get("text") == "Prompt library" for e in below(elements, panel))

        elements, keys = load("profile-editor")
        assert "instructions-input-0" in keys and "profile-menu-menu" not in keys
        for scene in ("sidebar-meetings", "sb-home-open", "sb-home-collapsed"):
            elements, keys = load(scene)
            texts = [e.get("text", "") for e in elements]
            assert not any(text in ("Areas", "No active areas yet.", "Show all") for text in texts)
            main = keys["main"]
            assert not any(e["role"] == "icon" for e in below(elements, keys["top-bar"]))
            meeting_rows = [e for e in below(elements, main) if e.get("key", "").startswith("meeting-") and e["key"][8:9].isdigit()]
            assert meeting_rows
            for meeting_row in meeting_rows:
                assert not any(e["role"] == "icon" for e in below(elements, meeting_row)), meeting_row["key"]
            if scene != "sb-home-collapsed":
                assert any(e["role"] == "icon" for e in below(elements, keys["nav-meetings"]))

    root = Path(__file__).resolve().parents[1]
    for name in ("src/core/agent_instructions/notes_core_tail.txt", "src/core/notes/tasks_redo_tail.txt"):
        prompt = (root / name).read_text()
        assert "Subtasks are usually empty" in prompt
        assert "Never split one action into steps yourself" in prompt
        assert "At most 4" in prompt
        assert "8 words" not in prompt
    with tempfile.TemporaryDirectory(prefix="review-fixture-", dir=root / "target") as temp:
        subprocess.run([sys.executable, str(root / "tools/prepare_tasks_v2_fixture.py"), temp], check=True, capture_output=True)
        todos = json.loads((Path(temp) / "tasks-v2-meetings/1800000000000-1/english-summary.json").read_text())["todo"]
        assert sum(bool(todo.get("subtasks")) for todo in todos) <= 1
        assert max(len(todo.get("subtasks", [])) for todo in todos) <= 2
    print(f"dev8 review UI: PASS ({len(index)} offscreen trees, both themes; six review fixes and profile-editor navigation)")


if __name__ == "__main__":
    check(Path(sys.argv[1]))
