"""Run the real window against a synthetic copy of the meetings folder and check that a meeting
added while it runs appears without a restart.

Usage: python tools/verify_ui.py <exe> [--wait SECONDS]
Writes artifacts/ui/native-before.png and native-after.png (the window as Windows draws it).
Uses a temporary APPDATA, so the real meetings, ticks and Hex settings are never touched.
"""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from PIL import Image, ImageChops

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)  # capture whole physical pixels
except OSError:
    pass
exe = Path(sys.argv[1]).resolve()
wait = int(sys.argv[sys.argv.index("--wait") + 1]) if "--wait" in sys.argv else 45
root = Path(__file__).resolve().parent.parent
out = root / "artifacts" / "ui"
out.mkdir(parents=True, exist_ok=True)
real = root / "tools/ui-fixtures/meetings"
KEEP = ("english-summary.json", "manifest.json", "english-transcript.md")
FIRST, SECOND = "1800000000000-1", "1800000000001-1"

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
user32.GetWindowDC.argtypes = [wintypes.HWND]
user32.GetWindowDC.restype = wintypes.HDC
user32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
user32.PrintWindow.argtypes = [wintypes.HWND, wintypes.HDC, wintypes.UINT]
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
gdi32.CreateCompatibleDC.argtypes = [wintypes.HDC]
gdi32.CreateCompatibleDC.restype = wintypes.HDC
gdi32.CreateCompatibleBitmap.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
gdi32.SelectObject.argtypes = [wintypes.HDC, wintypes.HGDIOBJ]
gdi32.DeleteObject.argtypes = [wintypes.HGDIOBJ]
gdi32.DeleteDC.argtypes = [wintypes.HDC]


class BitmapInfoHeader(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("width", wintypes.LONG), ("height", wintypes.LONG),
                ("planes", wintypes.WORD), ("bits", wintypes.WORD), ("compression", wintypes.DWORD),
                ("image_size", wintypes.DWORD), ("xppm", wintypes.LONG), ("yppm", wintypes.LONG),
                ("used", wintypes.DWORD), ("important", wintypes.DWORD)]


gdi32.GetDIBits.argtypes = [wintypes.HDC, wintypes.HBITMAP, wintypes.UINT, wintypes.UINT,
                            ctypes.c_void_p, ctypes.POINTER(BitmapInfoHeader), wintypes.UINT]


def capture(hwnd, path):
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w, h = rect.right - rect.left, rect.bottom - rect.top
    src = user32.GetWindowDC(hwnd)
    dc = gdi32.CreateCompatibleDC(src)
    bitmap = gdi32.CreateCompatibleBitmap(src, w, h)
    gdi32.SelectObject(dc, bitmap)
    user32.PrintWindow(hwnd, dc, 2)  # PW_RENDERFULLCONTENT
    header = BitmapInfoHeader(ctypes.sizeof(BitmapInfoHeader), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buffer = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(dc, bitmap, 0, h, buffer, ctypes.byref(header), 0)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(dc)
    user32.ReleaseDC(hwnd, src)
    image = Image.frombuffer("RGBA", (w, h), buffer, "raw", "BGRA", 0, 1).convert("RGB")
    image.save(path)
    return image


def copy_meeting(name, dest):
    target = dest / name
    target.mkdir(parents=True, exist_ok=True)
    for f in KEEP:
        if (real / name / f).exists():
            shutil.copy2(real / name / f, target / f)


temp = Path(tempfile.mkdtemp(prefix="meetings-verify-"))
meetings = temp / "appdata" / "hex" / "meetings"
copy_meeting(FIRST, meetings)
env = dict(os.environ, APPDATA=str(temp / "appdata"), HEX_MEETINGS_STATE_DIR=str(temp / "state"))
process = subprocess.Popen([str(exe)], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
windows = []


@callback_type
def collect(hwnd, _):
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if pid.value == process.pid and user32.IsWindowVisible(hwnd):
        title = ctypes.create_unicode_buffer(256)
        user32.GetWindowTextW(hwnd, title, len(title))
        if title.value == "Meetings":
            windows.append(hwnd)
    return True


result = {}
try:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline and not windows:
        assert process.poll() is None, "The window process exited early"
        user32.EnumWindows(collect, 0)
        time.sleep(0.2)
    assert windows, "No visible Meetings window"
    time.sleep(3)
    before = capture(windows[0], out / "native-before.png")
    time.sleep(2)
    settled = capture(windows[0], out / "native-settled.png")
    strip = (0, 0, min(380, before.size[0]), before.size[1])  # the meeting list column
    assert not ImageChops.difference(before.crop(strip), settled.crop(strip)).getbbox(), "The list was still changing"
    copy_meeting(SECOND, meetings)
    started = time.monotonic()
    appeared = None
    while time.monotonic() - started < wait:
        time.sleep(1)
        current = capture(windows[0], out / "native-after.png")
        if ImageChops.difference(before.crop(strip), current.crop(strip)).getbbox():
            appeared = round(time.monotonic() - started, 1)
            break
    result = {
        "window_opened": True,
        "second_meeting_copied_while_running": SECOND,
        "list_changed_after_seconds": appeared,
        "pixel_size": list(before.size),
    }
    assert appeared is not None, "The list did not change within the wait"
finally:
    print(json.dumps(result, indent=2))
    if windows:
        user32.PostMessageW(windows[0], 0x0010, 0, 0)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    shutil.rmtree(temp, ignore_errors=True)
