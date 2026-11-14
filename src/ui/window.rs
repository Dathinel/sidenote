//! Native window setup; the shared screens own the close decision.
use super::{DEFAULT_SIZE, State};

pub(super) fn configure(app: rune::App<State>) -> rune::App<State> {
    let close_handle = app.handle();
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
}

fn request_close(state: &mut State, tx: &mut rune::Tx) {
    if state.live_close() {
        tx.quit();
    }
}
