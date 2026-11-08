"""Measure the native proof window and verify a second launch exits and focuses it."""
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import subprocess
import sys
import time
from verify_listener import private_bytes

exe = Path(sys.argv[1]).resolve()
user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.GetForegroundWindow.restype = wintypes.HWND
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
process = subprocess.Popen([str(exe)], creationflags=subprocess.CREATE_NO_WINDOW)
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


try:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        assert process.poll() is None, "First instance exited before opening its window"
        user32.EnumWindows(collect, 0)
        if windows:
            break
        time.sleep(0.1)
    assert windows, "No visible Sidenote window"
    time.sleep(3)
    samples = []
    for _ in range(5):
        samples.append(private_bytes(process))
        time.sleep(1)
    second = subprocess.Popen([str(exe)], creationflags=subprocess.CREATE_NO_WINDOW)
    assert second.wait(timeout=10) == 0
    assert process.poll() is None
    foreground_pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), ctypes.byref(foreground_pid))
    assert foreground_pid.value == process.pid, "Second launch did not focus the original window"
    print(json.dumps({"visible_window": True, "second_launch_exited": True, "second_launch_focused_first": True, "private_bytes_samples": samples, "private_bytes_median": sorted(samples)[len(samples)//2]}, indent=2))
finally:
    if windows:
        user32.PostMessageW(windows[0], 0x0010, 0, 0)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
