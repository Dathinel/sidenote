use super::{
    Engine, Helper, JobState, NotesSettings, Request,
    helper::{self, Failure, Run},
    prompt, schema, transcript,
};
use crate::core::{self, agent_instructions::Agent, file_reads, store_io};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Job {
    #[serde(flatten)]
    pub state: JobState,
    pub attempts: u32,
    pub next_try_at: Option<i64>,
    pub helper: Option<Helper>,
    pub model: String,
    pub instructions: String,
    pub updated_at: i64,
    pub message: String,
    pub request: Option<Request>,
}
impl Job {
    fn new(request: Option<Request>) -> Self {
        Self {
            state: JobState::Queued,
            attempts: 0,
            next_try_at: None,
            helper: None,
            model: String::new(),
            instructions: "default".into(),
            updated_at: now(),
            message: "queued".into(),
            request,
        }
    }
    fn ready(&self, time: i64) -> bool {
        matches!(self.state, JobState::Queued | JobState::Running)
            || matches!(self.state, JobState::BackendError {next_try_ms: Some(at), ..} if at <= time)
    }
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn jobs(root: &Path) -> PathBuf {
    root.join("local/notes-jobs")
}
fn job_path(root: &Path, id: &str) -> PathBuf {
    jobs(root).join(format!("{id}.json"))
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
pub(super) fn read(root: &Path, id: &str) -> io::Result<Option<Job>> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    match store_io::read_bounded(&job_path(root, id), file_reads::JOURNAL_MAX) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::from),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn write(root: &Path, id: &str, job: &mut Job, state: JobState, message: &str) -> io::Result<()> {
    job.state = state;
    job.updated_at = now();
    job.message = message.into();
    job.next_try_at = match job.state {
        JobState::BackendError { next_try_ms, .. } => next_try_ms,
        _ => None,
    };
    store_io::write(&job_path(root, id), job)?;
    let _ = log(root, id, message);
    Ok(())
}
fn log(root: &Path, id: &str, message: &str) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("sidenote.log"))?;
    writeln!(file, "{} notes: {id} {message}", now())
}
fn manifest(folder: &Path) -> io::Result<serde_json::Value> {
    let bytes = store_io::read_bounded(&folder.join("manifest.json"), file_reads::JOURNAL_MAX)?;
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
        .map_err(io::Error::from)
}
fn complete(folder: &Path) -> bool {
    manifest(folder).is_ok_and(|m| m["status"] == "complete")
}

pub(super) fn enqueue_in(
    root: &Path,
    meetings: &Path,
    id: &str,
    mode: Option<Request>,
    settings: &NotesSettings,
) -> io::Result<()> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    if settings.engine() != Engine::App {
        if mode.is_some() {
            return Err(store_io::invalid(
                "Turn on built-in Meeting summary in Settings first",
            ));
        }
        return Ok(());
    }
    let folder = meetings.join(id);
    store_io::reject_reparse(&folder)?;
    if !complete(&folder) {
        return Err(store_io::invalid("Meeting is not complete"));
    }
    let sorting = matches!(mode, Some(Request::SortAgain | Request::RedoTasks));
    if folder.join("english-summary.json").exists() && !sorting {
        return Ok(());
    }
    if sorting && !folder.join("english-summary.json").is_file() {
        return Err(store_io::invalid("Make notes before sorting tasks"));
    }
    if let Some(job) = read(root, id)?
        && (mode.is_none() || matches!(job.state, JobState::Running | JobState::Queued))
    {
        return Ok(());
    }
    let _lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;
    // A different enqueue may have won while we acquired the writer lock.
    if let Some(job) = read(root, id)?
        && (mode.is_none() || matches!(job.state, JobState::Running | JobState::Queued))
    {
        return Ok(());
    }
    store_io::write(&job_path(root, id), &Job::new(mode))
}

fn failure(root: &Path, id: &str, job: &mut Job, error: Failure) -> io::Result<()> {
    let (state, message) = match error {
        Failure::Consent => (JobState::NoHelper, "notes permission changed"),
        Failure::Missing => (JobState::HelperMissing, "helper missing"),
        Failure::TimedOut => (JobState::TimedOut, "helper timed out"),
        Failure::BadOutput => (JobState::BadOutput, "invalid helper output"),
        Failure::Backend { exit } => {
            let delay = match job.attempts {
                1 => Some(15 * 60_000),
                2 => Some(60 * 60_000),
                _ => None,
            };
            (
                JobState::BackendError {
                    next_try_ms: delay.map(|d| now() + d),
                    exit,
                },
                "helper backend error",
            )
        }
    };
    write(root, id, job, state, message)
}
fn text(path: &Path, cap: u64) -> io::Result<String> {
    match store_io::read_bounded(path, cap) {
        Ok(bytes) => {
            String::from_utf8(bytes).map_err(|_| store_io::invalid("Invalid text encoding"))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}
fn teams(root: &Path) -> io::Result<Vec<schema::Team>> {
    let path = root.join("teams.json");
    match store_io::read_bounded(&path, file_reads::JOURNAL_MAX) {
        Ok(bytes) => {
            let v: serde_json::Value = serde_json::from_slice(&bytes)?;
            serde_json::from_value(v.get("teams").cloned().unwrap_or(v)).map_err(io::Error::from)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}
fn projects(root: &Path) -> io::Result<Vec<String>> {
    Ok(core::TaskStore::new(root.to_owned())
        .load_projects()?
        .projects)
}
fn consent(settings: &NotesSettings, manual: bool, end: Option<i64>) -> Option<JobState> {
    if settings.engine() != Engine::App
        || settings.effective_helper().is_none()
        || settings.enabled_at_ms.is_none()
    {
        return Some(JobState::NoHelper);
    }
    if !manual
        && (!settings.auto
            || end
                .zip(settings.enabled_at_ms)
                .is_none_or(|(end, enabled)| end <= enabled))
    {
        return Some(JobState::NeedsClick);
    }
    None
}

fn process(
    root: &Path,
    meetings: &Path,
    id: &str,
    job: &mut Job,
    settings: &NotesSettings,
    run: &mut impl Run,
) -> io::Result<()> {
    let folder = meetings.join(id);
    store_io::reject_reparse(&folder)?;
    let m = manifest(&folder)?;
    if m["status"] != "complete" {
        return write(
            root,
            id,
            job,
            JobState::NeedsClick,
            "meeting is not complete",
        );
    }
    let sort_only = matches!(job.request, Some(Request::SortAgain | Request::RedoTasks));
    if folder.join("english-summary.json").exists() && !sort_only {
        return write(
            root,
            id,
            job,
            JobState::Done {
                fallback: false,
                tasks_failed: !folder.join("tasks.json").exists(),
            },
            "existing notes preserved",
        );
    }
    let started = m["created_at_ms"]
        .as_i64()
        .or_else(|| id.split('-').next()?.parse().ok());
    let duration = m["duration_ms"]
        .as_f64()
        .filter(|d| d.is_finite() && *d >= 0.0);
    let end = m["ended_at_ms"].as_i64().or_else(|| {
        started
            .zip(duration)
            .map(|(s, d)| s.saturating_add(d as i64))
    });
    if let Some(state) = consent(settings, job.request.is_some(), end) {
        return write(
            root,
            id,
            job,
            state,
            "notes need setup or an explicit click",
        );
    }
    let mut transcript = if job.request == Some(Request::RedoTasks) {
        let existing = text(&folder.join("english-transcript.md"), file_reads::STORE_MAX)?;
        if existing.trim().is_empty() {
            transcript::read(&folder)?
        } else {
            existing
        }
    } else {
        transcript::write(&folder)?
    };
    if transcript.is_empty() {
        return write(root, id, job, JobState::Empty, "empty transcript");
    }
    let minutes = duration
        .map(|d| (d / 60_000.0).ceil() as u64)
        .unwrap_or_else(|| {
            core::parse_transcript(&transcript)
                .iter()
                .filter_map(|l| l.seconds)
                .max()
                .unwrap_or(0)
                / 60
        });
    if transcript.chars().count() > prompt::TRANSCRIPT_CAP {
        if job.request == Some(Request::FirstPart) {
            transcript = transcript
                .lines()
                .filter(|line| {
                    core::parse_transcript(line)
                        .first()
                        .and_then(|l| l.seconds)
                        .is_some_and(|s| s < 4 * 60 * 60)
                })
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(prompt::TRANSCRIPT_CAP)
                .collect();
        } else {
            return write(
                root,
                id,
                job,
                JobState::TooLong { minutes },
                "transcript exceeds size limit",
            );
        }
    }
    let typed = text(&folder.join("notes.md"), file_reads::JOURNAL_MAX)?;
    let glossary = run.glossary();
    if !prompt::bounded_inputs(&typed, &glossary) {
        return write(
            root,
            id,
            job,
            JobState::ContextTooLong {
                typed_notes: typed.chars().count() > prompt::TYPED_CAP,
                glossary: glossary.iter().map(|s| s.chars().count()).sum::<usize>()
                    > prompt::GLOSSARY_CAP,
            },
            "meeting context exceeds size limit",
        );
    }
    let helper = settings
        .effective_helper()
        .ok_or_else(|| store_io::invalid("No helper selected"))?;
    if !run.installed(helper) {
        return failure(root, id, job, Failure::Missing);
    }
    let model = if settings.model.trim().is_empty() {
        helper.default_model()
    } else {
        &settings.model
    };
    job.helper = Some(helper);
    job.model = model.into();
    job.attempts = job.attempts.saturating_add(1);
    write(root, id, job, JobState::Running, "running")?;
    let teams = teams(root)?;
    let projects = projects(root)?;
    let date = started
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
        .ok_or_else(|| store_io::invalid("Missing meeting date"))?;
    if job.request == Some(Request::RedoTasks) {
        return redo_tasks(
            job,
            run,
            Redo {
                root,
                folder: &folder,
                id,
                settings,
                transcript: &transcript,
                typed: &typed,
                date,
                teams: &teams,
                projects: &projects,
            },
        );
    }
    let (summary, fallback) = if sort_only {
        let raw = text(
            &folder.join("english-summary.json"),
            file_reads::SUMMARY_MAX,
        )?;
        (existing_summary(&raw)?, false)
    } else {
        if !folder.join("quick-title.json").exists() {
            let title_model = if helper == Helper::Claude {
                "claude-haiku-5-5"
            } else {
                model
            };
            match run
                .run(
                    helper,
                    title_model,
                    prompt::quick(&transcript),
                    helper::TITLE_TIMEOUT,
                )
                .ok()
                .and_then(|s| prompt::clean_quick(&s))
            {
                Some(title) => {
                    let _ = store_io::publish(
                        &folder.join("quick-title.json"),
                        &serde_json::to_vec(&serde_json::json!({"title":title}))?,
                    );
                }
                None => {
                    let _ = log(root, id, "quick title failed");
                }
            }
        }
        let title = core::read_quick_title(&folder)
            .or_else(|| m["title"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "not provided".into());
        let material = prompt::material(
            prompt::Details {
                title: &title,
                started_ms: started.unwrap_or(0),
                minutes,
                your_name: &settings.your_name,
            },
            &typed,
            &transcript,
            &teams,
            &glossary,
        );
        let call = prompt::Call {
            root,
            helper,
            model,
            timeout: helper::NOTES_TIMEOUT,
        };
        let answer = match call.ladder(run, Agent::Notes, &material, |raw| {
            schema::parse(raw, id, date, &transcript, &teams)
        }) {
            Ok(answer) => answer,
            Err(error) => return failure(root, id, job, error),
        };
        job.instructions = if answer.fallback {
            "default-fallback"
        } else if answer.custom {
            "custom"
        } else {
            "default"
        }
        .into();
        let mut summary = answer.value;
        summary.made_by = Some(prompt::provenance(
            helper,
            model,
            answer.custom,
            answer.fallback,
        ));
        let bytes = serde_json::to_vec_pretty(&summary)?;
        // An external scripts process may still finish while the helper is running.
        match store_io::publish(&folder.join("english-summary.json"), &bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return write(
                    root,
                    id,
                    job,
                    JobState::Done {
                        fallback: false,
                        tasks_failed: true,
                    },
                    "existing notes preserved",
                );
            }
            Err(e) => return Err(e),
        }
        (summary, answer.fallback)
    };
    // Preserve the exact summary on a sorting retry; only tasks.json is replaced.
    let material = prompt::task_material(&summary, &projects)?;
    let call = prompt::Call {
        root,
        helper,
        model,
        timeout: helper::TASKS_TIMEOUT,
    };
    let answer = call.ladder(run, Agent::Tasks, &material, |raw| {
        schema::sorting(raw, &summary, &projects, &settings.your_name)
    });
    let tasks_failed = match answer {
        Ok(answer) => {
            if answer.fallback {
                let _ = log(root, id, "tasks used default fallback");
            }
            store_io::write(&folder.join("tasks.json"), &answer.value).is_err()
        }
        Err(_) => {
            let _ = log(root, id, "task sorting failed");
            true
        }
    };
    write(
        root,
        id,
        job,
        JobState::Done {
            fallback,
            tasks_failed,
        },
        if tasks_failed {
            "done; task sorting failed"
        } else {
            "done"
        },
    )
}

/// All I/O is rooted explicitly, so tests never need the real app directories.
#[cfg(test)]
pub(super) fn drain(
    root: &Path,
    meetings: &Path,
    run: &mut impl Run,
    load: impl Fn() -> io::Result<NotesSettings>,
) -> io::Result<()> {
    let _worker = match store_io::lock(&jobs(root).join("worker.lock")) {
        Ok(lock) => lock,
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Ok(()),
        Err(e) => return Err(e),
    };
    drain_locked(root, meetings, run, load)
}
fn drain_locked(
    root: &Path,
    meetings: &Path,
    run: &mut impl Run,
    load: impl Fn() -> io::Result<NotesSettings>,
) -> io::Result<()> {
    loop {
        let settings = load()?;
        if settings.engine() != Engine::App {
            return Ok(());
        }
        let mut ready = Vec::new();
        let mut missing = Vec::new();
        for entry in fs::read_dir(jobs(root))? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !valid_id(id) {
                continue;
            }
            match read(root, id) {
                Ok(Some(job)) if job.ready(now()) => ready.push(id.to_owned()),
                Ok(Some(job)) if job.state == JobState::HelperMissing => {
                    missing.push((id.to_owned(), job))
                }
                Err(_) => {
                    let target = path.with_extension(format!(
                        "json.{}.quarantined",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                    ));
                    let message = if fs::rename(&path, target).is_ok() {
                        "unreadable job quarantined"
                    } else {
                        "unreadable job skipped; quarantine could not be written"
                    };
                    let _ = log(root, id, message);
                }
                _ => {}
            }
        }
        if !missing.is_empty()
            && !ready.is_empty()
            && settings
                .effective_helper()
                .is_some_and(|h| run.installed(h))
        {
            for (id, mut job) in missing {
                write(
                    root,
                    &id,
                    &mut job,
                    JobState::Queued,
                    "rechecking helper for new meeting",
                )?;
                ready.push(id);
            }
        }
        ready.sort_by_key(|id| {
            (
                id.split('-')
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0),
                id.clone(),
            )
        });
        let Some(id) = ready.into_iter().next() else {
            return Ok(());
        };
        let _job_lock = store_io::lock(&jobs(root).join(format!("{id}.lock")))?;
        let Some(mut job) = read(root, &id)? else {
            continue;
        };
        if !job.ready(now()) {
            continue;
        }
        if process(root, meetings, &id, &mut job, &settings, run).is_err() {
            // Filesystem and parse errors are intentionally reduced to a fixed message.
            write(
                root,
                &id,
                &mut job,
                JobState::BackendError {
                    next_try_ms: None,
                    exit: None,
                },
                "local notes data could not be read or written",
            )?;
        }
    }
}

pub fn enqueue(id: &str) -> io::Result<()> {
    let settings = super::settings()?;
    enqueue_in(
        &core::state_dir(),
        &core::meetings_dir(),
        id,
        None,
        &settings,
    )
}
pub fn catch_up() -> io::Result<()> {
    let settings = super::settings()?;
    if settings.engine() != Engine::App {
        return Ok(());
    }
    for entry in fs::read_dir(core::meetings_dir())? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if valid_id(&id)
            && complete(&entry.path())
            && enqueue_in(
                &core::state_dir(),
                &core::meetings_dir(),
                &id,
                None,
                &settings,
            )
            .is_err()
        {
            let _ = log(&core::state_dir(), &id, "could not enqueue notes");
        }
    }
    start_worker()
}
pub fn start_worker() -> io::Result<()> {
    if super::settings()?.engine() != Engine::App {
        return Ok(());
    }
    if !fs::read_dir(jobs(&core::state_dir())).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|s| s == "json"))
    }) {
        return Ok(());
    }
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--notes-pending")
        .arg("--data-dir")
        .arg(core::state_dir())
        .arg("--meetings-dir")
        .arg(core::meetings_dir())
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
pub fn run_pending() -> io::Result<()> {
    let root = core::state_dir();
    let meetings = core::meetings_dir();
    let _worker = match store_io::lock(&jobs(&root).join("worker.lock")) {
        Ok(lock) => lock,
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Ok(()),
        Err(e) => return Err(e),
    };
    drain_locked(
        &root,
        &meetings,
        &mut helper::Cli {
            root: Some(root.clone()),
        },
        super::settings,
    )
}

fn existing_summary(raw: &str) -> io::Result<schema::Summary> {
    let mut value: serde_json::Value = serde_json::from_str(raw)?;
    for todo in value["todo"].as_array_mut().into_iter().flatten() {
        if todo.get("title").is_none() {
            todo["title"] = todo["task"].clone();
        }
    }
    // Older notes omitted empty sections.
    for key in ["context", "topics", "todo", "to_settle"] {
        if value.get(key).is_none() {
            value[key] = serde_json::json!([]);
        }
    }
    serde_json::from_value(value).map_err(io::Error::from)
}
struct Redo<'a> {
    root: &'a Path,
    folder: &'a Path,
    id: &'a str,
    settings: &'a NotesSettings,
    transcript: &'a str,
    typed: &'a str,
    date: chrono::NaiveDate,
    teams: &'a [schema::Team],
    projects: &'a [String],
}
fn redo_tasks(job: &mut Job, run: &mut impl Run, redo: Redo<'_>) -> io::Result<()> {
    let Redo {
        root,
        folder,
        id,
        settings,
        transcript,
        typed,
        date,
        teams,
        projects,
    } = redo;
    let path = folder.join("english-summary.json");
    let raw = text(&path, file_reads::SUMMARY_MAX)?;
    let mut source: serde_json::Value = serde_json::from_str(&raw)?;
    let old: core::Summary = serde_json::from_value(source.clone())?;
    let material = format!(
        "Extract the tasks from this meeting. Only redo the tasks.\nTeams: {}\nTranscript:\n{}\nExisting notes:\n{}\nUser notes:\n{}",
        serde_json::to_string(teams)?,
        transcript,
        raw,
        typed
    );
    let helper = settings
        .effective_helper()
        .ok_or_else(|| store_io::invalid("No helper selected"))?;
    let model = if settings.model.trim().is_empty() {
        helper.default_model()
    } else {
        &settings.model
    };
    let call = prompt::Call {
        root,
        helper,
        model,
        timeout: helper::TASKS_TIMEOUT,
    };
    let answer = match call.ladder_output(run,Agent::Tasks,&material,|raw| {
        let tasks: serde_json::Value = schema::object(raw)?;
        let todo = tasks.get("todo").filter(|v| v.is_array()).ok_or("missing tasks list")?;
        let value = serde_json::json!({"title":"Tasks", "context":[], "topics":[], "to_settle":[], "todo":todo});
        schema::parse(&value.to_string(),id,date,transcript,teams)
    },Some(include_str!("tasks_redo_tail.txt"))) {
        Ok(answer) => answer,
        Err(e) => return failure(root,id,job,e),
    };
    source["todo"] = serde_json::to_value(&answer.value.todo)?;
    // Do not overwrite notes that changed while the helper was running.
    if text(&path, file_reads::SUMMARY_MAX)? != raw {
        return Err(store_io::invalid("Notes changed while tasks were running"));
    }
    let new: core::Summary = serde_json::from_value(source.clone())?;
    core::TaskStore::new(root.to_owned()).preserve_redo(id, &old.todo, &new.todo)?;
    store_io::write(&path, &source)?;
    let sorted = call.ladder(
        run,
        Agent::Tasks,
        &prompt::task_material(&answer.value, projects)?,
        |raw| schema::sorting(raw, &answer.value, projects, &settings.your_name),
    );
    let tasks_failed = match sorted {
        Ok(sorted) => store_io::write(&folder.join("tasks.json"), &sorted.value).is_err(),
        Err(_) => true,
    };
    write(
        root,
        id,
        job,
        JobState::Done {
            fallback: answer.fallback,
            tasks_failed,
        },
        "tasks redone",
    )
}
