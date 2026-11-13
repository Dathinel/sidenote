//! Device-local roles and a window-independent service lifetime.
#[cfg(target_os = "linux")]
mod schedule;
#[cfg(target_os = "linux")]
pub(crate) mod workers;
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
    #[cfg(windows)]
    let configured = selected
        .map(crate::core::DeviceSettings::set_role)
        .transpose()
        .and_then(|_| load_role(&crate::core::state_dir()));
    #[cfg(not(windows))]
    let configured = {
        if let Some(selected) = selected {
            crate::core::DeviceSettings::set_role(selected).map_err(|_| "DEVICE_CONFIG_FAILED")?;
        }
        load_role(&crate::core::state_dir())
    };
    let role = role_or_full(configured, || {
        crate::listener::log("DEVICE_ROLE_UNAVAILABLE; using full");
    });
    ROLE.set(role).map_err(|_| "DEVICE_CONFIG_FAILED")?;
    HEADLESS.store(
        role != DeviceRole::Full
            && args
                .iter()
                .any(|a| a == "--server" || a == "--listen" || a == "--mcp"),
        Ordering::Release,
    );
    Ok(())
}

fn load_role(root: &std::path::Path) -> std::io::Result<DeviceRole> {
    let bytes = match crate::core::store_io::read_bounded(
        &root.join("device.json"),
        crate::core::file_reads::SUMMARY_MAX,
    ) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(DeviceRole::Full),
        Err(error) => return Err(error),
    };
    let settings: crate::core::DeviceSettings =
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
    Ok(settings.role)
}

fn role_or_full(result: std::io::Result<DeviceRole>, report: impl FnOnce()) -> DeviceRole {
    result.unwrap_or_else(|_| {
        report();
        DeviceRole::Full
    })
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

fn sync_status(
    role: DeviceRole,
    root: &std::path::Path,
    reconcile: impl FnOnce() -> bool,
) -> (&'static str, &'static str) {
    // Full devices leave journal and relay ownership with the window.
    if role == DeviceRole::Full {
        return (
            "disabled",
            if cfg!(target_os = "linux") {
                "unavailable"
            } else {
                "disabled"
            },
        );
    }
    let sync_ok = reconcile();
    let relay = match crate::core::relay_preview::get(root) {
        #[cfg(target_os = "linux")]
        _ => "unavailable",
        #[cfg(not(target_os = "linux"))]
        Ok(Some(_)) => "running",
        #[cfg(not(target_os = "linux"))]
        Ok(None) => "disabled",
        #[cfg(not(target_os = "linux"))]
        Err(_) if cfg!(windows) => "retry",
        #[cfg(not(target_os = "linux"))]
        Err(_) => "UNAVAILABLE_ON_PLATFORM",
    };
    (if sync_ok { "ready" } else { "retry" }, relay)
}

fn supervise() {
    let root = crate::core::state_dir();
    #[cfg(target_os = "linux")]
    let mut listener = None;
    #[cfg(not(target_os = "linux"))]
    let mut listener = (role() == DeviceRole::Server).then(crate::listener::service_background);
    while !stopping() && !crate::updates::listener_stopping() {
        reap_children(false);
        let (sync, relay) = sync_status(role(), &root, || {
            #[cfg(target_os = "linux")]
            if role() != DeviceRole::Full {
                return crate::sync_owner::snapshot().errors == 0;
            }
            crate::core::reconcile_service()
        });
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
        #[cfg(target_os = "linux")]
        if schedule::tick().is_err() {
            eprintln!("SERVICE_SCHEDULE_UNAVAILABLE");
        }
        eprintln!(
            "service status=running role={} sync={} relay={} processing={} hex={} mcp=stdio ui={}",
            role().name(),
            sync,
            relay,
            processing,
            hex,
            ui_allowed()
        );
        #[cfg(target_os = "linux")]
        if crate::operations::record(&root, crate::operations::EventKind::ServiceTick).is_err() {
            eprintln!("OPS_LOG_UNAVAILABLE");
        }
        pause(Duration::from_secs(30));
    }
    STOP.store(true, Ordering::Release);
    #[cfg(target_os = "linux")]
    workers::stop();
    #[cfg(not(target_os = "linux"))]
    if role() != DeviceRole::Full {
        let _ = crate::core::relay_preview::disconnect(&root);
    }
    reap_children(true);
    drop(listener);
}

pub fn run() -> i32 {
    #[cfg(target_os = "linux")]
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("SERVICE_ROOT_REFUSED");
        return 2;
    }
    crate::listener::log(&format!(
        "start pid={} role={}",
        std::process::id(),
        role().name()
    ));
    #[cfg(windows)]
    if role() == DeviceRole::Full {
        crate::listener::run();
    }
    HEADLESS.store(true, Ordering::Release);
    #[cfg(target_os = "linux")]
    unsafe {
        libc::umask(0o077);
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0 {
            eprintln!("SERVICE_SUBREAPER_UNAVAILABLE");
            return 2;
        }
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("SERVICE_RUNTIME_FAILED");
            crate::listener::log("exit reason=service-runtime-error");
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
                    crate::listener::log("exit reason=service-signal-error");
                    return 2;
                }
            };
        let _instance =
            match crate::core::store_io::lock(&crate::core::state_dir().join("local/service.lock"))
            {
                Ok(instance) => instance,
                Err(error) => {
                    eprintln!("SERVICE_LOCK_FAILED");
                    crate::listener::log(&format!(
                        "exit reason=service-lock-error kind={:?}", error.kind()
                    ));
                    return 2;
                }
            };
        #[cfg(target_os = "linux")]
        {
            tokio::task::spawn_blocking(|| eprintln!("service linger={}", linger_status()));
            if role() != DeviceRole::Full {
                let root = crate::core::state_dir();
                let provision = tokio::task::spawn_blocking(move || {
                    let _lock = crate::core::store_io::lock(&root.join("local/relay/identity.lock"))
                        .map_err(|_| "STATE_STORAGE_UNAVAILABLE")?;
                    sidenote_sync::client::Device::open(&root.join("local/relay/replica.sqlite"))
                        .map_err(|error| {
                            if error.to_string().starts_with("UNSAFE_STATE_ANCESTOR") {
                                "UNSAFE_STATE_ANCESTOR: use private root/user-owned state ancestors"
                            } else { sidenote_sync::identity::SECRET_INIT_ERROR }
                        })?;
                    Ok::<_, &'static str>(())
                });
                let result = tokio::select! {
                    result = provision => result.unwrap_or(Err(sidenote_sync::identity::SECRET_INIT_ERROR)),
                    _ = terminate.recv() => {
                        sidenote_sync::identity::cancel_pending_initialization();
                        Err("SECRET_INITIALIZATION_CANCELLED")
                    },
                    _ = tokio::signal::ctrl_c() => {
                        sidenote_sync::identity::cancel_pending_initialization();
                        Err("SECRET_INITIALIZATION_CANCELLED")
                    }
                };
                if let Err(error) = result { eprintln!("{error}"); return 2; }
            }
        }
        #[cfg(target_os = "linux")]
        let sync_owner = if role() == DeviceRole::Full { None } else {
            match crate::sync_owner::Owner::start(&crate::core::state_dir(), _instance) {
                Ok(owner) => Some(owner),
                Err(_) => { eprintln!("SYNC_OWNER_UNAVAILABLE: check private state and control socket"); return 2; }
            }
        };
        #[cfg(windows)]
        let _update_instance = match crate::listener::acquire_lock() {
            Ok(instance) => instance,
            Err(error) => {
                eprintln!("SERVICE_LOCK_FAILED");
                crate::listener::log_lock_exit(&error);
                return 2;
            }
        };
        if let Some(source) = crate::updates::listener_stop_source() {
            crate::listener::log(&format!("exit reason=stop source=service-startup/{source}"));
            return 0;
        }
        #[cfg(windows)]
        crate::listener::watch_update_stop();
        let worker = match std::thread::Builder::new()
            .name("service".into())
            .spawn(supervise)
        {
            Ok(worker) => worker,
            Err(_) => {
                eprintln!("SERVICE_START_FAILED");
                crate::listener::log("exit reason=service-worker-spawn-error");
                return 2;
            }
        };
        let lifetime = async {
            while !stopping() && !worker.is_finished() {
                #[cfg(target_os = "linux")]
                if sync_owner.as_ref().is_some_and(|owner| !owner.healthy()) { break; }
                #[cfg(target_os = "linux")]
                reap_orphans(false);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        #[cfg(unix)]
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {}, _ = lifetime => {} }
        #[cfg(not(unix))]
        tokio::select! {
            _ = async {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::future::pending::<()>().await;
                }
            } => {},
            _ = lifetime => {}
        }
        STOP.store(true, Ordering::Release);
        #[cfg(target_os = "linux")]
        let owner_failed = sync_owner.as_ref().is_some_and(|owner| !owner.healthy());
        crate::core::ask::cancel_all(Duration::from_secs(3));
        let result = if worker.join().is_ok() { 0 } else { 2 };
        #[cfg(target_os = "linux")]
        drop(sync_owner);
        #[cfg(target_os = "linux")]
        let result = if owner_failed { eprintln!("SYNC_OWNER_STOPPED"); 2 } else { result };
        #[cfg(target_os = "linux")]
        reap_orphans(true);
        crate::listener::log(if result == 0 {
            "exit reason=service-stop source=supervisor"
        } else {
            "exit reason=service-worker-panic source=supervisor"
        });
        eprintln!("service status=stopped role={}", role().name());
        result
    })
}

#[cfg(target_os = "linux")]
fn linger_status() -> &'static str {
    static VALUE: OnceLock<&'static str> = OnceLock::new();
    VALUE.get_or_init(|| {
        let output = sidenote_sync::tree_worker::bounded_output(
            std::process::Command::new("/usr/bin/loginctl").args([
                "show-user",
                &unsafe { libc::geteuid() }.to_string(),
                "-p",
                "Linger",
                "--value",
            ]),
            Duration::from_secs(2),
        );
        match output.ok().filter(|o| o.status.success()).map(|o| o.stdout) {
            Some(bytes) if bytes == b"yes\n" => "enabled",
            Some(bytes) if bytes == b"no\n" => "disabled",
            _ => "unknown",
        }
    })
}

#[cfg(target_os = "linux")]
fn reap_orphans(stopping: bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let children =
            std::fs::read_to_string(format!("/proc/self/task/{}/children", std::process::id()))
                .unwrap_or_default();
        let mut found = false;
        for pid in children
            .split_whitespace()
            .filter_map(|p| p.parse::<i32>().ok())
        {
            // systemd-run is only launched by the separate worker frontend, never directly
            // by this service. These adopted proxies have no live Child handle here.
            let proxy = std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .is_ok_and(|n| n.trim() == "systemd-run");
            if stopping || proxy {
                found = true;
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG);
                }
            }
        }
        if !stopping || !found || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_read_failures_fall_back_once_without_writing_device_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("device.json");
        assert_eq!(
            role_or_full(load_role(root.path()), || panic!("missing file is normal")),
            DeviceRole::Full
        );
        assert!(!path.exists());
        let fallback = || {
            let mut reports = 0;
            let role = role_or_full(load_role(root.path()), || reports += 1);
            assert_eq!(role, DeviceRole::Full);
            assert_eq!(reports, 1);
        };
        for bytes in [b"{".as_slice(), br#"{"role":"unknown"}"#] {
            std::fs::write(&path, bytes).unwrap();
            fallback();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap(); // Unreadable as a regular file.
        fallback();
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::WouldBlock,
        ] {
            let mut reports = 0;
            assert_eq!(
                role_or_full(Err(std::io::Error::from(kind)), || reports += 1),
                DeviceRole::Full
            );
            assert_eq!(reports, 1);
        }
    }

    #[test]
    fn known_roles_and_old_settings_need_no_fallback_or_device_lock() {
        let root = tempfile::tempdir().unwrap();
        let _window = crate::core::store_io::lock(&root.path().join("device.lock")).unwrap();
        for (json, role) in [
            ("{}", DeviceRole::Full),
            (r#"{"role":"full"}"#, DeviceRole::Full),
            (r#"{"role":"server"}"#, DeviceRole::Server),
            (r#"{"role":"hub"}"#, DeviceRole::Hub),
        ] {
            std::fs::write(root.path().join("device.json"), json).unwrap();
            assert_eq!(
                role_or_full(load_role(root.path()), || panic!("unexpected fallback")),
                role
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn exclusively_locked_device_file_falls_back_once() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("device.json");
        std::fs::write(&path, br#"{"role":"server"}"#).unwrap();
        let _locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        let mut reports = 0;
        assert_eq!(
            role_or_full(load_role(root.path()), || reports += 1),
            DeviceRole::Full
        );
        assert_eq!(reports, 1);
    }

    #[test]
    fn full_service_leaves_window_store_and_relay_locks_alone() {
        let root = tempfile::tempdir().unwrap();
        let relay = root.path().join("local/relay");
        std::fs::create_dir_all(&relay).unwrap();
        std::fs::write(relay.join("settings.json"), br#"{"enabled":true}"#).unwrap();
        let _window =
            sidenote_sync::store::Store::open(&relay.join("replica.sqlite"), &[1; 16]).unwrap();
        let _config = crate::core::store_io::lock(&root.path().join("config.lock")).unwrap();
        assert_eq!(
            sync_status(DeviceRole::Full, root.path(), || panic!(
                "Full must not reconcile journals"
            )),
            (
                "disabled",
                if cfg!(target_os = "linux") {
                    "unavailable"
                } else {
                    "disabled"
                }
            )
        );
        assert!(!root.path().join(".stignore").exists());
        assert!(!root.path().join("device.json").exists());
    }

    #[test]
    fn server_and_hub_keep_journal_and_relay_duties() {
        let root = tempfile::tempdir().unwrap();
        for role in [DeviceRole::Server, DeviceRole::Hub] {
            let mut reconciled = false;
            assert_eq!(
                sync_status(role, root.path(), || {
                    reconciled = true;
                    true
                }),
                (
                    "ready",
                    if cfg!(target_os = "linux") {
                        "unavailable"
                    } else {
                        "disabled"
                    }
                )
            );
            assert!(reconciled);
        }
    }

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
