"""Rasterise the app icon SVGs, pack the multi-size .ico and render the concept preview sheet.

    uv run --with pillow --with resvg-py python tools/icon/build_icon.py [--concept NAME]

The ink icon uses dedicated drawings at 16, 20, 24, 32 and 40 px; larger sizes use the full SVG.
Older concepts fall back to their small SVG up to SMALL_MAX and tiny SVG at 16 px.
Outputs: assets/icon/png/<size>.png and assets/icon/meetings.ico for the chosen concept, and
artifacts/icon/preview.png (all concepts) plus preview-zoom.png (small sizes enlarged).
"""
import argparse
import io
import struct
from pathlib import Path

import resvg_py
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
ICON_DIR = ROOT / "assets" / "icon"
OUT_DIR = ROOT / "artifacts" / "icon"
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SMALL_MAX = 32
TINY_MAX = 16  # pixel-grid aligned drawing, where one exists
SIZE_SUFFIXES = {16: "tiny", 20: "20", 24: "24", 32: "small", 40: "40"}
CONCEPTS = {
    "sidenote-d3c-final-ink": "Sidenote  Ink",
    "concept-a-summary": "A  Summary bubble",
    "concept-b-check": "B  Bubble with check",
    "concept-c-card": "C  Meeting card",
}
DEFAULT_CONCEPT = "sidenote-d3c-final-ink"
LIGHT_BG, DARK_BG = (243, 243, 243), (32, 32, 32)


def render(concept: str, size: int) -> Image.Image:
    name = f"{concept}.svg"
    suffix = SIZE_SUFFIXES.get(size)
    if suffix and (ICON_DIR / f"{concept}-{suffix}.svg").exists():
        name = f"{concept}-{suffix}.svg"
    elif size <= TINY_MAX and (ICON_DIR / f"{concept}-tiny.svg").exists():
        name = f"{concept}-tiny.svg"
    elif size <= SMALL_MAX:
        name = f"{concept}-small.svg"
    png = resvg_py.svg_to_bytes(svg_path=str(ICON_DIR / name), width=size, height=size)
    return Image.open(io.BytesIO(bytes(png))).convert("RGBA")


def png_bytes(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def write_ico(path: Path, images: dict[int, Image.Image]) -> None:
    """PNG-in-ICO, one entry per size (Windows Vista and later). Pillow would resample one image."""
    sizes = sorted(images)
    blobs = [png_bytes(images[s]) for s in sizes]
    out = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    offset = 6 + 16 * len(sizes)
    for s, blob in zip(sizes, blobs):
        dim = 0 if s >= 256 else s
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    for blob in blobs:
        out += blob
    path.write_bytes(bytes(out))


def font(size: int) -> ImageFont.ImageFont:
    try:
        return ImageFont.truetype("segoeui.ttf", size)
    except OSError:
        return ImageFont.load_default(size)


def sheet(concepts: list[str], sizes: list[int], zoom: int = 1, title: str = "") -> Image.Image:
    """Concept rows by size columns, once on a light panel and once on a dark one."""
    pad, label_w, head = 20, 190, 44
    cols = [s * zoom for s in sizes]
    row_h = max(cols) + pad * 2
    panel_w = label_w + sum(c + pad * 2 for c in cols)
    img = Image.new("RGB", (panel_w * 2, head + row_h * len(concepts)), LIGHT_BG)
    d = ImageDraw.Draw(img)
    dark = Image.new("RGB", (panel_w, img.height - 0), DARK_BG)
    img.paste(dark, (panel_w, 0))
    for p, (bg, fg) in enumerate([(LIGHT_BG, (40, 40, 40)), (DARK_BG, (225, 225, 225))]):
        x0 = p * panel_w
        d.text((x0 + pad, 10), title or ("Light taskbar" if p == 0 else "Dark taskbar"), font=font(20), fill=fg)
        x = x0 + label_w
        for s, c in zip(sizes, cols):
            d.text((x + pad, head - 22), f"{s}px", font=font(14), fill=fg)
            x += c + pad * 2
        for r, name in enumerate(concepts):
            y = head + r * row_h
            if p == 0:
                d.text((x0 + pad, y + row_h // 2 - 10), CONCEPTS.get(name, name), font=font(16), fill=fg)
            x = x0 + label_w
            for s, c in zip(sizes, cols):
                tile = render(name, s)
                if zoom > 1:
                    tile = tile.resize((c, c), Image.NEAREST)
                img.paste(tile, (x + pad, y + (row_h - c) // 2), tile)
                x += c + pad * 2
    return img


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--concept", default=DEFAULT_CONCEPT, choices=list(CONCEPTS))
    args = ap.parse_args()

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (ICON_DIR / "png").mkdir(exist_ok=True)
    images = {s: render(args.concept, s) for s in SIZES}
    for s, im in images.items():
        im.save(ICON_DIR / "png" / f"{s}.png", optimize=True)
    write_ico(ICON_DIR / "meetings.ico", images)

    names = list(CONCEPTS)
    sheet(names, [16, 32, 48, 256]).save(OUT_DIR / "preview.png")
    sheet(names, [16, 20, 24, 32], zoom=6, title="Small sizes, 6x nearest").save(OUT_DIR / "preview-zoom.png")
    print(f"wrote {ICON_DIR / 'meetings.ico'} ({len(SIZES)} sizes) from {args.concept}")


if __name__ == "__main__":
    main()
