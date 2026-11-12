use super::{Control, Frame, RETIRE, callback_boundary};
use crate::{
    core::{self, store_io},
    meeting_icon::{card_text, draft_seen},
};
use std::{
    cell::{Cell, RefCell},
    io,
    mem::size_of,
    ptr,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT,
            VK_PRIOR, VK_RETURN, VK_RIGHT, VK_UP,
        },
        WindowsAndMessaging::*,
    },
};

const WIDTH: f32 = 104.0;
const HEIGHT: f32 = 44.0;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Copy)]
struct Layout {
    scale: f32,
    work: RECT,
    fraction: f32,
}
#[derive(Default, Clone)]
struct Card {
    text: String,
    id: String,
    raw: Option<String>,
    revision: u64,
    refreshed: Option<Instant>,
    updates: usize,
    minimum_live_gap_ms: Option<u128>,
}
struct Font(HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            if DeleteObject(self.0) == 0 {
                crate::listener::log("FI_FONT_RELEASE");
            }
        }
    }
}
struct SelectedFont {
    dc: HDC,
    prior: HGDIOBJ,
    _font: Rc<Font>,
}
impl SelectedFont {
    unsafe fn new(dc: HDC, font: Rc<Font>) -> Self {
        unsafe {
            Self {
                dc,
                prior: SelectObject(dc, font.0),
                _font: font,
            }
        }
    }
}
impl Drop for SelectedFont {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.prior);
        }
    }
}
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    prior: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}
impl Surface {
    fn new(width: i32, height: i32) -> io::Result<Self> {
        unsafe {
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
            Ok(Self {
                dc,
                bitmap,
                prior: SelectObject(dc, bitmap),
                bits: bits.cast(),
                width,
                height,
            })
        }
    }
    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
    fn save(&mut self, name: &str, alpha: bool) {
        let width = self.width;
        let height = self.height;
        let rgba: Vec<u8> = self
            .pixels()
            .iter()
            .flat_map(|p| {
                let a = if alpha { p >> 24 } else { 255 };
                [
                    ((p >> 16 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    ((p >> 8 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    ((p & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    a as u8,
                ]
            })
            .collect();
        let _ = image::save_buffer(
            core::state_dir().join("local").join(name),
            &rgba,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        );
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.prior);
            if DeleteObject(self.bitmap) == 0 {
                crate::listener::log("FI_BITMAP_RELEASE");
            }
            if DeleteDC(self.dc) == 0 {
                crate::listener::log("FI_DC_RELEASE");
            }
        }
    }
}
struct Paint {
    hwnd: HWND,
    ps: PAINTSTRUCT,
}
impl Paint {
    unsafe fn new(hwnd: HWND) -> (Self, HDC) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            (Self { hwnd, ps }, dc)
        }
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        unsafe {
            EndPaint(self.hwnd, &self.ps);
        }
    }
}

// Cells hold copyable geometry/handles. RefCell guards cover only pure Rust state updates.
struct View {
    icon: Cell<HWND>,
    card: Cell<HWND>,
    font: RefCell<Option<Rc<Font>>>,
    layout: Cell<Layout>,
    frame: RefCell<Frame>,
    content: RefCell<Card>,
    scroll: Cell<i32>,
    max_scroll: Cell<i32>,
    wheel: Cell<i32>,
    drag: Cell<Option<(i32, f32)>>,
    moved: Cell<bool>,
    reopen: Cell<bool>,
    replay: bool,
    control: Arc<Control>,
}
pub(super) struct NativeWindow {
    view: Box<View>,
    icon_class: Vec<u16>,
    card_class: Vec<u16>,
}
impl NativeWindow {
    pub(super) fn new(replay: bool, control: Arc<Control>) -> io::Result<Self> {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(ptr::null());
            let icon_class = wide(&control.icon_class);
            let card_class = wide(&control.card_class);
            let fraction = store_io::read::<Option<f32>>(
                &core::state_dir().join("local/floating-icon-position.json"),
            )
            .unwrap_or_default()
            .unwrap_or(0.6);
            let window = Self {
                view: Box::new(View {
                    icon: Cell::new(ptr::null_mut()),
                    card: Cell::new(ptr::null_mut()),
                    font: RefCell::new(None),
                    layout: Cell::new(Layout {
                        scale: 1.0,
                        work: std::mem::zeroed(),
                        fraction: if fraction.is_finite() {
                            fraction.clamp(0.0, 1.0)
                        } else {
                            0.6
                        },
                    }),
                    frame: RefCell::new(Frame::default()),
                    content: RefCell::new(Card::default()),
                    scroll: Cell::new(0),
                    max_scroll: Cell::new(0),
                    wheel: Cell::new(0),
                    drag: Cell::new(None),
                    moved: Cell::new(false),
                    reopen: Cell::new(false),
                    replay,
                    control,
                }),
                icon_class,
                card_class,
            };
            for (name, callback) in [
                (
                    &window.icon_class,
                    Some(icon_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
                (
                    &window.card_class,
                    Some(card_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
            ] {
                let wc = WNDCLASSW {
                    lpfnWndProc: callback,
                    hInstance: instance,
                    lpszClassName: name.as_ptr(),
                    hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                    ..std::mem::zeroed()
                };
                if RegisterClassW(&wc) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                window.icon_class.as_ptr(),
                wide("Sidenote meeting icon").as_ptr(),
                WS_POPUP,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                (&*window.view as *const View).cast(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            window.view.icon.set(hwnd);
            window.view.control.publish(hwnd, false);
            if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0 {
                return Err(io::Error::last_os_error());
            }
            window.view.geometry();
            Ok(window)
        }
    }
    pub(super) fn update(&self, frame: Frame) {
        let prior = self.view.frame.replace(frame.clone());
        let changed = prior.id != frame.id
            || prior.recording != frame.recording
            || prior.pill != frame.pill
            || prior.skipped != frame.skipped
            || (frame.pill && prior.elapsed != frame.elapsed);
        let card_changed = prior.id != frame.id
            || prior.recording != frame.recording
            || prior.raw != frame.raw
            || prior.skipped != frame.skipped;
        unsafe {
            let geometry = self.view.geometry();
            if (changed || geometry) && self.view.paint_icon().is_err() {
                crate::listener::log("FI_PAINT_FAILED");
            }
            if !self.view.control.stop.load(Ordering::Acquire)
                && IsWindowVisible(self.view.icon.get()) == 0
            {
                ShowWindow(self.view.icon.get(), SW_SHOWNOACTIVATE);
            }
            if !self.view.card.get().is_null() && IsWindowVisible(self.view.card.get()) != 0 {
                self.view.refresh_card(card_changed);
            }
        }
    }
    pub(super) fn pump(&self) -> bool {
        unsafe {
            let mut message = std::mem::zeroed();
            // Return to the stop/channel checks even when native messages keep arriving.
            for _ in 0..64 {
                if PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                if message.message == WM_QUIT {
                    return false;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjectsEx(0, ptr::null(), 50, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
        true
    }
}
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe {
            self.view.close_windows();
            let font = self.view.font.replace(None);
            drop(font);
            let instance = GetModuleHandleW(ptr::null());
            UnregisterClassW(self.card_class.as_ptr(), instance);
            UnregisterClassW(self.icon_class.as_ptr(), instance);
        }
    }
}

#[derive(Clone, Copy)]
enum Scroll {
    Lines(i32),
    Page(i32),
    Home,
    End,
}
impl View {
    fn px(&self, value: f32) -> i32 {
        (value * self.layout.get().scale).round() as i32
    }
    fn travel(&self) -> i32 {
        let g = self.layout.get();
        (g.work.bottom - g.work.top - self.px(HEIGHT + 16.0)).max(0)
    }
    fn point(&self) -> POINT {
        let g = self.layout.get();
        POINT {
            x: g.work.right - self.px(WIDTH + 8.0),
            y: g.work.top + self.px(8.0) + (g.fraction * self.travel() as f32).round() as i32,
        }
    }
    fn set_fraction(&self, fraction: f32) {
        let mut g = self.layout.get();
        g.fraction = fraction.clamp(0.0, 1.0);
        self.layout.set(g);
    }
    unsafe fn geometry(&self) -> bool {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            let (mut x, mut y) = (96, 96);
            if GetMonitorInfoW(monitor, &mut info) == 0
                || GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) != 0
            {
                return false;
            }
            let old = self.layout.get();
            let r = info.rcWork;
            let scale = x as f32 / 96.0;
            if old.scale == scale
                && (old.work.left, old.work.top, old.work.right, old.work.bottom)
                    == (r.left, r.top, r.right, r.bottom)
            {
                return false;
            }
            let font = CreateFontW(
                -(13.0 * scale).round() as i32,
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
            if font.is_null() {
                crate::listener::log("FI_FONT_FAILED");
                return false;
            }
            let prior = self.font.replace(Some(Rc::new(Font(font))));
            drop(prior);
            self.layout.set(Layout {
                scale,
                work: r,
                fraction: old.fraction,
            });
            let p = self.point();
            SetWindowPos(
                self.icon.get(),
                HWND_TOPMOST,
                p.x,
                p.y,
                self.px(WIDTH),
                self.px(HEIGHT),
                SWP_NOACTIVATE,
            );
            self.position_card();
            true
        }
    }
    unsafe fn position_card(&self) {
        unsafe {
            let hwnd = self.card.get();
            if hwnd.is_null() {
                return;
            }
            let g = self.layout.get();
            let p = self.point();
            let width = self.px(360.0).min(g.work.right - g.work.left);
            let height = self.px(300.0).min(g.work.bottom - g.work.top);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                (p.x - width - self.px(8.0)).max(g.work.left),
                p.y.min(g.work.bottom - height),
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }
    unsafe fn refresh_card(&self, force: bool) {
        unsafe {
            let frame = self.frame.borrow().clone();
            let card = self.content.borrow().clone();
            let now = Instant::now();
            let live = frame.raw.is_none() && frame.transcript_revision != card.revision;
            if !force
                && (!live
                    || card
                        .refreshed
                        .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)))
            {
                return;
            }
            let text = if frame.skipped {
                "Skipped. Short recordings are not processed.".into()
            } else {
                card_text(&frame.id, frame.recording, frame.raw.as_deref())
            };
            let mut next = Card {
                text,
                id: frame.id,
                raw: frame.raw,
                revision: frame.transcript_revision,
                refreshed: Some(now),
                updates: card.updates + 1,
                minimum_live_gap_ms: card.minimum_live_gap_ms,
            };
            if !force && let Some(at) = card.refreshed {
                let gap = now.duration_since(at).as_millis();
                next.minimum_live_gap_ms =
                    Some(next.minimum_live_gap_ms.map_or(gap, |old| old.min(gap)));
            }
            self.content.replace(next);
            InvalidateRect(self.card.get(), ptr::null(), 0);
            if self.replay {
                self.render_card();
            }
        }
    }
    unsafe fn open_card(&self) {
        unsafe {
            if self.control.stop.load(Ordering::Acquire) {
                return;
            }
            let mut hwnd = self.card.get();
            if hwnd.is_null() {
                hwnd = CreateWindowExW(
                    WS_EX_TOPMOST,
                    wide(&self.control.card_class).as_ptr(),
                    wide("Sidenote meeting").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    360,
                    300,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    (self as *const View).cast(),
                );
                if hwnd.is_null() {
                    return;
                }
                self.card.set(hwnd);
                self.control.publish(hwnd, true);
                if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0 {
                    self.destroy(hwnd);
                    return;
                }
            }
            self.scroll.set(0);
            self.max_scroll.set(0);
            self.position_card();
            self.refresh_card(true);
            // User clicks may activate this keyboard-usable card. Automatic show/refresh/resize never does.
            if !self.control.stop.load(Ordering::Acquire) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }
    unsafe fn hide_card(&self) {
        unsafe {
            self.content.replace(Card::default());
            self.scroll.set(0);
            self.max_scroll.set(0);
            let hwnd = self.card.get();
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    unsafe fn acknowledge(&self) {
        unsafe {
            let card = self.content.borrow().clone();
            if let Some(raw) = card.raw {
                draft_seen(&card.id, &raw);
            }
            self.hide_card();
        }
    }
    unsafe fn destroy(&self, hwnd: HWND) {
        unsafe {
            if hwnd.is_null() {
                return;
            }
            self.clear_handle(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if DestroyWindow(hwnd) == 0 && IsWindow(hwnd) != 0 {
                crate::listener::log("FI_DESTROY_FAILED");
            }
        }
    }
    fn clear_handle(&self, hwnd: HWND) {
        if self.card.get() == hwnd {
            self.card.set(ptr::null_mut());
            self.content.replace(Card::default());
        }
        if self.icon.get() == hwnd {
            self.icon.set(ptr::null_mut());
            self.control.stop.store(true, Ordering::Release);
        }
        self.control.clear(hwnd);
    }
    unsafe fn close_windows(&self) {
        unsafe {
            self.destroy(self.card.get());
            self.destroy(self.icon.get());
        }
    }
    unsafe fn scroll_by(&self, action: Scroll) {
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(self.card.get(), &mut r);
            let page = (r.bottom - self.px(64.0)).max(1);
            let current = self.scroll.get();
            let next = match action {
                Scroll::Lines(n) => current.saturating_add(n.saturating_mul(self.px(20.0))),
                Scroll::Page(n) => current.saturating_add(n.saturating_mul(page)),
                Scroll::Home => 0,
                Scroll::End => self.max_scroll.get(),
            };
            self.scroll.set(next.clamp(0, self.max_scroll.get()));
            InvalidateRect(self.card.get(), ptr::null(), 0);
        }
    }
    unsafe fn draw_card(&self, dc: HDC, bounds: RECT, card: &Card) {
        unsafe {
            let text = wide(&card.text);
            let footer = wide(if card.raw.is_some() {
                "Enter: mark seen    Esc: close"
            } else {
                "Esc: close"
            });
            FillRect(dc, &bounds, GetStockObject(WHITE_BRUSH) as HBRUSH);
            let Some(font) = self.font.borrow().clone() else {
                return;
            };
            let _font = SelectedFont::new(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00261f19);
            let margin = self.px(12.0);
            let bottom = (bounds.bottom - self.px(40.0)).max(margin);
            let mut measured = RECT {
                left: margin,
                top: margin,
                right: (bounds.right - margin).max(margin + 1),
                bottom: margin,
            };
            let flags = DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL;
            DrawTextW(dc, text.as_ptr(), -1, &mut measured, flags | DT_CALCRECT);
            let max = (measured.bottom - measured.top - (bottom - margin)).max(0);
            self.max_scroll.set(max);
            let scroll = self.scroll.get().clamp(0, max);
            self.scroll.set(scroll);
            let saved = SaveDC(dc);
            IntersectClipRect(dc, margin, margin, bounds.right - margin, bottom);
            let mut r = RECT {
                left: margin,
                top: margin - scroll,
                right: bounds.right - margin,
                bottom: measured.bottom - scroll,
            };
            DrawTextW(dc, text.as_ptr(), -1, &mut r, flags);
            if saved != 0 {
                RestoreDC(dc, saved);
            }
            let mut footer_rect = RECT {
                left: margin,
                top: bottom + self.px(8.0),
                right: bounds.right - margin,
                bottom: bounds.bottom,
            };
            DrawTextW(
                dc,
                footer.as_ptr(),
                -1,
                &mut footer_rect,
                DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }
    unsafe fn paint_card(&self, hwnd: HWND) {
        unsafe {
            let card = self.content.borrow().clone();
            let (_paint, dc) = Paint::new(hwnd);
            let mut r = std::mem::zeroed();
            GetClientRect(hwnd, &mut r);
            self.draw_card(dc, r, &card);
        }
    }
    unsafe fn render_card(&self) {
        unsafe {
            let card = self.content.borrow().clone();
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(self.card.get(), &mut r);
            if let Ok(mut surface) = Surface::new(r.right.max(1), r.bottom.max(1)) {
                self.draw_card(surface.dc, r, &card);
                surface.save("floating-icon-card.png", false);
            }
        }
    }
    unsafe fn paint_icon(&self) -> io::Result<()> {
        unsafe {
            let frame = self.frame.borrow().clone();
            let pill = frame.pill || frame.skipped;
            let (width, height) = (self.px(WIDTH), self.px(HEIGHT));
            let left = if pill { 0 } else { width - height };
            let label = wide(&if frame.skipped {
                "Skipped".into()
            } else {
                format!("{}:{:02}", frame.elapsed / 60, frame.elapsed % 60)
            });
            let mut surface = Surface::new(width, height)?;
            surface.pixels().fill(0x001f2630);
            let font = self
                .font
                .borrow()
                .clone()
                .ok_or_else(|| io::Error::other("Icon font unavailable"))?;
            {
                let _font = SelectedFont::new(surface.dc, font);
                SetBkMode(surface.dc, TRANSPARENT as i32);
                SetTextColor(surface.dc, 0x00faf3ea);
                if pill {
                    let mut r = RECT {
                        left: self.px(if frame.skipped { 8.0 } else { 37.0 }),
                        top: 0,
                        right: width - self.px(8.0),
                        bottom: height,
                    };
                    DrawTextW(
                        surface.dc,
                        label.as_ptr(),
                        -1,
                        &mut r,
                        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                    );
                }
                let brush = CreateSolidBrush(if frame.recording {
                    0x00a6d5a0
                } else {
                    0x00b5c9ed
                });
                if pill && !frame.skipped {
                    for (offset, h) in [(15.0, 10.0), (21.0, 20.0), (27.0, 14.0)] {
                        let r = RECT {
                            left: self.px(offset),
                            right: self.px(offset + 3.0),
                            top: self.px((HEIGHT - h) / 2.0),
                            bottom: self.px((HEIGHT + h) / 2.0),
                        };
                        FillRect(surface.dc, &r, brush);
                    }
                } else if !pill {
                    let prior = SelectObject(surface.dc, brush);
                    let pen = SelectObject(surface.dc, GetStockObject(NULL_PEN));
                    let c = left + height / 2;
                    let r = self.px(5.0);
                    Ellipse(surface.dc, c - r, height / 2 - r, c + r, height / 2 + r);
                    SelectObject(surface.dc, pen);
                    SelectObject(surface.dc, prior);
                }
                DeleteObject(brush);
            }
            let radius = height as f32 / 2.0;
            for y in 0..height {
                for x in 0..width {
                    let cx = (x as f32 + 0.5).clamp(left as f32 + radius, width as f32 - radius);
                    let distance =
                        ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - radius).powi(2)).sqrt();
                    let a = ((radius - distance + 0.5).clamp(0.0, 1.0) * 255.0).round() as u32;
                    let p = &mut surface.pixels()[(y * width + x) as usize];
                    *p = (a << 24)
                        | (((*p >> 16 & 255) * a / 255) << 16)
                        | (((*p >> 8 & 255) * a / 255) << 8)
                        | ((*p & 255) * a / 255);
                }
            }
            if self.replay {
                let name = if frame.skipped {
                    "floating-icon-skipped.png"
                } else if pill {
                    "floating-icon-pill.png"
                } else {
                    "floating-icon-bead.png"
                };
                if !core::state_dir().join("local").join(name).exists() {
                    surface.save(name, true);
                }
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            if UpdateLayeredWindow(
                self.icon.get(),
                ptr::null_mut(),
                &self.point(),
                &SIZE {
                    cx: width,
                    cy: height,
                },
                surface.dc,
                &POINT { x: 0, y: 0 },
                0,
                &blend,
                ULW_ALPHA,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let region = CreateRoundRectRgn(left, 0, width + 1, height + 1, height, height);
            if region.is_null() {
                return Err(io::Error::last_os_error());
            }
            if SetWindowRgn(self.icon.get(), region, 1) == 0 {
                DeleteObject(region);
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

unsafe fn view<'a>(hwnd: HWND, msg: u32, l: LPARAM) -> Option<&'a View> {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const View).as_ref()
    }
}
pub(super) unsafe fn failed(hwnd: HWND) {
    unsafe {
        if let Some(v) = view(hwnd, 0, 0) {
            v.control.retire();
        }
    }
}

unsafe fn common(hwnd: HWND, msg: u32, l: LPARAM) -> Option<LRESULT> {
    unsafe {
        let v = view(hwnd, msg, l)?;
        if msg == RETIRE {
            v.control.stop.store(true, Ordering::Release);
            v.close_windows();
            PostQuitMessage(0);
            return Some(0);
        }
        if msg == WM_NCDESTROY {
            v.clear_handle(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        None
    }
}
unsafe extern "system" fn icon_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    callback_boundary("FI_ICON_CALLBACK", hwnd, || unsafe {
        icon_message(hwnd, msg, w, l)
    })
}
unsafe fn icon_message(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = common(hwnd, msg, l) {
            return result;
        }
        if let Some(v) = view(hwnd, msg, l) {
            match msg {
                m if m == WM_APP + 40 && v.replay => {
                    v.open_card();
                    return 0;
                }
                m if m == WM_APP + 41 && v.replay => {
                    return if w == 6 {
                        v.frame.borrow().skipped as isize
                    } else {
                        0
                    };
                }
                m if m == WM_APP + 45 && v.replay => {
                    let _ = std::fs::write(
                        core::state_dir().join("local/floating-icon-stall-started"),
                        b"started",
                    );
                    std::thread::sleep(Duration::from_secs(5));
                    return 0;
                }
                m if m == WM_APP + 46 && v.replay => panic!("synthetic callback failure"),
                WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
                WM_CLOSE => return 0,
                WM_LBUTTONDOWN => {
                    let mut p = POINT {
                        x: l as i16 as i32,
                        y: (l >> 16) as i16 as i32,
                    };
                    ClientToScreen(hwnd, &mut p);
                    v.drag.set(Some((p.y, v.layout.get().fraction)));
                    v.moved.set(false);
                    v.reopen
                        .set(!v.card.get().is_null() && IsWindowVisible(v.card.get()) != 0);
                    SetCapture(hwnd);
                    return 0;
                }
                WM_MOUSEMOVE => {
                    if let Some((start, fraction)) = v.drag.get() {
                        let mut p = POINT {
                            x: l as i16 as i32,
                            y: (l >> 16) as i16 as i32,
                        };
                        ClientToScreen(hwnd, &mut p);
                        let dy = p.y - start;
                        if dy.abs() > v.px(4.0) {
                            v.moved.set(true);
                        }
                        if v.moved.get() {
                            if v.reopen.get() {
                                v.hide_card();
                            }
                            v.set_fraction(fraction + dy as f32 / v.travel().max(1) as f32);
                            let p = v.point();
                            SetWindowPos(
                                hwnd,
                                HWND_TOPMOST,
                                p.x,
                                p.y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                            v.position_card();
                        }
                        return 0;
                    }
                }
                WM_LBUTTONUP => {
                    if v.drag.take().is_some() {
                        ReleaseCapture();
                        if v.moved.get() {
                            if store_io::write(
                                &core::state_dir().join("local/floating-icon-position.json"),
                                &v.layout.get().fraction,
                            )
                            .is_err()
                            {
                                crate::listener::log("FI_POSITION_SAVE");
                            }
                            if v.reopen.get() {
                                v.open_card();
                            }
                        } else {
                            v.open_card();
                        }
                        return 0;
                    }
                }
                WM_CAPTURECHANGED => {
                    if let Some((_, fraction)) = v.drag.take() {
                        v.set_fraction(fraction);
                        let p = v.point();
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            p.x,
                            p.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                        v.position_card();
                        if v.reopen.get() {
                            v.open_card();
                        }
                    }
                    return 0;
                }
                WM_DPICHANGED | WM_DISPLAYCHANGE => {
                    v.geometry();
                    let _ = v.paint_icon();
                    return 0;
                }
                _ => (),
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
unsafe extern "system" fn card_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    callback_boundary("FI_CARD_CALLBACK", hwnd, || unsafe {
        card_message(hwnd, msg, w, l)
    })
}
unsafe fn card_message(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = common(hwnd, msg, l) {
            return result;
        }
        if let Some(v) = view(hwnd, msg, l) {
            match msg {
                WM_GETTEXT | WM_GETTEXTLENGTH | WM_GETOBJECT | WM_PRINT | WM_PRINTCLIENT => {
                    return 0;
                }
                WM_CLOSE => {
                    v.hide_card();
                    return 0;
                }
                WM_PAINT => {
                    v.paint_card(hwnd);
                    return 0;
                }
                WM_ERASEBKGND => return 1,
                WM_SIZE => {
                    InvalidateRect(hwnd, ptr::null(), 0);
                    return 0;
                }
                WM_KEYDOWN => {
                    let action = match w as u16 {
                        VK_UP | VK_LEFT => Some(Scroll::Lines(-1)),
                        VK_DOWN | VK_RIGHT => Some(Scroll::Lines(1)),
                        VK_PRIOR => Some(Scroll::Page(-1)),
                        VK_NEXT => Some(Scroll::Page(1)),
                        VK_HOME => Some(Scroll::Home),
                        VK_END => Some(Scroll::End),
                        _ => None,
                    };
                    if let Some(action) = action {
                        v.scroll_by(action);
                        return 0;
                    }
                    if w as u16 == VK_ESCAPE {
                        v.hide_card();
                        return 0;
                    }
                    if w as u16 == VK_RETURN {
                        v.acknowledge();
                        return 0;
                    }
                }
                WM_MOUSEWHEEL => {
                    let delta = (w >> 16) as i16 as i32;
                    let sum = v.wheel.get() + delta;
                    v.wheel.set(sum % 120);
                    v.scroll_by(Scroll::Lines(-3 * (sum / 120)));
                    return 0;
                }
                m if m == WM_APP + 41 && v.replay => {
                    return match w {
                        0 => v.content.borrow().updates as isize,
                        1 => v.content.borrow().text.len() as isize,
                        2 => v.scroll.get() as isize,
                        3 => v.max_scroll.get() as isize,
                        4 => IsWindowVisible(hwnd) as isize,
                        5 => v.content.borrow().minimum_live_gap_ms.unwrap_or(0) as isize,
                        _ => 0,
                    };
                }
                m if m == WM_APP + 42 && v.replay => {
                    let action = match w {
                        0 => Scroll::Lines(-1),
                        1 => Scroll::Lines(1),
                        2 => Scroll::Page(-1),
                        3 => Scroll::Page(1),
                        4 => Scroll::Home,
                        5 => Scroll::End,
                        6 => Scroll::Lines(-3),
                        _ => Scroll::Lines(3),
                    };
                    v.scroll_by(action);
                    return v.scroll.get() as isize;
                }
                m if m == WM_APP + 43 && v.replay => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                m if m == WM_APP + 44 && v.replay => {
                    v.acknowledge();
                    return 0;
                }
                m if m == WM_APP + 46 && v.replay => panic!("synthetic callback failure"),
                _ => (),
            }
        }
        // In particular, SC_CLOSE can synchronously re-enter WM_CLOSE without any state borrow.
        DefWindowProcW(hwnd, msg, w, l)
    }
}
