"""Check the requested offscreen frames and the real conflict-button proof."""
import argparse
import json
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frames", type=Path)
    parser.add_argument("proof", type=Path)
    args = parser.parse_args()
    for theme in ("light", "dark"):
        for scene in ("preview", "pair-code", "pair-confirm", "paired", "conflict"):
            path = args.frames / f"relay-{scene}-{theme}.png"
            header = path.read_bytes()[:24]
            assert header[:8] == b"\x89PNG\r\n\x1a\n", f"Invalid PNG: {path.name}"
            assert struct.unpack(">II", header[16:24]) == (1280, 800), path.name
        before = json.loads((args.proof / f"relay-conflict-{theme}.json").read_text())
        after = json.loads((args.proof / f"relay-conflict-resolved-{theme}.json").read_text())
        before_text = list(strings(before, "text"))
        after_text = list(strings(after, "text"))
        assert any("2 versions" in text for text in before_text), theme
        for device in ("DEMO-PC", "Travel laptop"):
            assert any(device in text and "T" in text for text in before_text), theme
        assert sum("relay-use-version-" in key for key in strings(before, "key")) >= 2, theme
        assert not any("relay-use-version-" in key for key in strings(after, "key")), theme
        assert "Recent task choices" in after_text, theme
        assert any("chose Morgan for owner" in text for text in after_text), theme
    print("PASS: 10 frames at 1280x800; both themes resolve a conflict and record the choice")


if __name__ == "__main__":
    main()
