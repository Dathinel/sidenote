"""Exercise the seven Hex section controls offscreen, on fake data, with no desktop input.

Run after building sidenote.exe: python tools/prove_hex_detect.py --phase before|after --out DIR
"""
import argparse
import json
import pathlib
import shutil

import render_settings_build as renderer

TABS = ("Engine", "Shortcuts", "Dictation", "Speech model", "Compatibility", "Connection", "Hex Labs")
# (label, scene, words that must appear, Open Hex offered)
# The highlighted control is not checked: the page scroll-spy picks the last group at the page
# bottom, so a short page highlights a later group than the one tapped.
CASES = (
    ("running", "settings-hex", (), False),
    ("update", "settings-hex-update", (), False),
    ("stopped", "settings-hex-installed", ("Hex not running",), True),
    ("stale", "settings-hex-stale", ("Could not confirm Hex recording status",), False),
)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", choices=("before", "after"), required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    # The shared renderer reports paths relative to the repository, so render inside it, then copy.
    renderer.OUT = renderer.ROOT / "artifacts" / f"hex-detect-{args.phase}"
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    try:
        run(args)
    finally:
        shutil.copytree(renderer.OUT, args.out / args.phase, dirs_exist_ok=True)


def run(args):
    failures = []
    count = 0
    for state, scene, words, offers_open in CASES:
        for tab, title in enumerate(TABS):
            name = f"{state}-{tab}"
            renderer.render((name, scene, 1280, "dark", 900, [renderer.tap(f"settings-jump-{tab}")]))
            tree = json.loads((renderer.OUT / f"settings-{name}-1280-dark.json").read_text(encoding="utf-8-sig"))
            nodes = {node.get("key"): node for node in tree["elements"] if node.get("key")}
            target = nodes.get(f"settings-group-{tab}")
            visible = target and target["rect"][1] < 900 and sum(target["rect"][1::2]) > 0
            if not visible:
                failures.append(f"{state}: {title} has no visible destination")
            text = [node.get("text", "") for node in tree["elements"]]
            for phrase in words:
                if not any(phrase in line for line in text):
                    failures.append(f"{state}: {title} lacks the words '{phrase}'")
            if ("hex-setup-open" in nodes) != offers_open:
                failures.append(f"{state}: {title} Open Hex offered={'hex-setup-open' in nodes}, want {offers_open}")
            count += 1
    print(f"COVERED {count} OF {len(CASES) * len(TABS)} section switches", flush=True)
    assert not failures, "\n".join(failures)
    print("PASS: all seven Hex sections reachable in running, update, stopped and stale states", flush=True)


if __name__ == "__main__":
    main()
