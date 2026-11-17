"""Check the MCP Settings list using isolated offscreen renders."""
import json
from pathlib import Path
import render_settings_build as renderer


def main():
    renderer.OUT = Path(r"E:/Personal-Projects/reviews/hex-meetings-2026-11-17/mcp-oneclick")
    renderer.OUT.mkdir(parents=True, exist_ok=True)
    count = 0
    for state in ("added", "mixed", "none"):
        for width in (1280, 390):
            name = "mcp-" + state
            renderer.render((name, "settings-" + name, width, "light", 2200, None))
            tree = json.loads((renderer.OUT / f"settings-{name}-{width}-light.json").read_text(encoding="utf-8-sig"))
            controls = {e["key"]: e for e in tree["elements"] if e.get("key")}
            expected = 5 if state != "mixed" else 4
            rows = [c for k, c in controls.items() if k.startswith("connect-") and k.endswith("-add")]
            assert len(rows) == expected, (state, width, len(rows))
            for control in rows:
                x, y, w, h = control["rect"]
                assert 0 <= x < x+w <= width and 0 <= y < y+h <= 2200, (state, width, control)
            assert sum(bool(c.get("focusable")) for c in rows) == (2 if state == "mixed" else 0)
            if state == "added": assert all(c.get("label") == "Added ✓" for c in rows)
            if state == "none": assert all(c.get("label") == "Not installed" for c in rows)
            assert controls["connect-all"]["focusable"] == (state == "mixed")
            count += 1
    print(f"PASS: {count} MCP Settings renders, status labels, actions and viewport bounds")


if __name__ == "__main__":
    main()
