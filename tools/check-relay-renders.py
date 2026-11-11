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
    args = parser.parse_args()
    for theme in ("light", "dark"):
        for scene in ("preview", "pair-code", "pair-confirm", "paired", "conflict", "remove-confirm"):
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
        assert "Recent task choices" in after_text, theme
        assert any("chose Morgan for owner" in text for text in after_text), theme
        for scene in ("preview", "paired", "entry", "remove-confirm"):
            tree = frame(args.proof, scene, theme)
            text = list(strings(tree, "text"))
            assert "Only tasks you add by hand use the relay. A backup stays on this PC." in text, scene
            assert "This device" not in text, scene
            labs = element(tree, "relay-labs-card")["rect"]
            for key in ("relay-pair-card", "relay-peers-card"):
                other = element(tree, key)["rect"]
                assert abs(labs[0] - other[0]) < 0.5 and abs(labs[2] - other[2]) < 0.5, (scene, key)
            assert element(tree, "relay-preview")["fill"].endswith("00"), "row paint covers the shared border"
        preview = frame(args.proof, "preview", theme)
        assert not any(n.get("key") == "relay-join-code" for n in preview["elements"]), "entry is not folded"
        host = element(preview, "relay-host")
        assert host["fill"] == ("#2f5d8aff" if theme == "light" else "#8db5dcff"), "host action is not primary"
        entry = frame(args.proof, "entry", theme)
        assert element(entry, "relay-join-code"), "code link did not reveal the field"
        pair = next(n for n in nodes(entry["accessibility"]) if n.get("key") == "relay-join")
        assert "disabled" in pair.get("states", []), "empty code enabled pairing"
        assert "click" not in pair.get("actions", []) and "press" not in pair.get("actions", []), "disabled Pair has a handler"
        removal = frame(args.proof, "remove-confirm", theme)
        assert "Remove Travel laptop? It stops syncing. Your other devices will need to pair again." in list(strings(removal, "text")), theme
        assert element(removal, "relay-remove-confirm") and element(removal, "relay-remove-cancel"), theme
        cancelled = frame(args.proof, "remove-cancelled", theme)
        assert not any(n.get("key") == "relay-remove-prompt" for n in cancelled["elements"]), "Cancel kept the prompt"
        assert element(cancelled, "relay-paired-device-1"), "Cancel removed the device"
    print("PASS: 12 frames at 1280x800; aligned cards, folded guarded entry, removal/Cancel and conflict choice in both themes")


if __name__ == "__main__":
    main()
