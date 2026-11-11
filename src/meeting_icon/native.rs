use super::{card_text, draft_seen};
use crate::core::{self, store_io};
use std::{cell::RefCell, io, mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "SidenoteListenerMeetingIcon";
const CARD_CLASS: &str = "SidenoteListenerMeetingCard";
const WIDTH: f32 = 104.0;
const HEIGHT: f32 = 44.0;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct View {
    icon: HWND,
    card: HWND,
    edit: HWND,
    font: HFONT,
    scale: f32,
    work: RECT,
    fraction: f32,
    pill: bool,
    recording: bool,
    elapsed: u64,
    id: String,
    raw: Option<String>,
    shown_raw: Option<String>,
    drag: Option<(i32, f32)>,
    moved: bool,
    reopen: bool,
    render: bool,
}

pub(super) struct Host {
    view: Box<RefCell<View>>,
}

impl Host {
    pub(super) fn new(render: bool) -> io::Result<Self> {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(ptr::null());
            let class = wide(CLASS);
            let card_class = wide(CARD_CLASS);
            for (name, callback) in [
                (
                    &class,
                    Some(icon_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
                (
                    &card_class,
                    Some(card_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
            ] {
                let wc = WNDCLASSW {
                    lpfnWndProc: callback,
                    hInstance: instance,
                    lpszClassName: name.as_ptr(),
                    hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                    hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
                    ..std::mem::zeroed()
                };
                if RegisterClassW(&wc) == 0 {
                    UnregisterClassW(class.as_ptr(), instance);
                    return Err(io::Error::last_os_error());
                }
            }
            let fraction: f32 =
                store_io::read(&core::state_dir().join("local/floating-icon-position.json"))
                    .unwrap_or(0.6);
            let host = Self {
                view: Box::new(RefCell::new(View {
                    icon: ptr::null_mut(),
                    card: ptr::null_mut(),
                    edit: ptr::null_mut(),
                    font: ptr::null_mut(),
                    scale: 1.0,
                    work: std::mem::zeroed(),
                    fraction: if fraction.is_finite() {
                        fraction.clamp(0.0, 1.0)
                    } else {
                        0.6
                    },
                    pill: false,
                    recording: true,
                    elapsed: 0,
                    id: String::new(),
                    raw: None,
                    shown_raw: None,
                    drag: None,
                    moved: false,
                    reopen: false,
                    render,
                })),
            };
            let view = &*host.view as *const RefCell<View>;
            let icon = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                class.as_ptr(),
                wide("Sidenote meeting icon").as_ptr(),
                WS_POPUP,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                view.cast(),
            );
            if icon.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut v = host.view.borrow_mut();
            v.icon = icon;
            if SetWindowDisplayAffinity(icon, WDA_EXCLUDEFROMCAPTURE) == 0 {
                return Err(io::Error::last_os_error());
            }
            v.geometry();
            v.paint()?;
            ShowWindow(icon, SW_SHOWNOACTIVATE);
            drop(v);
            Ok(host)
        }
    }

    pub(super) fn update(
        &mut self,
        id: &str,
        recording: bool,
        pill: bool,
        elapsed: u64,
        raw: Option<String>,
    ) {
        let mut v = self.view.borrow_mut();
        let changed = v.id != id
            || v.recording != recording
            || v.pill != pill
            || (pill && v.elapsed != elapsed);
        let card_changed = v.id != id || v.recording != recording || v.raw != raw;
        v.id = id.into();
        v.recording = recording;
        v.pill = pill;
        v.elapsed = elapsed;
        v.raw = raw;
        let geometry = unsafe { v.geometry() };
        if (changed || geometry) && unsafe { v.paint() }.is_err() {
            crate::listener::log("floating icon paint unavailable");
        }
        if card_changed && !v.card.is_null() && unsafe { IsWindowVisible(v.card) } != 0 {
            unsafe {
                v.refresh_card();
            }
        }
    }

    pub(super) fn pump(&mut self) {
        unsafe {
            let mut message = std::mem::zeroed();
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjectsEx(0, ptr::null(), 50, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let v = self.view.borrow();
            if !v.card.is_null() {
                DestroyWindow(v.card);
            }
            if !v.icon.is_null() {
                DestroyWindow(v.icon);
            }
            if !v.font.is_null() {
                DeleteObject(v.font);
            }
            let instance = GetModuleHandleW(ptr::null());
            UnregisterClassW(wide(CARD_CLASS).as_ptr(), instance);
            UnregisterClassW(wide(CLASS).as_ptr(), instance);
        }
    }
}

impl View {
    fn px(&self, value: f32) -> i32 {
        (value * self.scale).round() as i32
    }
    fn travel(&self) -> i32 {
        (self.work.bottom - self.work.top - self.px(HEIGHT + 16.0)).max(0)
    }
    fn point(&self) -> POINT {
        POINT {
            x: self.work.right - self.px(WIDTH + 8.0),
            y: self.work.top + self.px(8.0) + (self.fraction * self.travel() as f32).round() as i32,
        }
    }
    unsafe fn geometry(&mut self) -> bool {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return false;
            }
            let mut x = 96;
            let mut y = 96;
            if windows_sys::Win32::UI::HiDpi::GetDpiForMonitor(
                monitor,
                windows_sys::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI,
                &mut x,
                &mut y,
            ) != 0
            {
                return false;
            }
            let scale = x as f32 / 96.0;
            let r = info.rcWork;
            let changed = self.scale != scale
                || (
                    self.work.left,
                    self.work.top,
                    self.work.right,
                    self.work.bottom,
                ) != (r.left, r.top, r.right, r.bottom);
            self.work = r;
            self.scale = scale;
            if changed {
                if !self.font.is_null() {
                    DeleteObject(self.font);
                }
                self.font = CreateFontW(
                    -self.px(13.0),
                    0,
                    0,
                    0,
                    500,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    ANTIALIASED_QUALITY as u32,
                    0,
                    wide("Segoe UI").as_ptr(),
                );
                let point = self.point();
                SetWindowPos(
                    self.icon,
                    HWND_TOPMOST,
                    point.x,
                    point.y,
                    self.px(WIDTH),
                    self.px(HEIGHT),
                    SWP_NOACTIVATE,
                );
                self.position_card();
                if !self.edit.is_null() {
                    SendMessageW(self.edit, WM_SETFONT, self.font as usize, 1);
                    self.layout_edit();
                }
            }
            changed
        }
    }
    unsafe fn position_card(&self) {
        unsafe {
            if self.card.is_null() {
                return;
            }
            let p = self.point();
            let width = self.px(360.0).min(self.work.right - self.work.left);
            let height = self.px(300.0).min(self.work.bottom - self.work.top);
            SetWindowPos(
                self.card,
                HWND_TOPMOST,
                (p.x - width - self.px(8.0)).max(self.work.left),
                p.y.min(self.work.bottom - height),
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }
    unsafe fn refresh_card(&mut self) {
        unsafe {
            SetWindowTextW(
                self.edit,
                wide(&card_text(&self.id, self.recording, self.raw.as_deref())).as_ptr(),
            );
            self.shown_raw = self.raw.clone();
        }
    }
    unsafe fn open_card(&mut self) {
        unsafe {
            if self.card.is_null() {
                self.card = CreateWindowExW(
                    WS_EX_TOPMOST,
                    wide(CARD_CLASS).as_ptr(),
                    wide("Sidenote meeting").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    360,
                    300,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    GetWindowLongPtrW(self.icon, GWLP_USERDATA) as *const _,
                );
                if self.card.is_null() {
                    return;
                }
                if SetWindowDisplayAffinity(self.card, WDA_EXCLUDEFROMCAPTURE) == 0 {
                    DestroyWindow(self.card);
                    self.card = ptr::null_mut();
                    return;
                }
                self.edit = CreateWindowExW(
                    0,
                    wide("EDIT").as_ptr(),
                    ptr::null(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | ES_MULTILINE as u32
                        | ES_READONLY as u32
                        | ES_AUTOVSCROLL as u32,
                    12,
                    12,
                    320,
                    240,
                    self.card,
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    ptr::null(),
                );
                if self.edit.is_null() {
                    DestroyWindow(self.card);
                    self.card = ptr::null_mut();
                    return;
                }
            }
            self.position_card();
            self.layout_edit();
            SendMessageW(self.edit, WM_SETFONT, self.font as usize, 1);
            self.refresh_card();
            ShowWindow(self.card, SW_SHOWNOACTIVATE);
        }
    }
    unsafe fn layout_edit(&self) {
        unsafe {
            if self.edit.is_null() {
                return;
            }
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(self.card, &mut r);
            SetWindowPos(
                self.edit,
                ptr::null_mut(),
                12,
                12,
                (r.right - 24).max(1),
                (r.bottom - 24).max(1),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    unsafe fn paint(&self) -> io::Result<()> {
        unsafe {
            let width = self.px(WIDTH);
            let height = self.px(HEIGHT);
            let left = if self.pill { 0 } else { width - height };
            // The HWND and backing bitmap never change size as speech comes and goes.
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err(io::Error::last_os_error());
            }
            let old_bitmap = SelectObject(dc, bitmap);
            let pixels =
                std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize);
            pixels.fill(0x001f2630);
            let old_font = SelectObject(dc, self.font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00faf3ea);
            if self.pill {
                let label = wide(&format!("{}:{:02}", self.elapsed / 60, self.elapsed % 60));
                let mut r = RECT {
                    left: self.px(37.0),
                    top: 0,
                    right: width - self.px(8.0),
                    bottom: height,
                };
                DrawTextW(
                    dc,
                    label.as_ptr(),
                    -1,
                    &mut r,
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
            let brush = CreateSolidBrush(if self.recording {
                0x00a6d5a0
            } else {
                0x00b5c9ed
            });
            if self.pill {
                for (offset, h) in [(15.0, 10.0), (21.0, 20.0), (27.0, 14.0)] {
                    let r = RECT {
                        left: self.px(offset),
                        right: self.px(offset + 3.0),
                        top: self.px((HEIGHT - h) / 2.0),
                        bottom: self.px((HEIGHT + h) / 2.0),
                    };
                    FillRect(dc, &r, brush);
                }
            } else {
                let previous = SelectObject(dc, brush);
                let pen = SelectObject(dc, GetStockObject(NULL_PEN));
                let c = left + height / 2;
                let radius = self.px(5.0);
                Ellipse(
                    dc,
                    c - radius,
                    height / 2 - radius,
                    c + radius,
                    height / 2 + radius,
                );
                SelectObject(dc, pen);
                SelectObject(dc, previous);
            }
            DeleteObject(brush);
            // GDI text leaves alpha undefined. Restore capsule coverage and premultiply RGB.
            let radius = height as f32 / 2.0;
            for y in 0..height {
                for x in 0..width {
                    let cx = (x as f32 + 0.5).clamp(left as f32 + radius, width as f32 - radius);
                    let distance =
                        ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - radius).powi(2)).sqrt();
                    let alpha = ((radius - distance + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32;
                    let p = &mut pixels[(y * width + x) as usize];
                    *p = (alpha << 24)
                        | (((*p >> 16 & 255) * alpha / 255) << 16)
                        | (((*p >> 8 & 255) * alpha / 255) << 8)
                        | ((*p & 255) * alpha / 255);
                }
            }
            let point = self.point();
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            if self.render {
                let path = core::state_dir().join("local").join(if self.pill {
                    "floating-icon-pill.png"
                } else {
                    "floating-icon-bead.png"
                });
                if !path.exists() {
                    let rgba: Vec<u8> = pixels
                        .iter()
                        .flat_map(|p| {
                            let a = p >> 24;
                            [
                                ((p >> 16 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                                ((p >> 8 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                                ((p & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                                a as u8,
                            ]
                        })
                        .collect();
                    let _ = image::save_buffer(
                        &path,
                        &rgba,
                        width as u32,
                        height as u32,
                        image::ColorType::Rgba8,
                    );
                }
            }
            let result = UpdateLayeredWindow(
                self.icon,
                ptr::null_mut(),
                &point,
                &SIZE {
                    cx: width,
                    cy: height,
                },
                dc,
                &POINT { x: 0, y: 0 },
                0,
                &blend,
                ULW_ALPHA,
            );
            let error = io::Error::last_os_error();
            SelectObject(dc, old_font);
            SelectObject(dc, old_bitmap);
            DeleteObject(bitmap);
            DeleteDC(dc);
            if result == 0 {
                return Err(error);
            }
            // A native region excludes empty pixels even for windows in other processes.
            let region = CreateRoundRectRgn(left, 0, width + 1, height + 1, height, height);
            if region.is_null() {
                return Err(io::Error::last_os_error());
            }
            if SetWindowRgn(self.icon, region, 1) == 0 {
                DeleteObject(region);
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

unsafe fn view<'a>(hwnd: HWND, msg: u32, l: LPARAM) -> Option<&'a RefCell<View>> {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<View>).as_ref()
    }
}

unsafe extern "system" fn icon_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let Some(mut v) = view(hwnd, msg, l).and_then(|v| v.try_borrow_mut().ok()) else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        match msg {
            m if m == WM_APP + 40 && v.render => {
                v.open_card();
                0
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_LBUTTONDOWN => {
                let mut p = POINT {
                    x: (l as i16) as i32,
                    y: ((l >> 16) as i16) as i32,
                };
                ClientToScreen(hwnd, &mut p);
                v.drag = Some((p.y, v.fraction));
                v.moved = false;
                v.reopen = !v.card.is_null() && IsWindowVisible(v.card) != 0;
                SetCapture(hwnd);
                0
            }
            WM_MOUSEMOVE if v.drag.is_some() => {
                let (start, fraction) = v.drag.unwrap();
                let mut p = POINT {
                    x: (l as i16) as i32,
                    y: ((l >> 16) as i16) as i32,
                };
                ClientToScreen(hwnd, &mut p);
                let dy = p.y - start;
                if dy.abs() > v.px(4.0) {
                    v.moved = true;
                }
                if v.moved {
                    if v.reopen {
                        ShowWindow(v.card, SW_HIDE);
                    }
                    v.fraction = (fraction + dy as f32 / v.travel().max(1) as f32).clamp(0.0, 1.0);
                    let point = v.point();
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        point.x,
                        point.y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    v.position_card();
                }
                0
            }
            WM_LBUTTONUP if v.drag.is_some() => {
                v.drag = None;
                ReleaseCapture();
                if v.moved {
                    if store_io::write(
                        &core::state_dir().join("local/floating-icon-position.json"),
                        &v.fraction,
                    )
                    .is_err()
                    {
                        crate::listener::log("could not save floating icon position");
                    }
                    if v.reopen {
                        v.open_card();
                    }
                } else {
                    v.open_card();
                }
                0
            }
            WM_CAPTURECHANGED => {
                if let Some((_, fraction)) = v.drag.take() {
                    v.fraction = fraction;
                    let point = v.point();
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        point.x,
                        point.y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    v.position_card();
                    if v.reopen {
                        v.open_card();
                    }
                }
                0
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => {
                v.geometry();
                let _ = v.paint();
                0
            }
            WM_CLOSE => 0,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe extern "system" fn card_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let Some(v) = view(hwnd, msg, l).and_then(|v| v.try_borrow_mut().ok()) else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        match msg {
            WM_SIZE if !v.edit.is_null() => {
                v.layout_edit();
                0
            }
            WM_CLOSE => {
                if let Some(raw) = &v.shown_raw {
                    draft_seen(&v.id, raw);
                }
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
