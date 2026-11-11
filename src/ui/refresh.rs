//! Focus and visible-window timer refresh. One worker, no filesystem watcher.
use super::{Waker, check};
use sidenote::core::{self, Meeting, Saved};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant, SystemTime},
};
const EVERY: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Options {
    pub check_date: Option<chrono::NaiveDate>,
    pub memory: bool,
    pub check: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            check_date: None,
            memory: true,
            check: true,
        }
    }
}
pub(super) struct Data {
    pub meetings: Vec<Meeting>,
    pub short_recordings: Option<Vec<core::recording_guard::ShortRecording>>,
    pub notes_jobs: std::collections::BTreeMap<String, super::notes_status::Job>,
    pub retained: Vec<PathBuf>,
    pub temporary: bool,
    pub errors: Vec<String>,
    pub pipeline: core::Pipeline,
    pub shared: Option<bool>,
    pub projects: Option<core::Projects>,
    pub companies: Option<core::companies::Companies>,
    pub overrides: Option<core::Overrides>,
    pub cutoff: Option<Option<chrono::NaiveDate>>,
    pub saved: HashMap<String, Saved>,
    pub memory: Option<core::memory::MemoryFile>,
    pub check: Option<check::Refresh>,
}
pub(crate) struct Result {
    pub(super) generation: u64,
    pub(super) options: Options,
    pub(super) data: Option<Box<Data>>,
}
fn loaded<T>(result: std::io::Result<T>) -> Option<T> {
    result
        .inspect_err(|e| {
            core::file_reads::error(&core::state_dir().join("state"), e);
        })
        .ok()
}
impl Data {
    pub(super) fn read(options: Options) -> Self {
        let scope = core::file_reads::Scope::begin();
        let device = core::DeviceSettings::load();
        let shared = device.as_ref().ok().and_then(|d| {
            core::file_reads::entries(&core::state_dir().join("state"))
                .ok()
                .map(|entries| {
                    entries.iter().any(|e| {
                        let name = e.file_name();
                        let name = name.to_string_lossy();
                        name != d.id
                            && name.len() == 32
                            && name.bytes().all(|c| c.is_ascii_hexdigit())
                            && core::file_reads::directory(&e.path())
                    })
                })
        });
        let device_error = device.is_err();
        let (meetings, mut errors) = core::scan();
        let short_recordings = loaded(core::recording_guard::short_recordings(
            &core::state_dir(),
            &core::meetings_dir(),
        ));
        let shared = shared.map(|seen| seen || meetings.iter().any(|m| m.device.is_some()));
        let notes_jobs = super::notes_status::read(&core::state_dir(), &meetings);
        let pipeline = core::load_pipeline(&meetings);
        let store = core::TaskStore::default();
        let saved_store = core::StateStore::default();
        let companies = loaded(core::companies::Companies::load(&core::state_dir()));
        let company_threads: Vec<_> = companies
            .iter()
            .flat_map(|f| f.catalog.records.keys())
            .map(|id| core::company_ask_key(id))
            .collect();
        let saved = meetings
            .iter()
            .map(|m| m.id.as_str())
            .chain(std::iter::once(core::ALL_KEY))
            .chain(company_threads.iter().map(String::as_str))
            .filter_map(|id| loaded(saved_store.load(id)).map(|s| (id.to_string(), s)))
            .collect();
        let projects = loaded(store.load_projects());
        let overrides = loaded(store.load_overrides());
        let cutoff = loaded(store.load_settings()).map(|s| s.cutoff());
        let memory = if options.memory {
            loaded(core::memory::MemoryStore::default().load())
        } else {
            None
        };
        let check = options
            .check
            .then(|| check::CheckState::read_refresh(options.check_date))
            .filter(|c| {
                if let Some(notice) = &c.notice {
                    errors.push(notice.clone());
                    false
                } else {
                    true
                }
            });

        let reads = scope.stats();
        if device_error {
            errors.push("Device settings could not be read. Refresh will retry.".into());
        }
        if reads.skipped > 0 {
            errors.push(format!(
                "Skipped {} unreadable or unsafe files. Previous data is kept where a read failed.",
                reads.skipped
            ));
        }
        Self {
            meetings,
            short_recordings,
            notes_jobs,
            retained: scope.failed(),
            temporary: reads.temporary,
            errors,
            pipeline,
            shared,
            projects,
            companies,
            overrides,
            cutoff,
            saved,
            memory,
            check,
        }
    }
}
pub(super) fn retain_catalog(current: &[Meeting], incoming: &mut Vec<Meeting>, failed: &[PathBuf]) {
    for old in current {
        if failed
            .iter()
            .any(|p| old.folder.starts_with(p) || p.starts_with(&old.folder))
            && !incoming.iter().any(|m| m.folder == old.folder)
        {
            incoming.push(old.clone());
        }
    }
    incoming.sort_by(|a, b| b.started.cmp(&a.started).then(b.id.cmp(&a.id)));
}
#[derive(PartialEq, Eq)]
pub(super) struct Metadata {
    files: Vec<(PathBuf, u64, Option<SystemTime>)>,
    pub skipped: usize,
    temporary: bool,
}
fn walk(path: &Path, out: &mut Metadata, depth: usize) {
    use std::os::windows::fs::MetadataExt;
    if depth > 8 {
        out.skipped += 1;
        return;
    }
    if !core::file_reads::directory(path) {
        if path.exists() {
            out.skipped += 1;
        }
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            out.temporary |= matches!(e.raw_os_error(), Some(32 | 33));
            out.skipped += 1;
            return;
        }
    };
    for (n, e) in entries.enumerate() {
        if n >= core::file_reads::ENTRIES_MAX {
            out.skipped += 1;
            break;
        }
        let e = match e {
            Ok(e) => e,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        let path = e.path();
        let m = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                out.skipped += 1;
                continue;
            }
        };
        if path.as_os_str().len() > 1024 || m.file_attributes() & 0x400 != 0 {
            out.skipped += 1;
            continue;
        }
        if m.is_dir() {
            if path.extension().is_some_and(|e| e == "json") {
                out.skipped += 1;
                continue;
            }
            walk(&path, out, depth + 1);
        } else if core::file_reads::regular(&m)
            && path
                .extension()
                .is_some_and(|e| matches!(e.to_str(), Some("json" | "md" | "ndjson")))
        {
            out.files.push((path, m.len(), m.modified().ok()));
        } else if !m.is_file() {
            out.skipped += 1;
        }
    }
}
pub(super) fn metadata() -> Metadata {
    let mut out = Metadata {
        files: Vec::new(),
        skipped: 0,
        temporary: false,
    };
    let root = core::state_dir();
    for folder in [
        root.join("state"),
        root.join("memory"),
        root.join("checks"),
        root.join("local/notes-jobs"),
        root.join("local/draft-summaries"),
        root.join("local/recording-guard"),
        core::meetings_dir(),
        core::peers_dir(),
    ] {
        walk(&folder, &mut out, 0);
    }
    if let Ok(entries) = core::file_reads::entries(&root) {
        for e in entries {
            if e.path().extension().is_some_and(|e| e == "json") {
                if let Ok(m) = core::file_reads::checked_metadata(&e.path()) {
                    out.files.push((e.path(), m.len(), m.modified().ok()));
                } else {
                    out.skipped += 1;
                }
            }
        }
    }
    out.files.sort();
    out
}
#[derive(Clone)]
pub(super) struct Handle {
    tx: Arc<SyncSender<()>>,
    options: Arc<Mutex<Options>>,
    slot: Arc<Mutex<Option<Result>>>,
    dirty: Arc<AtomicBool>,
}
impl Handle {
    pub fn request(&self, options: Options) {
        let mut current = self.options.lock().unwrap();
        if *current != options {
            *current = options;
            self.dirty.store(true, Ordering::Release);
        }
        drop(current);
        let _ = self.tx.try_send(());
    }
    pub fn invalidate(&self) {
        self.dirty.store(true, Ordering::Release);
    }
    pub fn take(&self) -> Option<Result> {
        self.slot.lock().unwrap().take()
    }
}
fn visible() -> bool {
    super::hex::window_shown()
}
pub(super) fn updated_at(
    previous: Option<Instant>,
    shared: Option<bool>,
    changed: bool,
) -> Option<Instant> {
    if shared == Some(false) {
        None
    } else if changed && shared == Some(true) {
        Some(Instant::now())
    } else {
        previous
    }
}
fn refresh_needed(force: bool, changed: bool, retry: &Retry) -> bool {
    force || changed || retry.ready()
}
#[derive(Default)]
struct Retry {
    attempts: usize,
    due: Option<Instant>,
}
impl Retry {
    fn ready(&self) -> bool {
        self.due.is_some_and(|d| Instant::now() >= d)
    }
    fn complete(&mut self, temporary: bool) {
        if temporary {
            self.failed();
        } else {
            self.clear();
        }
    }
    fn clear(&mut self) {
        *self = Self::default();
    }
    fn failed(&mut self) {
        let delays = [10, 30, 60, 300];
        self.due = Some(Instant::now() + Duration::from_secs(delays[self.attempts.min(3)]));
        self.attempts = self.attempts.saturating_add(1);
    }
}
static PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn read_caught(options: Options) -> (Data, bool) {
    read_caught_with(|| Data::read(options))
}
fn read_caught_with(read: impl FnOnce() -> Data + std::panic::UnwindSafe) -> (Data, bool) {
    match std::panic::catch_unwind(read) {
        Ok(data) => (data, false),
        Err(_) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            (
                Data {
                    short_recordings: None,
                    meetings: vec![],
                    notes_jobs: Default::default(),
                    retained: vec![core::meetings_dir(), core::peers_dir()],
                    temporary: false,
                    errors: vec!["Refresh hit a problem and will retry".into()],
                    pipeline: HashMap::new(),
                    shared: None,
                    projects: None,
                    companies: None,
                    overrides: None,
                    cutoff: None,
                    saved: HashMap::new(),
                    memory: None,
                    check: None,
                },
                true,
            )
        }
    }
}
pub(super) fn start(waker: Waker, initial: Options) -> Handle {
    let (tx, rx) = mpsc::sync_channel(1);
    let tx = Arc::new(tx);
    let options = Arc::new(Mutex::new(initial));
    let worker_options = options.clone();
    let slot = Arc::new(Mutex::new(None));
    let worker_slot = slot.clone();
    let dirty = Arc::new(AtomicBool::new(true));
    let worker_dirty = dirty.clone();
    std::thread::spawn(move || {
        let mut previous = None;
        let mut retry = Retry::default();
        let mut next = Instant::now();
        loop {
            if let Err(mpsc::RecvTimeoutError::Disconnected) =
                rx.recv_timeout(next.saturating_duration_since(Instant::now()))
            {
                break;
            }
            next = Instant::now() + EVERY;
            if !visible() {
                continue;
            }
            let generation = core::mutation_generation();
            let options = *worker_options.lock().unwrap();
            let changed = metadata();
            let force = worker_dirty.swap(false, Ordering::AcqRel);
            let data = if refresh_needed(force, previous.as_ref() != Some(&changed), &retry) {
                let (data, panicked) = read_caught(options);
                retry.complete(panicked || changed.temporary || data.temporary);
                Some(Box::new(data))
            } else {
                None
            };
            previous = Some(changed);
            let result = Result {
                generation,
                options,
                data,
            };
            let mut slot = worker_slot.lock().unwrap();
            // An unchanged poll must not replace changed data the UI has yet to take.
            let old = if result.data.is_none() && slot.is_some() {
                None
            } else {
                slot.replace(result)
            };
            drop(slot);
            drop(old);
            waker();
        }
    });
    Handle {
        tx,
        options,
        slot,
        dirty,
    }
}

#[cfg(test)]
#[path = "final_refresh_proofs.rs"]
mod final_refresh_proofs;
