"""Check the invariant painted right cap in the synthetic 60 fps resize recording."""
import json
import pathlib
import statistics
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
output = root / "artifacts/floating-icon-spike"
video = output / "hybrid-resize.mkv"
info = json.loads(subprocess.check_output([
    "ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
    "stream=width,height,r_frame_rate", "-of", "json", str(video),
]))["streams"][0]
width, height = info["width"], info["height"]
pixels = subprocess.check_output([
    "ffmpeg", "-v", "error", "-i", str(video), "-f", "rawvideo",
    "-pix_fmt", "rgb24", "-",
])
frame_bytes = width * height * 3
assert len(pixels) % frame_bytes == 0
means = []
for frame in range(len(pixels) // frame_bytes):
    samples = [pixels[frame * frame_bytes + (y * width + x) * 3 + c]
               for y in range(height - 10, height - 6)
               for x in range(width - 18, width - 14) for c in range(3)]
    means.append([sum(samples[c::3]) / 16 for c in range(3)])
baseline = [statistics.median(p[c] for p in means[:30]) for c in range(3)]
deltas = [max(abs(p[c] - baseline[c]) for c in range(3)) for p in means]
bad = [i for i, delta in enumerate(deltas) if delta > 8]
result = {"frames": len(means), "frameRate": info["r_frame_rate"],
          "rightCapBaselineRGB": baseline, "maxChannelDeviation": max(deltas),
          "framesWithRightCapChange": len(bad), "firstBadFrames": bad[:20],
          "scope": "Paint continuity in a fixed 4x4 right-cap region; not a whole-window flicker guarantee"}
(output / "visual-continuity.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
assert not bad, "Painted cap changed during native resize"
