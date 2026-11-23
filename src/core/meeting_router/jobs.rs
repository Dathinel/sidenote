//! Durable per-meeting requests. The lock serializes drains, never enqueues.
use super::*;
pub(crate) mod handoff;
mod notes;
#[cfg(all(test, windows, feature = "asr-runtime"))]
pub(crate) mod r1_tests;
pub use handoff::{BatchOutput, commit_batches};
#[cfg(test)]
pub(crate) use notes::notes_input;
pub(crate) use notes::notes_input_sized;
pub(crate) fn notes_enabled(root: &Path) -> io::Result<bool> {
    Ok(notes::enabled(root).ok().flatten().is_some())
}
use crate::{
    core::model_catalog::speech_packs::{self, PackRefusal, RouteSnapshot},
    platform::CommandExt,
};
use std::{
    collections::HashSet,
    io,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Pending {
    id: String,
    generation: u128,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Job {
    id: String,
    folder: PathBuf,
    baseline: Vec<Chunk>,
    config: RoutingConfig,
    languages: Vec<String>,
    input_revision: String,
    /// Pinned when the drain opens the request; the child refuses a pin that has changed.
    routes: RouteSnapshot,
    #[serde(default)]
    generation: u128,
    windows: serde_json::Value,
    #[serde(skip)]
    input: Option<super::inputs::Snapshot>,
}
#[derive(Default, Serialize, Deserialize)]
struct Outcome {
    success: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    error: Option<String>,
}

const CPU_ASSETS_UNAVAILABLE: &str = "cpu_language_assets_unavailable";
const SPEECH_PACK_REFUSED: &str = "speech_pack_refused";
const PAUSED_RECORDING: &str = "paused: recording";
const PAUSED_STATUS_UNKNOWN: &str = "paused: recording status unknown";
const RECORDING_POLL: Duration = Duration::from_secs(2);

struct RecordingWatch<'a> {
    probe: Option<&'a mut dyn FnMut(Duration) -> hex_bridge::HexResult<bool>>,
    interval: Duration,
    stopping: &'a dyn Fn() -> bool,
}

impl Default for RecordingWatch<'_> {
    fn default() -> Self {
        Self {
            probe: None,
            interval: RECORDING_POLL,
            stopping: &crate::service::stopping,
        }
    }
}

impl RecordingWatch<'_> {
    fn pause(&mut self, remaining: Duration) -> Option<&'static str> {
        match self.probe.as_mut()?(remaining) {
            Ok(false) => None,
            Ok(true) => Some(PAUSED_RECORDING),
            Err(_) => Some(PAUSED_STATUS_UNKNOWN),
        }
    }

    fn admission_paused(&mut self, work: &Path, pending: &Pending) -> io::Result<bool> {
        let Some(pause) = self.pause(RECORDING_POLL) else {
            return Ok(false);
        };
        record_pause(
            work,
            pending,
            &Outcome {
                error: Some(pause.into()),
                ..Outcome::default()
            },
        )?;
        Ok(true)
    }
}

#[derive(Debug)]
struct CpuAssetsUnavailable(Option<u8>);
impl std::fmt::Display for CpuAssetsUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CPU_ASSETS_UNAVAILABLE)
    }
}
impl std::error::Error for CpuAssetsUnavailable {}

#[cfg(all(windows, feature = "asr-runtime"))]
fn require_cpu_assets(config: &RoutingConfig) -> std::result::Result<(), CpuAssetsUnavailable> {
    use super::super::model_catalog::ModelStore;
    if crate::core::asr::verify_runtime(&config.runtime_dll).is_err() {
        return Err(CpuAssetsUnavailable(None));
    }
    let Some(routes) = &config.after_stop_languages else {
        return Err(CpuAssetsUnavailable(None));
    };
    let forced = config.language_mode.strip_prefix("forced:").or_else(|| {
        (routes.supporting.is_empty() || config.language_mode == "off")
            .then_some(routes.primary.as_str())
    });
    let store = ModelStore::new(config.model_state_dir.clone());
    let mut models = std::iter::once(super::vad::MODEL_ID)
        .chain(forced.is_none().then_some("anuvani-lid-int8"))
        .chain(routes.routes.iter().filter_map(|(language, route)| {
            (route.provider == "local" && forced.is_none_or(|only| language == only))
                .then_some(route.model.as_str())
        }));
    if models.all(|id| store.installed_paths(id).is_ok_and(|paths| paths.is_some())) {
        return Ok(());
    }
    let ready = (forced.is_none() && config.language_mode == "auto")
        .then(|| store.incomplete_cpu_language_pack_count())
        .flatten();
    Err(CpuAssetsUnavailable(ready))
}

fn record_cpu_assets_failure(
    work: &Path,
    generation: u128,
    failure: &CpuAssetsUnavailable,
) -> io::Result<()> {
    let mut attempt = serde_json::json!({
        "generation": generation,
        "child": Outcome { error:Some(CPU_ASSETS_UNAVAILABLE.into()), ..Outcome::default() },
        "retry_after_ms": chrono::Utc::now().timestamp_millis()+30_000
    });
    if let Some(ready) = failure.0 {
        attempt["cpu_models_ready"] = ready.into();
    }
    store_io::write(&work.join("attempt.json"), &attempt)
}

/// Refused before any child, model load or audio work. The pending entry stays for retry.
fn record_refusal(work: &Path, generation: u128, refusal: &PackRefusal) -> io::Result<()> {
    let child = Outcome {
        error: Some(SPEECH_PACK_REFUSED.into()),
        ..Outcome::default()
    };
    let retry = chrono::Utc::now().timestamp_millis() + 30_000;
    let attempt = serde_json::json!({"generation":generation, "child":child,
        "refusal":refusal, "retry_after_ms":retry});
    store_io::write(&work.join("attempt.json"), &attempt)
}
/// Cached range readings are reused only under the route pins that produced them.
fn bind_readings(batches: &Path, routes: &RouteSnapshot) -> Result<()> {
    let pins = serde_json::to_vec(routes)?;
    if fs::read(batches.join("routes.json")).ok().as_ref() != Some(&pins) {
        match fs::remove_dir_all(batches) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => fs::create_dir_all(batches)?,
        }
        store_io::atomic_write(&batches.join("routes.json"), &pins)?;
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn queue(root: &Path) -> PathBuf {
    root.join("local/router-queue")
}
pub(crate) fn recycle_busy(root: &Path, id: &str) -> io::Result<bool> {
    Ok(queue(root).join(format!("{id}.json")).try_exists()?
        || super::super::meeting_delete::lock_busy(&queue(root).join(format!("{id}.lock")))?)
}

fn destination(root: &Path, id: &str) -> PathBuf {
    root.join("local/transcripts")
        .join(id)
        .join("transcript.segments.v1.json")
}
fn pending_lock(root: &Path, id: &str) -> io::Result<fs::File> {
    let start = Instant::now();
    loop {
        match store_io::lock(&queue(root).join(format!("{id}.lock"))) {
            Ok(lock) => return Ok(lock),
            Err(e)
                if ((cfg!(windows) && e.raw_os_error() == Some(32))
                    || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock))
                    && start.elapsed() < Duration::from_secs(10) =>
            {
                thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(e),
        }
    }
}
fn acknowledge(root: &Path, id: &str, path: &Path, bytes: &[u8]) -> Result<()> {
    let _lock = pending_lock(root, id)?;
    if fs::read(path).ok().as_deref() == Some(bytes) {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn configured_at(root: &Path) -> io::Result<bool> {
    Ok(super::super::Config::read_config(root)?
        .is_some_and(|config| config.features.indic_engine && config.pipeline.enabled()))
}

const ENGINE_OFF: &str = "Hindi and Telugu routing is off. Check Settings > Experimental features.";

pub fn configured() -> bool {
    configured_at(&super::super::state_dir()).unwrap_or(false)
}
pub fn enqueue(id: &str) -> Result<()> {
    let root = super::super::state_dir();
    if !configured_at(&root)? {
        return Err(ENGINE_OFF.into());
    }
    enqueue_in(&root, &super::super::meetings_dir(), id)
}
pub(crate) fn enqueue_in(root: &Path, meetings: &Path, id: &str) -> Result<()> {
    enqueue_with(root, meetings, id, false)
}
fn enqueue_with(root: &Path, meetings: &Path, id: &str, automatic: bool) -> Result<()> {
    let _update_activity = crate::updates::activity()?;
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
    let Some(_recycle) = super::super::meeting_delete::claim_write(root, id)? else {
        return Ok(());
    };
    let input = match super::inputs::Snapshot::capture(&meetings.join(id)) {
        Ok(input) => input,
        Err(_) => return Ok(()),
    };
    if super::super::recording_guard::eligibility_captured(
        root,
        id,
        &input.manifest,
        &input.transcript,
    )? != super::super::recording_guard::Eligibility::Eligible
    {
        return Ok(());
    }
    let _lock = pending_lock(root, id)?;
    if (automatic && automatic_accepted_locked(root, meetings, id)?)
        || queue(root).join(format!("{id}.json")).exists()
    {
        return Ok(());
    }
    if !automatic {
        match fs::remove_file(queue(root).join(format!("{id}.completed"))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let generation = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    if notes::enabled(root)?.is_some() {
        revision::begin(destination(root, id).parent().unwrap(), generation)?;
    }
    store_io::write(
        &queue(root).join(format!("{id}.json")),
        &Pending {
            id: id.into(),
            generation,
        },
    )?;
    Ok(())
}
/// Automatic discovery must not create a new generation for unchanged input.
/// Only successful output completes a revision; failed attempts remain retryable.
pub(crate) fn enqueue_automatic(id: &str) -> Result<()> {
    let root = super::super::state_dir();
    if !configured_at(&root)? {
        return Ok(());
    }
    enqueue_with(&root, &super::super::meetings_dir(), id, true)
}
fn automatic_revision(root: &Path, id: &str) -> io::Result<Option<String>> {
    match store_io::read_bounded(&queue(root).join(format!("{id}.completed")), 1024) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| store_io::invalid("Invalid completion receipt")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn automatic_accepted_locked(root: &Path, meetings: &Path, id: &str) -> Result<bool> {
    let pending: Option<Pending> = store_io::read(&queue(root).join(format!("{id}.json")))?;
    if let Some(pending) = pending {
        if pending.id != id {
            return Err("Invalid queued meeting".into());
        }
        return Ok(true);
    }
    match snapshot_in(root, meetings, id)? {
        Some(job) => completed(root, &job),
        None => Ok(false),
    }
}
fn completed(root: &Path, job: &Job) -> Result<bool> {
    Ok(
        automatic_revision(root, &job.id)?.as_deref() == Some(job.input_revision.as_str())
            && (!notes_enabled(root)? || handoff::completed(root, &job.id, &job.input_revision)),
    )
}
#[cfg(any(windows, test))]
pub(crate) fn automatic_accepted(id: &str) -> Result<bool> {
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
    let root = super::super::state_dir();
    let Some(_recycle) = super::super::meeting_delete::claim_write(&root, id)? else {
        return Ok(false);
    };
    let _lock = pending_lock(&root, id)?;
    automatic_accepted_locked(&root, &super::super::meetings_dir(), id)
}
fn complete_automatic(root: &Path, id: &str, revision: &str) -> io::Result<()> {
    // Persist before acknowledging: a crash between these writes safely reuses output.
    store_io::atomic_write(
        &queue(root).join(format!("{id}.completed")),
        revision.as_bytes(),
    )
}

fn load_config(root: &Path) -> Result<RoutingConfig> {
    let app = super::super::Config::load_in(root)?;
    let mut config = app.pipeline;
    config.meeting_languages = super::super::model_catalog::load_meeting_languages(root)?;
    #[cfg(windows)]
    if app.features.indic_engine && app.labs.after_stop_language_routing && config.enabled() {
        config
            .after_stop_languages
            .get_or_insert_with(LanguageRoutes::cpu_after_stop);
        if config.model_state_dir.as_os_str().is_empty() {
            config.model_state_dir = root.to_owned();
        }
        if config.runtime_dll.as_os_str().is_empty() {
            config.runtime_dll =
                std::env::current_exe()?.with_file_name(crate::platform::asr_library_name());
        }
    }
    config.validate()?;
    Ok(config)
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn automatic_runtime_comes_from_the_package_and_explicit_path_wins() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path();
    let name = crate::platform::asr_library_name();
    let old_state_dll = root.join(name);
    fs::write(&old_state_dll, b"old unverified runtime").unwrap();
    let mut settings = crate::core::Config::load_in(root).unwrap();
    settings.features.indic_engine = true;
    settings.save(root).unwrap();
    let app = std::env::current_exe().unwrap();
    assert_eq!(
        load_config(root).unwrap().runtime_dll,
        app.with_file_name(name)
    );

    let explicit = root.join("explicit.dll");
    let mut config = crate::core::Config::load_in(root).unwrap();
    config.pipeline.runtime_dll = explicit.clone();
    store_io::write(&root.join("config.json"), &config).unwrap();
    assert_eq!(load_config(root).unwrap().runtime_dll, explicit);
}

pub(crate) fn read_baseline(bytes: &[u8]) -> Result<Vec<Chunk>> {
    let mut baseline = Vec::new();
    for (i, line) in std::str::from_utf8(bytes)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        let text = v["text"].as_str().unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let start = v["start_ms"].as_f64().unwrap_or(0.0) / 1000.0;
        let end = v["end_ms"].as_f64().unwrap_or((start + 0.1) * 1000.0) / 1000.0;
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
            return Err("Invalid baseline interval".into());
        }
        baseline.push(Chunk {
            id: format!("english-{i}"),
            start,
            end,
            track: v["source"].as_str().unwrap_or("system").into(),
            speaker: v["speaker"].as_str().map(str::to_owned),
            turn: None,
            english: text.into(),
            audio_start: None,
            words: super::capture_words(&v, start, end, text),
        });
    }
    Ok(baseline)
}

fn snapshot(root: &Path, id: &str) -> Result<Option<Job>> {
    snapshot_in(root, &super::super::meetings_dir(), id)
}

fn snapshot_in(root: &Path, meetings: &Path, id: &str) -> Result<Option<Job>> {
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
    let folder = meetings.join(id);
    let input = super::inputs::Snapshot::capture(&folder)?;
    if super::super::recording_guard::eligibility_captured(
        root,
        id,
        &input.manifest,
        &input.transcript,
    )? != super::super::recording_guard::Eligibility::Eligible
    {
        return Ok(None);
    }
    let baseline = read_baseline(&input.transcript)?;
    let config = load_config(root)?;
    let windows = input.windows.clone();
    let revision = cache::key(
        &[],
        &configuration_key(&config)?,
        &input.revision()?,
        "pinned-input-v1",
    );
    Ok(Some(Job {
        id: id.into(),
        folder,
        baseline,
        languages: config.meeting_languages.clone(),
        routes: config.route_snapshot(speech_packs::registry()?),
        config,
        input_revision: revision,
        generation: 0,
        windows,
        input: Some(input),
    }))
}

fn seconds_arg(flag: &str) -> Option<f64> {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2)
        .find(|a| a[0] == flag)
        .and_then(|a| a[1].parse::<f64>().ok())
        .filter(|s| s.is_finite() && (1.0..=86400.0).contains(s))
}
fn path_arg(flag: &str) -> Option<PathBuf> {
    let args: Vec<_> = std::env::args_os().collect();
    args.windows(2)
        .find(|a| a[0] == flag)
        .map(|a| PathBuf::from(&a[1]))
}
#[cfg(windows)]
fn resume(child: &std::process::Child) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::*,
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut next = Thread32First(snapshot, &mut entry);
        let mut result = Err(io::Error::other("Suspended routing thread unavailable"));
        while next != 0 {
            if entry.th32OwnerProcessID == child.id() {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                result = if thread.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    let value = ResumeThread(thread);
                    let error = io::Error::last_os_error();
                    CloseHandle(thread);
                    if value == u32::MAX {
                        Err(error)
                    } else {
                        Ok(())
                    }
                };
                break;
            }
            next = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        result
    }
}
fn supervise(
    mut command: Command,
    seconds: f64,
    active: Option<&Path>,
    watch: &mut RecordingWatch<'_>,
) -> Outcome {
    if let Err(error) = crate::platform::contain_command(&mut command) {
        return Outcome {
            error: Some(error.to_string()),
            ..Outcome::default()
        };
    }
    // CREATE_SUSPENDED closes the spawn/attach gap before any descendants can be created.
    command
        .creation_flags(0x08000004)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(_) => {
            return Outcome {
                error: Some("process unavailable".into()),
                ..Outcome::default()
            };
        }
    };
    let job = match crate::platform::ProcessJob::attach(&child) {
        Ok(j) => j,
        Err(_) => {
            let _ = child.kill();
            let status = child.wait().ok();
            return Outcome {
                exit_code: status.and_then(|s| s.code()),
                error: Some("process containment failed".into()),
                ..Outcome::default()
            };
        }
    };
    #[cfg(windows)]
    if resume(&child).is_err() {
        drop(job);
        let _ = child.kill();
        let status = child.wait().ok();
        return Outcome {
            exit_code: status.and_then(|s| s.code()),
            error: Some("process resume failed".into()),
            ..Outcome::default()
        };
    }
    if let Some(path) = active {
        let _ = store_io::write(path, &serde_json::json!({"pid":child.id()}));
    }
    let start = Instant::now();
    let budget = Duration::try_from_secs_f64(seconds).unwrap_or_default();
    let mut next_probe = watch.interval;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break Outcome {
                    success: status.success(),
                    exit_code: status.code(),
                    ..Outcome::default()
                };
            }
            state => {
                let elapsed = start.elapsed();
                let mut pause = None;
                if state.is_ok() && !(watch.stopping)() && elapsed < budget {
                    if watch.probe.is_some() && elapsed >= next_probe {
                        next_probe = elapsed + watch.interval;
                        pause = watch.pause(budget - elapsed);
                    }
                    if pause.is_none() && !(watch.stopping)() && start.elapsed() < budget {
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                }
                drop(job); // Kill the complete tree before waiting, including normalization/Hex descendants.
                let _ = child.kill();
                let status = child.wait().ok();
                if let Some(path) = active {
                    let _ = fs::remove_file(path);
                }
                return Outcome {
                    exit_code: status.and_then(|s| s.code()),
                    timed_out: pause.is_none() && state.is_ok() && !(watch.stopping)(),
                    error: Some(pause.unwrap_or("process deadline or wait failure").into()),
                    ..Outcome::default()
                };
            }
        }
    };
    drop(job);
    if let Some(path) = active {
        let _ = fs::remove_file(path);
    }
    outcome
}

// The Stop deadline belongs only to notes_input. Every attempt gets a fresh budget.
fn run_worker(
    command: Command,
    duration: f64,
    config: Option<&crate::core::Config>,
    active: Option<&Path>,
    watch: &mut RecordingWatch<'_>,
) -> Result<Outcome> {
    let computed = (duration * 4.0 + 120.0).clamp(120.0, 86400.0);
    let budget = if let Some(config) = config {
        let value = config
            .engine_settings
            .stage_deadline_seconds
            .filter(|&v| v != 0);
        if value.is_some_and(|v| v > 3600) {
            return Err("Invalid ASR attempt budget".into());
        }
        f64::from(value.unwrap_or(computed.min(3600.0) as u32))
    } else {
        computed
    };
    Ok(supervise(
        command,
        seconds_arg("--router-timeout-seconds").unwrap_or(budget),
        active,
        watch,
    ))
}

struct Cleanup(Vec<PathBuf>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}
fn cleanup_work(work: &Path) {
    if let Some(folder) = fs::read(work.join("request.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["folder"].as_str().map(PathBuf::from))
        && folder.parent() == Some(work)
        && folder
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("inputs-"))
        && fs::symlink_metadata(&folder).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        fs::remove_dir_all(folder).ok();
    }
    // Only generated WAVs in this validated meeting's work directory, never source audio.
    if let Ok(entries) = fs::read_dir(work) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "wav") {
                let _ = fs::remove_file(p);
            }
        }
    }
}

pub fn run_one() -> Result<()> {
    let request = path_arg("--router-request").ok_or("Missing router request")?;
    let root = super::super::state_dir();
    if !configured_at(&root)? {
        return Err(ENGINE_OFF.into());
    }
    run_one_in(&request, &root)
}
pub(crate) fn run_one_in(request: &Path, root: &Path) -> Result<()> {
    let mut job: Job =
        serde_json::from_slice(&super::inputs::Snapshot::read_file(request, 16 << 20)?)?;
    let Some(_recycle) = super::super::meeting_delete::claim_write(root, &job.id)? else {
        return Ok(());
    };
    job.config.meeting_languages = job.languages;
    job.config
        .verify_routes(&job.routes, speech_packs::registry()?)?;
    let work = request.parent().ok_or("Missing work directory")?;
    let labs = notes_enabled(root)?;
    if labs
        && handoff::assembled(
            root,
            &job.id,
            job.generation,
            &job.input_revision,
            &job.baseline,
        )
        .is_ok()
    {
        return Ok(());
    }
    #[cfg(not(windows))]
    if labs {
        return Err(super::hex_bridge::HexFailure::Unsupported.into());
    }
    let normalization_started = Instant::now();
    let mut normalized = Cleanup(Vec::new());
    let duration = job.baseline.iter().map(|c| c.end).fold(0.0, f64::max);
    let mut paths = Vec::new();
    for track in ["system", "microphone"] {
        let input = job.folder.join(format!("{track}.wav"));
        if !input.exists() {
            continue;
        }
        if Audio::open(&[(track.into(), input.clone())]).is_ok() {
            paths.push((track.into(), input));
            continue;
        }
        let output = work.join(format!("{track}-16k.wav"));
        normalized.0.push(output.clone());
        let estimate = hound::WavReader::open(&input)
            .ok()
            .map(|r| r.duration() as f64 / r.spec().sample_rate as f64)
            .unwrap_or(duration);
        let deadline = seconds_arg("--normalization-timeout-seconds")
            .unwrap_or((estimate * 0.5 + 15.0).clamp(30.0, 7200.0));
        let mut command = Command::new(path_arg("--ffmpeg").unwrap_or_else(|| "ffmpeg".into()));
        command
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(input)
            .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(&output);
        let outcome = supervise(command, deadline, None, &mut RecordingWatch::default());
        store_io::write(&work.join("normalization-outcome.json"), &outcome)?;
        if !outcome.success {
            return Err("Audio normalization failed or timed out".into());
        }
        Audio::open(&[(track.into(), output.clone())])?;
        paths.push((track.into(), output));
    }
    if labs {
        let _ = store_io::write(
            &work.join("normalization.clock.json"),
            &serde_json::json!({"seconds":normalization_started.elapsed().as_secs_f64()}),
        );
        #[cfg(windows)]
        {
            let mut config = super::super::Config::load_in(root)?;
            config.pipeline = job.config.clone();
            #[cfg(feature = "asr-runtime")]
            config.labs.language_detection_settings.mark_after_stop();
            let batches = work.join("batches");
            fs::create_dir_all(&batches)?;
            super::production::run(
                &job.folder,
                &paths,
                &job.baseline,
                &config,
                &batches,
                (&job.input_revision, job.generation),
                None,
            )?;
        }
        handoff::assembled(
            root,
            &job.id,
            job.generation,
            &job.input_revision,
            &job.baseline,
        )?;
        return Ok(());
    }
    let mut audio = Audio::open(&paths)?;
    let detected = audio.speech_chunks()?;
    let mut chunks = Vec::new();
    for (track, path) in &paths {
        let count = hound::WavReader::open(path)?.duration() as usize;
        let mut windows = prepared_chunks(&job.windows, track, count).unwrap_or_else(|| {
            detected
                .iter()
                .filter(|c| &c.track == track)
                .cloned()
                .collect()
        });
        for chunk in &mut windows {
            chunk.id = format!("{track}-{}", chunk.id);
            chunk.track = track.clone();
            chunk.english.clear();
        }
        chunks.extend(windows);
    }
    chunks.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut result = route(Request {
        audio: &mut audio,
        baseline: job.baseline,
        chunks,
        config: &job.config,
        hex: &HexSpeech::default(),
        work,
        cache: &super::super::state_dir().join("local/cache"),
    })?;
    result.input_checksum = job.input_revision;
    write_transcript(&work.join("result.json"), &result)
}

pub fn run_pending() -> Result<()> {
    run_pending_with(|_job, request| {
        #[cfg(all(windows, feature = "asr-runtime"))]
        if notes_enabled(&super::super::state_dir())? {
            require_cpu_assets(&_job.config)?;
        }
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--route-one")
            .arg("--router-request")
            .arg(request)
            .arg("--data-dir")
            .arg(super::super::state_dir())
            .arg("--meetings-dir")
            .arg(super::super::meetings_dir());
        Ok(command)
    })
}
pub(crate) fn run_pending_with(worker: impl FnMut(&Job, &Path) -> Result<Command>) -> Result<()> {
    let _update_activity = crate::updates::activity()?;
    let root = super::super::state_dir();
    let args: Vec<_> = std::env::args().collect();
    let automatic = crate::service::active() || args.iter().any(|arg| arg == "--automatic");
    if let Some(id) = args
        .windows(2)
        .find(|a| a[0] == "--meeting-id")
        .map(|a| a[1].as_str())
    {
        if automatic {
            enqueue_automatic(id)?;
        } else {
            enqueue(id)?;
        }
    }
    let mut watch = RecordingWatch::default();
    #[cfg(windows)]
    let mut probe = |remaining| {
        let speech = path_arg("--hex-cli")
            .map_or_else(HexSpeech::default, |executable| HexSpeech { executable });
        hex_bridge::HexBridge::new(speech).recording(remaining)
    };
    #[cfg(windows)]
    if !cfg!(test) {
        watch.probe = Some(&mut probe);
    }
    let meetings = super::super::meetings_dir();
    run_pending_in(root, &meetings, &args, automatic, worker, &mut watch)
}

fn run_pending_in(
    root: PathBuf,
    meetings: &Path,
    args: &[String],
    automatic: bool,
    mut worker: impl FnMut(&Job, &Path) -> Result<Command>,
    watch: &mut RecordingWatch<'_>,
) -> Result<()> {
    if !configured_at(&root)? {
        return Ok(());
    }
    let _lock = match store_io::lock(&root.join("local/meeting-router.lock")) {
        Ok(lock) => lock,
        Err(e)
            if ((cfg!(windows) && e.raw_os_error() == Some(32))
                || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock)) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    fs::create_dir_all(queue(&root))?;
    let mut attempted = HashSet::new();
    loop {
        if (watch.stopping)() {
            return Ok(());
        }
        let mut found = false;
        for entry in fs::read_dir(queue(&root))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let pending: Pending = serde_json::from_slice(&bytes)?;
            if !valid_id(&pending.id) {
                return Err("Invalid queued meeting".into());
            }
            if !configured_at(&root)? {
                return Ok(());
            }
            if !attempted.insert((pending.id.clone(), pending.generation)) {
                continue;
            }
            let Some(_recycle) = super::super::meeting_delete::claim_write(&root, &pending.id)?
            else {
                continue;
            };
            found = true;
            let mut job = match snapshot_in(&root, meetings, &pending.id) {
                Ok(Some(job)) => job,
                Ok(None) => continue,
                Err(_) => {
                    store_io::write(
                        &root
                            .join("local/router-work")
                            .join(&pending.id)
                            .join("attempt.json"),
                        &serde_json::json!({"generation":pending.generation,"setup_failed":true,"pending_retained":true,"child":{"success":false,"exit_code":null,"timed_out":false,"error":"setup_failed"}}),
                    )?;
                    continue;
                }
            };
            let destination = destination(&root, &job.id);
            let labs = notes_enabled(&root)?;
            if completed(&root, &job)? {
                acknowledge(&root, &job.id, &path, &bytes)?;
                continue;
            }
            if automatic && !retry_ready(&root, &pending) {
                continue;
            }
            if !labs
                && job.config.meeting_languages == ["en"]
                && !matches!(
                    job.config.language_mode.as_str(),
                    "forced:hi" | "forced:te" | "forced:mixed"
                )
            {
                if destination.exists()
                    && !(watch.stopping)()
                    && snapshot_in(&root, meetings, &job.id).is_ok_and(|latest| {
                        latest.is_some_and(|latest| latest.input_revision == job.input_revision)
                    })
                {
                    let mut english = english_fallback(
                        &job.baseline,
                        &job.config,
                        job.input_revision.clone(),
                        "English declared",
                    );
                    for segment in &mut english.segments {
                        segment.status = "declared".into();
                    }
                    english.warnings.clear();
                    write_transcript(&destination, &english)?;
                }
                if (watch.stopping)()
                    || !snapshot_in(&root, meetings, &job.id).is_ok_and(|latest| {
                        latest.is_some_and(|latest| latest.input_revision == job.input_revision)
                    })
                {
                    continue;
                }
                complete_automatic(&root, &job.id, &job.input_revision)?;
                acknowledge(&root, &job.id, &path, &bytes)?;
                continue;
            }
            if !labs
                && let Some(old) = fs::read(&destination)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Transcript>(&b).ok())
                && old.input_checksum == job.input_revision
                && !old.degraded()
            {
                complete_automatic(&root, &job.id, &job.input_revision)?;
                acknowledge(&root, &job.id, &path, &bytes)?;
                continue;
            }
            let work = root.join("local/router-work").join(&job.id);
            fs::create_dir_all(&work)?;
            cleanup_work(&work);
            // The planner refuses for every caller; labs jobs refuse here too, before any launch,
            // so the notes can say why.
            if labs {
                if let Some(refusal) = speech_packs::first_refusal(&job.routes) {
                    record_refusal(&work, pending.generation, refusal)?;
                    continue;
                }
                // A locked cache file holds back only this meeting, not the whole queue.
                if bind_readings(&work.join("batches"), &job.routes).is_err() {
                    let failed = serde_json::json!({"generation":pending.generation,"setup_failed":true,"pending_retained":true,"child":{"success":false,"exit_code":null,"timed_out":false,"error":"setup_failed"}});
                    store_io::write(&work.join("attempt.json"), &failed)?;
                    continue;
                }
            }
            let admitted = Instant::now();
            if watch.admission_paused(&work, &pending)? {
                continue;
            }
            let request = work.join("request.json");
            job.generation = pending.generation;
            let mut command = match worker(&job, &request) {
                Ok(command) => command,
                Err(error) => match error.downcast_ref::<CpuAssetsUnavailable>() {
                    Some(failure) => {
                        record_cpu_assets_failure(&work, pending.generation, failure)?;
                        continue;
                    }
                    None => return Err(error),
                },
            };
            let staged = job
                .input
                .as_mut()
                .ok_or("Missing pinned inputs")?
                .stage(&work)?;
            job.folder = staged.path().to_owned();
            store_io::write(&request, &job)?;
            let _ = fs::remove_file(work.join("result.json"));
            let _ = fs::remove_file(work.join("normalization-outcome.json"));
            let _ = fs::remove_file(work.join("failure.json"));
            for flag in ["--ffmpeg", "--normalization-timeout-seconds", "--hex-cli"] {
                if let Some(value) = args.windows(2).find(|a| a[0] == flag).map(|a| &a[1]) {
                    command.args([flag, value]);
                }
            }
            let duration = job.baseline.iter().map(|c| c.end).fold(0.0, f64::max);
            let settings = if labs { notes::enabled(&root)? } else { None };
            if admitted.elapsed() >= RECORDING_POLL && watch.admission_paused(&work, &pending)? {
                cleanup_work(&work);
                continue;
            }
            if (watch.stopping)() || !configured_at(&root)? {
                cleanup_work(&work);
                return Ok(());
            }
            let mut outcome = run_worker(
                command,
                duration,
                settings.as_ref(),
                Some(&work.join("active.json")),
                watch,
            )?;
            if !configured_at(&root)? {
                cleanup_work(&work);
                return Ok(());
            }
            if matches!(
                outcome.error.as_deref(),
                Some(PAUSED_RECORDING | PAUSED_STATUS_UNKNOWN)
            ) {
                record_pause(&work, &pending, &outcome)?;
                cleanup_work(&work);
                continue;
            }
            if let Some(error) = fs::read(work.join("failure.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["error"].as_str().map(str::to_owned))
            {
                outcome.error = Some(error);
            }
            let mut stable = !(watch.stopping)();
            match snapshot_in(&root, meetings, &job.id) {
                Ok(Some(latest)) if latest.input_revision != job.input_revision => {
                    stable = false;
                    outcome.success = false;
                    outcome.error = Some("input changed during routing".into());
                }
                Ok(None) | Err(_) => {
                    stable = false;
                    outcome.success = false;
                    outcome.error = Some("input unavailable after routing".into());
                }
                _ => {}
            }
            let assembled = if labs && stable && outcome.success {
                let commit_started = Instant::now();
                let committed = commit_batches(
                    &root,
                    &job.id,
                    pending.generation,
                    &job.input_revision,
                    &job.baseline,
                );
                // A timing receipt must not invalidate an already published revision.
                let _ = store_io::write(
                    &work.join("commit.clock.json"),
                    &serde_json::json!({"seconds":commit_started.elapsed().as_secs_f64()}),
                );
                match committed {
                    Ok(id) => revision::load(destination.parent().unwrap(), Some(&id))?
                        .map(|(_, r)| r.transcript),
                    Err(_) => {
                        outcome.success = false;
                        outcome.error = Some("batch assembly failed".into());
                        None
                    }
                }
            } else {
                None
            };
            let published_labs = assembled.is_some();
            let result = if labs {
                assembled
            } else if outcome.success {
                fs::read(work.join("result.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Transcript>(&b).ok())
                    .filter(|r| {
                        r.input_checksum == job.input_revision
                            && serde_json::to_vec(&r.baseline).ok()
                                == serde_json::to_vec(&job.baseline).ok()
                    })
            } else {
                None
            };
            if result.is_none() {
                outcome.success = false;
                if outcome.error.is_none() {
                    outcome.error = Some("routing output absent or invalid".into());
                }
            }
            let mut result = result.unwrap_or_else(|| {
                english_fallback(
                    &job.baseline,
                    &job.config,
                    job.input_revision.clone(),
                    &format!(
                        "routing child failed (exit {:?}, timeout {}): {}",
                        outcome.exit_code,
                        outcome.timed_out,
                        outcome.error.as_deref().unwrap_or("process stopped")
                    ),
                )
            });
            if labs && !published_labs {
                super::baseline::reconcile_exact(&mut result, &job.baseline);
            }
            let successful = stable && outcome.success && !result.degraded();
            let published_fallback = if !labs {
                publish_attempt(&destination, &result, successful, stable)?
            } else {
                false
            };
            let normalization = fs::read(work.join("normalization-outcome.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
            store_io::write(
                &work.join("attempt.json"),
                &serde_json::json!({"generation":pending.generation,"child":outcome,"published_fallback":published_fallback,"retry_after_ms":chrono::Utc::now().timestamp_millis()+30_000,"segments":result.segments.len(),"normalization":normalization}),
            )?;
            cleanup_work(&work);
            if outcome
                .error
                .as_deref()
                .is_some_and(|e| e.contains("FATAL:"))
            {
                return Err("FATAL: Hex selection check failed; pending retained".into());
            }
            if successful {
                complete_automatic(&root, &job.id, &job.input_revision)?;
                acknowledge(&root, &job.id, &path, &bytes)?;
            }
        }
        if !found {
            break;
        }
    }
    Ok(())
}

fn record_pause(work: &Path, pending: &Pending, outcome: &Outcome) -> io::Result<()> {
    store_io::write(
        &work.join("attempt.json"),
        &serde_json::json!({"generation":pending.generation,"child":outcome,"pending_retained":true,"retry_after_ms":chrono::Utc::now().timestamp_millis()+30_000}),
    )
}

fn publish_attempt(
    destination: &Path,
    result: &Transcript,
    successful: bool,
    stable: bool,
) -> Result<bool> {
    if successful {
        write_transcript(destination, result)?;
    } else if stable && !crate::service::stopping() {
        match store_io::publish(destination, &serde_json::to_vec_pretty(result)?) {
            Ok(()) => return Ok(true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}
fn retry_ready(root: &Path, pending: &Pending) -> bool {
    let receipt = root
        .join("local/router-work")
        .join(&pending.id)
        .join("attempt.json");
    !fs::read(receipt)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| {
            v["generation"].as_u64().map(u128::from) == Some(pending.generation)
                && v["retry_after_ms"]
                    .as_i64()
                    .is_some_and(|t| t > chrono::Utc::now().timestamp_millis())
        })
}
pub fn start_worker() -> io::Result<()> {
    if !configured()
        || !fs::read_dir(queue(&super::super::state_dir())).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                if e.path().extension().is_none_or(|x| x != "json") {
                    return false;
                }
                let pending = fs::read(e.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Pending>(&b).ok());
                let Some(pending) = pending else {
                    return true;
                };
                snapshot(&super::super::state_dir(), &pending.id).is_ok_and(|job| job.is_some())
                    && retry_ready(&super::super::state_dir(), &pending)
            })
        })
    {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if crate::service::active() {
        return crate::service::workers::start("routing", || {
            let _ = run_pending();
        });
    }
    let child = crate::platform::worker_command(std::env::current_exe()?)?
        .arg("--route-pending")
        .arg("--automatic")
        .arg("--data-dir")
        .arg(super::super::state_dir())
        .arg("--meetings-dir")
        .arg(super::super::meetings_dir())
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    crate::service::own_child(child)
}
fn prepared_chunks(
    value: &serde_json::Value,
    track: &str,
    sample_count: usize,
) -> Option<Vec<Chunk>> {
    let mut spans: Vec<_> = value["segments"]
        .as_array()?
        .iter()
        .filter(|s| s["Track"].as_str() == Some(format!("{track}.wav").as_str()))
        .collect();
    spans.sort_by(|a, b| {
        a["Start"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&b["Start"].as_f64().unwrap_or(0.0))
    });
    let mut chunks = Vec::new();
    let mut previous: Option<(String, f64)> = None;
    let mut turn = 0;
    for span in spans {
        let mut start = span["Start"].as_f64()?;
        let end = span["End"].as_f64()?;
        if !start.is_finite()
            || !end.is_finite()
            || start < 0.0
            || end <= start
            || end > sample_count as f64 / 16000.0
        {
            return None;
        }
        let speaker = span["Speaker"].as_str()?.to_owned();
        if previous
            .as_ref()
            .is_none_or(|(who, until)| who != &speaker || start - until > 1.5)
        {
            turn += 1;
        }
        previous = Some((speaker.clone(), end));
        while start < end {
            let cut = (start + 24.0).min(end);
            if cut - start < 0.1 {
                return None;
            }
            chunks.push(Chunk {
                id: format!("prepared-{}", chunks.len()),
                start,
                end: cut,
                track: track.into(),
                speaker: Some(speaker.clone()),
                turn: Some(format!("{track}-{turn}")),
                english: String::new(),
                audio_start: None,
                words: Vec::new(),
            });
            start = cut;
        }
    }
    (!chunks.is_empty()).then_some(chunks)
}

// These fixtures mutate process-wide paths/config and drain every queued job.
#[cfg(test)]
pub(crate) fn isolated_test() -> bool {
    const FLAG: &str = "SIDENOTE_ISOLATED_ROUTER_TEST";
    let thread = thread::current();
    let name = thread.name().unwrap();
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env(FLAG, name);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "isolated test failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[cfg(test)]
mod recording_guard_tests {
    use super::*;
    #[test]
    fn automatic_completion_survives_crash_before_queue_acknowledgement() {
        let root = tempfile::tempdir().unwrap();
        let id = "1700000000000-1";
        let pending = queue(root.path()).join(format!("{id}.json"));
        store_io::atomic_write(&pending, b"pending generation").unwrap();
        let bytes = fs::read(&pending).unwrap();
        assert!(automatic_revision(root.path(), id).unwrap().is_none());
        complete_automatic(root.path(), id, "input-a").unwrap();
        assert_eq!(
            automatic_revision(root.path(), id).unwrap().as_deref(),
            Some("input-a")
        );
        assert_eq!(fs::read(&pending).unwrap(), bytes);
        // A newer enqueue must survive an older worker's acknowledgement.
        store_io::atomic_write(&pending, b"new generation").unwrap();
        acknowledge(root.path(), id, &pending, &bytes).unwrap();
        assert_eq!(fs::read(&pending).unwrap(), b"new generation");
        acknowledge(root.path(), id, &pending, b"new generation").unwrap();
        assert!(!pending.exists());
        assert_eq!(
            automatic_revision(root.path(), id).unwrap().as_deref(),
            Some("input-a")
        );
    }
    #[test]
    fn recording_source_reconnect_and_worker_recovery_reuse_completion() {
        use crate::core;
        if isolated_test() {
            return;
        }
        const ID: &str = "9944771122-887768";
        let root = core::state_dir();
        let folder = core::meetings_dir().join(ID);
        fs::create_dir_all(&folder).unwrap();
        store_io::write(
            &folder.join("manifest.json"),
            &serde_json::json!({"status":"complete","duration_ms":30000}),
        )
        .unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
        )
        .unwrap();
        let mut config = core::Config::load().unwrap();
        config.pipeline.profile = "experimental".into();
        config.features.indic_engine = true;
        config.labs.after_stop_language_routing = false;
        store_io::write(&root.join("config.json"), &config).unwrap();
        let dest = destination(&root, ID);
        write_transcript(
            &dest,
            &english_fallback(&[], &config.pipeline, "seed".into(), "synthetic"),
        )
        .unwrap();
        enqueue_automatic(ID).unwrap();
        let pending = queue(&root).join(format!("{ID}.json"));
        let generation = fs::read(&pending).unwrap();
        run_pending().unwrap();
        assert!(!pending.exists());
        let output = fs::read(&dest).unwrap();
        let modified = fs::metadata(&dest).unwrap().modified().unwrap();
        crate::listener::catch_up_routing_automatic(&core::meetings_dir()).unwrap();
        assert!(!pending.exists(), "reconnect created another generation");
        // Restore the real pending generation: crash after receipt, before acknowledgement.
        store_io::atomic_write(&pending, &generation).unwrap();
        run_pending().unwrap();
        assert!(!pending.exists());
        assert_eq!(fs::read(&dest).unwrap(), output);
        assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), modified);
        enqueue(ID).unwrap();
        assert!(pending.is_file());
        assert!(automatic_revision(&root, ID).unwrap().is_none());
        fs::remove_file(pending).unwrap();
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn recording_source_replay_after_finished_job_preserves_generation_and_publication() {
        use crate::core::{
            self,
            recording_source::{CompletedMeeting, Error, Session, Source},
        };
        if isolated_test() {
            return;
        }
        const ID: &str = "9944771122-887767";
        struct One;
        impl Source for One {
            fn poll(&mut self) -> std::result::Result<Vec<CompletedMeeting>, Error> {
                Ok(vec![CompletedMeeting::capture(&core::meetings_dir(), ID)?])
            }
        }
        let root = core::state_dir();
        let folder = core::meetings_dir().join(ID);
        fs::create_dir_all(&folder).unwrap();
        store_io::write(
            &folder.join("manifest.json"),
            &serde_json::json!({"status":"complete","duration_ms":30000}),
        )
        .unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            serde_json::json!({"text":vec!["synthetic";25].join(" ")}).to_string(),
        )
        .unwrap();
        let mut config = core::Config::load().unwrap();
        config.pipeline.profile = "experimental".into();
        config.features.indic_engine = true;
        config.labs.after_stop_language_routing = false;
        store_io::write(&root.join("config.json"), &config).unwrap();
        let dest = destination(&root, ID);
        write_transcript(
            &dest,
            &english_fallback(&[], &config.pipeline, "seed".into(), "synthetic"),
        )
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cursor = temp.path().join("source.json");
        let mut first = Session::open(One, cursor.clone()).unwrap();
        assert_eq!(
            first.drain(|m| {
                enqueue_automatic(m.id()).map_err(io::Error::other)?;
                run_pending().map_err(io::Error::other)?;
                Err(io::ErrorKind::Interrupted.into())
            }),
            Err(Error::Dispatch(io::ErrorKind::Interrupted))
        );
        drop(first);
        let pending = queue(&root).join(format!("{ID}.json"));
        assert!(!pending.exists());
        let completed = automatic_revision(&root, ID).unwrap().unwrap();
        let output = fs::read(&dest).unwrap();
        let modified = fs::metadata(&dest).unwrap().modified().unwrap();
        Session::open(One, cursor)
            .unwrap()
            .drain(|m| {
                enqueue_automatic(m.id()).map_err(io::Error::other)?;
                assert!(!pending.exists(), "replay created another generation");
                run_pending().map_err(io::Error::other)
            })
            .unwrap();
        assert_eq!(
            automatic_revision(&root, ID).unwrap().as_deref(),
            Some(completed.as_str())
        );
        assert_eq!(fs::read(&dest).unwrap(), output);
        assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), modified);
        // An explicit retry still creates a fresh request after automatic completion.
        enqueue(ID).unwrap();
        assert!(pending.is_file());
        fs::remove_file(pending).unwrap();
    }
    #[test]
    fn routing_enqueue_skips_short_and_unknown_and_deduplicates_kept() {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-routing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let meetings = root.join("meetings");
        assert!(enqueue_in(&root, &meetings, &"1".repeat(101)).is_err());
        let id = "1793946600000-1";
        let folder = meetings.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("manifest.json"),
            serde_json::json!({"status":"complete","duration_ms":29999}).to_string(),
        )
        .unwrap();
        enqueue_in(&root, &meetings, id).unwrap();
        assert!(!queue(&root).join(format!("{id}.json")).exists());
        fs::write(folder.join("transcript.ndjson"), r#"{"text":"abandoned"}"#).unwrap();
        enqueue_in(&root, &meetings, id).unwrap();
        assert!(!queue(&root).join(format!("{id}.json")).exists());
        super::super::super::recording_guard::keep_in(&root, id).unwrap();
        enqueue_in(&root, &meetings, id).unwrap();
        let path = queue(&root).join(format!("{id}.json"));
        let before = fs::read(&path).unwrap();
        enqueue_in(&root, &meetings, id).unwrap();
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    #[test]
    fn failure_timeout_instability_preserve_success_and_retry() {
        let root = tempfile::tempdir().unwrap();
        let dest = destination(root.path(), "123-4");
        let config = RoutingConfig::default();
        let mut good = english_fallback(&[], &config, "distinct-good".into(), "seed");
        good.warnings.clear();
        write_transcript(&dest, &good).unwrap();
        let original = fs::read(&dest).unwrap();
        let fallback = english_fallback(&[], &config, "new-input".into(), "forced failure");
        for stable in [true, false] {
            assert!(!publish_attempt(&dest, &fallback, false, stable).unwrap());
            assert_eq!(fs::read(&dest).unwrap(), original);
        }
        let pending = Pending {
            id: "123-4".into(),
            generation: 1,
        };
        let receipt = root.path().join("local/router-work/123-4/attempt.json");
        store_io::write(&receipt, &serde_json::json!({"generation":1,"retry_after_ms":chrono::Utc::now().timestamp_millis()+30000})).unwrap();
        assert!(!retry_ready(root.path(), &pending));
        store_io::write(
            &receipt,
            &serde_json::json!({"generation":1,"retry_after_ms":0}),
        )
        .unwrap();
        assert!(retry_ready(root.path(), &pending));
        assert!(!queue(root.path()).join("123-4.completed").exists());
        fs::remove_file(&dest).unwrap();
        assert!(!publish_attempt(&dest, &fallback, false, false).unwrap());
        assert!(!dest.exists());
        assert!(publish_attempt(&dest, &fallback, false, true).unwrap());
        assert!(!publish_attempt(&dest, &fallback, false, true).unwrap());
        assert!(publish_attempt(&dest, &good, true, true).is_ok());
        assert_eq!(fs::read(&dest).unwrap(), original);
    }
}

/// Production path for speech pack refusals and route pins: a real drain opens the request.
#[cfg(test)]
mod route_pin_tests {
    use super::*;
    use crate::core::model_catalog::{catalog, speech_packs::*};
    use crate::core::{Config, meeting_router::fallback::Reason, meetings_dir, state_dir};
    use serde_json::json;
    /// A finished 30 s meeting with 25 synthetic words, so it queues, under the given routing.
    fn meeting(id: &str, labs: bool, languages: &[&str], hi: &str, mode: &str) -> PathBuf {
        let (root, folder) = (state_dir(), meetings_dir().join(id));
        fs::create_dir_all(&folder).unwrap();
        let manifest = json!({"status":"complete", "created_at_ms":1793946600000_i64,
            "ended_at_ms":0, "duration_ms":30000});
        store_io::write(&folder.join("manifest.json"), &manifest).unwrap();
        let text = vec!["synthetic"; 25].join(" ");
        let line = json!({"start_ms":1000, "end_ms":2000, "source":"system", "text":text});
        fs::write(folder.join("transcript.ndjson"), line.to_string()).unwrap();
        let mut config = Config::default();
        config.pipeline.profile = "experimental".into();
        config.features.indic_engine = true;
        config.pipeline.language_mode = mode.into();
        config.labs.after_stop_language_routing = labs;
        let mut routes = LanguageRoutes::cpu_after_stop();
        routes.routes.get_mut("hi").unwrap().model = hi.into();
        config.pipeline.after_stop_languages = Some(routes);
        store_io::write(&root.join("config.json"), &config).unwrap();
        let languages: Vec<String> = languages.iter().map(|l| l.to_string()).collect();
        crate::core::model_catalog::save_meeting_languages(&root, &languages).unwrap();
        enqueue_in(&root, &meetings_dir(), id).unwrap();
        root
    }
    #[test]
    fn disabled_language_engine_leaves_pending_work_for_reenable() {
        if isolated_test() {
            return;
        }
        let id = "1793946600000-424243";
        let root = meeting(id, true, &["hi", "te"], "indicconformer-hi-ctc", "auto");
        let pending = queue(&root).join(format!("{id}.json"));
        let queued = fs::read(&pending).unwrap();
        let mut settings = Config::load_in(&root).unwrap();
        settings.features.indic_engine = false;
        settings.save(&root).unwrap();
        assert!(
            enqueue(id)
                .unwrap_err()
                .to_string()
                .contains("routing is off")
        );
        run_pending_with(|_, _| panic!("disabled routing launched a worker")).unwrap();
        assert_eq!(fs::read(pending).unwrap(), queued);
    }
    #[test]
    fn route_request_refuses_invalid_pack_hash_or_decoder_before_launch() {
        if isolated_test() {
            return;
        }
        let (id, all) = ("1793946600000-424242", ["en", "hi", "te"]);
        let (hi, swift) = ("indicconformer-hi-ctc", "whisper-hindi2hinglish-swift");
        let decoder = shipped_with(|d| d["packs"][1]["decoder"] = json!("whisper-seq2seq"));
        let te_hash = catalog_with_hash("indicconformer-te-ctc", 0, "");
        let (shipped, models) = (shipped_with(|_| {}), catalog());
        for (bytes, catalog, hi, mode, expected) in [
            (&decoder, models, hi, "auto", Some((code::DECODER, "hi"))),
            (&shipped, te_hash, hi, "auto", Some((code::HASH, "te"))),
            (&shipped, models, swift, "auto", Some((code::ROUTE, "hi"))),
            (&shipped, models, swift, "forced:en", None),
            (&shipped, models, hi, "auto", None),
        ] {
            use_test_registry(Registry::parse(bytes, catalog).unwrap());
            let root = meeting(id, true, &all, hi, mode);
            let mut launched = false;
            let drained = run_pending_with(|_, _| {
                launched = true;
                Err("launched".into())
            });
            let Some((refused, language)) = expected else {
                assert_eq!(drained.unwrap_err().to_string(), "launched");
                continue;
            };
            assert!(drained.is_ok() && !launched, "{refused}");
            let attempt = fs::read(root.join("local/router-work").join(id).join("attempt.json"));
            let attempt: serde_json::Value = serde_json::from_slice(&attempt.unwrap()).unwrap();
            let expected = json!({"code":refused, "subject":language});
            assert_eq!(attempt["refusal"], expected);
            let transcript = meetings_dir().join(id).join("transcript.ndjson");
            let baseline = || Ok(read_baseline(&fs::read(transcript)?).unwrap());
            let notes = notes_input(&root, id, None, baseline, 0, 0).unwrap();
            assert_eq!(notes.unwrap().2[0].reason, Reason::SpeechPackRefused);
        }
        // Without labs the worker never reads the after-stop routes, so a refused one never blocks it.
        meeting(id, false, &all, swift, "auto");
        let drained = run_pending_with(|_, _| Err("launched".into()));
        assert_eq!(drained.unwrap_err().to_string(), "launched");
        // An English-only meeting never opens the refused Hindi route, so it completes.
        let root = meeting(id, false, &["en"], swift, "auto");
        run_pending_with(|_, _| Err("launched".into())).unwrap();
        assert!(!queue(&root).join(format!("{id}.json")).exists());
        assert!(automatic_revision(&root, id).unwrap().is_some());
    }
    #[test]
    fn a_new_pack_revision_changes_the_job_and_refuses_the_old_pin() {
        if isolated_test() {
            return;
        }
        let (id, hi, all) = (
            "1793946600000-424243",
            "indicconformer-hi-ctc",
            ["en", "hi", "te"],
        );
        let root = meeting(id, true, &all, hi, "auto");
        let old = snapshot(&root, id).unwrap().unwrap();
        let request = root.join("pinned-request.json");
        store_io::write(&request, &old).unwrap();
        // The same model id with new Telugu tokens, as an app update could ship.
        let catalog = catalog_with_hash("indicconformer-te-ctc", 1, "a".repeat(64).leak());
        use_test_registry(Registry::parse(&shipped_with(|_| {}), catalog).unwrap());
        let new = snapshot(&root, id).unwrap().unwrap();
        assert_ne!(old.input_revision, new.input_revision);
        assert_eq!(old.routes["hi"], new.routes["hi"]);
        assert_ne!(old.routes["te"], new.routes["te"]);
        let refusal = run_one_in(&request, &root).unwrap_err().to_string();
        assert_eq!(refusal, "speech_pack_revision_changed: te");
        // Readings cached under the old pins are dropped before the worker starts.
        let batches = root.join("local/router-work").join(id).join("batches");
        fs::create_dir_all(&batches).unwrap();
        let new_pins = serde_json::to_vec(&new.routes).unwrap();
        fs::write(
            batches.join("routes.json"),
            serde_json::to_vec(&old.routes).unwrap(),
        )
        .unwrap();
        fs::write(batches.join("old.result.json"), "reading from the old pack").unwrap();
        let drained = run_pending_with(|job, _| {
            assert_eq!(job.routes, new.routes);
            assert!(!batches.join("old.result.json").exists());
            let pins = fs::read(batches.join("routes.json")).unwrap();
            assert_eq!(pins, new_pins);
            Err("launched".into())
        });
        assert_eq!(drained.unwrap_err().to_string(), "launched");
    }
}
