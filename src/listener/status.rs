//! Continuous, content-free status evidence, independent of the UI and finished-event Lab.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    sync::{Mutex, mpsc},
    time::Duration,
};
#[cfg(all(windows, feature = "desktop"))]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingSignal {
    Started,
    Stopped,
}
static SUBSCRIBERS: Mutex<Vec<mpsc::SyncSender<MeetingSignal>>> = Mutex::new(Vec::new());
/// Subscribers receive transitions in this listener process. Slow subscribers never block polling.
pub fn subscribe() -> mpsc::Receiver<MeetingSignal> {
    let (tx, rx) = mpsc::sync_channel(16);
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        subscribers.push(tx);
    }
    rx
}
fn publish(signal: MeetingSignal) {
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        subscribers.retain(|tx| {
            !matches!(
                tx.try_send(signal),
                Err(mpsc::TrySendError::Disconnected(_))
            )
        });
    }
    crate::hex_engine::meeting_event();
}
pub fn remember_window(hwnd: isize) {
    let _ = crate::core::store_io::write(
        &crate::core::state_dir().join("local/status-window.json"),
        &json!({"hwnd":hwnd,"pid":std::process::id()}),
    );
}
#[cfg(all(windows, feature = "desktop"))]
fn visible() -> bool {
    if !crate::service::ui_allowed() {
        return false;
    }
    let Some(v) = fs::read(crate::core::state_dir().join("local/status-window.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    else {
        return false;
    };
    let (Some(hwnd), Some(expected)) = (v["hwnd"].as_i64(), v["pid"].as_u64()) else {
        return false;
    };
    let mut pid = 0;
    // These calls only query a window, and reject a handle reused by another process.
    unsafe {
        GetWindowThreadProcessId(hwnd as *mut _, &mut pid);
        u64::from(pid) == expected
            && IsWindowVisible(hwnd as *mut _) != 0
            && IsIconic(hwnd as *mut _) == 0
    }
}
fn append(path: &std::path::Path, value: &Value, cap: u64) -> io::Result<()> {
    fs::create_dir_all(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let bytes = serde_json::to_vec(value)?;
    if fs::metadata(path).is_ok_and(|m| m.len() + bytes.len() as u64 + 1 > cap) {
        let old = path.with_extension("previous.ndjson");
        match fs::remove_file(&old) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        fs::rename(path, old)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")
}
#[derive(Default)]
struct Transitions {
    active: bool,
}
impl Transitions {
    fn update(&mut self, status: &Value) -> Option<MeetingSignal> {
        let active = match status["meeting_state"].as_str()? {
            "starting" | "recording" | "paused" | "stopping" | "finishing" => true,
            "idle" | "stopped" | "complete" | "finished" | "transcribing" => false,
            _ => return None,
        };
        if active == self.active {
            return None;
        }
        self.active = active;
        Some(if active {
            MeetingSignal::Started
        } else {
            MeetingSignal::Stopped
        })
    }
}
fn record(status: Option<&Value>, error: Option<&str>, shown: bool) -> Value {
    json!({"timestamp":chrono::Utc::now().to_rfc3339(),
        "app_version":status.map(|s| &s["app_version"]), "protocol":status.map(|s| &s["protocol"]),
        "meeting_state":status.map(|s| &s["meeting_state"]), "capture_state":status.map(|s| &s["capture_state"]),
        "capture_kind":status.map(|s| &s["capture_kind"]), "has_elapsed":status.map(|s| &s["has_elapsed"]),
        "has_id":status.map(|s| &s["has_id"]), "status_error":error, "window_visible":shown})
}
pub(super) fn start() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| {
        let state = crate::core::state_dir();
        let Ok(_instance) = crate::core::store_io::lock(&state.join("local/status-listener.lock"))
        else {
            return;
        };
        let path = state.join("local/status.ndjson");
        let mut transitions = Transitions::default();
        let (measurement, samples) = mpsc::sync_channel::<(Value, bool)>(1);
        let evidence_state = state.clone();
        let evidence_worker = std::thread::spawn(move || {
            let mut evidence = crate::core::meeting_evidence::Collector::new(&evidence_state);
            let mut failed = false;
            while let Ok((status, enabled)) = samples.recv() {
                let error = evidence.tick(&status, enabled).is_err();
                if error && !failed {
                    super::log("window measurement pending; retrying");
                }
                failed = error;
            }
        });
        let mut log_failed = false;
        'poll: while !crate::updates::listener_stopping() {
            let status = crate::hex_engine::listener_status();
            // Never format transport errors: a future server error could contain content.
            let error = status
                .as_ref()
                .err()
                .map(|e| match e {
                    crate::hex_engine::HexError::HexNotRunning => "not_running",
                    crate::hex_engine::HexError::Timeout => "timeout",
                    crate::hex_engine::HexError::Unsupported => "unsupported_protocol",
                    _ => "status_failed",
                })
                .or_else(|| {
                    status
                        .as_ref()
                        .ok()
                        .filter(|s| s["protocol"] != 4)
                        .map(|_| "unsupported_protocol")
                });
            let entry = record(status.as_ref().ok(), error, visible());
            let failed = append(&path, &entry, 1024 * 1024).is_err();
            if failed && !log_failed {
                super::log("could not write status evidence");
            }
            log_failed = failed;
            if let Ok(status) = status
                && status["protocol"] == 4
            {
                if let Some(signal) = transitions.update(&status) {
                    publish(signal);
                }
                let config =
                    crate::core::store_io::defer_busy(crate::core::config::Config::load_in(&state));
                // File conversion cannot delay status evidence or start/stop signals.
                if let Ok(Some(config)) = config {
                    let _ = measurement.try_send((status, config.labs.process_meeting_windows));
                }
            }
            for _ in 0..12 {
                if crate::updates::listener_stopping() {
                    break 'poll;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        drop(measurement);
        let _ = evidence_worker.join();
    })
}
#[cfg(not(all(windows, feature = "desktop")))]
fn visible() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_survive_unknown_status_and_stop_only_on_confirmed_idle() {
        let mut t = Transitions::default();
        for (state, expected) in [
            ("recording", Some(MeetingSignal::Started)),
            ("recording", None),
            ("unknown", None),
            ("paused", None),
            ("stopping", None),
            ("transcribing", Some(MeetingSignal::Stopped)),
            ("idle", None),
            ("idle", None),
        ] {
            assert_eq!(t.update(&json!({"meeting_state":state})), expected);
        }
    }
    #[test]
    fn log_allowlist_and_rotation_never_include_content_or_identifiers() {
        let value = json!({"app_version":"test","protocol":4,"meeting_state":"recording","capture_state":"idle","capture_kind":null,"has_elapsed":true,"has_id":true,
            "id":"sensitive-id","elapsed":123456,"title":"sensitive-title","text":"sensitive-text","token":"sensitive-token"});
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("status.ndjson");
        let entry = record(Some(&value), None, false);
        let cap = serde_json::to_vec(&entry).unwrap().len() as u64 + 2;
        for _ in 0..4 {
            append(&path, &entry, cap).unwrap();
        }
        for p in [path.clone(), path.with_extension("previous.ndjson")] {
            let b = fs::read_to_string(p).unwrap();
            assert!(!b.contains("sensitive"));
            assert!(!b.contains("123456"));
            assert_eq!(b.lines().count(), 1);
            assert!(b.contains("window_visible"));
        }
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }
}
