//! Durable per-meeting requests. The lock serializes drains, never enqueues.
use super::*;
use crate::platform::CommandExt;
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
struct Job {
    id: String,
    folder: PathBuf,
    baseline: Vec<Chunk>,
    config: RoutingConfig,
    languages: Vec<String>,
    input_revision: String,
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

fn valid_id(id: &str) -> bool {
    id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn queue(root: &Path) -> PathBuf {
    root.join("local/router-queue")
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
pub(crate) fn has_result(id: &str) -> bool {
    valid_id(id) && destination(&super::super::state_dir(), id).is_file()
}
pub fn configured() -> bool {
    fs::read(super::super::state_dir().join("config.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v["pipeline"]["profile"] == "experimental")
}
pub fn enqueue(id: &str) -> Result<()> {
    enqueue_in(
        &super::super::state_dir(),
        &super::super::meetings_dir(),
        id,
    )
}
fn enqueue_in(root: &Path, meetings: &Path, id: &str) -> Result<()> {
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
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
    if queue(root).join(format!("{id}.json")).exists() {
        return Ok(());
    }
    store_io::write(
        &queue(root).join(format!("{id}.json")),
        &Pending {
            id: id.into(),
            generation: SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        },
    )?;
    Ok(())
}
fn load_config(root: &Path) -> Result<RoutingConfig> {
    let mut config = super::super::Config::load_in(root)?.pipeline;
    config.meeting_languages = super::super::model_catalog::load_meeting_languages(root)?;
    config.validate()?;
    Ok(config)
}
fn snapshot(root: &Path, id: &str) -> Result<Job> {
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
    let folder = super::super::meetings_dir().join(id);
    let input = super::inputs::Snapshot::capture(&folder)?;
    if super::super::recording_guard::eligibility_captured(
        root,
        id,
        &input.manifest,
        &input.transcript,
    )? != super::super::recording_guard::Eligibility::Eligible
    {
        return Err("Meeting is not eligible".into());
    }
    let mut baseline = Vec::new();
    for (i, line) in std::str::from_utf8(&input.transcript)?.lines().enumerate() {
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
        });
    }
    let config = load_config(root)?;
    let windows = input.windows.clone();
    let revision = cache::key(
        &[],
        &configuration_key(&config)?,
        &input.revision()?,
        "pinned-input-v1",
    );
    Ok(Job {
        id: id.into(),
        folder,
        baseline,
        languages: config.meeting_languages.clone(),
        config,
        input_revision: revision,
        windows,
        input: Some(input),
    })
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
fn supervise(mut command: Command, seconds: f64, active: Option<&Path>) -> Outcome {
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
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break Outcome {
                    success: status.success(),
                    exit_code: status.code(),
                    ..Outcome::default()
                };
            }
            Ok(None) if start.elapsed().as_secs_f64() < seconds => {
                thread::sleep(Duration::from_millis(50))
            }
            value => {
                drop(job); // Kill the complete tree before waiting, including normalization/Hex descendants.
                let _ = child.kill();
                let status = child.wait().ok();
                if let Some(path) = active {
                    let _ = fs::remove_file(path);
                }
                return Outcome {
                    exit_code: status.and_then(|s| s.code()),
                    timed_out: value.is_ok(),
                    error: Some("process deadline or wait failure".into()),
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
    let mut job: Job = serde_json::from_slice(&fs::read(&request)?)?;
    job.config.meeting_languages = job.languages;
    let work = request.parent().ok_or("Missing work directory")?;
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
        let outcome = supervise(command, deadline, None);
        store_io::write(&work.join("normalization-outcome.json"), &outcome)?;
        if !outcome.success {
            return Err("Audio normalization failed or timed out".into());
        }
        Audio::open(&[(track.into(), output.clone())])?;
        paths.push((track.into(), output));
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
    let _update_activity = crate::updates::activity()?;
    let root = super::super::state_dir();
    let args: Vec<_> = std::env::args().collect();
    let automatic = args.iter().any(|arg| arg == "--automatic");
    if let Some(id) = args
        .windows(2)
        .find(|a| a[0] == "--meeting-id")
        .map(|a| a[1].as_str())
    {
        enqueue(id)?;
    }
    if !configured() {
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
            if !attempted.insert((pending.id.clone(), pending.generation)) {
                continue;
            }
            if automatic && completed_attempt(&root, &pending) {
                continue;
            }
            found = true;
            let mut job = match snapshot(&root, &pending.id) {
                Ok(job) => job,
                Err(_) => {
                    store_io::write(
                        &root
                            .join("local/router-work")
                            .join(&pending.id)
                            .join("attempt.json"),
                        &serde_json::json!({"setup_failed":true,"pending_retained":true}),
                    )?;
                    continue;
                }
            };
            let destination = destination(&root, &job.id);
            if job.config.meeting_languages == ["en"]
                && !matches!(
                    job.config.language_mode.as_str(),
                    "forced:hi" | "forced:mixed"
                )
            {
                if destination.exists() {
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
                acknowledge(&root, &job.id, &path, &bytes)?;
                continue;
            }
            if let Some(old) = fs::read(&destination)
                .ok()
                .and_then(|b| serde_json::from_slice::<Transcript>(&b).ok())
                && old.input_checksum == job.input_revision
                && !old.degraded()
            {
                acknowledge(&root, &job.id, &path, &bytes)?;
                continue;
            }
            let work = root.join("local/router-work").join(&job.id);
            fs::create_dir_all(&work)?;
            cleanup_work(&work);
            let staged = job
                .input
                .as_mut()
                .ok_or("Missing pinned inputs")?
                .stage(&work)?;
            job.folder = staged.path().to_owned();
            let request = work.join("request.json");
            store_io::write(&request, &job)?;
            let _ = fs::remove_file(work.join("result.json"));
            let _ = fs::remove_file(work.join("normalization-outcome.json"));
            let _ = fs::remove_file(work.join("failure.json"));
            let mut command = Command::new(std::env::current_exe()?);
            command
                .arg("--route-one")
                .arg("--router-request")
                .arg(&request)
                .arg("--data-dir")
                .arg(&root)
                .arg("--meetings-dir")
                .arg(super::super::meetings_dir());
            for flag in ["--ffmpeg", "--normalization-timeout-seconds"] {
                if let Some(value) = args.windows(2).find(|a| a[0] == flag).map(|a| &a[1]) {
                    command.args([flag, value]);
                }
            }
            let duration = job.baseline.iter().map(|c| c.end).fold(0.0, f64::max);
            let timeout = seconds_arg("--router-timeout-seconds")
                .unwrap_or((duration * 4.0 + 120.0).clamp(120.0, 86400.0));
            let mut outcome = supervise(command, timeout, Some(&work.join("active.json")));
            if let Some(error) = fs::read(work.join("failure.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["error"].as_str().map(str::to_owned))
            {
                outcome.error = Some(error);
            }
            match snapshot(&root, &job.id) {
                Ok(latest) if latest.input_revision != job.input_revision => {
                    job = latest;
                    outcome.success = false;
                    outcome.error = Some("input changed during routing".into());
                }
                Err(_) => {
                    outcome.success = false;
                    outcome.error = Some("input unavailable after routing".into());
                }
                _ => {}
            }
            let result = if outcome.success {
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
            let result = result.unwrap_or_else(|| {
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
            write_transcript(&destination, &result)?;
            let normalization = fs::read(work.join("normalization-outcome.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
            store_io::write(
                &work.join("attempt.json"),
                &serde_json::json!({"generation":pending.generation,"child":outcome,"published_fallback":result.degraded(),"segments":result.segments.len(),"normalization":normalization}),
            )?;
            cleanup_work(&work);
            if outcome
                .error
                .as_deref()
                .is_some_and(|e| e.contains("FATAL:"))
            {
                return Err("FATAL: Hex selection check failed; English fallback published".into());
            }
            if !result.degraded() {
                acknowledge(&root, &job.id, &path, &bytes)?;
            }
        }
        if !found {
            break;
        }
    }
    Ok(())
}

fn completed_attempt(root: &Path, pending: &Pending) -> bool {
    let receipt = root
        .join("local/router-work")
        .join(&pending.id)
        .join("attempt.json");
    fs::read(receipt)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v["generation"].as_u64().map(u128::from) == Some(pending.generation))
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
                snapshot(&super::super::state_dir(), &pending.id).is_ok()
                    && !completed_attempt(&super::super::state_dir(), &pending)
            })
        })
    {
        return Ok(());
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
            });
            start = cut;
        }
    }
    (!chunks.is_empty()).then_some(chunks)
}

#[cfg(test)]
mod recording_guard_tests {
    use super::*;
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
