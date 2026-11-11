"""Check the requested offscreen frames and the real conflict-button proof."""
import argparse
import json
import re
import struct
from pathlib import Path


def strings(value, field):
    if isinstance(value, dict):
        if isinstance(value.get(field), str):
            yield value[field]
        for child in value.values():
            yield from strings(child, field)
    elif isinstance(value, list):
        for child in value:
            yield from strings(child, field)


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def frame(folder, scene, theme):
    return json.loads((folder / f"relay-{scene}-{theme}.json").read_text(encoding="utf-8"))


def element(tree, key):
    return next(item for item in tree["elements"] if item.get("key") == key)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frames", type=Path)
    parser.add_argument("proof", type=Path)
    parser.add_argument("--operations", type=Path)
    args = parser.parse_args()
    for theme in ("light", "dark"):
        for scene in ("preview", "pair-code", "pair-confirm", "paired", "conflict", "removal-unavailable"):
            path = args.frames / f"relay-{scene}-{theme}.png"
            header = path.read_bytes()[:24]
            assert header[:8] == b"\x89PNG\r\n\x1a\n", f"Invalid PNG: {path.name}"
            assert struct.unpack(">II", header[16:24]) == (1280, 800), path.name
        before = json.loads((args.proof / f"relay-conflict-{theme}.json").read_text(encoding="utf-8"))
        after = json.loads((args.proof / f"relay-conflict-resolved-{theme}.json").read_text(encoding="utf-8"))
        before_text = list(strings(before, "text"))
        after_text = list(strings(after, "text"))
        assert "Review the field guide \u00b7 Owner has 2 versions" in before_text, theme
        assert "Keep this" in before_text, theme
        assert not any(re.search(r"\d{4}-\d{2}-\d{2}T\d", text) for text in before_text), theme
        for device in ("DEMO-PC", "Travel laptop"):
            assert any(device in text and re.search(r"today, \d{2}:\d{2}", text) for text in before_text), theme
        assert sum("relay-use-version-" in key for key in strings(before, "key")) >= 2, theme
        assert not any("relay-use-version-" in key for key in strings(after, "key")), theme
        assert "Recent choices" in after_text, theme
        assert any("chose Morgan for owner" in text for text in after_text), theme
        for scene in ("preview", "paired", "entry", "removal-unavailable"):
            tree = frame(args.proof, scene, theme)
            text = list(strings(tree, "text"))
            assert "Tasks, teams, People labels, projects, Memory and your name use the relay. A backup stays on this PC.\nA lost or stolen device can't be removed in this preview. If that happens, turn off relay sync on all devices." in text, scene
            assert "This device" not in text, scene
            labs = element(tree, "relay-labs-card")["rect"]
            for key in ("relay-pair-card", "relay-peers-card"):
                other = element(tree, key)["rect"]
                assert abs(labs[0] - other[0]) < 0.5 and abs(labs[2] - other[2]) < 0.5, (scene, key)
            assert element(tree, "relay-preview")["fill"].endswith("00"), "row paint covers the shared border"
        shared_before = frame(args.proof, "shared-conflict", theme)
        shared_after = frame(args.proof, "shared-resolved", theme)
        shared_text = list(strings(shared_before, "text"))
        assert "Memory \u00b7 Text has 2 versions" in shared_text, theme
        for value in ("Ship the revised field guide", "Review the field guide with the team"):
            assert value in shared_text, theme
        assert not any("relay-use-version-" in key for key in strings(shared_after, "key")), theme
        assert "Recent choices" in list(strings(shared_after, "text")), theme
        preview = frame(args.proof, "preview", theme)
        assert not any(n.get("key") == "relay-join-code" for n in preview["elements"]), "entry is not folded"
        host = element(preview, "relay-host")
        assert host["fill"] == ("#1f2328ff" if theme == "light" else "#ecececff"), "host action must use dev16 quiet ink primary paint"
        entry = frame(args.proof, "entry", theme)
        assert element(entry, "relay-join-code"), "code link did not reveal the field"
        pair = next(n for n in nodes(entry["accessibility"]) if n.get("key") == "relay-join")
        assert "disabled" in pair.get("states", []), "empty code enabled pairing"
        assert "click" not in pair.get("actions", []) and "press" not in pair.get("actions", []), "disabled Pair has a handler"
        removal = frame(args.proof, "removal-unavailable", theme)
        assert "Removing a device is not available in this preview" in list(strings(removal,"text")), theme
        assert element(removal,"relay-paired-device-1"), theme
        assert not any(n.get("key", "").startswith("relay-remove-") for n in removal["elements"]), theme
    if args.operations:
        expected = {
            "operational": "Relay reachable. Both devices must be online to exchange changes.",
            "unreachable": "Relay unavailable. Check the address and your connection. Edits stay on this PC.",
            "wrong-address": "This address did not answer as a Sidenote relay. Check the address in Advanced.",
            "update-required": "Another device runs a newer Sidenote. Update this one to keep syncing.",
        }
        for theme in ("light", "dark"):
            for scene, message in expected.items():
                tree = frame(args.operations, scene, theme)
                text = list(strings(tree, "text"))
                assert message in text, (scene, theme)
                assert "Last relay contact" in text, (scene, theme)
                for key in ("relay-address-input", "relay-last-contact", "relay-contact-status"):
                    x, y, width, height = element(tree, key)["rect"]
                    assert 248 <= x < x + width <= 1280, (scene, key)
                    assert 58 <= y < y + height <= 800, (scene, key)
                assert not any(re.search(r"\d{4}-\d{2}-\d{2}T\d", value) for value in text), scene
                if scene == "wrong-address":
                    assert "Not yet contacted" in text, theme
                else:
                    assert any(re.search(r"\d{1,2} \w+ \d{4}, \d{2}:\d{2}", value) for value in text), scene
                if scene == "update-required":
                    assert "Relay changes are read-only. Folder sync keeps working." in text, theme
                    assert not any(n.get("key") == "relay-host" for n in tree["elements"]), theme
                png = args.operations / f"relay-{scene}-{theme}.png"
                assert struct.unpack(">II", png.read_bytes()[16:24]) == (1280, 800), png.name
    print("PASS: Sync frames, shared Memory choices, aligned cards, pairing/removal and optional operational states in both themes")


if __name__ == "__main__":
    main()
