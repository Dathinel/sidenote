//! OS arrivals share the focus/fallback worker and a one-slot trigger queue.
//! notify 7.0.0 is CC0-1.0 and reuses locked Windows bindings; default features are off.
use super::{Waker, check};
use notify::{RecursiveMode, Watcher};
use sidenote::core::{self, Meeting, Saved};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
const DEBOUNCE: Duration = Duration::from_millis(200);
const FALLBACK: Duration = Duration::from_secs(60);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
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
#[derive(Clone)]
pub(super) struct Stamp {
    pub sequence: u64,
    pub generation: u64,
    pub revisions: core::StoreRevisions,
    pub valid: bool,
}
#[derive(Clone)]
pub(super) struct Handle {
    tx: Arc<SyncSender<()>>,
    slot: Arc<Mutex<Option<Box<Snapshot>>>>,
    options: Arc<Mutex<Options>>,
}
impl Handle {
    pub fn send(&self, _signal: ()) -> Result<(), mpsc::SendError<()>> {
        match self.tx.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(())) => Err(mpsc::SendError(())),
        }
    }

    pub fn request(&self, options: Options) {
        *self.options.lock().unwrap() = options;
        let _ = self.tx.try_send(());
    }
    pub fn take(&self) -> Option<Box<Snapshot>> {
        self.slot.lock().unwrap().take()
    }
}
pub struct Snapshot {
    pub(super) stamp: Stamp,
    pub(super) options: Options,
    pub(super) check: Option<check::Refresh>,
    pub(super) worker: std::thread::ThreadId,
    pub meetings: Vec<Meeting>,
    pub errors: Vec<String>,
    pub pipeline: core::Pipeline,
    pub projects: Option<core::Projects>,
    pub overrides: Option<core::Overrides>,
    pub cutoff: Option<Option<chrono::NaiveDate>>,
    pub saved: HashMap<String, Saved>,
    pub memory: Option<core::memory::MemoryFile>,
    pub remote_revision: String,
}
impl Snapshot {
    pub(super) fn read() -> Self {
        Self::read_with(Options::default())
    }
    pub(super) fn read_with(options: Options) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = core::mutation_generation();
        let before = core::store_revisions(&core::state_dir());
        let (meetings, errors) = core::scan();
        let pipeline = core::load_pipeline(&meetings);
        let store = core::TaskStore::default();
        let saved_store = core::StateStore::default();
        let saved = meetings
            .iter()
            .map(|m| m.id.as_str())
            .chain(std::iter::once(core::ALL_KEY))
            .filter_map(|id| saved_store.load(id).ok().map(|s| (id.to_string(), s)))
            .collect();
        let remote_revision = remote_revision(&meetings);
        let projects = store.load_projects().ok();
        let overrides = store.load_overrides().ok();
        let cutoff = store.load_settings().ok().map(|s| s.cutoff());
        let memory = if options.memory {
            core::memory::MemoryStore::default().load().ok()
        } else {
            None
        };
        let check = options
            .check
            .then(|| check::CheckState::read_refresh(options.check_date));
        let after = core::store_revisions(&store.root);
        let valid = before.is_ok()
            && after.is_ok()
            && before.as_ref().ok() == after.as_ref().ok()
            && generation == core::mutation_generation();
        Self {
            meetings,
            errors,
            pipeline,
            projects,
            overrides,
            cutoff,
            saved,
            memory,
            remote_revision,
            options,
            check,
            worker: std::thread::current().id(),
            stamp: Stamp {
                sequence,
                generation,
                revisions: after.unwrap_or_default(),
                valid,
            },
        }
    }
}
pub(super) fn remote_revision(meetings: &[Meeting]) -> String {
    let peers: Vec<_> = meetings
        .iter()
        .filter(|m| m.device.is_some())
        .map(|m| (&m.id, &m.summary_raw, &m.transcript_raw, &m.notes))
        .collect();
    format!(
        "{}:{}",
        core::incoming_revision(&core::state_dir()).unwrap_or_default(),
        serde_json::to_string(&peers).unwrap_or_default()
    )
}
fn temporary(path: &Path) -> bool {
    path.components().any(|c| {
        let n = c.as_os_str().to_string_lossy();
        (n.starts_with(".syncthing.") && n.ends_with(".tmp")) || n.starts_with("~syncthing~")
    })
}
fn relevant(path: &Path, roots: &[PathBuf], own: &Path) -> bool {
    if temporary(path) || path.starts_with(own) {
        return false;
    }
    roots
        .iter()
        .any(|r| path.starts_with(r) || r.starts_with(path))
}
fn log_once(logged: &std::sync::atomic::AtomicBool) {
    if !logged.swap(true, std::sync::atomic::Ordering::Relaxed) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(core::log_path())
        {
            let _ = writeln!(
                f,
                "Arrival watcher failed; focus and 60-second refresh remain active"
            );
        }
    }
}
pub(super) fn start(waker: Waker, initial: Options) -> Handle {
    let (tx, rx) = mpsc::sync_channel(1);
    let tx = Arc::new(tx);
    let weak: Weak<SyncSender<()>> = Arc::downgrade(&tx);
    let slot = Arc::new(Mutex::new(None));
    let worker_slot = slot.clone();
    let options = Arc::new(Mutex::new(initial));
    let worker_options = options.clone();
    std::thread::spawn(move || {
        let data = core::state_dir();
        let roots = vec![data.join("state"), core::peers_dir(), core::meetings_dir()];
        let own = data.join("state").join(
            core::DeviceSettings::load()
                .map(|d| d.id)
                .unwrap_or_default(),
        );
        let logged = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_logged = logged.clone();
        let callback_roots = roots.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
                Ok(e)
                    if !matches!(e.kind, notify::EventKind::Access(_))
                        && (e.need_rescan()
                            || e.paths.iter().any(|p| relevant(p, &callback_roots, &own))) =>
                {
                    if let Some(tx) = weak.upgrade() {
                        let _ = tx.try_send(());
                    }
                }
                Err(_) => {
                    log_once(&callback_logged);
                    if let Some(tx) = weak.upgrade() {
                        let _ = tx.try_send(());
                    }
                }
                _ => {}
            })
            .ok();
        if let Some(w) = &mut watcher {
            let mut watched = std::collections::BTreeSet::new();
            for root in &roots {
                let parent = root.parent().unwrap_or(root).to_path_buf();
                let mut parent = parent.as_path();
                while !parent.is_dir() {
                    let Some(p) = parent.parent() else {
                        break;
                    };
                    parent = p;
                }
                if watched.insert(parent.to_path_buf())
                    && w.watch(parent, RecursiveMode::Recursive).is_err()
                {
                    log_once(&logged);
                }
            }
        } else {
            log_once(&logged);
        }
        let options = *worker_options.lock().unwrap();
        let first = Snapshot::read_with(options);
        *worker_slot.lock().unwrap() = Some(Box::new(first));
        waker();
        let mut next_fallback = Instant::now() + FALLBACK;
        loop {
            match rx.recv_timeout(next_fallback.saturating_duration_since(Instant::now())) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(()) => {
                    // Fixed batch window bounds sustained bursts as well as quiet single writes.
                    let deadline = Instant::now() + DEBOUNCE;
                    while Instant::now() < deadline {
                        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Ok(()) => {}
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= next_fallback {
                next_fallback = Instant::now() + FALLBACK;
            }
            let options = *worker_options.lock().unwrap();
            let snapshot = Snapshot::read_with(options);
            *worker_slot.lock().unwrap() = Some(Box::new(snapshot));
            waker();
        }
    });
    Handle { tx, slot, options }
}
