//! Standalone Rune/Win32 feasibility probe. Never initializes Sidenote or Hex services.
#![windows_subsystem = "windows"]
use rune::{
    Align, App, AppSpec, Color, Cx, DisplayTarget, El, Key, KeyEvent, OverlayHandle, OverlayInput,
    OverlayOptions, OverlaySpec, Phase, ScreenEdge, TextEdit, Tray, TrayEvent, TrayIcon, TrayItem,
    TraySpec, Tx, WindowHandle, WindowOptions, WindowSpec, col, el, row, text, text_input,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetProcessDpiAwarenessContext,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const ICON: &str = "Sidenote floating icon spike";
const PANEL: &str = "Sidenote floating panel spike";
const BG: Color = Color {
    r: 0.12,
    g: 0.15,
    b: 0.19,
    a: 1.0,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Work {
    monitor: isize,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    monitor_right: i32,
    scale: f32,
}
impl Work {
    fn primary() -> Option<Self> {
        // The nearest monitor to (0,0) is primary in the Windows desktop coordinate space.
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let (mut x, mut y) = (96, 96);
            let dpi_result = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
            if dpi_result != 0 {
                return None;
            }
            Some(Self {
                monitor: monitor as isize,
                left: info.rcWork.left,
                top: info.rcWork.top,
                right: info.rcWork.right,
                bottom: info.rcWork.bottom,
                monitor_right: info.rcMonitor.right,
                scale: x as f32 / 96.0,
            })
        }
    }
    fn travel(self, height: f32) -> f32 {
        ((self.bottom - self.top) as f32 / self.scale - height - 16.0).max(0.0)
    }
    fn margin(self, fraction: f32, height: f32) -> f32 {
        8.0 + fraction.clamp(0.0, 1.0) * self.travel(height)
    }
}

struct Shared {
    icon: Option<OverlayHandle<Icon>>,
    panel: Option<WindowHandle>,
    fraction: f32,
    pill: bool,
    hybrid: bool,
    animated_width: Option<f32>,
    panel_open: bool,
    atomic_anchor: bool,
    work: Work,
    state_path: PathBuf,
}
impl Shared {
    fn size(&self) -> (f32, f32) {
        if self.hybrid {
            return (
                self.animated_width
                    .unwrap_or(if self.pill { 104.0 } else { 44.0 }),
                44.0,
            );
        }
        if self.pill {
            (136.0, 38.0)
        } else {
            (48.0, 48.0)
        }
    }
    fn place(&self) {
        let (w, h) = self.size();
        let margin = self.work.margin(self.fraction, h);
        if self.atomic_anchor {
            let hwnd = own_window(ICON);
            if !hwnd.is_null() {
                let right = self.work.right - (8.0 * self.work.scale).round() as i32;
                unsafe {
                    assert_ne!(
                        SetWindowSubclass(hwnd, Some(anchor), 0x53494445, right as usize),
                        0
                    );
                }
            }
        }
        if let Some(icon) = &self.icon {
            icon.set_display(DisplayTarget::Primary);
            icon.resize((w, h));
            icon.set_align(
                Align::End,
                8.0 + (self.work.monitor_right - self.work.right) as f32 / self.work.scale,
            );
            icon.place(ScreenEdge::Top, margin);
        }
    }
    fn position_panel(&self) {
        let (w, h) = self.size();
        let margin = self.work.margin(self.fraction, h);
        if let Some(panel) = &self.panel {
            let scale = self.work.scale;
            let panel_w = (360.0 * scale).round() as i32;
            let panel_h = (260.0 * scale).round() as i32;
            let x = (self.work.right - ((w + 16.0) * scale).round() as i32 - panel_w)
                .max(self.work.left);
            let y = (self.work.top + (margin * scale).round() as i32)
                .min((self.work.bottom - panel_h).max(self.work.top));
            panel.set_position((x, y));
            panel.resize_instant((360.0, 260.0));
        }
    }
    fn open_panel(&mut self) {
        if self.hybrid {
            self.animated_width = Some(44.0);
            self.place();
        }
        self.position_panel();
        if let Some(panel) = &self.panel {
            panel.open();
            self.panel_open = true;
        }
    }
    fn save(&self) {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent).expect("spike state directory");
        }
        std::fs::write(&self.state_path, self.fraction.to_string()).expect("save spike position");
        println!(
            "saved fraction={:.6} margin={:.2}",
            self.fraction,
            self.work.margin(self.fraction, self.size().1)
        );
    }
    fn nudge(&mut self, points: f32) {
        self.fraction =
            (self.fraction + points / self.work.travel(self.size().1).max(1.0)).clamp(0.0, 1.0);
        self.place();
        self.save();
    }
}
// Rune requests size and position separately; bind the right edge to each native change.
unsafe extern "system" fn anchor(
    hwnd: HWND,
    message: u32,
    w: usize,
    l: isize,
    id: usize,
    data: usize,
) -> isize {
    unsafe {
        if message == WM_WINDOWPOSCHANGING {
            let position = &mut *(l as *mut WINDOWPOS);
            let mut rect = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 {
                let width = if position.flags & SWP_NOSIZE != 0 {
                    rect.right - rect.left
                } else {
                    position.cx
                };
                if position.flags & SWP_NOMOVE != 0 {
                    position.y = rect.top;
                }
                position.x = data as i32 - width;
                position.flags &= !SWP_NOMOVE;
            }
        } else if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(anchor), id);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
struct Icon {
    shared: Arc<Mutex<Shared>>,
    start: f32,
    moved: bool,
    reported: bool,
    reopen_on_drop: bool,
}
struct Panel {
    shared: Arc<Mutex<Shared>>,
    note: TextEdit,
}
fn move_panel_closed(s: &mut Panel, _: &mut Tx) {
    s.shared.lock().unwrap().panel_open = false;
    println!("panel closed; icon stays alive");
}
fn icon_view(s: &Icon, _: &mut Cx<Icon>) -> El {
    let pill = s.shared.lock().unwrap().size().0 >= 90.0;
    el().key("icon")
        .w_full()
        .h_full()
        .radius(24.0)
        .fill(BG)
        .center()
        .hit()
        .child(text(if pill { "●  12:34  spike" } else { "●" }).color(Color::WHITE))
        .on_drag(|s: &mut Icon, d, tx| {
            tx.instant();
            let mut shared = s.shared.lock().unwrap();
            if d.phase == Phase::Down {
                s.start = shared.fraction;
                s.moved = false;
                s.reopen_on_drop = shared.panel_open;
            }
            if d.dy.abs() > 4.0 {
                if !s.moved && shared.panel_open {
                    if let Some(panel) = &shared.panel {
                        panel.hide();
                    }
                    shared.panel_open = false;
                    println!("panel hidden for drag");
                }
                s.moved = true;
            }
            if s.moved {
                shared.fraction = if d.cancelled() {
                    s.start
                } else {
                    (s.start + d.dy / shared.work.travel(shared.size().1).max(1.0)).clamp(0.0, 1.0)
                };
                shared.place();
            }
            println!(
                "drag {:?} dy={:.2} fraction={:.6} cancelled={}",
                d.phase,
                d.dy,
                shared.fraction,
                d.cancelled()
            );
            if d.phase == Phase::Up {
                if s.moved {
                    shared.save();
                    if s.reopen_on_drop {
                        shared.open_panel();
                        println!("panel reopened after drop");
                    }
                } else if !d.cancelled() {
                    shared.open_panel();
                    println!("panel open requested by icon click");
                }
            }
        })
}
fn button(label: &'static str, f: impl Fn(&mut Panel, &mut Tx) + 'static) -> El {
    el().padding(8.0)
        .fill(Color::BLACK)
        .radius(4.0)
        .child(text(label).color(Color::WHITE))
        .on_press_tx(f)
}
fn panel_view(s: &Panel, _: &mut Cx<Panel>) -> El {
    println!("panel render note={:?}", s.note.text);
    col()
        .w_full()
        .h_full()
        .padding(16.0)
        .gap(12.0)
        .fill(BG)
        .child(text("Floating icon spike").color(Color::WHITE))
        .child(text("Drag icon vertically. Focus Move for ↑ / ↓.").color(Color::WHITE))
        .child(
            text_input(&s.note, rune::tok::type_::BODY)
                .key("note")
                .w_full()
                .h(48.0)
                .fill(Color::WHITE)
                .color(Color::BLACK)
                .on_edit(|s: &mut Panel| &mut s.note),
        )
        .child(
            row()
                .gap(8.0)
                .child(
                    button("Move ↑ / ↓", |_, _| {})
                        .key("move")
                        .focusable(true)
                        .on_key(|s: &mut Panel, ev: &KeyEvent, _| {
                            let delta = match ev.key {
                                Key::Up => -16.0,
                                Key::Down => 16.0,
                                _ => return false,
                            };
                            s.shared.lock().unwrap().nudge(delta);
                            true
                        }),
                )
                .child(button("Bead / pill", |s, _| {
                    let mut shared = s.shared.lock().unwrap();
                    shared.pill = !shared.pill;
                    shared.animated_width = None;
                    if let Some(icon) = &shared.icon {
                        icon.update(|_, _| {});
                    }
                    shared.place();
                })),
        )
        .child(
            row()
                .gap(8.0)
                .child(button("Close panel", |_, tx| tx.close_window()))
                .child(button("Quit spike", |_, tx| tx.quit())),
        )
}
fn poll(s: &mut Icon, _: f64, tx: &mut Tx) -> bool {
    tx.wake_after(Duration::from_millis(500));
    if !s.reported
        && let Some(icon) = &s.shared.lock().unwrap().icon
    {
        let stats = icon.surface_stats();
        if stats.totals.presents > 0 {
            println!(
                "overlay renderer={:?}; {}",
                icon.surface_description(),
                stats.summary()
            );
            s.reported = true;
        }
    }
    let Some(work) = Work::primary() else {
        return false;
    };
    let mut shared = s.shared.lock().unwrap();
    if work != shared.work {
        println!("primary/work/DPI changed: {work:?}");
        shared.work = work;
        shared.place();
        if shared.panel_open {
            shared.position_panel();
        }
    }
    false
}
fn own_window(title: &str) -> HWND {
    let wide: Vec<_> = title.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            hwnd
        } else {
            std::ptr::null_mut()
        }
    }
}
fn main() {
    // Query physical monitor geometry before winit constructs its event loop.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<_> = std::env::args_os().collect();
    let state_path = args.windows(2).find(|a| a[0] == "--state").map_or_else(
        || PathBuf::from("artifacts/floating-icon-spike/position.txt"),
        |a| PathBuf::from(&a[1]),
    );
    let fraction = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|n| n.is_finite())
        .unwrap_or(0.4)
        .clamp(0.0, 1.0);
    let pill = args.iter().any(|a| a == "--pill");
    let resize_proof = args.iter().any(|a| a == "--resize-proof");
    let visual_proof = args.iter().any(|a| a == "--visual-proof");
    let atomic_anchor = args.iter().any(|a| a == "--atomic-anchor");
    let hybrid = resize_proof || args.iter().any(|a| a == "--hybrid");
    let mut app = App::new(AppSpec::new(ICON, (1.0, 1.0), || (), |_, _| el()))
        .windowless()
        .args([]);
    let shared = Arc::new(Mutex::new(Shared {
        icon: None,
        panel: None,
        fraction,
        pill,
        hybrid,
        animated_width: None,
        panel_open: false,
        atomic_anchor,
        work: Work::primary().expect("primary monitor geometry"),
        state_path,
    }));
    println!(
        "startup fraction={fraction:.6} pill={pill} work={:?}",
        shared.lock().unwrap().work
    );
    let panel_state = Panel {
        shared: shared.clone(),
        note: TextEdit::new(""),
    };
    let panel_app = App::new(AppSpec::new(
        PANEL,
        (360.0, 260.0),
        || unreachable!(),
        panel_view,
    ))
    .init(move || panel_state)
    .on_close(move_panel_closed);
    let (_, panel) = app.window(WindowSpec::from_app("panel", panel_app).options(WindowOptions {
        borderless: true, always_on_top: true, visible: false, min_size: Some((0.0, 0.0)), resizable: false,
        ..Default::default()
    }).on_created(|window| {
        window.with_raw_handle(|raw| {
            if let rune::raw_window_handle::RawWindowHandle::Win32(h) = raw.as_raw() {
                unsafe {
                    let hwnd = h.hwnd.get() as HWND;
                    let ok = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
                    let mut affinity = 0;
                    let read = GetWindowDisplayAffinity(hwnd, &mut affinity);
                    println!("panel SetWindowDisplayAffinity={ok} GetWindowDisplayAffinity={read} affinity={affinity:#x}");
                    assert_ne!(ok, 0, "panel capture exclusion failed");
                }
            }
        }).expect("panel native handle");
    }));
    shared.lock().unwrap().panel = Some(panel);
    let size = shared.lock().unwrap().size();
    let icon = app.overlay(
        OverlaySpec {
            name: ICON,
            size,
            edge: ScreenEdge::Top,
            margin: 8.0,
            align: Align::End,
            inset: 8.0,
            interactive: true,
            view: icon_view,
        }
        .options(OverlayOptions {
            input: OverlayInput::All,
            // Only this synthetic inspection mode allows recording the placeholder's resize.
            exclude_from_capture: !visual_proof,
            ..Default::default()
        })
        .poll(poll)
        .on_created(move |created| {
            println!("overlay created {created:?}");
            unsafe {
                let hwnd = own_window(ICON);
                let mut affinity = 0;
                let ok = GetWindowDisplayAffinity(hwnd, &mut affinity);
                println!("icon GetWindowDisplayAffinity={ok} affinity={affinity:#x}");
                assert!(!hwnd.is_null());
                assert_eq!(
                    affinity,
                    if visual_proof {
                        WDA_NONE
                    } else {
                        WDA_EXCLUDEFROMCAPTURE
                    }
                );
            }
        }),
        Icon {
            shared: shared.clone(),
            start: fraction,
            moved: false,
            reported: false,
            reopen_on_drop: false,
        },
    );
    shared.lock().unwrap().icon = Some(icon.clone());
    shared.lock().unwrap().place();
    icon.show();
    let tray_icon = TrayIcon::rgba(16, 16, [230, 155, 45, 255].repeat(16 * 16));
    let tray_overlay = icon.clone();
    let quit = app.handle();
    let tray = Tray::new(
        TraySpec::new("sidenote-floating-spike", "Sidenote spike", tray_icon)
            .tooltip("Sidenote floating icon spike")
            .menu(vec![
                TrayItem::action("show-panel", "Show meeting panel"),
                TrayItem::action("quit", "Quit spike"),
            ]),
        move |event| {
            println!("tray event {event:?}");
            match event {
                TrayEvent::Activate { .. } => {
                    tray_overlay.update(|s, _| s.shared.lock().unwrap().open_panel())
                }
                TrayEvent::Menu(id) if id == "show-panel" => {
                    tray_overlay.update(|s, _| s.shared.lock().unwrap().open_panel())
                }
                TrayEvent::Menu(id) if id == "quit" => quit.quit(),
                _ => {}
            }
        },
    )
    .expect("Rune Windows tray");
    println!("tray created: Show meeting panel");
    if resize_proof {
        let resize_icon = icon.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(8));
            println!("resize proof begin");
            for _ in 0..10 {
                for (from, to, steps, millis) in [(44.0, 104.0, 7, 220), (104.0, 44.0, 6, 180)] {
                    for step in 1..=steps {
                        let width = from + (to - from) * step as f32 / steps as f32;
                        resize_icon.update(move |s, tx| {
                            if s.shared.lock().unwrap().atomic_anchor {
                                tx.instant();
                            }
                            let mut shared = s.shared.lock().unwrap();
                            if shared.panel_open {
                                return;
                            }
                            shared.animated_width = Some(width);
                            shared.place();
                        });
                        std::thread::sleep(Duration::from_millis(millis / steps));
                    }
                    std::thread::sleep(Duration::from_millis(800));
                }
            }
            println!(
                "resize proof end; {}",
                resize_icon.surface_stats().summary()
            );
        });
    }
    if let Some(seconds) = args
        .windows(2)
        .find(|a| a[0] == "--seconds")
        .and_then(|a| a[1].to_str()?.parse::<u64>().ok())
    {
        let handle = app.handle();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            handle.quit();
        });
    }
    app.run();
    drop(tray);
    println!("tray removed; spike exiting");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_clamping_and_dpi() {
        for scale in [1.0, 1.5] {
            for (left, top, right, bottom) in [(0, 0, 1920, 1032), (-1920, -120, 0, 912)] {
                let work = Work {
                    monitor: 0,
                    left,
                    top,
                    right,
                    bottom,
                    monitor_right: right,
                    scale,
                };
                for height in [38.0, 48.0] {
                    assert_eq!(work.margin(-4.0, height), 8.0);
                    assert!(
                        (work.margin(4.0, height) + height + 8.0 - (bottom - top) as f32 / scale)
                            .abs()
                            < 0.01
                    );
                }
            }
        }
        let tiny = Work {
            monitor: 0,
            left: 0,
            top: 0,
            right: 100,
            bottom: 40,
            monitor_right: 100,
            scale: 1.5,
        };
        assert_eq!(tiny.travel(48.0), 0.0);
    }
}
