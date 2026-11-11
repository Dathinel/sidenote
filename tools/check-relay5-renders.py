"""Check the slice 5 Sync notices and Advanced rows in offscreen frames."""
import json
from pathlib import Path
import struct
import sys


folder = Path(sys.argv[1])
scenes = ("preview", "operational", "unreachable", "wrong-address", "update-required")
for theme in ("light", "dark"):
    for scene in scenes:
        stem = f"relay-{scene}-{theme}"
        assert struct.unpack(">II", (folder / f"{stem}.png").read_bytes()[16:24]) == (1280, 800)
        frame = json.loads((folder / f"{stem}.json").read_text(encoding="utf-8"))
        elements = frame["elements"]
        by_key = {node["key"]: node for node in elements if "key" in node}
        assert not by_key["settings-title"]["path"].startswith(by_key["hex-scroll"]["path"] + "/"), "Sync uses the dev16 Settings header"
        assert not any(node.get("label") == "Breadcrumb" for node in elements), "Settings has no breadcrumb"
        text = [node["text"] for node in elements if "text" in node]
        assert "0 meetings from other devices." not in text, stem
        if scene in ("unreachable", "wrong-address", "update-required"):
            banner = by_key["relay-status-banner"]
            assert banner["role"] == "alert" and 57 <= banner["rect"][1] <= 59, stem
            icons = [node for node in elements if node.get("role") == "icon" and node.get("path", "").startswith(banner["path"] + "/")]
            assert len(icons) == 1 and icons[0]["rect"][2:] == [16.0, 16.0], stem
            assert by_key["relay-contact-status"]["text"] in text, stem
        else:
            assert "relay-status-banner" not in by_key, stem
        if scene != "preview":
            assert "Your relay's address, like wss://relay.example.com. Press Enter to save." in text, stem
            assert "Last time this PC reached the relay." in text, stem
            for key in ("relay-address", "relay-last-contact", "feature-Other devices", "this-pc-name"):
                row = by_key[key]
                assert not any(node.get("text") == "This PC" and node.get("path", "").startswith(row["path"] + "/") for node in elements), (stem, key)
print("PASS: 10 frames; pinned notices with 16 px icons, simplified help, no Advanced badges or loose zero count")
