//! One long-lived replica owner, with content-free health and continuous ingestion.
use crate::core::{self, relay_preview as relay, store_io};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

mod commands;
mod inventory;
mod pages;
use commands::Pair;
const MAX_REQUEST: usize = 64 * 1024;
const MAX_REPLY: usize = 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Ingested {
    #[serde(default)]
    pub revision: String,
    pub device: Option<String>,
    pub id: String,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub conflict: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub generation: u64,
    pub source_revision: String,
    pub meetings: Vec<Ingested>,
    pub tasks: usize,
    pub errors: usize,
    pub relay_enabled: bool,
    pub relay_status: String,
    pub peers: usize,
    pub identity: String,
}
static HEALTH: OnceLock<Mutex<Snapshot>> = OnceLock::new();
pub fn snapshot() -> Snapshot {
    HEALTH
        .get_or_init(Mutex::default)
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}
fn invalid() -> io::Error {
    io::Error::other("SYNC_CONTROL_INVALID_REQUEST")
}

struct Controller {
    root: PathBuf,
    pair: Option<Pair>,
    state: Snapshot,
}
impl Controller {
    fn new(root: &Path) -> io::Result<Self> {
        let state = store_io::read(&root.join("local/sync/index.json"))?;
        let mut result = Self {
            root: root.into(),
            pair: None,
            state,
        };
        result.tick()?;
        Ok(result)
    }
    fn tick(&mut self) -> io::Result<()> {
        self.poll_pair();
        let settings = relay::Settings::load(&self.root)?;
        let mut errors = usize::from(!core::reconcile_service());
        let (meetings, warnings) = inventory::observe(&self.state.meetings);
        errors += warnings;
        let preview = relay::get(&self.root)?;
        let (tasks, relay_status, identity) = if let Some(preview) = preview {
            (
                preview.tasks()?,
                preview.session_status()?,
                preview.public_fingerprint(),
            )
        } else {
            (Vec::new(), "disabled".into(), self.state.identity.clone())
        };
        let revision = blake3::hash(&serde_json::to_vec(&(
            meetings
                .iter()
                .map(|m| (&m.device, &m.id, &m.revision))
                .collect::<Vec<_>>(),
            &tasks,
            relay::revision(&self.root)?,
            core::service_journal_revision(&self.root)?,
        ))?)
        .to_hex()
        .to_string();
        if revision != self.state.source_revision {
            self.state.generation = self.state.generation.saturating_add(1);
        }
        self.state.source_revision = revision;
        self.state.meetings = meetings;
        self.state.tasks = tasks.len();
        self.state.errors = errors;
        self.state.relay_enabled = settings.enabled;
        self.state.relay_status = relay_status;
        self.state.peers = settings.peers.len();
        self.state.identity = identity;
        let path = self.root.join("local/sync/index.json");
        let prior: Snapshot = store_io::read(&path)?;
        if serde_json::to_vec(&prior)? != serde_json::to_vec(&self.state)? {
            store_io::write(&path, &self.state)?;
        }
        if let Ok(mut health) = HEALTH.get_or_init(Mutex::default).lock() {
            *health = self.state.clone();
        }
        Ok(())
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.pair.take();
        let _ = relay::disconnect(&self.root);
    }
}

pub struct Owner {
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
    controller: Arc<Mutex<Controller>>,
    _lock: std::fs::File,
}
impl Owner {
    pub fn start(root: &Path, lock: std::fs::File) -> io::Result<Self> {
        let controller = Arc::new(Mutex::new(Controller::new(root)?));
        let stop = Arc::new(AtomicBool::new(false));
        let mut owner = Self {
            _lock: lock,
            stop: stop.clone(),
            threads: Vec::new(),
            controller: controller.clone(),
        };
        owner
            .threads
            .push(
                thread::Builder::new()
                    .name("sync-ingestion".into())
                    .spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            if let Ok(mut control) = controller.lock()
                                && control.tick().is_err()
                            {
                                control.state.errors = control.state.errors.saturating_add(1);
                                if let Ok(mut health) = HEALTH.get_or_init(Mutex::default).lock() {
                                    *health = control.state.clone();
                                }
                            }
                            for _ in 0..10 {
                                if stop.load(Ordering::Acquire) {
                                    return;
                                }
                                thread::sleep(Duration::from_millis(100));
                            }
                        }
                    })?,
            );
        Ok(owner)
    }
    pub fn request(&self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        if bytes.len() > MAX_REQUEST {
            return Err(invalid());
        }
        commands::respond(&self.controller, bytes)
    }
    pub fn healthy(&self) -> bool {
        self.threads.iter().all(|t| !t.is_finished())
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn meeting_revision(meeting: &core::Meeting) -> String {
    let mut hash = blake3::Hasher::new();
    for text in [
        &meeting.title,
        &meeting.summary_raw,
        &meeting.notes,
        &meeting.transcript_raw,
        &meeting.routed_revision,
    ] {
        hash.update(&(text.len() as u64).to_be_bytes());
        hash.update(text.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}
