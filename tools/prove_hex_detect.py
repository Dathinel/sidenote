"""Exercise Hex Settings section links offscreen with synthetic data."""

import argparse
import json
from pathlib import Path
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_settings_build as renderer


SECTIONS = ("Engine", "Shortcuts", "Dictation", "Speech model", "Compatibility", "Connection", "Hex Labs")
CASES = (
    ("settings-hex", None, None, False),
    ("settings-hex-update", "Hex will update to 0.0.0-dev-1085 when it restarts.", None, False),
    ("settings-hex-installed", None, "Hex is not running.", True),
    ("settings-hex-stale", None, "Could not confirm Hex recording status.", False),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True, help="output directory inside the repository")
    args = parser.parse_args()
    renderer.EXE = args.exe.resolve(strict=True)
    renderer.OUT = args.out.resolve()
    if not renderer.OUT.is_relative_to(renderer.ROOT):
        parser.error("--out must be inside the repository")
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    failures = []
    with tempfile.TemporaryDirectory(prefix="sidenote-hex-render-") as state:
        renderer.STATE = Path(state)
        for scene, update, reason, offers_open in CASES:
            for tab, title in enumerate(SECTIONS):
                name = f"{scene}-{tab}"
                events = [renderer.tap(f"settings-jump-{(tab + 1) % len(SECTIONS)}", .3),
                          renderer.tap(f"settings-jump-{tab}", .8)]
                renderer.render((name, scene, 1280, "dark", 900, events))
                tree = json.loads((renderer.OUT / f"settings-{name}-1280-dark.json").read_text(encoding="utf-8-sig"))
                nodes = {node["key"]: node for node in tree["elements"] if node.get("key")}
                target = nodes.get(f"settings-group-{tab}")
                viewport = nodes["hex-scroll"]["rect"]
                if not target or max(target["rect"][1], viewport[1]) >= min(
                    sum(target["rect"][1::2]), sum(viewport[1::2])
                ):
                    failures.append(f"{scene}: {title} has no visible destination")
                if not nodes[f"settings-jump-{tab}"].get("itemSelected"):
                    failures.append(f"{scene}: {title} did not become selected")
                if update and nodes.get("hex-pending-update", {}).get("text") != update:
                    failures.append(f"{scene}: {title} has no pending update notice")
                if reason:
                    status = nodes.get("hex-unavailable-reason", {})
                    if not (status.get("text", "").startswith(reason)
                            and 0 <= status["rect"][1] < sum(status["rect"][1::2]) <= 900):
                        failures.append(f"{scene}: {title} has no visible unavailable reason")
                if ("hex-setup-open" in nodes) != offers_open:
                    failures.append(f"{scene}: {title} has the wrong Open Hex action")
    assert not failures, "\n".join(failures)
    print(f"PASS: checked {len(CASES) * len(SECTIONS)} Hex Settings section switches")


if __name__ == "__main__":
    main()
