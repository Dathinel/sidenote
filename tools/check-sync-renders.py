"""Check the relay/sync matrix from render_ui.ps1 -SyncProofs."""
import hashlib
import json
from pathlib import Path
import struct
import sys

folder = Path(sys.argv[1])
scenes = (
    "relay-preview", "relay-timeout", "relay-unconfirmed", "relay-operational",
    "relay-unreachable", "relay-wrong-address", "relay-update-required",
    "relay-pair-code", "relay-pair-confirm", "relay-paired",
    "relay-removal-unavailable", "relay-conflict", "relay-shared-conflict",
    "relay-shared-resolved", "relay-entry", "relay-conflict-resolved",
    "sync-memory-merged", "sync-task-merged", "sync-task-kept",
    "sync-arrival-draft", "sync-footer-now", "sync-footer-age", "sync-footer-off",
    "settings-sync", "settings-sync-expanded",
)
expected = {(f"{scene}-{width}", theme) for scene in scenes
            for width in (1280, 390) for theme in ("light", "dark")}
index = json.loads((folder / "render-index.json").read_text(encoding="utf-8-sig"))
assert len(index) == len(expected), "Incomplete render matrix"
assert {(row["Scene"], row["Theme"]) for row in index} == expected
proofs = []
for row in index:
    stem = f'{row["Scene"]}-{row["Theme"]}'
    png = folder / f"{stem}.png"
    data = png.read_bytes()
    width = int(row["Size"].split("x")[0])
    assert data[:8] == b"\x89PNG\r\n\x1a\n", stem
    assert struct.unpack(">II", data[16:24]) == (width, 800), stem
    frame = json.loads((folder / f"{stem}.json").read_text(encoding="utf-8-sig"))
    assert not frame["warnings"], (stem, frame["warnings"])
    elements = frame["elements"]
    keys = {node["key"]: node for node in elements if "key" in node}
    if row["Scene"].startswith(("relay-", "settings-sync")):
        assert "settings-title" in keys, stem
        assert not any(node.get("label") == "Breadcrumb" for node in elements), stem
    if row["Scene"].startswith("settings-sync"):
        assert "relay-preview" in keys, stem
        assert "relay-pair-card" not in keys, "Relay must default off"
    if row["Scene"].startswith("relay-unconfirmed-"):
        row_node = keys["relay-paired-device-1"]
        device = keys["relay-device-1"]
        notice = keys["relay-unconfirmed-notice-1"]
        assert notice["path"].startswith(device["path"] + "/"), stem
        assert row_node["path"].startswith(device["path"] + "/"), stem
        assert notice["rect"][1] >= row_node["rect"][1] + row_node["rect"][3] - 0.5, stem
        assert "relay-unconfirmed-notice-0" not in keys, stem
        assert "relay-unconfirmed-notice" not in keys, stem
    proofs.append({"frame": stem, "sha256": hashlib.sha256(data).hexdigest()})
(folder / "render-evidence.json").write_text(json.dumps(proofs, indent=2))
print(f"PASS: {len(proofs)} relay/sync PNGs and trees, 1280/390 px, light/dark; default off and device warning placement checked")
