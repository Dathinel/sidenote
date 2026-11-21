mod window;
pub(super) fn render_synthetic() -> io::Result<()> {
    window::render_synthetic()
}
use std::{
    io,
    os::windows::io::AsRawHandle,
    path::Path,
    sync::{
        Arc, Mutex, Once,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, WAIT_OBJECT_0},
    Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute},
    System::Threading::{GetCurrentProcessId, WaitForSingleObject},
    UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId, PostMessageW, WM_APP},
};

const RETIRE: u32 = WM_APP + 49;
const JOIN_BOUND: Duration = Duration::from_secs(2);
static GENERATION: AtomicUsize = AtomicUsize::new(1);
static RETIRING: Mutex<Vec<Retiring>> = Mutex::new(Vec::new());
const MAX_RETIRING: usize = 2;
static DISABLED: AtomicBool = AtomicBool::new(false);
thread_local! { static CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

pub(super) fn capture_visible(root: &Path) -> bool {
    crate::core::Config::read_config(root)
        .ok()
        .flatten()
        .is_some_and(|config| config.meeting_icon_in_captures)
}

#[derive(Clone, Default)]
pub(super) struct Frame {
    pub id: String,
    pub recording: bool,
    pub pill: bool,
    pub skipped: bool,
    pub notice: Option<super::Notice>,
    pub elapsed: u64,
    pub raw: Option<String>,
    pub transcript_revision: u64,
}

#[derive(Default)]
struct Handles {
    icon: isize,
    card: isize,
}

struct Startup {
    at: Instant,
    stage: Mutex<&'static str>,
}
impl Startup {
    fn new() -> Self {
        Self {
            at: Instant::now(),
            stage: Mutex::new("thread_pending"),
        }
    }
    fn enter(&self, stage: &'static str) {
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = stage;
        self.log("FI_START_STAGE");
    }
    fn log(&self, event: &str) {
        let stage = *self.stage.lock().unwrap_or_else(|e| e.into_inner());
        crate::listener::log_process(&format!(
            "{event} stage={stage} elapsed_ms={}",
            self.at.elapsed().as_millis()
        ));
    }
    fn wait(&self, started: &mpsc::Receiver<io::Result<()>>) -> io::Result<()> {
        match started.recv_timeout(JOIN_BOUND) {
            Ok(result) => result,
            Err(error) => {
                self.log(match error {
                    mpsc::RecvTimeoutError::Timeout => "FI_START_TIMEOUT",
                    mpsc::RecvTimeoutError::Disconnected => "FI_START_CHANNEL_CLOSED",
                });
                Err(io::Error::other("Icon startup unavailable"))
            }
        }
    }
}

struct Control {
    startup: Startup,
    stop: AtomicBool,
    handles: Mutex<Handles>,
    icon_class: String,
    card_class: String,
}
impl Control {
    fn new() -> Self {
        let id = GENERATION.fetch_add(1, Ordering::Relaxed);
        Self {
            startup: Startup::new(),
            stop: AtomicBool::new(false),
            handles: Mutex::new(Handles::default()),
            icon_class: format!("SidenoteListenerMeetingIcon-{id}"),
            card_class: format!("SidenoteListenerMeetingCard-{id}"),
        }
    }
    fn owns(&self, hwnd: HWND) -> bool {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != GetCurrentProcessId() {
                return false;
            }
            let mut name = [0u16; 128];
            let n = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
            let name = String::from_utf16_lossy(&name[..n.max(0) as usize]);
            name == self.icon_class || name == self.card_class
        }
    }
    fn publish(&self, hwnd: HWND, card: bool) {
        let mut handles = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        if card {
            handles.card = hwnd as isize;
        } else {
            handles.icon = hwnd as isize;
        }
        if self.stop.load(Ordering::Acquire) {
            Self::cloak(hwnd);
        }
    }
    fn clear(&self, hwnd: HWND) {
        let mut h = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        if h.card == hwnd as isize {
            h.card = 0;
        }
        if h.icon == hwnd as isize {
            h.icon = 0;
        }
    }
    fn cloak(hwnd: HWND) -> bool {
        let yes = 1i32;
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_CLOAK as u32, (&yes as *const i32).cast(), 4) == 0
        }
    }
    fn retire(&self) -> bool {
        self.stop.store(true, Ordering::Release);
        let h = self.handles.lock().unwrap_or_else(|e| e.into_inner());
        let mut hidden = true;
        for handle in [h.icon, h.card].into_iter().filter(|h| *h != 0) {
            let hwnd = handle as HWND;
            if self.owns(hwnd) {
                // DWM cloaking does not wait for the retiring window's message pump.
                hidden &= Self::cloak(hwnd);
                unsafe {
                    PostMessageW(hwnd, RETIRE, 0, 0);
                }
            }
        }
        if !hidden {
            crate::listener::log("FI_CLOAK_FAILED");
        }
        hidden
    }
}

struct Retiring {
    control: Arc<Control>,
    worker: JoinHandle<()>,
}
fn thread_exited(worker: &JoinHandle<()>, timeout_ms: u32) -> bool {
    // Wait for native thread termination, including TLS cleanup, before calling Rust join.
    unsafe { WaitForSingleObject(worker.as_raw_handle(), timeout_ms) == WAIT_OBJECT_0 }
}
fn reap() -> bool {
    let mut workers = RETIRING.lock().unwrap_or_else(|e| e.into_inner());
    let mut hidden = true;
    let mut i = 0;
    while i < workers.len() {
        if thread_exited(&workers[i].worker, 0) {
            let worker = workers.swap_remove(i);
            if worker.worker.join().is_err() {
                crate::listener::log("FI_WORKER_PANIC");
            }
        } else {
            hidden &= workers[i].control.retire();
            i += 1;
        }
    }
    hidden
}

pub(super) fn maintenance() {
    reap();
}

pub(super) fn disabled() -> bool {
    DISABLED.load(Ordering::Acquire)
}

pub(super) fn diagnostics() -> (usize, bool) {
    (
        RETIRING.lock().unwrap_or_else(|e| e.into_inner()).len(),
        disabled(),
    )
}

pub(super) fn callback_boundary(
    code: &'static str,
    hwnd: HWND,
    f: impl FnOnce() -> isize,
) -> isize {
    let previous = CALLBACK.with(|b| b.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CALLBACK.with(|b| b.set(previous));
    match result {
        Ok(value) => value,
        Err(_) => {
            crate::listener::log(code);
            unsafe {
                window::failed(hwnd);
            }
            0
        }
    }
}

fn panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prior = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Callback payloads can contain meeting data; only the boundary logs its fixed code.
            if !CALLBACK.try_with(|b| b.get()).unwrap_or(false) {
                prior(info);
            }
        }));
    });
}

pub(super) struct Host {
    send: Option<mpsc::SyncSender<Frame>>,
    control: Arc<Control>,
    worker: Option<JoinHandle<()>>,
}
impl Host {
    pub(super) fn new(replay: bool, root: &Path) -> io::Result<Self> {
        if disabled() {
            return Err(io::Error::other("Icon disabled"));
        }
        if !reap() {
            return Err(io::Error::other("Retiring icon cannot be hidden"));
        }
        panic_hook();
        let (send, receive) = mpsc::sync_channel(1);
        let (ready, started) = mpsc::sync_channel(1);
        let control = Arc::new(Control::new());
        let worker_control = control.clone();
        let root = root.to_owned();
        let worker = std::thread::Builder::new()
            .name("meeting-icon-window".into())
            .spawn(move || {
                worker_control.startup.enter("thread_entry");
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let window =
                        match window::NativeWindow::new(replay, root, worker_control.clone()) {
                            Ok(window) => window,
                            Err(error) => {
                                let _ = ready.send(Err(error));
                                return;
                            }
                        };
                    if ready.send(Ok(())).is_err() {
                        return;
                    }
                    worker_control.startup.enter("ready_sent");
                    while !worker_control.stop.load(Ordering::Acquire) {
                        match receive.try_recv() {
                            Ok(frame) => window.update(frame),
                            Err(mpsc::TryRecvError::Empty) => (),
                            Err(mpsc::TryRecvError::Disconnected) => break,
                        }
                        if !window.pump() {
                            break;
                        }
                    }
                }));
                if result.is_err() {
                    crate::listener::log("FI_WORKER_PANIC");
                    worker_control.retire();
                }
            })?;
        let host = Self {
            send: Some(send),
            control,
            worker: Some(worker),
        };
        host.control.startup.wait(&started)?;
        Ok(host)
    }

    pub(super) fn update(&self, frame: Frame) -> bool {
        reap();
        !self.control.stop.load(Ordering::Acquire)
            && self.send.as_ref().is_some_and(|s| {
                !matches!(s.try_send(frame), Err(mpsc::TrySendError::Disconnected(_)))
            })
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.control.retire();
        self.send.take();
        if let Some(worker) = self.worker.take() {
            if thread_exited(&worker, JOIN_BOUND.as_millis() as u32) {
                if worker.join().is_err() {
                    crate::listener::log("FI_WORKER_PANIC");
                }
            } else {
                crate::listener::log("FI_TEARDOWN_TIMEOUT");
                reap();
                let mut retiring = RETIRING.lock().unwrap_or_else(|e| e.into_inner());
                if retiring.len() < MAX_RETIRING {
                    retiring.push(Retiring {
                        control: self.control.clone(),
                        worker,
                    });
                } else if !DISABLED.swap(true, Ordering::AcqRel) {
                    // The third timed-out thread stays cloaked and detaches. Never kill a Rust
                    // thread or spawn another overlay after this process-wide circuit opens.
                    crate::listener::log("ICON_DISABLED_STUCK_WORKERS");
                }
            }
        }
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn recorder_ready_stalled_start_names_the_stage_and_closed_channel_is_distinct() {
        let startup = Startup::new();
        let (send, receive) = mpsc::sync_channel(1);
        let stage = "class_registration";
        startup.enter(stage);
        assert!(startup.wait(&receive).is_err());
        drop(send);
        assert!(startup.wait(&receive).is_err());
        let log = std::fs::read_to_string(crate::core::log_path()).unwrap();
        assert!(log.contains("FI_START_TIMEOUT stage=class_registration elapsed_ms="));
        assert!(log.contains("FI_START_CHANNEL_CLOSED stage=class_registration elapsed_ms="));
    }
}
