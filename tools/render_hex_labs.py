"""Offscreen Hex Labs proofs using synthetic fixtures and the existing renderer."""
import argparse
import json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import render_settings_build as renderer

ROOT = Path(__file__).resolve().parents[1]
renderer.OUT = ROOT / "artifacts/hex-labs"
renderer.STATE = ROOT / "target/hex-labs-render"
renderer.OUT.mkdir(parents=True, exist_ok=True)
def legacy():
    renderer.OUT = ROOT / "artifacts/hex-labs/legacy"
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
    args = parser.parse_args()
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
                ("labs-confirm", [renderer.tap("settings-jump-6", .3), renderer.tap("hex-lab-enable-subtitle-export", .7)]),
            ):
                jobs.append((name, "settings-hex", width, theme, 2400, events))
    with ThreadPoolExecutor(max_workers=2) as pool:
        paths = list(pool.map(renderer.render, jobs))
    for path in paths:
        tree = json.loads((ROOT / path).with_suffix(".json").read_text(encoding="utf-8-sig"))
        elements = tree["elements"]
        assert any(e.get("key") == "hex-lab-future-synthetic-lab" for e in elements), path
        assert any(e.get("text") == "New in Hex, not reviewed by Sidenote yet" for e in elements), path
        keys = {e.get("key") for e in elements}
        for lab, expected in {"live-chapters": "On", "subtitle-export": "Off", "api-timestamps": "Unknown", "future-synthetic-lab": "Off"}.items():
            state = next(e for e in elements if e.get("key") == f"hex-lab-state-{lab}")
            assert state.get("text") == expected, (path, lab, state)
        assert "hex-lab-enable-future-synthetic-lab" not in keys, path
        assert "hex-lab-enable-api-timestamps" not in keys, path
        assert "hex-lab-enable-live-chapters" not in keys, path
        if "labs-confirm" in path:
            assert "hex-lab-confirm-dialog" in keys, path
            assert any(e.get("text") == "Enable Subtitle export in Hex?" for e in elements), path
        else:
            assert "hex-lab-confirm-dialog" not in keys, path
        assert not tree["warnings"], (path, tree["warnings"])
    (renderer.OUT / "render-manifest.json").write_text(json.dumps(paths, indent=2) + "\n")
    print(f"PASS: {len(paths)} isolated Labs renders")
if __name__ == "__main__":
    main()
