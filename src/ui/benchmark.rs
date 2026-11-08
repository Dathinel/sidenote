//! Opt-in, app-owned transcript traversal for memory measurements, without desktop input.
use super::{PendingScroll, State};
use rune::AppHandle;
use std::{path::PathBuf, time::Duration};

struct HeadlessMemory(PathBuf);

pub(super) fn configure(mut app: rune::App<State>) -> rune::App<State> {
    if let Some(path) = sidenote::env_var("SIDENOTE_BENCH_HEADLESS_READY") {
        assert!(
            std::env::args().any(|a| a == "--snap"),
            "memory probe requires --snap"
        );
        app = app.compositor(HeadlessMemory(PathBuf::from(path)));
    }
    if sidenote::env_var("SIDENOTE_BENCH_OFFSCREEN").is_some() {
        app = app.window_options(rune::WindowOptions {
            visible: false,
            position: Some((-32000, -32000)),
            ..Default::default()
        });
    }
    start(app.handle());
    app
}

pub(super) fn show_offscreen(win: &rune::WindowHandle, hwnd: isize) -> bool {
    if sidenote::env_var("SIDENOTE_BENCH_OFFSCREEN").is_none() {
        return false;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    // The creation callback owns this live HWND. Stay hidden until both position and style are verified.
    unsafe {
        let handle = hwnd as *mut std::ffi::c_void;
        assert_eq!(
            IsWindowVisible(handle),
            0,
            "benchmark window must start hidden"
        );
        let style = GetWindowLongPtrW(handle, GWL_EXSTYLE);
        SetWindowLongPtrW(
            handle,
            GWL_EXSTYLE,
            style | WS_EX_NOACTIVATE as isize | WS_EX_TOOLWINDOW as isize,
        );
        assert_ne!(
            SetWindowPos(
                handle,
                std::ptr::null_mut(),
                -32000,
                -32000,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER
            ),
            0
        );
        assert_ne!(
            GetWindowLongPtrW(handle, GWL_EXSTYLE) & WS_EX_NOACTIVATE as isize,
            0
        );
        let mut rect = std::mem::zeroed();
        assert_ne!(GetWindowRect(handle, &mut rect), 0);
        assert!(rect.right <= GetSystemMetrics(SM_XVIRTUALSCREEN));
        assert!(rect.bottom <= GetSystemMetrics(SM_YVIRTUALSCREEN));
        ShowWindow(handle, SW_SHOWNOACTIVATE);
        assert_ne!(GetForegroundWindow(), handle);
        if let Some(path) = sidenote::env_var("SIDENOTE_BENCH_WINDOW_READY") {
            let _ = std::fs::write(path, "offscreen-noactivate");
        }
    }
    win.show();
    true
}

impl rune::Compositor<State> for HeadlessMemory {
    fn render(&mut self, frame: &mut rune::Frame<'_, State>) {
        frame.draw_ui();
    }

    fn summary(&mut self, _: &wgpu::Device) -> Option<String> {
        let _ = std::fs::write(&self.0, "ready");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !self.0.with_extension("release").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }
}

pub(super) fn start(app: AppHandle<State>) {
    let Some(path) = sidenote::env_var("SIDENOTE_BENCH_COMMAND").map(PathBuf::from) else {
        return;
    };
    std::thread::spawn(move || {
        loop {
            if std::fs::read_to_string(&path).is_ok_and(|s| s.trim() == "scroll") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let (send, recv) = std::sync::mpsc::channel();
        app.update(move |s, _| {
            let meeting = s.meetings.iter().max_by_key(|m| m.minutes.unwrap_or(0));
            let info = meeting.map(|m| (m.id.clone(), m.lines.len(), m.minutes));
            if let Some((id, _, _)) = &info {
                s.selected = Some(id.clone());
                s.transcript_open = true;
                s.load_saved();
            }
            let _ = send.send(info);
        });
        let Ok(Some((id, lines, minutes))) = recv.recv_timeout(Duration::from_secs(10)) else {
            return;
        };
        for line in (0..lines).step_by(4).chain(lines.checked_sub(1)) {
            app.update(move |s, _| {
                s.pending_scroll = Some(PendingScroll {
                    key: format!("line-{line}"),
                    center: true,
                    left: 1,
                });
            });
            std::thread::sleep(Duration::from_millis(80));
        }
        app.update(move |_, _| {
            let _ = std::fs::write(
                path.with_extension("done"),
                serde_json::json!({"meeting": id, "lines": lines, "minutes": minutes}).to_string(),
            );
        });
    });
}
