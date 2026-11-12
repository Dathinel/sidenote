//! Device-local roles and a window-independent service lifetime.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceRole {
    #[default]
    Full,
    Server,
    Hub,
}
impl DeviceRole {
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Server => "server",
            Self::Hub => "hub",
        }
    }
}
static ROLE: OnceLock<DeviceRole> = OnceLock::new();
static HEADLESS: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
struct Worker {
    child: std::process::Child,
    _job: crate::platform::ProcessJob,
}
// Handles are only accessed under this mutex; Windows job handles support cross-thread close.
#[cfg(windows)]
unsafe impl Send for Worker {}
static CHILDREN: std::sync::Mutex<Vec<Worker>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn own_child(mut child: std::process::Child) -> std::io::Result<()> {
    if !active() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    let job = match crate::platform::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    match CHILDREN.lock() {
        Ok(mut children) => {
            children.push(Worker { child, _job: job });
            Ok(())
        }
        Err(_) => {
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            Err(std::io::Error::other("SERVICE_WORKER_FAILED"))
        }
    }
}

fn reap_children(stop: bool) {
    if let Ok(mut children) = CHILDREN.lock() {
        let mut running = Vec::new();
        for mut worker in children.drain(..) {
            if stop {
                drop(worker._job);
                let _ = worker.child.kill();
                let _ = worker.child.wait();
            } else if !matches!(worker.child.try_wait(), Ok(Some(_))) {
                running.push(worker);
            }
        }
        *children = running;
    }
}

pub fn stopping() -> bool {
    STOP.load(Ordering::Acquire)
}
pub(crate) fn active() -> bool {
    HEADLESS.load(Ordering::Acquire)
}
pub fn role() -> DeviceRole {
    ROLE.get().copied().unwrap_or_default()
}

pub fn configure(args: &[OsString]) -> Result<(), &'static str> {
    let mut selected = None;
    let mut input = args.iter();
    while let Some(arg) = input.next() {
        if arg == "--device-role" {
            selected = Some(match input.next().and_then(|a| a.to_str()) {
                Some("full") => DeviceRole::Full,
                Some("server") => DeviceRole::Server,
                Some("hub") => DeviceRole::Hub,
                _ => return Err("INVALID_DEVICE_ROLE"),
            });
        }
    }
    if let Some(value) = selected {
        crate::core::DeviceSettings::set_role(value).map_err(|_| "DEVICE_CONFIG_FAILED")?;
    }
    let settings = crate::core::DeviceSettings::read_in(&crate::core::state_dir())
        .map_err(|_| "DEVICE_CONFIG_FAILED")?;
    ROLE.set(settings.role)
        .map_err(|_| "DEVICE_CONFIG_FAILED")?;
    HEADLESS.store(
        args.iter()
            .any(|a| a == "--server" || a == "--listen" || a == "--mcp"),
        Ordering::Release,
    );
    Ok(())
}

pub fn requested(args: &[OsString]) -> bool {
    if args.iter().any(|a| a == "--server" || a == "--listen") {
        return true;
    }
    // Persisted roles apply to ordinary launch, leaving explicit one-shot commands usable.
    role() != DeviceRole::Full
        && args.iter().all(|a| {
            !a.to_string_lossy().starts_with("--")
                || a == "--data-dir"
                || a == "--meetings-dir"
                || a == "--peers-dir"
                || a == "--device-role"
        })
}

/// The floating icon must call this before creating a thread, HWND or graphics context.
pub fn ui_allowed() -> bool {
    if !cfg!(feature = "desktop") || role() != DeviceRole::Full {
        return false;
    }
    #[cfg(all(windows, feature = "desktop"))]
    {
        crate::platform::desktop_session()
    }
    #[cfg(all(windows, not(feature = "desktop")))]
    {
        false
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
    }
}

pub(crate) fn pause(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while !stopping()
        && !crate::updates::listener_stopping()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(
            Duration::from_millis(100)
                .min(deadline.saturating_duration_since(std::time::Instant::now())),
        );
    }
}

fn supervise() {
    let root = crate::core::state_dir();
    let mut listener = None;
    while !stopping() && !crate::updates::listener_stopping() {
        reap_children(false);
        let sync_ok = crate::core::reconcile_service();
        let relay = match crate::core::relay_preview::get(&root) {
            Ok(Some(_)) => "running",
            Ok(None) => "disabled",
            Err(_) if cfg!(windows) => "retry",
            Err(_) => "UNAVAILABLE_ON_PLATFORM",
        };
        let hex = if role() != DeviceRole::Hub
            && crate::core::meetings_dir()
                .parent()
                .is_some_and(|p| p.join("local-api.json").is_file())
        {
            listener.get_or_insert_with(crate::listener::service_background);
            "configured"
        } else {
            "unavailable"
        };
        let processing = if role() == DeviceRole::Hub {
            "disabled"
        } else if crate::listener::service_tick().is_ok() {
            "ready"
        } else {
            "unavailable"
        };
        eprintln!(
            "service status=running role={} sync={} relay={} processing={} hex={} mcp=stdio ui={}",
            role().name(),
            if sync_ok { "ready" } else { "retry" },
            relay,
            processing,
            hex,
            ui_allowed()
        );
        pause(Duration::from_secs(30));
    }
    STOP.store(true, Ordering::Release);
    let _ = crate::core::relay_preview::disconnect(&root);
    reap_children(true);
    drop(listener);
}

pub fn run() -> i32 {
    HEADLESS.store(true, Ordering::Release);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("SERVICE_RUNTIME_FAILED");
            return 2;
        }
    };
    runtime.block_on(async {
        #[cfg(unix)]
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    eprintln!("SERVICE_SIGNAL_FAILED");
                    return 2;
                }
            };
        let _instance =
            match crate::core::store_io::lock(&crate::core::state_dir().join("local/service.lock"))
            {
                Ok(instance) => instance,
                Err(_) => {
                    eprintln!("SERVICE_LOCK_FAILED");
                    return 2;
                }
            };
        #[cfg(windows)]
        let _update_instance = match crate::updates::listener_lock() {
            Ok(instance) if !crate::updates::listener_stopping() => instance,
            Ok(_) => return 0,
            Err(_) => { eprintln!("SERVICE_LOCK_FAILED"); return 2; }
        };
        if crate::core::DeviceSettings::load().is_err() {
            eprintln!("DEVICE_CONFIG_FAILED");
            return 2;
        }
        let worker = match std::thread::Builder::new()
            .name("service".into())
            .spawn(supervise)
        {
            Ok(worker) => worker,
            Err(_) => {
                eprintln!("SERVICE_START_FAILED");
                return 2;
            }
        };
        let lifetime = async {
            while !stopping() && !worker.is_finished() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        #[cfg(unix)]
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {}, _ = lifetime => {} }
        #[cfg(not(unix))]
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = lifetime => {} }
        STOP.store(true, Ordering::Release);
        crate::core::ask::cancel_all(Duration::from_secs(3));
        let result = if worker.join().is_ok() { 0 } else { 2 };
        eprintln!("service status=stopped role={}", role().name());
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roles_migrate_and_reject_unknown_values() {
        let old: crate::core::DeviceSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.role, DeviceRole::Full);
        for role in [DeviceRole::Full, DeviceRole::Server, DeviceRole::Hub] {
            assert_eq!(
                serde_json::from_str::<DeviceRole>(&serde_json::to_string(&role).unwrap()).unwrap(),
                role
            );
        }
        assert!(serde_json::from_str::<DeviceRole>("\"unknown\"").is_err());
    }
}
