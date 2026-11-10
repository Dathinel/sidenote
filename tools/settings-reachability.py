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
    for tree in trees:
        assert not json.loads(tree.read_text(encoding="utf-8-sig"))["warnings"], f"Render warnings in {tree}"
    all_nodes = [node for tree in trees for node in nodes(json.loads(tree.read_text(encoding="utf-8-sig")))]
    keys = {node["key"] for node in all_nodes if isinstance(node.get("key"), str)}
    missing = {key: [control for control in controls if control not in keys]
               for key, controls in manifest["keys"].items()}
    missing = {key: controls for key, controls in missing.items() if controls}
    assert not missing, f"Unreachable legacy settings: {missing}"
    for name in ("General", "Meetings", "Prompt library", "People", "Hex and dictation", "AI helpers", "Appearance and startup", "Sync", "Updates"):
        assert f"settings-page-{name}" in keys, name
    search = json.loads((folder / "settings-search-1280-light.json").read_text(encoding="utf-8-sig"))
    search_nodes = list(nodes(search))
    assert any(node.get("key") == "hs-meetings.silence_threshold" and node.get("rect", [0,-1,0,0])[1] >= 108 and node.get("rect", [0,0,0,0])[1] + node.get("rect", [0,0,0,0])[3] <= 800 and node.get("fill") == "#fff3c4ff" for node in search_nodes), "Search did not reveal the Advanced row"
    assert any(node.get("key") == "settings-advanced" and node.get("expanded") for node in search_nodes), "Search did not open Advanced"
    print(f"PASS: all {len(manifest['keys'])} legacy setting keys reachable; nine pages and search reveal verified in {len(trees)} render trees.")


if __name__ == "__main__":
    main()
