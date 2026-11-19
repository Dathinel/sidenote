"""Check the MCP Settings list using isolated offscreen renders."""
import json
import sys
from pathlib import Path
import render_settings_build as renderer


def main():
    renderer.OUT = Path(sys.argv[1] if len(sys.argv) > 1 else r"E:/Personal-Projects/reviews/hex-meetings-2026-11-19/mcp-oneclick-simplify/renders")
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    count = 0
    for state in ("added", "mixed", "none"):
        for width in (1280, 390):
            for theme in ("light", "dark"):
                name = "mcp-" + state
                renderer.render((name, "settings-" + name, width, theme, 2200, None))
                tree = json.loads((renderer.OUT / f"settings-{name}-{width}-{theme}.json").read_text(encoding="utf-8-sig"))
                controls = {e["key"]: e for e in tree["elements"] if e.get("key")}
                texts = [e.get("text", "") for e in tree["elements"]]
                # Claude Code and Codex keep Add; the three JSON apps are copy and paste only.
                status = [c for k, c in controls.items() if k.startswith("connect-") and k.endswith("-add")]
                copy = [c for k, c in controls.items() if k.startswith("connect-") and k.endswith("-copy")]
                folder = [c for k, c in controls.items() if k.startswith("connect-") and k.endswith("-folder")]
                expected = {"added": (5, 0), "mixed": (2, 2), "none": (5, 0)}[state]
                assert (len(status), len(copy)) == expected and len(folder) == len(copy), (state, width, len(status), len(copy), len(folder))
                for control in status + copy + folder:
                    x, y, w, h = control["rect"]
                    assert 0 <= x < x+w <= width and 0 <= y < y+h <= 2200, (state, width, control)
                assert sum("Paste this inside" in t for t in texts) == len(copy), (state, width)
                assert sum("If the file is empty, use:" in t for t in texts) == len(copy), (state, width)
                assert all(c.get("focusable") and c.get("label") == "Copy config" for c in copy)
                assert all(c.get("focusable") and c.get("label") == "Open config folder" for c in folder)
                if state == "added": assert all(c.get("label") == "Added ✓" for c in status)
                if state == "none": assert all(c.get("label") == "Not installed" for c in status)
                if state == "mixed": assert sum(bool(c.get("focusable")) for c in status) == 1
                assert controls["connect-all"]["focusable"] == (state == "mixed")
                count += 1
    print(f"PASS: {count} MCP Settings renders, status labels, actions and viewport bounds")


if __name__ == "__main__":
    main()
