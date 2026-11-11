"""Verify every dev.9 setting key against the rendered Settings element trees."""
import json
from pathlib import Path
import sys


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def main():
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("artifacts/settings")
    manifest = json.loads((Path(__file__).parent / "ui-fixtures/settings-legacy-keys.json").read_text())
    trees = list(folder.glob("settings-*-1280-light.json"))
    assert trees, f"No render trees in {folder}"
    for tree in folder.glob("settings-*.json"):
        assert not json.loads(tree.read_text(encoding="utf-8-sig"))["warnings"], f"Render warnings in {tree}"
    all_nodes = [node for tree in trees for node in nodes(json.loads(tree.read_text(encoding="utf-8-sig")))]
    keys = {node["key"] for node in all_nodes if isinstance(node.get("key"), str)}
    missing = {key: [control for control in controls if control not in keys]
               for key, controls in manifest["keys"].items()}
    missing = {key: controls for key, controls in missing.items() if controls}
    assert not missing, f"Unreachable legacy settings: {missing}"
    for name in ("General", "Meetings", "Prompt library", "People", "Hex and dictation", "Models and agents", "Appearance and startup", "Sync", "Privacy and data", "Updates", "Diagnostics"):
        assert f"settings-page-{name}" in keys, name
    search = json.loads((folder / "settings-search-1280-light.json").read_text(encoding="utf-8-sig"))
    search_nodes = list(nodes(search))
    assert any(node.get("key") == "hs-meetings.silence_threshold" and node.get("rect", [0,-1,0,0])[1] >= 108 and node.get("rect", [0,0,0,0])[1] + node.get("rect", [0,0,0,0])[3] <= 800 and node.get("fill") == "#fff3c4ff" for node in search_nodes), "Search did not reveal the Advanced row"
    assert any(node.get("key") == "settings-advanced" and node.get("expanded") for node in search_nodes), "Search did not open Advanced"
    mapping=json.loads((Path(__file__).parent / "ui-fixtures/settings-search-map.json").read_text(encoding="utf-8"))
    assert len(mapping)==len(manifest["keys"])==49
    for item in mapping:
        assert item["key"] in manifest["keys"],item["key"]
        assert item["preserve_controls"]==manifest["keys"][item["key"]],item["key"]
    setup_states = {
        "hex-setup": ("Install Hex", "Sidenote needs Hex"),
        "hex-installing": ("Install Hex", "12 of 48 MB (25%)"),
        "hex-install-failed": ("Try again", "Could not download Hex"),
        "hex-installed": ("Open Hex", "Hex is installed"),
    }
    for theme in ("dark", "light"):
        for width in (1280, 390):
            for state, (label, message) in setup_states.items():
                path = folder / f"settings-{state}-{width}-{theme}.json"
                tree = json.loads(path.read_text(encoding="utf-8-sig"))
                controls = {n["key"]: n for n in tree["elements"] if n.get("key")}
                assert "hex-setup" in controls, path
                progress = controls["hex-install-progress"]
                assert message in progress["text"], path
                assert progress["role"] == ("alert" if state == "hex-install-failed" else "status"), path
                action = "hex-setup-open" if state == "hex-installed" else "hex-install"
                assert controls[action]["label"] == label, path
                for key in (action, "hex-download-page", "hex-install-recheck"):
                    control = controls[key]
                    assert control["focusable"] == (state != "hex-installing" or key == "hex-download-page"), (path, key)
                    x, y, w, h = control["rect"]
                    assert 0 <= x < x + w <= width and 0 <= y < y + h <= 900, (path, key)
            for state in ("hex-meetings-off", "hex-meetings-enable"):
                path = folder / f"settings-{state}-{width}-{theme}.json"
                tree = json.loads(path.read_text(encoding="utf-8-sig"))
                controls = {n["key"]: n for n in tree["elements"] if n.get("key")}
                assert "hex-meetings-setup" in controls, path
                assert "Settings > Labs > Meetings" in controls["hex-meetings-instructions"]["text"], path
                assert any(n.get("text") == "Turn on Meetings in Hex" for n in tree["elements"]), path
                assert any(n.get("text") == "Hex is running. Check its status here before recording." for n in tree["elements"]), path
                assert ("hex-meetings-enable" in controls) == (state == "hex-meetings-enable"), path
                actions = ["hex-meetings-recheck"] + (["hex-meetings-enable"] if state == "hex-meetings-enable" else [])
                for key in actions:
                    assert controls[key]["focusable"], (path, key)
                    x, y, w, h = controls[key]["rect"]
                    assert 0 <= x < x + w <= width and 0 <= y < y + h <= 900, (path, key)
    print(f"PASS: all {len(manifest['keys'])} legacy setting keys reachable; eleven pages and search reveal verified in {len(trees)} render trees.")
    print("PASS: all 16 Hex setup renders show the expected copy, controls, busy state and visible actions.")
    print("PASS: all 8 Meetings-off renders show instructions, re-check and only supported enable actions.")


if __name__ == "__main__":
    main()
