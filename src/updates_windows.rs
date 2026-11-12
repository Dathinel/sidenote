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
    if !verify_sha256(&mut file, &asset.SHA256).map_err(|_| "Could not read the update.")? {
        return Err("The update checksum does not match. Try again.".into());
    }
    Ok(())
}

pub(crate) fn verify_sha256(reader: &mut impl Read, expected: &str) -> io::Result<bool> {
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .eq_ignore_ascii_case(expected))
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
const LISTENER_TASK_SCRIPT: &str = r#"
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

    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            LISTENER_TASK_SCRIPT,
        ])
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
                return Err("The meeting listener setup did not finish. Try again.".into());
            }
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err("Sidenote could not prepare its meeting listener. Try again. If this continues, reinstall Sidenote for your Windows account.".into())
    }
}
#[derive(Clone, Debug, Default)]
pub struct ListenerStatus {
    pub ready: bool,
    pub error: Option<String>,
}
impl ListenerStatus {
    fn complete(&mut self, result: &Result<(), String>) {
        self.ready = result.is_ok();
        self.error = result.as_ref().err().cloned();
    }
}
static LISTENER: std::sync::Mutex<ListenerStatus> = std::sync::Mutex::new(ListenerStatus {
    ready: false,
    error: None,
});
// Preparation, recovery probes and apply must not compete for the listener lock.
static LISTENER_CHECK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn listener_status() -> ListenerStatus {
    if !installed() {
        return ListenerStatus {
            ready: true,
            error: None,
        };
    }
    LISTENER.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn listener_ready() -> bool {
    listener_status().ready
}
pub fn listener_installed() {
    if installed() {
        let result = listener_task(false, true);
        if let Err(error) = result {
            LISTENER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .complete(&Err(error));
        }
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
    listener_stop_source().is_some()
}
pub(crate) fn listener_stop_source() -> Option<&'static str> {
    if crate::service::stopping() {
        return Some("service-stop");
    }
    if !installed() {
        return None;
    }
    let path = local("update-listener.stop");
    let Ok(bytes) = fs::read(&path) else {
        return None;
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
    current.then_some("update-stop-marker")
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
    let _check = LISTENER_CHECK.lock().unwrap_or_else(|e| e.into_inner());
    let result = prepare_listener();
    LISTENER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .complete(&result);
    result
}
/// Called only by the window's background worker; never starts or stops a task.
pub fn probe_listener() -> Option<ListenerStatus> {
    if !installed() {
        return None;
    }
    let _check = LISTENER_CHECK.try_lock().ok()?;
    if listener_stopping() {
        return None;
    }
    // The lock probe is cheap and runs every tick. The task check starts PowerShell, so it runs
    // only when a held lock would clear an earlier error, which keeps task validation failures.
    let recovering = LISTENER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .error
        .is_some();
    let result = match listener_probe(listener_lock) {
        Ok(()) if recovering => listener_task(false, false),
        other => other,
    };
    if listener_stopping() {
        return None;
    }
    let mut status = LISTENER.lock().unwrap_or_else(|e| e.into_inner());
    status.complete(&result);
    Some(status.clone())
}
fn listener_probe(mut probe: impl FnMut() -> io::Result<Option<File>>) -> Result<(), String> {
    match probe() {
        Ok(lock) => {
            drop(lock);
            Err("The meeting listener is not running. Try again.".into())
        }
        Err(error) if error.raw_os_error() == Some(32) => Ok(()),
        Err(_) => Err("Could not check whether the listener is running.".into()),
    }
}
fn wait_for_listener(
    deadline: Instant,
    mut probe: impl FnMut() -> io::Result<Option<File>>,
    mut wait: impl FnMut(Duration),
) -> Result<(), String> {
    loop {
        match probe() {
            Err(error) if error.raw_os_error() == Some(32) => return Ok(()),
            Err(_) => return Err("Could not check whether the listener is running.".into()),
            Ok(lock) => drop(lock),
        }
        if Instant::now() >= deadline {
            return Err(
                "The meeting listener did not start. Try again, or reinstall it if this continues."
                    .into(),
            );
        }
        wait(Duration::from_millis(100));
    }
}
fn prepare_listener() -> Result<(), String> {
    if !installed() {
        return Ok(());
    }
    if listener_stopping() {
        return Err("Sidenote is updating its meeting listener. Wait for the update to finish, then reopen Sidenote.".into());
    }
    LISTENER.lock().unwrap_or_else(|e| e.into_inner()).ready = false;
    listener_task(false, true)?;
    match listener_lock() {
        Ok(lock) => {
            drop(lock);
            task_command(true)?;
            wait_for_listener(
                Instant::now() + Duration::from_secs(8),
                listener_lock,
                std::thread::sleep,
            )?;
        }
        Err(error) if error.raw_os_error() == Some(32) => {}
        Err(_) => return Err("Could not check whether the listener is running.".into()),
    }
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
    complete_pipeline_install()?;
    if listener_stopping() {
        return Ok(());
    }
    if pending.is_some() {
        apply(true, Vec::new(), publish)?;
    }
    Ok(())
}
const PIPELINE_INSTALL_ERROR: &str = "The update could not install the meeting scripts. Check private pipeline settings, close processing jobs and reopen Sidenote to retry; the previous generation and original script backups are retained.";

fn complete_pipeline_install() -> Result<(), String> {
    let marker = local("pipeline-install.pending");
    if !marker.exists() {
        return Ok(());
    }
    if fs::read(&marker).map_err(|_| PIPELINE_INSTALL_ERROR)? != VERSION.as_bytes() {
        return Err(PIPELINE_INSTALL_ERROR.into());
    }
    install_pipeline_scripts()?;
    let mut status = load();
    if status.error.as_deref() == Some(PIPELINE_INSTALL_ERROR) {
        status.phase = Phase::Idle;
        status.error = None;
        save(&status).map_err(|_| PIPELINE_INSTALL_ERROR)?;
    }
    fs::remove_file(marker).map_err(|_| PIPELINE_INSTALL_ERROR.into())
}

fn install_pipeline_scripts() -> Result<(), String> {
    // Only an installed app update or its pending retry reaches this path.
    let files = std::collections::BTreeMap::from([
        (
            "hex-meeting-compare-delayed.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-compare-delayed.ps1"),
        ),
        (
            "hex-meeting-compare-granola.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-compare-granola.py"),
        ),
        (
            "hex-meeting-english.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-english.ps1"),
        ),
        (
            "hex-meeting-notes.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-notes.ps1"),
        ),
        (
            "hex-meeting-publish.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-publish.py"),
        ),
        (
            "hex-meeting-quick-title.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-quick-title.ps1"),
        ),
        (
            "hex-meeting-render.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-render.py"),
        ),
        (
            "hex-meeting-tasks.py",
            include_str!("../scripts/meeting-pipeline/hex-meeting-tasks.py"),
        ),
        (
            "hex-meeting-watch.ps1",
            include_str!("../scripts/meeting-pipeline/hex-meeting-watch.ps1"),
        ),
        (
            "hex-pipeline-cli.ps1",
            include_str!("../scripts/meeting-pipeline/hex-pipeline-cli.ps1"),
        ),
        (
            "hex-pipeline-run.py",
            include_str!("../scripts/meeting-pipeline/hex-pipeline-run.py"),
        ),
        (
            "meeting-audio.py",
            include_str!("../scripts/meeting-pipeline/meeting-audio.py"),
        ),
        (
            "meeting-notes-prompt.md",
            include_str!("../scripts/meeting-pipeline/meeting-notes-prompt.md"),
        ),
        (
            "meeting-translate-prompt.md",
            include_str!("../scripts/meeting-pipeline/meeting-translate-prompt.md"),
        ),
    ]);
    let sha256 = |content: &str| {
        Sha256::digest(content.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let hashes: std::collections::BTreeMap<_, _> = files
        .iter()
        .map(|(name, content)| (*name, sha256(content)))
        .collect();
    let entry = include_str!("pipeline_entry.ps1");
    let payload = serde_json::json!({
        "version": VERSION, "files": files, "hashes": hashes,
        "entry": entry, "entry_hash": sha256(entry),
    });
    let error = PIPELINE_INSTALL_ERROR;
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("pipeline_install.ps1"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| error)?;
    let written = child.stdin.take().ok_or(error).and_then(|mut stdin| {
        stdin
            .write_all(payload.to_string().as_bytes())
            .map_err(|_| error)
    });
    // Wait for staging to finish even if stdin failed; publication is one pointer switch.
    let status = child.wait().map_err(|_| error)?;
    written?;
    if !status.success() {
        return Err(error.into());
    }
    Ok(())
}

pub fn after_update() {
    if !installed() {
        return;
    }
    let result = (|| {
        fs::create_dir_all(directory()).map_err(|_| PIPELINE_INSTALL_ERROR)?;
        fs::write(local("pipeline-install.pending"), VERSION.as_bytes())
            .map_err(|_| PIPELINE_INSTALL_ERROR)?;
        complete_pipeline_install()
    })();
    if let Err(error) = result {
        let _ = save(&Status {
            phase: Phase::Error,
            error: Some(error),
            ..load()
        });
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
    let _check = LISTENER_CHECK.lock().unwrap_or_else(|e| e.into_inner());
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

    fn open_lock(path: &std::path::Path) -> io::Result<Option<File>> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
            .map(Some)
    }
    #[test]
    fn window_probe_releases_its_handle_before_waiting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("listener.lock");
        let held = std::cell::RefCell::new(None);
        let mut waits = 0;
        wait_for_listener(
            Instant::now() + Duration::from_secs(30),
            || open_lock(&path),
            |_| {
                *held.borrow_mut() = open_lock(&path).unwrap();
                waits += 1;
            },
        )
        .unwrap();
        assert_eq!(waits, 1);
    }
    #[test]
    fn readiness_recovers_when_the_listener_holds_the_lock_and_detects_death() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("listener.lock");
        let mut status = ListenerStatus::default();
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(!status.ready);
        assert!(status.error.is_some());
        // A free-lock probe must also leave no handle behind.
        let listener = open_lock(&path).unwrap();
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(status.ready);
        assert!(status.error.is_none());
        drop(listener);
        status.complete(&listener_probe(|| open_lock(&path)));
        assert!(!status.ready);
        assert!(status.error.is_some());
    }
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
    fn mismatched_listener_task_is_never_started() {
        // Mock every task cmdlet; this never queries, registers or starts a real task.
        let prelude = r#"
function Get-ScheduledTask {
    $action = [PSCustomObject]@{ Execute = $env:SIDENOTE_TASK_EXE; Arguments = $env:SIDENOTE_TASK_ARGS }
    $principal = [PSCustomObject]@{ UserId = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value; LogonType = 'Interactive'; RunLevel = 'Limited' }
    $settings = [PSCustomObject]@{ Enabled = $true }
    switch ($env:SIDENOTE_TEST_MISMATCH) {
        'exe' { $action.Execute = 'other.exe' }
        'args' { $action.Arguments = '--other' }
        'privilege' { $principal.RunLevel = 'Highest' }
        'disabled' { $settings.Enabled = $false }
    }
    [PSCustomObject]@{ TaskName = $env:SIDENOTE_TASK_NAME; Actions = @($action); Principal = $principal; Settings = $settings }
}
function Register-ScheduledTask { throw 'Registration is forbidden in this test' }
function Start-ScheduledTask { $script:started = $true }
$script:started = $false
try {
"#;
        let suffix = r#"
} catch { if ($script:started) { exit 2 }; exit 1 }
if ($script:started) { exit 0 }; exit 3
"#;
        for mismatch in ["none", "exe", "args", "privilege", "disabled"] {
            let status = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!("{prelude}\n{LISTENER_TASK_SCRIPT}\n{suffix}"),
                ])
                .env("SIDENOTE_TASK_NAME", "Sidenote mock listener")
                .env("SIDENOTE_TASK_EXE", "sidenote-test.exe")
                .env("SIDENOTE_TASK_ARGS", "--listen")
                .env("SIDENOTE_TASK_RUN", "yes")
                .env("SIDENOTE_TASK_REGISTER", "no")
                .env("SIDENOTE_TEST_MISMATCH", mismatch)
                .creation_flags(0x08000000)
                .status()
                .unwrap();
            assert_eq!(
                status.code(),
                Some(if mismatch == "none" { 0 } else { 1 }),
                "{mismatch}"
            );
        }
    }

    #[test]
    fn listener_failure_is_independent_and_retry_clears_it_only_on_success() {
        let mut listener = ListenerStatus::default();
        listener.complete(&Err(
            "Listener task does not target this user and install".into()
        ));
        assert!(!listener.ready);
        assert!(listener.error.is_some());
        listener.complete(&Err("Still mismatched".into()));
        assert!(!listener.ready);
        assert_eq!(listener.error.as_deref(), Some("Still mismatched"));
        listener.complete(&Ok(()));
        assert!(listener.ready);
        assert!(listener.error.is_none());
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
