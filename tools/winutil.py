"""Small Windows helpers shared by the measure and verify scripts (ctypes only)."""
import ctypes
from ctypes import wintypes
import time

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except OSError:
    pass

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi")

callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.c_void_p]
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
user32.GetDpiForWindow.argtypes = [wintypes.HWND]
user32.GetDpiForWindow.restype = wintypes.UINT
dwmapi.DwmGetWindowAttribute.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]

WM_CLOSE = 0x0010
DWMWA_CLOAKED = 14
STILL_ACTIVE = 259
SYNCHRONIZE = 0x00100000
PROCESS_QUERY_LIMITED = 0x1000


def meetings_window(pid, require_painted=True):
    """The visible Meetings window of `pid`. Rune keeps a new window cloaked until its first
    frame has been presented, so an uncloaked window is one that has painted."""
    found = []
    errors = []

    @callback_type
    def collect(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value != pid or not user32.IsWindowVisible(hwnd):
            return True
        title = ctypes.create_unicode_buffer(64)
        user32.GetWindowTextW(hwnd, title, len(title))
        if title.value != "Meetings":
            return True
        cloaked = wintypes.DWORD(0)
        result = dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, ctypes.byref(cloaked), 4)
        if result < 0:
            errors.append(result)
            return False
        if cloaked.value == 0 or not require_painted:
            found.append(hwnd)
        return True

    user32.EnumWindows(collect, 0)
    if errors:
        raise ctypes.WinError(errors[0])
    return found[0] if found else None


def wait_window(process, timeout=30, require_painted=True):
    start = time.perf_counter()
    while time.perf_counter() - start < timeout:
        assert process.poll() is None, "The app exited before its window opened"
        hwnd = meetings_window(process.pid, require_painted)
        if hwnd:
            return hwnd, time.perf_counter()
        time.sleep(0.002)
    raise AssertionError("No Meetings window appeared")


def close_window(hwnd):
    user32.PostMessageW(hwnd, WM_CLOSE, 0, 0)


class ProcessEntry(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("pid", wintypes.DWORD),
                ("heap", ctypes.c_size_t), ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
                ("parent", wintypes.DWORD), ("priority", wintypes.LONG), ("flags", wintypes.DWORD),
                ("exe", wintypes.WCHAR * 260)]


kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]


def processes():
    """{pid: (parent pid, exe name)} for every running process."""
    snapshot = kernel32.CreateToolhelp32Snapshot(0x2, 0)
    table = {}
    entry = ProcessEntry()
    entry.size = ctypes.sizeof(entry)
    ok = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
    while ok:
        table[entry.pid] = (entry.parent, entry.exe)
        ok = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    kernel32.CloseHandle(snapshot)
    return table


def descendants(pid):
    """Every process below `pid`, as {pid: exe name}."""
    table = processes()
    found, frontier = {}, [pid]
    while frontier:
        parent = frontier.pop()
        for child, (owner, exe) in table.items():
            if owner == parent and child not in found and child != pid:
                found[child] = exe
                frontier.append(child)
    return found


def alive(pid):
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED, False, pid)
    if not handle:
        return False
    code = wintypes.DWORD()
    kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
    kernel32.CloseHandle(handle)
    return code.value == STILL_ACTIVE


class Input(ctypes.Structure):
    class _Union(ctypes.Union):
        class _Key(ctypes.Structure):
            _fields_ = [("vk", wintypes.WORD), ("scan", wintypes.WORD), ("flags", wintypes.DWORD),
                        ("time", wintypes.DWORD), ("extra", ctypes.c_size_t)]

        class _Mouse(ctypes.Structure):
            _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("data", wintypes.DWORD),
                        ("flags", wintypes.DWORD), ("time", wintypes.DWORD), ("extra", ctypes.c_size_t)]

        _fields_ = [("ki", _Key), ("mi", _Mouse)]

    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", _Union)]


user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(Input), ctypes.c_int]
user32.GetForegroundWindow.restype = wintypes.HWND
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.GetCursorPos.argtypes = [ctypes.POINTER(wintypes.POINT)]
user32.WindowFromPoint.argtypes = [wintypes.POINT]
user32.WindowFromPoint.restype = wintypes.HWND
user32.GetAncestor.argtypes = [wintypes.HWND, wintypes.UINT]
user32.GetAncestor.restype = wintypes.HWND


class NotInFront(Exception):
    """The target window is not the foreground window, so no input was sent."""


def _front(hwnd):
    return user32.GetForegroundWindow() == hwnd


user32.AttachThreadInput.argtypes = [wintypes.DWORD, wintypes.DWORD, wintypes.BOOL]
user32.BringWindowToTop.argtypes = [wintypes.HWND]
user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
user32.IsIconic.argtypes = [wintypes.HWND]
kernel32.GetCurrentThreadId.restype = wintypes.DWORD


def bring_to_front(hwnd, timeout=3.0):
    """Foreground the window by sharing input state with the current foreground thread (no
    keys are sent) and prove it."""
    if user32.IsIconic(hwnd):
        user32.ShowWindow(hwnd, 9)  # SW_RESTORE
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        current = user32.GetForegroundWindow()
        theirs = user32.GetWindowThreadProcessId(current, None) if current else 0
        mine = kernel32.GetCurrentThreadId()
        if theirs and theirs != mine:
            user32.AttachThreadInput(mine, theirs, True)
        user32.BringWindowToTop(hwnd)
        user32.SetForegroundWindow(hwnd)
        if theirs and theirs != mine:
            user32.AttachThreadInput(mine, theirs, False)
        time.sleep(0.15)
        if _front(hwnd):
            return
    raise NotInFront("could not bring the Meetings window to the front")


def _send(hwnd, *items):
    # Every batch re-checks the foreground window: input never goes to another application.
    if not _front(hwnd):
        raise NotInFront("the Meetings window lost the foreground; nothing was sent")
    array = (Input * len(items))(*items)
    user32.SendInput(len(items), array, ctypes.sizeof(Input))


def click(hwnd, x, y):
    """Left click at client pixel (x, y), only if that point is this window."""
    ox, oy, *_ = client_origin(hwnd)
    point = wintypes.POINT(ox + x, oy + y)
    if user32.GetAncestor(user32.WindowFromPoint(point), 2) != hwnd:  # GA_ROOT
        raise NotInFront("the click point is not over the Meetings window")
    # Let the window see the pointer arrive before the press, as a hand would.
    user32.SetCursorPos(point.x - 6, point.y)
    time.sleep(0.12)
    user32.SetCursorPos(point.x, point.y)
    time.sleep(0.12)
    down, up = Input(type=0), Input(type=0)
    down.mi.flags, up.mi.flags = 0x0002, 0x0004
    _send(hwnd, down)
    time.sleep(0.06)
    _send(hwnd, up)
    time.sleep(0.2)


user32.MapVirtualKeyW.argtypes = [wintypes.UINT, wintypes.UINT]


def type_text(hwnd, text, per_key=0.03):
    """Types lowercase letters, digits and spaces as real key presses (scan codes). Unicode
    injection is ignored by the window, so other characters are not supported."""
    for ch in text:
        vk = 0x20 if ch == " " else ord(ch.upper())
        assert ch == " " or ch.isalnum() and ch.isascii(), f"unsupported character {ch!r}"
        scan = user32.MapVirtualKeyW(vk, 0)
        for flags in (0x0008, 0x0008 | 0x0002):  # KEYEVENTF_SCANCODE, then key up
            item = Input(type=1)
            item.ki.scan, item.ki.flags = scan, flags
            _send(hwnd, item)
        time.sleep(per_key)


def press_enter(hwnd):
    for flags in (0, 0x0002):
        item = Input(type=1)
        item.ki.vk, item.ki.flags = 0x0D, flags
        _send(hwnd, item)


def client_origin(hwnd):
    """Client area top-left on screen, and its size, in physical pixels."""
    rect = wintypes.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(rect))
    origin = wintypes.POINT(0, 0)
    user32.ClientToScreen(hwnd, ctypes.byref(origin))
    return origin.x, origin.y, rect.right, rect.bottom, user32.GetDpiForWindow(hwnd) / 96


gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
user32.GetWindowDC.argtypes = [wintypes.HWND]
user32.GetWindowDC.restype = wintypes.HDC
user32.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
user32.PrintWindow.argtypes = [wintypes.HWND, wintypes.HDC, wintypes.UINT]
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
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


def capture(hwnd, path=None):
    """The window as Windows draws it (PrintWindow), as a PIL RGB image."""
    from PIL import Image
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w, h = rect.right - rect.left, rect.bottom - rect.top
    src = user32.GetWindowDC(hwnd)
    dc = gdi32.CreateCompatibleDC(src)
    bitmap = gdi32.CreateCompatibleBitmap(src, w, h)
    gdi32.SelectObject(dc, bitmap)
    user32.PrintWindow(hwnd, dc, 2)
    header = BitmapInfoHeader(ctypes.sizeof(BitmapInfoHeader), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
    buffer = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(dc, bitmap, 0, h, buffer, ctypes.byref(header), 0)
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(dc)
    user32.ReleaseDC(hwnd, src)
    image = Image.frombuffer("RGBA", (w, h), buffer, "raw", "BGRA", 0, 1).convert("RGB")
    if path:
        image.save(path)
    return image
