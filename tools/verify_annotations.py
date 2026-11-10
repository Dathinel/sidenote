"""Check all synthetic annotation renders and their evidence-to-frame geometry."""
import json
from pathlib import Path
import sys

from PIL import Image

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("artifacts/annotations")
fixture = json.loads((Path(__file__).resolve().parents[1] / "tests/fixtures/annotations/synthetic.scrawl/scrawl.json").read_text())
checked = 0
for scene in ("annotations-list", "annotation-capture", "annotations-empty", "annotation-transcript"):
    for width in (1280, 640):
        for theme in ("light", "dark"):
            name = f"{scene}-{width}-{theme}"
            tree = json.loads((root / f"{name}.json").read_text(encoding="utf-8-sig"))
            assert not tree["warnings"], (name, tree["warnings"])
            assert Image.open(root / f"{name}.png").size == (width, 800), name
            nodes = tree["elements"]
            keyed = {n["key"]: n for n in nodes if n.get("key")}
            texts = [n["text"] for n in nodes if n.get("text")]
            assert "annotations-pane" in keyed, name
            if width == 1280:
                assert "nav-annotations" in keyed, name
                assert keyed["nav-meetings"]["rect"][1] < keyed["nav-annotations"]["rect"][1], name
            else:
                assert "nav-scrim" not in keyed, (name, "Sidebar drawer covers the page")
            if scene == "annotations-list":
                assert "annotation-row-0" in keyed, name
                assert "Northwind Board" in texts, name
                assert any("12" in t and "words" in t and "3" in t and "marks" in t and "0:22" in t for t in texts), name
                title = next(n for n in nodes if n.get("text") == fixture["target"]["windowTitle"])
                assert title["rect"][3] <= 48.1, (name, title["rect"])
            elif scene == "annotations-empty":
                assert "No annotations yet" in texts, name
                assert any("In Hex, turn on Annotate" in t for t in texts), name
                assert not any(k.startswith("annotation-row-") for k in keyed), name
            elif scene == "annotation-transcript":
                assert "annotation-transcript" in keyed, name
                full = keyed["annotation-transcript-words-0"]["text"]
                assert full == " ".join(w["text"] for w in fixture["transcript"]["words"]), name
            else:
                assert "annotation-show-files" in keyed, name
                for number, mark in enumerate(fixture["marks"], 1):
                    image = keyed[f"annotation-image-{mark['frame']}"]["rect"]
                    box = keyed[f"annotation-box-{number}"]["rect"]
                    expected = [image[0]+mark["boxNorm"][0]*image[2], image[1]+mark["boxNorm"][1]*image[3], mark["boxNorm"][2]*image[2], mark["boxNorm"][3]*image[3]]
                    assert all(abs(a-b) < 1.5 for a,b in zip(box,expected)), (name, number, box, expected)
                    assert f"annotation-badge-{number}" in keyed, name
                    assert f"annotation-callout-{number}" in keyed, name
                    assert mark["description"] in texts, name
                assert keyed["annotation-frame-1"]["rect"][1] < keyed["annotation-frame-2"]["rect"][1], name
                assert "Loading frame image..." not in texts, name
            checked += 1
print(f"PASS: {checked} renders; both themes and sizes; three mapped marks, boxes, callouts, full transcript, list metadata and empty state.")
