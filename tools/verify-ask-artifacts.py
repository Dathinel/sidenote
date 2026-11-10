"""Check every PNG/tree pair in the Ask review artifact folder."""
import hashlib
import json
from pathlib import Path
import struct
import sys

root = Path(sys.argv[1])
binary = root.resolve().parents[1] / "target/x86_64-pc-windows-gnu/debug/sidenote.exe"
built = binary.stat().st_mtime
receipts = []
for png in sorted(root.rglob("*.png")):
    tree_path = png.with_suffix(".json")
    assert tree_path.exists(), png
    assert min(png.stat().st_mtime, tree_path.stat().st_mtime) >= built, (png, "stale capture")
    tree = json.loads(tree_path.read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (png, tree["warnings"])
    data = png.read_bytes()
    width, height = struct.unpack(">II", data[16:24])
    assert width in (640, 1280) and height == 800, (png, width, height)
    for node in tree["elements"]:
        for field in ("text", "label", "help"):
            assert "helper" not in str(node.get(field, "")).lower(), (png, node)
    receipts.append(dict(path=str(png.resolve()), width=width, height=height,
                         sha256=hashlib.sha256(data).hexdigest()))
assert len(receipts) == 138, len(receipts)
(root / "render-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
print(f"PASS: all {len(receipts)} PNG/tree pairs, sizes, warning-free layouts and visible agent wording; SHA-256 receipts recorded.")
