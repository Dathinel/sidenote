//! Native window setup; the shared screens own the close decision.
#[cfg(windows)]
use super::DEFAULT_SIZE;
use super::State;
#[cfg(not(windows))]
static WINDOW: std::sync::OnceLock<rune::AppHandle<State>> = std::sync::OnceLock::new();

pub(super) fn configure(app: rune::App<State>) -> rune::App<State> {
    let close_handle = app.handle();
    #[cfg(not(windows))]
    let app = app.keep_running().on_close(request_close);
    #[cfg(target_os = "linux")]
    let app = app.turn_scope(
        |state| state.turn_lock_budget = Some(sidenote::core::UiLockBudget::begin()),
        |state| state.turn_lock_budget = None,
    );
    // The OS places a new window at a cascade offset, which at 125% scaling on a 1080p screen
    // pushes the bottom (and the Ask bar) off screen. Fit the size to the work area and centre.
    app.on_window(move |win| {
        native_window(win, &close_handle);
    })
}

#[cfg(windows)]
fn native_window(win: &rune::WindowHandle, close_handle: &rune::AppHandle<State>) {
    let hwnd = win
        .with_raw_handle(|h| match h.as_raw() {
            rune::raw_window_handle::RawWindowHandle::Win32(w) => w.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0);
    if hwnd != 0 {
        let handle = close_handle.clone();
        if let Err(error) = sidenote::platform::guard_window_close(hwnd, move || {
            handle.update(request_close);
        }) {
            sidenote::platform::show_error(&format!("Could not protect unsaved notes: {error}"));
            close_handle.quit();
            return;
        }
        super::hex::remember_window(hwnd);
        sidenote::platform::set_window_icon(hwnd);
        if super::benchmark::show_offscreen(win, hwnd) {
            return;
        }
        win.resize_instant(sidenote::platform::fit_to_work_area(hwnd, DEFAULT_SIZE));
    }
    win.center();
    if hwnd != 0 {
        meeting_icon::register(hwnd as _, close_handle);
    }
}

fn request_close(state: &mut State, tx: &mut rune::Tx) {
    if state.live_close() {
        tx.quit();
    } else {
        // Rune hides a kept-alive window after on_close; reopen when draft saving failed.
        #[cfg(not(windows))]
        if let Some(window) = WINDOW.get() {
            window.open();
        }
    }
}

#[cfg(not(windows))]
fn native_window(win: &rune::WindowHandle, handle: &rune::AppHandle<State>) {
    let _ = WINDOW.set(handle.clone());
    win.center();
}
#[cfg(not(windows))]
pub(super) fn shown() -> bool {
    WINDOW.get().is_none_or(rune::AppHandle::is_visible)
}

#[cfg(windows)]
pub(super) mod meeting_icon {
    use super::State;
    use sidenote::meeting_icon::{WINDOW_NUB_ARG, beside, can_restore, open_window_message};
    use std::{cell::Cell, mem::size_of, ptr, time::Instant};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    };

    #[derive(Clone, Copy)]
    struct Placement {
        hwnd: HWND,
        original: [i32; 4],
        placed: [i32; 4],
    }

    thread_local! {
        static PLACEMENT: Cell<Option<Placement>> = const { Cell::new(None) };
        static OPEN_LIVE: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    fn rect(hwnd: HWND) -> Option<[i32; 4]> {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetWindowRect(hwnd, &mut r) } != 0;
        ok.then_some([r.left, r.top, r.right - r.left, r.bottom - r.top])
    }

    fn move_window(hwnd: HWND, bounds: [i32; 4]) -> bool {
        let [x, y, width, height] = bounds;
        let set = || unsafe {
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
            ) != 0
        };
        // Winit adjusts the first move if it crosses into a different DPI.
        set() && (rect(hwnd) == Some(bounds) || set())
    }

    unsafe fn open(hwnd: HWND, icon: HWND, app: &rune::AppHandle<State>) {
        unsafe {
            let mut class = [0; 64];
            let len = GetClassNameW(icon, class.as_mut_ptr(), class.len() as i32) as usize;
            let class = String::from_utf16_lossy(&class[..len]);
            if !class.starts_with("SidenoteListenerMeetingIcon-") {
                return;
            }
            let Some(icon_rect) = rect(icon) else { return };
            let monitor = MonitorFromWindow(icon, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return;
            }
            let previous = PLACEMENT.take();
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let Some(current) = rect(hwnd) else { return };
            let original = previous
                .filter(|p| can_restore(p.placed, Some(current)))
                .map_or(current, |p| p.original);
            if IsZoomed(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let work = info.rcWork;
            let dpi = GetDpiForWindow(icon).max(96) as i32;
            let target = beside(
                [work.left, work.top, work.right, work.bottom],
                icon_rect,
                400 * dpi / 96,
                current[3],
                8 * dpi / 96,
            );
            if move_window(hwnd, target) {
                PLACEMENT.set(rect(hwnd).map(|placed| Placement {
                    hwnd,
                    original,
                    placed,
                }));
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            OPEN_LIVE.set(Some(Instant::now()));
            app.update(|_, _| {});
        }
    }

    pub(super) fn register(hwnd: HWND, app: &rune::AppHandle<State>) {
        let data = Box::into_raw(Box::new(app.clone()));
        unsafe {
            if SetWindowSubclass(hwnd, Some(message), 1, data as usize) == 0 {
                drop(Box::from_raw(data));
            } else if let Some(icon) = std::env::args()
                .find_map(|arg| arg.strip_prefix(WINDOW_NUB_ARG)?.parse::<usize>().ok())
            {
                open(hwnd, icon as HWND, app);
            }
        }
    }

    unsafe extern "system" fn message(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            let data = data as *mut rune::AppHandle<State>;
            if msg >= 0xc000 && msg == open_window_message() {
                open(hwnd, wp as HWND, &*data);
                return 0;
            }
            if msg == WM_WINDOWPOSCHANGED {
                let saved = PLACEMENT.get();
                PLACEMENT.set(saved.filter(|p| can_restore(p.placed, rect(hwnd))));
            }
            if msg == WM_NCDESTROY {
                PLACEMENT.set(None);
                OPEN_LIVE.set(None);
                RemoveWindowSubclass(hwnd, Some(message), id);
                drop(Box::from_raw(data));
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
    }

    pub(in crate::ui) fn poll(s: &mut State) -> bool {
        let pending = OPEN_LIVE.get();
        if pending.is_some() && s.hex_recording() {
            OPEN_LIVE.set(None);
            s.open_live();
            return true;
        }
        if pending.is_some_and(|at| at.elapsed().as_secs() < 10) {
            return false;
        }
        OPEN_LIVE.set(None);
        let finished = ["idle", "stopped", "complete", "finished"];
        if let Some(Ok(state)) = &s.hex.status
            && finished.contains(&state.meeting_state.as_str())
            && let Some(saved) = PLACEMENT.take()
            && can_restore(saved.placed, rect(saved.hwnd))
        {
            move_window(saved.hwnd, saved.original);
        }
        false
    }
}
