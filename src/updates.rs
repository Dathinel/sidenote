//! Public and local Velopack feeds. Checks and downloads never apply an update.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{fs::OpenOptionsExt, process::CommandExt},
    path::PathBuf,
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
use velopack::{
    UpdateCheck, UpdateManager, VelopackAsset,
    sources::{FileSource, HttpSource, UpdateSource},
};

#[derive(Clone)]
struct LocalSource(PathBuf);
fn valid_asset(asset: &VelopackAsset) -> bool {
    asset.PackageId == "sidenote"
        && !asset.FileName.is_empty()
        && asset
            .FileName
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && asset.FileName.ends_with(".nupkg")
        && asset.SHA256.len() == 64
        && asset.SHA256.bytes().all(|c| c.is_ascii_hexdigit())
}
impl UpdateSource for LocalSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &velopack::bundle::Manifest,
        id: &str,
    ) -> Result<velopack::VelopackAssetFeed, velopack::Error> {
        let feed = FileSource::new(&self.0).get_release_feed(channel, app, id)?;
        if !feed.Assets.iter().all(valid_asset) {
            return Err(velopack::Error::Generic("Invalid local feed asset".into()));
        }
        Ok(feed)
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        destination: &str,
        progress: Option<mpsc::Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        if !valid_asset(asset) {
            return Err(velopack::Error::Generic("Invalid local feed asset".into()));
        }
        let mut source = File::open(self.0.join(&asset.FileName))?;
        let mut output = File::create(destination)?;
        let mut bytes = [0u8; 128 * 1024];
        let mut copied = 0u64;
        loop {
            let count = source.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            output.write_all(&bytes[..count])?;
            copied += count as u64;
            if let Some(tx) = &progress {
                let _ = tx.send((copied.saturating_mul(100) / asset.Size.max(1)).min(100) as i16);
            }
        }
        output.sync_all()?;
        Ok(())
    }
    fn clone_boxed(&self) -> Box<dyn UpdateSource> {
        Box::new(self.clone())
    }
}
fn cache_path(asset: &VelopackAsset) -> Result<PathBuf, String> {
    if !valid_asset(asset) {
        return Err("The update package is invalid.".into());
    }
    let locator = velopack::locator::auto_locate_app_manifest(
        velopack::locator::LocationContext::FromCurrentExe,
    )
    .map_err(|_| "Could not locate installed update packages.")?;
    Ok(locator.get_packages_dir().join(&asset.FileName))
}
fn verify(asset: &VelopackAsset) -> Result<(), String> {
    if !valid_asset(asset) {
        return Err("The update package is invalid.".into());
    }
    let mut file = File::open(cache_path(asset)?).map_err(|_| "Download the update again.")?;
    if file
        .metadata()
        .map_err(|_| "Could not read the update.")?
        .len()
        != asset.Size
    {
        return Err("The update size does not match. Try again.".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Could not read the update.")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if !hash
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .eq_ignore_ascii_case(&asset.SHA256)
    {
        return Err("The update checksum does not match. Try again.".into());
    }
    Ok(())
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const PUBLIC_FEED: &str =
    "https://github.com/harshanandak/hex-meetings/releases/latest/download/";

pub const FOCUS_CHECK_AFTER: Duration = Duration::from_secs(30 * 60);
pub enum CheckRequest {
    Manual,
    Focus,
}
fn focus_check_due(last_attempt: Instant, now: Instant) -> bool {
    now.duration_since(last_attempt) > FOCUS_CHECK_AFTER
}
/// Dropping the sender stops the six-hour worker after its current check/download.
pub fn start(publish: impl Fn(Status) + Send + 'static) -> Option<mpsc::Sender<CheckRequest>> {
    if !installed() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = write_report(&load());
        let mut last_attempt = Instant::now();
        let _ = check(&publish);
        loop {
            match rx.recv_timeout(CHECK_EVERY) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(CheckRequest::Focus) if !focus_check_due(last_attempt, Instant::now()) => {
                    continue;
                }
                _ => {}
            }
            last_attempt = Instant::now();
            let _ = check(&publish);
        }
    });
    Some(tx)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Manual,
    Preparing,
    StoppingListener,
    Checking,
    Available,
    Downloading,
    Ready,
    Error,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub phase: Phase,
    pub version: String,
    pub progress: u8,
    pub notes: String,
    pub checked: Option<String>,
    pub error: Option<String>,
}
pub fn installed() -> bool {
    static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *INSTALLED.get_or_init(|| {
        std::env::var_os("LOCALAPPDATA").is_some()
            && velopack::locator::auto_locate_app_manifest(
                velopack::locator::LocationContext::FromCurrentExe,
            )
            .is_ok()
    })
}
pub fn directory() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Sidenote/updates")
}
fn local(name: &str) -> PathBuf {
    directory().join(name)
}
pub fn load() -> Status {
    if !installed() {
        return Status {
            phase: Phase::Manual,
            ..Default::default()
        };
    }
    fs::read(local("updates.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save(status: &Status) -> io::Result<()> {
    if !installed() {
        return Ok(());
    }
    let path = local("updates.json");
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(status)?)?;
    crate::platform::replace_file(&temp, &path)?;
    write_report(status).map(|_| ())
}
pub fn feed() -> Option<PathBuf> {
    crate::env_var("SIDENOTE_UPDATE_FEED").map(PathBuf::from)
}
pub fn source_label() -> String {
    feed().map_or_else(|| PUBLIC_FEED.into(), |p| p.display().to_string())
}
fn manager() -> Result<UpdateManager, String> {
    let source: Box<dyn UpdateSource> = if let Some(feed) = feed() {
        if !feed.is_absolute() || feed.to_string_lossy().contains("://") {
            return Err("The update feed must be an absolute folder or file-share path.".into());
        }
        Box::new(LocalSource(feed))
    } else {
        Box::new(HttpSource::new(PUBLIC_FEED))
    };
    UpdateManager::new_boxed(source, None, None)
        .map_err(|_| "Updates need a Sidenote Setup install. Manual builds cannot update.".into())
}
pub fn check(mut publish: impl FnMut(Status)) -> Result<Status, String> {
    if !installed() {
        return Ok(load());
    }
    let _check = exclusive("update-check.lock").map_err(|_| "Another update check is running.")?;
    let mut status = Status {
        phase: Phase::Checking,
        checked: Some(chrono::Local::now().to_rfc3339()),
        ..Default::default()
    };
    publish(status.clone());
    let result = (|| {
        let mgr = manager()?;
        let update = mgr
            .check_for_updates()
            .map_err(|_| "Could not read the update feed. Check your connection and try again.")?;
        match update {
            UpdateCheck::UpdateAvailable(mut update) => {
                // Reconstructed delta ZIPs are not byte-identical to the feed's full SHA256.
                update.BaseRelease = None;
                update.DeltasToTarget.clear();
                let cached = cache_path(&update.TargetFullRelease)?;
                if cached.exists() && verify(&update.TargetFullRelease).is_err() {
                    fs::rename(&cached, cached.with_extension("rejected"))
                        .map_err(|_| "Could not set aside an invalid cached update.")?;
                }
                status.version = update.TargetFullRelease.Version.clone();
                status.notes = update
                    .TargetFullRelease
                    .NotesMarkdown
                    .chars()
                    .take(16000)
                    .collect();
                status.phase = Phase::Available;
                publish(status.clone());
                status.phase = Phase::Downloading;
                publish(status.clone());
                let (tx, rx) = mpsc::channel();
                std::thread::scope(|scope| {
                    let worker = scope.spawn(|| mgr.download_updates(&update, Some(tx)));
                    for progress in rx {
                        status.progress = progress.clamp(0, 100) as u8;
                        publish(status.clone());
                    }
                    worker
                        .join()
                        .map_err(|_| "The update download stopped.")?
                        .map_err(|_| "Could not download or verify the update. Try again.")
                })?;
                verify(&update.TargetFullRelease)?;
                fs::write(
                    local("update-ready.json"),
                    serde_json::to_vec(&update.TargetFullRelease)
                        .map_err(|_| "Could not save the update.")?,
                )
                .map_err(|_| "Could not save the update.")?;
                status.phase = Phase::Ready;
                status.progress = 100;
            }
            UpdateCheck::NoUpdateAvailable => status.phase = Phase::Idle,
            _ => return Err("This update feed does not match the installed app.".into()),
        }
        Ok(())
    })();
    if let Err(error) = result {
        status.phase = Phase::Error;
        status.error = Some(error);
    }
    save(&status).map_err(|_| "Could not save update status.")?;
    publish(status.clone());
    Ok(status)
}
pub fn exclusive(name: &str) -> io::Result<File> {
    fs::create_dir_all(local(""))?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(local(name))
}
/// Shared readers represent work. Apply needs an exclusive writer of the same file.
pub fn activity() -> io::Result<Option<File>> {
    if !installed() {
        return Ok(None);
    }
    if listener_stopping() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "An update is being installed",
        ));
    }
    fs::create_dir_all(local(""))?;
    let path = local("update-activity.lock");
    if !path.exists() {
        let _ = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path);
    }
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map(Some)
}
fn recording_idle() -> Result<(), String> {
    match crate::hex_engine::HexEngine::new().status() {
        Ok(s) if s.recording.is_none() && s.dictating == Some(false) && s.meeting_state == "idle" => Ok(()),
        Ok(_) => Err("Finish recording before installing the update.".into()),
        Err(_) => match crate::platform::hex_running() {
            Ok(false) => Ok(()),
            _ => Err("Hex is running but its recording status cannot be read. Treat it as recording and try again later.".into()),
        },
    }
}
fn task() -> Result<String, String> {
    let name = crate::env_var("SIDENOTE_LISTENER_TASK")
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Sidenote meeting listener".into());
    if !name.starts_with("Sidenote ") || name.contains(['\'', '"', '\r', '\n', '\\', '/']) {
        return Err("The listener task must use a Sidenote task name.".into());
    }
    Ok(name)
}
fn task_command(run: bool) -> Result<(), String> {
    let name = task()?;
    let exe = std::env::current_exe().map_err(|_| "Could not locate Sidenote.")?;
    let expected_args = if name.starts_with("Sidenote updates proof ") {
        let data = crate::env_var("SIDENOTE_DATA_DIR")
            .ok_or("Proof task needs an isolated data folder.")?;
        format!(
            "--listen --listener-isolated --data-dir \"{}\" --listener-localappdata \"{}\"",
            data.to_string_lossy(),
            std::env::var_os("LOCALAPPDATA")
                .ok_or("Proof task needs isolated LocalAppData.")?
                .to_string_lossy()
        )
    } else {
        "--listen".into()
    };
    let script = " $t=Get-ScheduledTask -TaskName $env:SIDENOTE_TASK_NAME -ErrorAction Stop; $a=@($t.Actions); if($a.Count -ne 1 -or $a[0].Execute -ne $env:SIDENOTE_TASK_EXE -or $a[0].Arguments -ne $env:SIDENOTE_TASK_ARGS){throw 'Listener task does not target this install'}; if($env:SIDENOTE_TASK_RUN -eq 'yes'){Start-ScheduledTask -InputObject $t}";
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("SIDENOTE_TASK_NAME", name)
        .env("SIDENOTE_TASK_EXE", exe)
        .env("SIDENOTE_TASK_ARGS", expected_args)
        .env("SIDENOTE_TASK_RUN", if run { "yes" } else { "no" })
        .creation_flags(0x08000000)
        .output()
        .map_err(|_| "Could not check the listener task.")?;
    if out.status.success() {
        Ok(())
    } else {
        Err("The listener task is missing or does not target this install. Configure a Sidenote task first.".into())
    }
}
#[derive(Serialize, Deserialize)]
struct StopMarker {
    pid: u32,
    created: i64,
}
fn write_stop(pid: u32) -> io::Result<()> {
    let temp = local("update-listener.stop.tmp");
    fs::write(
        &temp,
        serde_json::to_vec(&StopMarker {
            pid,
            created: chrono::Utc::now().timestamp(),
        })?,
    )?;
    crate::platform::replace_file(&temp, &local("update-listener.stop"))
}
pub fn listener_stopping() -> bool {
    if !installed() {
        return false;
    }
    let path = local("update-listener.stop");
    let Ok(bytes) = fs::read(&path) else {
        return false;
    };
    let current = serde_json::from_slice::<StopMarker>(&bytes)
        .ok()
        .is_some_and(|marker| {
            let age = chrono::Utc::now().timestamp() - marker.created;
            age < 600 && crate::platform::process_running(marker.pid)
        });
    if !current {
        // Do not remove a fresh request that replaced the stale marker while it was read.
        if fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
            let _ = fs::remove_file(path);
        }
    }
    current
}
pub fn window_lock() -> io::Result<Option<File>> {
    if !installed() {
        return Ok(None);
    }
    exclusive("update-window.lock").map(Some)
}
pub fn listener_lock() -> io::Result<Option<File>> {
    if !installed() {
        return Ok(None);
    }
    exclusive("update-listener.lock").map(Some)
}
pub fn ensure_listener() -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    if listener_stopping() {
        return Ok(());
    }
    match listener_lock() {
        Ok(lock) => {
            drop(lock);
            task_command(true)
        }
        Err(error) if error.raw_os_error() == Some(32) => Ok(()),
        Err(_) => Err("Could not check whether the listener is running.".into()),
    }
}
/// Runs on the window's worker, including recovery after a failed asynchronous apply.
pub fn window_start(publish: impl Fn(Phase)) -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    let pending = manager()
        .ok()
        .and_then(|mgr| mgr.get_update_pending_restart());
    ensure_listener()?;
    if listener_stopping() {
        return Ok(());
    }
    if pending.is_some() {
        apply(true, Vec::new(), publish)?;
    }
    Ok(())
}
pub fn after_update() {
    if !installed() {
        return;
    }
    let _ = write_report(&load());
    if load().version == VERSION {
        let _ = save(&Status {
            checked: load().checked,
            ..Default::default()
        });
    }
}
pub fn restarted() {
    after_update();
    restart_listener();
}
fn restart_listener() {
    if local("update-listener.resume").exists() {
        let _ = fs::remove_file(local("update-listener.stop"));
        if task_command(true).is_ok() {
            let _ = fs::remove_file(local("update-listener.resume"));
        } else {
            let _ = save(&Status { phase: Phase::Error, error: Some("The update finished, but the listener could not restart. Check its Sidenote task.".into()), ..load() });
        }
    }
}
pub fn apply(
    from_window: bool,
    mut restart_args: Vec<String>,
    mut publish: impl FnMut(Phase),
) -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    if listener_stopping() {
        return Err("An update is already being installed.".into());
    }
    publish(Phase::Preparing);
    let args: Vec<_> = std::env::args().skip(1).collect();
    for pair in args.windows(2).filter(|pair| {
        matches!(
            pair[0].as_str(),
            "--data-dir" | "--meetings-dir" | "--peers-dir"
        )
    }) {
        restart_args.extend(pair.iter().cloned());
    }
    let _check =
        exclusive("update-check.lock").map_err(|_| "Wait for the update download to finish.")?;
    let _window = if from_window {
        None
    } else {
        Some(
            exclusive("update-window.lock")
                .map_err(|_| "Close the Sidenote window before using the update command.")?,
        )
    };
    let _gate = exclusive("update-activity.lock")
        .map_err(|_| "Wait for Ask, routing and notes jobs to finish before installing.")?;
    let mgr = manager()?;
    let asset = mgr
        .get_update_pending_restart()
        .ok_or("Download an update first.")?;
    let verified: VelopackAsset = fs::read(local("update-ready.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Check for updates to verify the downloaded package first.")?;
    if verified.Version != asset.Version || verified.FileName != asset.FileName {
        return Err("Check for updates again before installing.".into());
    }
    verify(&verified)?;
    recording_idle()?;
    task_command(false)?;
    fs::write(local("update-listener.resume"), b"resume")
        .map_err(|_| "Could not save listener restart state.")?;
    publish(Phase::StoppingListener);
    write_stop(std::process::id()).map_err(|_| "Could not stop the listener.")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let stopped = loop {
        if let Ok(lock) = listener_lock() {
            break Ok(lock);
        }
        if Instant::now() >= deadline {
            break Err("The listener is still busy. Try again later.");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let result = stopped.and_then(|_listener| {
        recording_idle().map_err(|_| "Recording started. Try again when it is finished.")?;
        mgr.wait_exit_then_apply_updates(&asset, true, true, restart_args)
            .map_err(|_| "Could not start the update. Try again.")?;
        // Update.exe now waits for this process to exit, and nothing here can cancel it.
        // Never roll back past this point: a restarted listener could be mid-job when the
        // updater replaces files. If the handoff below fails, the stop marker keeps this
        // process's PID, which is dead after exit, so the next start restarts the listener.
        if let Some(pid) = crate::platform::updater_child().ok().flatten() {
            let _ = write_stop(pid);
        }
        Ok(())
    });
    if let Err(error) = result {
        restart_listener();
        return Err(error.into());
    }
    std::process::exit(0)
}
fn report(status: &Status) -> serde_json::Value {
    serde_json::json!({"current_version":VERSION,"reported_at":chrono::Utc::now().to_rfc3339(),
        "data_dir":crate::core::state_dir(),"peers_dir":crate::core::peers_dir(),
        "feed":source_label(),"listener_task":task().ok(),"update_dir":directory(),
        "installed":installed(),"status":status})
}
fn write_report(status: &Status) -> io::Result<serde_json::Value> {
    let report = report(status);
    if installed() {
        let path = local("update-report.json");
        fs::create_dir_all(directory())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, serde_json::to_vec_pretty(&report)?)?;
        crate::platform::replace_file(&temp, &path)?;
    }
    Ok(report)
}
pub fn cli(args: &[std::ffi::OsString]) -> i32 {
    let result = if args.iter().any(|a| a == "--update-window-start") {
        window_start(|_| {}).map(|_| load())
    } else if args.iter().any(|a| a == "--update-probe") {
        recording_idle().map(|_| load())
    } else if args.iter().any(|a| a == "--update-apply") {
        apply(false, vec!["--update-status".into()], |phase| {
            let _ = save(&Status { phase, ..load() });
        })
        .map(|_| load())
    } else if args.iter().any(|a| a == "--update-check") {
        check(|_| {})
    } else {
        Ok(load())
    };
    let status = match result {
        Ok(s) => s,
        Err(error) => Status {
            phase: Phase::Error,
            error: Some(error),
            ..load()
        },
    };
    let _ = save(&status);
    let report = write_report(&status).unwrap_or_else(|_| report(&status));
    println!("{report}");
    i32::from(status.phase == Phase::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_checks_wait_more_than_thirty_minutes_since_the_attempt() {
        let attempt = Instant::now();
        assert!(!focus_check_due(attempt, attempt));
        assert!(!focus_check_due(attempt, attempt + FOCUS_CHECK_AFTER));
        assert!(focus_check_due(
            attempt,
            attempt + FOCUS_CHECK_AFTER + Duration::from_nanos(1)
        ));
        // A failed check also becomes the latest attempt.
        let retry = attempt + FOCUS_CHECK_AFTER + Duration::from_secs(1);
        assert!(!focus_check_due(retry, retry + Duration::from_secs(60)));
    }

    #[test]
    fn rejects_untrusted_package_paths_before_cache_lookup() {
        for name in [
            "../sidenote.nupkg",
            "other/sidenote.nupkg",
            "https://example.com/a.nupkg",
        ] {
            let asset = VelopackAsset {
                PackageId: "sidenote".into(),
                FileName: name.into(),
                SHA256: "a".repeat(64),
                ..Default::default()
            };
            assert!(!valid_asset(&asset));
            assert_eq!(
                cache_path(&asset).unwrap_err(),
                "The update package is invalid."
            );
        }
    }
}
