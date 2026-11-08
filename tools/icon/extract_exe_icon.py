"""Read the icon resources out of a built exe and save them as PNGs, to prove the icon is embedded.

    uv run --with pillow python tools/icon/extract_exe_icon.py target/release/hex-meetings.exe artifacts/icon

Parses the PE resource directory directly (RT_GROUP_ICON 14 and RT_ICON 3), so it needs no
Windows API, window or focus. Writes from-exe.png (the 256 px image) and from-exe-<size>.png.
"""
import io
import struct
import sys
from pathlib import Path

from PIL import Image

RT_ICON, RT_GROUP_ICON = 3, 14


class Pe:
    def __init__(self, data: bytes):
        self.d = data
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        assert data[pe : pe + 4] == b"PE\0\0", "not a PE file"
        nsec = struct.unpack_from("<H", data, pe + 6)[0]
        opt_size = struct.unpack_from("<H", data, pe + 20)[0]
        opt = pe + 24
        magic = struct.unpack_from("<H", data, opt)[0]
        dd = opt + (112 if magic == 0x20B else 96)
        self.res_rva = struct.unpack_from("<I", data, dd + 16)[0]
        self.secs = []
        sec = opt + opt_size
        for i in range(nsec):
            vsize, va, rsize, raw = struct.unpack_from("<IIII", data, sec + 40 * i + 8)
            self.secs.append((va, max(vsize, rsize), raw))
        assert self.res_rva, "exe has no resource section"

    def off(self, rva: int) -> int:
        for va, size, raw in self.secs:
            if va <= rva < va + size:
                return rva - va + raw
        raise ValueError(f"rva {rva:#x} outside sections")

    def entries(self, base: int, node: int):
        """(name_or_id, is_dir, target_offset_in_resource_block) of one resource directory."""
        o = self.off(base) + node
        n = sum(struct.unpack_from("<HH", self.d, o + 12))
        for i in range(n):
            name, target = struct.unpack_from("<II", self.d, o + 16 + 8 * i)
            yield name & 0x7FFFFFFF, bool(target & 0x80000000), target & 0x7FFFFFFF

    def leaf(self, base: int, node: int) -> bytes:
        rva, size = struct.unpack_from("<II", self.d, self.off(base) + node)
        o = self.off(rva)
        return self.d[o : o + size]

    def first_leaf_of_type(self, kind: int, ident: int | None = None) -> dict[int, bytes]:
        base = self.res_rva
        out = {}
        for tid, is_dir, tnode in self.entries(base, 0):
            if tid != kind or not is_dir:
                continue
            for rid, rdir, rnode in self.entries(base, tnode):
                if ident is not None and rid != ident:
                    continue
                if not rdir:
                    continue
                for _lang, ldir, lnode in self.entries(base, rnode):
                    out[rid] = self.leaf(base, lnode) if not ldir else b""
                    break
        return out


def main(exe: str, out_dir: str) -> None:
    pe = Pe(Path(exe).read_bytes())
    groups = pe.first_leaf_of_type(RT_GROUP_ICON)
    assert groups, "no RT_GROUP_ICON in the exe"
    first_id = min(groups)  # the lowest ID is the icon Explorer and the taskbar show
    group = groups[first_id]
    _, _, count = struct.unpack_from("<HHH", group, 0)
    icons = pe.first_leaf_of_type(RT_ICON)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    biggest = None
    print(f"group icon id {first_id}: {count} images")
    for i in range(count):
        w, h, _c, _r, _planes, bits, size, rid = struct.unpack_from("<BBBBHHIH", group, 6 + 14 * i)
        w, h = w or 256, h or 256
        blob = icons[rid]
        img = Image.open(io.BytesIO(blob)) if blob[:4] == b"\x89PNG" else None
        assert img is not None, f"{w}x{h} image is not PNG-in-ICO"
        img = img.convert("RGBA")
        assert img.size == (w, h), (img.size, w, h)
        img.save(out / f"from-exe-{w}.png")
        print(f"  {w}x{h}  {len(blob)} bytes")
        if biggest is None or w > biggest[0]:
            biggest = (w, img)
    biggest[1].save(out / "from-exe.png")
    print(f"wrote {out / 'from-exe.png'}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
