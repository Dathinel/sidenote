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
    sources::{FileSource, UpdateSource},
};

// Velopack's HttpSource constructs an unbounded default agent internally.
#[derive(Clone)]
struct HttpSource {
    base: String,
    agent: ureq::Agent,
    feed_deadline: Duration,
    download_deadline: Duration,
}
impl HttpSource {
    fn new(base: &str) -> Self {
        Self {
            base: format!("{}/", base.trim_end_matches('/')),
            agent: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_recv_response(Some(Duration::from_secs(20)))
                .timeout_recv_body(Some(Duration::from_secs(300)))
                .build()
                .into(),
            feed_deadline: Duration::from_secs(30),
            download_deadline: Duration::from_secs(300),
        }
    }
    fn url(&self, name: &str) -> Result<url::Url, velopack::Error> {
        Ok(url::Url::parse(&self.base)?.join(name)?)
    }
    fn get(
        &self,
        url: &url::Url,
        deadline: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, velopack::Error> {
        self.agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(deadline))
            .build()
            .call()
            .map_err(|error| {
                velopack::Error::Generic(format!("Update request failed: {error}. Try again."))
            })
    }
}
impl UpdateSource for HttpSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &velopack::bundle::Manifest,
        id: &str,
    ) -> Result<velopack::VelopackAssetFeed, velopack::Error> {
        let mut url = self.url(&format!("releases.{channel}.json"))?;
        url.query_pairs_mut()
            .append_pair("localVersion", &app.version.to_string())
            .append_pair("id", &app.id)
            .append_pair("stagingId", id);
        let json = self
            .get(&url, self.feed_deadline)?
            .body_mut()
            .read_to_string()
            .map_err(|error| {
                velopack::Error::Generic(format!("Update feed failed: {error}. Try again."))
            })?;
        let feed: velopack::VelopackAssetFeed = serde_json::from_str(&json)?;
        if !feed.Assets.iter().all(valid_asset) {
            return Err(velopack::Error::Generic("Invalid public feed asset".into()));
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
            return Err(velopack::Error::Generic("Invalid public feed asset".into()));
        }
        let mut response = self.get(&self.url(&asset.FileName)?, self.download_deadline)?;
        let mut reader = response.body_mut().as_reader();
        let mut output = File::create(destination)?;
        let mut bytes = [0u8; 128 * 1024];
        let mut copied = 0u64;
        loop {
            let count = reader.read(&mut bytes)?;
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
    publish(status.clone());
    save(&status).map_err(|_| "Could not save update status.")?;
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
    listener_task(run, false)
}
fn listener_task(run: bool, register: bool) -> Result<(), String> {
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
    let script = r#"
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$sid = $identity.User.Value
$t = Get-ScheduledTask -TaskPath '\' | Where-Object TaskName -eq $env:SIDENOTE_TASK_NAME
if (!$t -and $env:SIDENOTE_TASK_REGISTER -eq 'yes') {
    $action = New-ScheduledTaskAction -Execute $env:SIDENOTE_TASK_EXE -Argument $env:SIDENOTE_TASK_ARGS
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $sid
    $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskPath '\' -TaskName $env:SIDENOTE_TASK_NAME -Action $action -Trigger $trigger -Principal $principal -Settings $settings | Out-Null
}
$t = Get-ScheduledTask -TaskPath '\' -TaskName $env:SIDENOTE_TASK_NAME
$a = @($t.Actions)
$owner = $t.Principal.UserId
if ($owner -ne $sid) { $owner = ([Security.Principal.NTAccount]$owner).Translate([Security.Principal.SecurityIdentifier]).Value }
if ($a.Count -ne 1 -or $a[0].Execute -ne $env:SIDENOTE_TASK_EXE -or $a[0].Arguments -ne $env:SIDENOTE_TASK_ARGS -or $owner -ne $sid -or $t.Principal.LogonType -ne 'Interactive' -or $t.Principal.RunLevel -ne 'Limited' -or !$t.Settings.Enabled) { throw 'Listener task does not target this user and install' }
if ($env:SIDENOTE_TASK_RUN -eq 'yes') { Start-ScheduledTask -InputObject $t }
"#;
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("SIDENOTE_TASK_NAME", name)
        .env("SIDENOTE_TASK_EXE", exe)
        .env("SIDENOTE_TASK_ARGS", expected_args)
        .env("SIDENOTE_TASK_RUN", if run { "yes" } else { "no" })
        .env(
            "SIDENOTE_TASK_REGISTER",
            if register { "yes" } else { "no" },
        )
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "Could not check the listener task.")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "The meeting listener setup did not finish. Reopen Sidenote to retry.".into(),
                );
            }
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err("Sidenote could not prepare its meeting listener. Reopen Sidenote to retry. If this continues, reinstall Sidenote for your Windows account.".into())
    }
}
static LISTENER_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn listener_ready() -> bool {
    !installed() || LISTENER_READY.load(std::sync::atomic::Ordering::Acquire)
}
pub fn listener_installed() {
    if installed()
        && let Err(error) = listener_task(false, true)
    {
        let _ = save(&Status {
            phase: Phase::Error,
            error: Some(error),
            ..load()
        });
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
#[derive(Debug)]
pub enum WindowLockError {
    AlreadyOpen,
    Io(io::Error),
}
impl From<io::Error> for WindowLockError {
    fn from(error: io::Error) -> Self {
        if error.raw_os_error() == Some(32) {
            Self::AlreadyOpen
        } else {
            Self::Io(error)
        }
    }
}
pub fn window_lock() -> Result<Option<File>, WindowLockError> {
    if !installed() {
        return Ok(None);
    }
    exclusive("update-window.lock")
        .map(Some)
        .map_err(Into::into)
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
        return Err("Sidenote is updating its meeting listener. Wait for the update to finish, then reopen Sidenote.".into());
    }
    LISTENER_READY.store(false, std::sync::atomic::Ordering::Release);
    listener_task(false, true)?;
    match listener_lock() {
        Ok(lock) => {
            drop(lock);
            task_command(true)?;
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                match listener_lock() {
                    Err(error) if error.raw_os_error() == Some(32) => break,
                    Err(_) => return Err("Could not check whether the listener is running.".into()),
                    Ok(_) if Instant::now() >= deadline => return Err("The meeting listener did not start. Reopen Sidenote to retry, or reinstall it if this continues.".into()),
                    Ok(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        Err(error) if error.raw_os_error() == Some(32) => {}
        Err(_) => return Err("Could not check whether the listener is running.".into()),
    }
    LISTENER_READY.store(true, std::sync::atomic::Ordering::Release);
    Ok(())
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
mod window_lock_tests {
    use super::*;

    #[test]
    fn only_a_sharing_violation_means_an_existing_window() {
        assert!(matches!(
            WindowLockError::from(io::Error::from_raw_os_error(32)),
            WindowLockError::AlreadyOpen
        ));
        for code in [2, 3, 5, 19, 112] {
            assert!(
                matches!(WindowLockError::from(io::Error::from_raw_os_error(code)), WindowLockError::Io(error) if error.raw_os_error() == Some(code))
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The stalled peer exits when the operation returns; no detached request or worker remains.
    fn with_stalled_server(headers: bool, test: impl FnOnce(HttpSource)) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut source = HttpSource::new(&format!("http://{}", listener.local_addr().unwrap()));
        source.agent = ureq::Agent::config_builder().proxy(None).build().into();
        source.feed_deadline = Duration::from_secs(1);
        source.download_deadline = Duration::from_secs(1);
        let (tx, rx) = mpsc::channel();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(8);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if Instant::now() >= until
                            || rx.try_recv() != Err(mpsc::TryRecvError::Empty)
                        {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Local accept failed: {error}"),
                }
            };
            if headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\n{")
                    .unwrap();
            }
            let _ = rx.recv_timeout(Duration::from_secs(5));
        });
        let started = Instant::now();
        test(source);
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = tx.send(());
        server.join().unwrap();
    }

    #[test]
    fn public_feed_expires_while_waiting_for_headers_or_body() {
        for headers in [false, true] {
            with_stalled_server(headers, |source| {
                let error = source
                    .get_release_feed("win", &velopack::bundle::Manifest::default(), "test")
                    .unwrap_err();
                assert!(error.to_string().contains("Try again"), "{error}");
            });
        }
    }

    #[test]
    fn public_download_expires_and_releases_the_destination() {
        with_stalled_server(true, |source| {
            let root = std::env::temp_dir().join(format!("update-timeout-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let destination = root.join("sidenote.nupkg");
            let asset = VelopackAsset {
                PackageId: "sidenote".into(),
                FileName: "sidenote.nupkg".into(),
                SHA256: "a".repeat(64),
                Size: 10000,
                ..Default::default()
            };
            let error = source
                .download_release_entry(&asset, destination.to_str().unwrap(), None)
                .unwrap_err();
            assert!(
                destination.exists(),
                "Request failed before download started: {error}"
            );
            assert!(
                error.to_string().to_lowercase().contains("timeout"),
                "{error}"
            );
            // Windows refuses this rename if a timed-out writer still owns the file.
            fs::rename(&destination, root.join("retry.nupkg")).unwrap();
            fs::remove_dir_all(root).unwrap();
        });
    }

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
