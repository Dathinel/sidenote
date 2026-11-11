"""Offscreen Hex Labs proofs using synthetic fixtures and the existing renderer."""
import argparse
import json
import struct
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import render_settings_build as renderer

ROOT = Path(__file__).resolve().parents[1]
renderer.OUT = ROOT / "artifacts/hex-labs"
renderer.STATE = ROOT / "target/hex-labs-render"
renderer.OUT.mkdir(parents=True, exist_ok=True)
def legacy():
    renderer.OUT = renderer.OUT / "legacy"
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    jobs = [(name, scene, 1280, "light", height, None) for name, scene, height in renderer.PAGES]
    for name, scene, events in [
        ("recording-expanded", "settings-recording-expanded", None),
        ("summaries-expanded", "settings-summaries-expanded", None),
        ("helpers-expanded", "settings-helpers-expanded", None),
        ("speech-expanded", "settings-speech-expanded", None),
        ("hex-scrolled", "settings-hex-expanded", [renderer.tap("settings-jump-5")]),
        ("privacy-expanded", "settings-privacy", [renderer.tap("notes-privacy-details")]),
        ("diagnostics-expanded", "settings-diagnostics", None),
        ("sync-expanded", "settings-sync-expanded", None),
        ("people-editor", "people-team-editor", [renderer.tap("team-advanced")]),
    ]:
        jobs.append((name, scene, 1280, "light", 2200, events))
    events = json.loads((ROOT / "tools/ui-scripts/settings-search.json").read_text())["events"][:-1]
    jobs.append(("search", "settings-search", 1280, "light", 800, events))
    with ThreadPoolExecutor(max_workers=2) as pool:
        paths = list(pool.map(renderer.render, jobs))
    (renderer.OUT / "render-manifest.json").write_text(json.dumps(paths, indent=2) + "\n")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--legacy", action="store_true")
    parser.add_argument("--exe", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/hex-labs")
    args = parser.parse_args()
    renderer.OUT = args.out.resolve()
    renderer.STATE = ROOT / "target/hex-labs-render" / renderer.OUT.name
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    if args.exe:
        renderer.EXE = args.exe.resolve()
    if args.legacy:
        legacy()
        return
    jobs = []
    for theme in ("light", "dark"):
        for width in (1280, 390):
            for name, events in (
                ("labs", [renderer.tap("settings-jump-6", .3)]),
                ("labs-confirm", [renderer.tap("settings-jump-6", .3), renderer.tap("hex-lab-enable-subtitle-export", .7), renderer.tap("settings-jump-6", 1.1)]),
            ):
                jobs.append((name, "settings-hex", width, theme, 1800, events))
    with ThreadPoolExecutor(max_workers=2) as pool:
        paths = list(pool.map(renderer.render, jobs))
    for path in paths:
        tree = json.loads((ROOT / path).with_suffix(".json").read_text(encoding="utf-8-sig"))
        elements = tree["elements"]
        assert any(e.get("key") == "hex-lab-future-synthetic-lab" for e in elements), path
        assert any(e.get("text") == "New in Hex, not reviewed by Sidenote yet" for e in elements), path
        keys = {e.get("key") for e in elements}
        for lab, expected in {"live-chapters": "On", "subtitle-export": "Off", "annotate": "On", "meetings": "On", "future-synthetic-lab": "Off", "api-processing": "On"}.items():
            state = next(e for e in elements if e.get("key") == f"hex-lab-state-{lab}")
            assert state.get("text") == expected, (path, lab, state)
        with (ROOT / path).open("rb") as png:
            width, height = struct.unpack("!II", png.read(24)[16:24])
        assert width in (1280, 390), path
        for group in ("affects", "can-use", "new"):
            key = f"hex-labs-group-{group}"
            assert key in keys, path
            x, y, w, h = next(e["rect"] for e in elements if e.get("key") == key)
            assert x >= 0 and y >= 0 and x+w <= width+1 and y+h <= height+1, (path, key, [x, y, w, h])
        for name in ("Affects Sidenote", "Sidenote can use", "New in Hex", "Meetings (beta)", "Annotate", "New Hex experiment", "Unload model when idle", "Switch model with keyboard language", "Process API transcripts"):
            assert any(e.get("text") == name for e in elements), (path, name)
        assert "hex-lab-api-timestamps" not in keys, path
        assert "hex-lab-vocabulary-biasing" not in keys, path
        assert "hex-labs-show-all" in keys, path
        assert sum(e.get("key") == "hex-labs-show-all" for e in elements) == 1, path
        assert any(e.get("text") == "Hex Labs that change what Sidenote sees." for e in elements), path
        assert sum(e.get("text") == "New in Hex, not reviewed by Sidenote yet" for e in elements) == 1, path
        for internal in ("Api Processing", "App Model Switching", "Unload Idle", "meetings", "gpu overlay"):
            assert not any(e.get("text") == internal for e in elements), (path, internal)
        assert "hex-lab-enable-future-synthetic-lab" not in keys, path
        assert "hex-lab-enable-api-timestamps" not in keys, path
        assert "hex-lab-enable-live-chapters" not in keys, path
        if "labs-confirm" in path:
            assert "hex-lab-confirm-dialog" in keys, path
            dialog = next(e["rect"] for e in elements if e.get("key") == "hex-lab-confirm-dialog")
            def inside(e):
                x, y, w, h = e.get("rect", [-1, -1, 0, 0])
                return x >= dialog[0] and y >= dialog[1] and x+w <= dialog[0]+dialog[2]+1 and y+h <= dialog[1]+dialog[3]+1
            copy = [e for e in elements if e.get("text") and inside(e)]
            expected = ["Turn on Subtitle export in Hex?", "Hex can export a meeting transcript as a subtitle file.", "You can turn it off in Hex \u203a Labs."]
            positions = [next(e["rect"][1] for e in copy if e["text"] == text) for text in expected]
            assert all(a < b for a, b in zip(positions, positions[1:])), (path, positions)
            assert all(any(e["text"] == text for e in copy) for text in ("Cancel", "Turn on")), path
            assert not any("Consider user-initiated" in e.get("text", "") for e in copy), path
        else:
            assert "hex-lab-confirm-dialog" not in keys, path
        assert not tree["warnings"], (path, tree["warnings"])
    (renderer.OUT / "render-manifest.json").write_text(json.dumps(paths, indent=2) + "\n")
    print(f"PASS: {len(paths)} isolated Labs renders")
if __name__ == "__main__":
    main()
