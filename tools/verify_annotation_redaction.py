"""Exercise redaction gestures offscreen and compare every source pixel at two UI sizes."""
from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
from PIL import Image

repo = Path(__file__).resolve().parents[1]
exe = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).resolve()
fixture = repo / "tests/fixtures/annotations/synthetic.scrawl/frames/frame-01.png"
source_bytes = fixture.read_bytes()
source = Image.open(fixture).convert("RGBA")
results = []
for width in (1280, 640):
    tree = json.loads((out / f"annotation-review-{width}-light.json").read_text(encoding="utf-8-sig"))
    rect = next(n["rect"] for n in tree["elements"] if n.get("key") == "review-image-1")
    x, y, w, h = rect
    state_parent = (repo / "target/annotation-drag-state").resolve()
    state_parent.mkdir(parents=True, exist_ok=True)
    state = Path(tempfile.mkdtemp(prefix=f"{width}-", dir=state_parent)).resolve()
    assert state.parent == state_parent
    for sub in ("appdata", "localappdata", "peers"):
        (state / sub).mkdir()
    (state / "hex-fixture.json").write_bytes((repo / "tools/ui-fixtures/hex.json").read_bytes())
    events = []
    if width == 640:
        events.append({"t": .2, "type": "tap", "key": "sidebar-toggle"})
    events += [
        {"t": .5, "type": "tap", "key": "review-undo-1"},
        {"t": .8, "type": "drag", "x": x+w*.25, "y": y+h*.125, "dx": w*.25, "dy": h*.125, "dur": .3},
        {"t": 2.0, "type": "mark", "name": "end"},
    ]
    script = out / f"annotation-redact-{width}.json"
    script.write_text(json.dumps({"events": events}, indent=2))
    env = dict(os.environ, APPDATA=str(state / "appdata"), LOCALAPPDATA=str(state / "localappdata"), SIDENOTE_STATE_DIR=str(state))
    args = [str(exe), "--data-dir", str(state), "--peers-dir", str(state/"peers"), "--meetings-dir", str(repo/"tools/ui-fixtures/meetings"), "--snap", str(out/f"annotation-redact-{width}.png"), "--tree", str(out/f"annotation-redact-{width}-tree.json"), "--scene", "annotation-review", "--theme", "light", "--size", f"{width}x800", "--script", str(script), "--at", "mark:end"]
    with (out/f"annotation-redact-{width}.log").open("w") as log:
        subprocess.run(args, cwd=repo, env=env, stdout=log, stderr=log, check=True, timeout=45, creationflags=subprocess.CREATE_NO_WINDOW)
    manifests = []
    for path in state.rglob("approval.json"):
        approval = json.loads(path.read_text())
        masks = approval["review"][0]["masks"]
        if len(masks) == 1 and all(abs(a-b)<.00001 for a,b in zip(masks[0], [.25,.125,.25,.125])):
            manifests.append((path, approval))
    assert len(manifests) == 1, (width, "The actual drag did not bake the expected normalized box", [json.loads(p.read_text())["review"] for p in state.rglob("approval.json")])
    path, approval = manifests[0]
    assert approval["approved_at_ms"] == 0
    sent = Image.open(path.parent/approval["frames"][0]["original"]).convert("RGBA")
    assert sent.size == source.size
    for yy in range(source.height):
        for xx in range(source.width):
            expected = (0,0,0,255) if 120 <= xx < 240 and 33 <= yy < 68 else source.getpixel((xx,yy))
            assert sent.getpixel((xx,yy)) == expected, (width,xx,yy)
    assert fixture.read_bytes() == source_bytes
    assert not list(state.rglob("note.json"))
    final_tree = json.loads((out/f"annotation-redact-{width}-tree.json").read_text(encoding="utf-8-sig"))
    assert not final_tree["warnings"], final_tree["warnings"]
    results.append({"width":width,"display_rect":rect,"normalized_box":approval["review"][0]["masks"][0],"source_pixel_box":[120,33,240,68],"all_pixels_checked":source.width*source.height,"source_unchanged":True,"approved_at_ms":0})
(out/"redaction-drag-proof.json").write_text(json.dumps(results,indent=2))
print("PASS: actual offscreen drags at 1280 and 640 bake identical exact source pixels; all outside pixels and originals unchanged; no approval or AI call.")
