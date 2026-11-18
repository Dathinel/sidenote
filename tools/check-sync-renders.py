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
    "relay-devices-none", "relay-devices-mixed", "relay-devices-long",
    "relay-devices-unknown", "relay-devices-renamed", "relay-devices-twins",
    "relay-devices-rename", "relay-devices-rename-empty",
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
        assert keys["sync-devices-no-online"]["text"] == "Folder sync lists the meetings each device recorded. It does not show whether a device is online.", stem
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
    if row["Scene"].startswith("relay-devices-"):
        scene = row["Scene"].rsplit("-", 1)[0]
        for node in elements:
            if "rect" in node:
                assert node["rect"][0] + node["rect"][2] <= width + 1, (stem, node.get("key"), node["rect"])
        text = {k: node.get("text", "") for k, node in keys.items()}
        card = keys["relay-peers-card"]["rect"]
        for i in range(8):
            name = keys.get(f"relay-device-name-{i}")
            if not name:
                continue
            assert name["rect"][0] + name["rect"][2] <= card[0] + card[2] + 0.5, (stem, "name past card", i)
            status = keys.get(f"relay-presence-{i}") or keys.get(f"relay-this-device-{i}")
            if width <= 400:
                assert abs(status["rect"][0] - name["rect"][0]) < 1.0, (stem, "status not under name", i)
                assert status["rect"][1] >= name["rect"][1] + name["rect"][3] - 0.5, (stem, "status not below name", i)
            else:
                assert status["rect"][0] > name["rect"][0] + 100, (stem, "status not on the right", i)
        assert "relay-this-device-0" in keys and "relay-presence-0" not in keys, stem
        if scene == "relay-devices-none":
            assert "relay-no-peers" in keys, stem
        if scene == "relay-devices-mixed":
            assert text["relay-presence-1"] == "Online", stem
            assert text["relay-presence-2"].startswith("Offline, last seen yesterday, 18:40"), stem
            assert text["relay-presence-3"].startswith("Offline, last seen"), stem
            assert "relay-lost-notice-3" in keys and "relay-lost-notice-2" not in keys, stem
            assert "relay-no-peers" not in keys, stem
        if scene == "relay-devices-unknown":
            assert text["relay-presence-1"] == text["relay-presence-2"] == "Not known", stem
            assert not any(k.startswith("relay-lost-notice") for k in keys), stem
        if scene == "relay-devices-long":
            assert text["relay-presence-2"] == "Offline, not seen yet", stem
            assert "relay-unconfirmed-notice-3" in keys, stem
        if scene == "relay-devices-renamed":
            assert "device-name-input" in keys and "relay-paired-device-0" in keys, stem
        # These two run tools/ui-scripts/relay-rename*.json: tap the real field, type, press Enter.
        if scene == "relay-devices-rename":
            assert text["relay-device-name-0"] == "DEMO-PC 2", (stem, "the saved name did not reach the row")
            assert "device-name-error" not in keys, stem
        if scene == "relay-devices-rename-empty":
            assert text["relay-device-name-0"] == "DEMO-PC", (stem, "an empty name changed the row")
            assert text["device-name-error"] == "Use 1 to 80 characters, with no control characters.", stem
            assert keys["device-name-error"].get("role") == "alert", stem
        tags = {k: node.get("text", "") for k, node in keys.items() if k.startswith("relay-device-tag-")}
        if scene == "relay-devices-twins":
            assert text["relay-device-name-1"] == text["relay-device-name-2"] == "Laptop", stem
            assert text["relay-device-name-3"] == text["relay-device-name-4"], stem
            assert tags == {"relay-device-tag-1": "(a1b2)", "relay-device-tag-2": "(c3d4)",
                            "relay-device-tag-3": "(e5f6)", "relay-device-tag-4": "(0789)"}, (stem, tags)
            for i in range(1, 5):
                name, tag = keys[f"relay-device-name-{i}"]["rect"], keys[f"relay-device-tag-{i}"]["rect"]
                assert tag[0] >= name[0] + name[2] - 0.5, (stem, "tag not after name", i)
                assert abs(tag[1] - name[1]) < 4, (stem, "tag not on the name line", i)
                assert tag[0] + tag[2] <= card[0] + card[2] + 0.5, (stem, "tag past card", i)
        else:
            assert not tags, (stem, "only devices with the same name get a tag")
    proofs.append({"frame": stem, "sha256": hashlib.sha256(data).hexdigest()})
(folder / "render-evidence.json").write_text(json.dumps(proofs, indent=2))
print(f"PASS: {len(proofs)} relay/sync PNGs and trees, 1280/390 px, light/dark; default off and device warning placement checked")
