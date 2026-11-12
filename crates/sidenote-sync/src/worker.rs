//! One long-lived helper per peer; each helper has a private 512 MiB Windows Job budget.
//! No peer-controlled Automerge decode/receive/generate runs on the caller's stack.
use crate::preflight::{self, MAX_BULK, MAX_DOC};
use crate::{
    status::{Failure, Reason},
    store::Snapshot,
};
use anyhow::{Result, ensure};
use automerge::ChangeHash;
use automerge::{
    ActorId, Automerge, ReadDoc,
    sync::{self, SyncDoc},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
#[cfg(windows)]
use std::{path::Path, sync::Arc};
use std::{path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MEMORY_LIMIT: usize = 512 * 1024 * 1024;
pub const MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);
fn load_timeout(bytes: usize) -> Duration {
    (MESSAGE_TIMEOUT + Duration::from_secs(bytes.div_ceil(64 * 1024) as u64))
        .min(Duration::from_secs(120))
}
const IPC_LIMIT: usize = 100 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct Request {
    pub domain: String,
    pub actor: [u8; 32],
    pub snapshot: Snapshot,
    #[serde(with = "serde_bytes")]
    pub incoming: Vec<u8>,
    pub ack_previous: bool,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Response {
    pub creator: Option<[u8; 32]>,
    pub delta: Vec<Vec<u8>>,
    pub heads: Vec<ChangeHash>,
    pub bytes: usize,
    pub compact: Option<Vec<u8>>,
    pub orphans: usize,
    #[serde(with = "serde_bytes")]
    pub outgoing: Vec<u8>,
    pub caught_up: bool,
    pub received: bool,
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Accepted(Response),
    Rejected(Reason),
}

pub struct Pool {
    executable: PathBuf,
    #[cfg(windows)]
    slots: Arc<tokio::sync::Semaphore>,
}
impl Pool {
    pub fn adjacent() -> Result<Self> {
        let path = std::env::current_exe().map_err(|_| Failure(Reason::WorkerUnavailable))?;
        let folder = path.parent().ok_or(Failure(Reason::WorkerUnavailable))?;
        let name = if cfg!(windows) {
            "sidenote-sync-worker.exe"
        } else {
            "sidenote-sync-worker"
        };
        let candidate = if folder
            .file_name()
            .is_some_and(|n| n == "deps" || n == "examples")
        {
            folder
                .parent()
                .ok_or(Failure(Reason::WorkerUnavailable))?
                .join(name)
        } else {
            folder.join(name)
        };
        Self::new(candidate)
    }
    pub fn new(executable: PathBuf) -> Result<Self> {
        ensure!(
            executable.is_absolute() && executable.is_file(),
            Failure(Reason::WorkerUnavailable)
        );
        #[cfg(windows)]
        {
            Ok(Self {
                executable,
                slots: Arc::new(tokio::sync::Semaphore::new(4)),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = executable;
            Err(Failure(Reason::WorkerUnavailable).into())
        }
    }
    pub(crate) fn spawn(&self) -> Result<Worker> {
        #[cfg(windows)]
        {
            let slot = self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| Failure(Reason::WorkerUnavailable))?;
            platform::spawn(&self.executable, Arc::new(platform::Job::new()?), slot)
        }
        #[cfg(not(windows))]
        {
            let _ = &self.executable;
            Err(Failure(Reason::WorkerUnavailable).into())
        }
    }
}

pub(crate) struct Worker {
    input: tokio::fs::File,
    output: tokio::fs::File,
    #[cfg(windows)]
    process: std::os::windows::io::OwnedHandle,
    #[cfg(windows)]
    _job: Arc<platform::Job>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
impl Worker {
    pub async fn request(&mut self, request: &Request) -> Result<Response> {
        let bytes = postcard::to_allocvec(request).map_err(|_| Failure(Reason::Protocol))?;
        ensure!(bytes.len() <= IPC_LIMIT, Failure(Reason::DocumentTooLarge));
        let exchange = async {
            self.input.write_u32(bytes.len() as u32).await?;
            self.input.write_all(&bytes).await?;
            self.input.flush().await?;
            let len = self.output.read_u32().await? as usize;
            ensure!(len <= IPC_LIMIT, "worker reply cap");
            let mut reply = vec![0; len];
            self.output.read_exact(&mut reply).await?;
            Ok::<_, anyhow::Error>(postcard::from_bytes::<Reply>(&reply)?)
        };
        let load_bytes = request.snapshot.chunks.iter().map(Vec::len).sum::<usize>();
        let deadline = load_timeout(load_bytes);
        match tokio::time::timeout(deadline, exchange).await {
            Ok(Ok(Reply::Accepted(reply))) => Ok(reply),
            Ok(Ok(Reply::Rejected(reason))) => Err(Failure(reason).into()),
            Ok(Err(_)) => {
                self.terminate();
                Err(Failure(Reason::WorkerCrashed).into())
            }
            Err(_) => {
                self.terminate();
                Err(Failure(Reason::WorkerTimedOut).into())
            }
        }
    }
    fn terminate(&self) {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(
                    self.process.as_raw_handle(),
                    1,
                );
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct Hot {
    domain: String,
    doc: Automerge,
    hashes: BTreeSet<[u8; 32]>,
    durable: BTreeSet<[u8; 32]>,
    bytes: usize,
}
#[derive(Default)]
struct Engine {
    hot: Option<Hot>,
    states: BTreeMap<String, sync::State>,
    pending: Vec<[u8; 32]>,
}
impl Engine {
    fn process(&mut self, request: Request) -> Result<Response> {
        if request.ack_previous
            && let Some(hot) = self.hot.as_mut()
        {
            hot.durable.extend(self.pending.drain(..));
        }
        self.pending.clear();
        ensure!(crate::store::valid_domain(&request.domain), "domain");
        if self.hot.as_ref().is_none_or(|h| h.domain != request.domain) {
            self.hot = Some(Hot {
                domain: request.domain.clone(),
                doc: Automerge::new(),
                hashes: BTreeSet::new(),
                durable: BTreeSet::new(),
                bytes: 0,
            });
        }
        let hot = self.hot.as_mut().ok_or(Failure(Reason::Protocol))?;
        if request.snapshot.reset {
            // A newer compact baseline must merge with an uncommitted receive during a retry.
            hot.hashes.clear();
            hot.durable.clear();
            hot.bytes = 0;
        }
        hot.doc.set_actor(ActorId::from(request.actor.as_slice()));
        for bytes in request.snapshot.chunks {
            preflight::document(&bytes).map_err(|_| Failure(Reason::CorruptDocument))?;
            for chunk in preflight::split_chunks(&bytes)? {
                hot.durable.insert(crate::identity::hash(chunk));
            }
            if hot.hashes.insert(crate::identity::hash(&bytes)) {
                hot.bytes = hot
                    .bytes
                    .checked_add(bytes.len())
                    .ok_or(Failure(Reason::DocumentTooLarge))?;
                ensure!(hot.bytes <= MAX_DOC, Failure(Reason::DocumentTooLarge));
                hot.doc
                    .load_incremental(&bytes)
                    .map_err(|_| Failure(Reason::CorruptDocument))?;
            }
        }
        let before = request.snapshot.heads;
        let state = self.states.entry(request.domain).or_default();
        let received = !request.incoming.is_empty();
        if received {
            preflight::sync(&request.incoming)?;
            let message = sync::Message::decode(&request.incoming)?;
            hot.doc.receive_sync_message(state, message)?;
        }
        let mut delta: Vec<Vec<u8>> = preflight::split_chunks(&hot.doc.save_after(&before))?
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect();
        // save_after excludes the orphan queue. Persist its normalized change chunks too.
        let mut orphans = 0usize;
        if !hot.doc.get_missing_deps(&[]).is_empty() {
            let saved = hot.doc.save_nocompress();
            ensure!(saved.len() <= MAX_DOC, Failure(Reason::DocumentTooLarge));
            for bytes in preflight::split_chunks(&saved)?.into_iter().skip(1) {
                let change = automerge::Change::from_bytes(bytes.to_vec())?;
                orphans += change.raw_bytes().len();
                delta.push(change.raw_bytes().to_vec());
            }
        }
        for bytes in &delta {
            if hot.hashes.insert(crate::identity::hash(bytes)) {
                hot.bytes = hot
                    .bytes
                    .checked_add(bytes.len())
                    .ok_or(Failure(Reason::DocumentTooLarge))?;
            }
        }
        delta.retain(|bytes| !hot.durable.contains(&crate::identity::hash(bytes)));
        self.pending = delta
            .iter()
            .map(|bytes| crate::identity::hash(bytes))
            .collect();
        hot.bytes = hot.doc.save_nocompress().len();
        ensure!(hot.bytes <= MAX_DOC, Failure(Reason::DocumentTooLarge));
        let compact = Some(hot.doc.save());
        let heads = hot.doc.get_heads();
        let creator = hot
            .doc
            .get_changes(&[])
            .into_iter()
            .filter(|change| change.deps().is_empty())
            .min_by_key(|change| change.hash())
            .and_then(|change| change.actor_id().to_bytes().try_into().ok());
        let caught_up = state.their_heads.as_ref().is_some_and(|h| h == &heads);
        let received = received || !delta.is_empty() || before != heads;
        // Incoming state cannot advance the sender's in-flight cursor until SQLite commits.
        let outgoing = if received {
            Vec::new()
        } else {
            hot.doc
                .generate_sync_message(state)
                .map(|m| m.encode())
                .unwrap_or_default()
        };
        ensure!(
            outgoing.len() <= MAX_BULK,
            Failure(Reason::DocumentTooLarge)
        );
        Ok(Response {
            creator,
            delta,
            heads,
            bytes: hot.bytes,
            compact,
            orphans,
            outgoing,
            caught_up,
            received,
        })
    }
}

/// Entry point of the bundled helper; stdout is binary IPC and stderr is discarded.
pub fn suppress_crash_dialogs() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX, SetErrorMode,
        };
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }
}
pub fn run_stdio() -> Result<()> {
    if !cfg!(windows) {
        anyhow::bail!("UNAVAILABLE_ON_PLATFORM");
    }
    suppress_crash_dialogs();
    #[cfg(windows)]
    platform::require_job()?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut engine = Engine::default();
    loop {
        let mut length = [0; 4];
        if let Err(error) = input.read_exact(&mut length) {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(());
            }
            return Err(error.into());
        }
        let len = u32::from_be_bytes(length) as usize;
        ensure!(len <= IPC_LIMIT, "request cap");
        let mut bytes = vec![0; len];
        input.read_exact(&mut bytes)?;
        let result = postcard::from_bytes::<Request>(&bytes)
            .map_err(anyhow::Error::from)
            .and_then(|r| engine.process(r));
        let reply = match result {
            Ok(response) => Reply::Accepted(response),
            Err(error) => Reply::Rejected(
                error
                    .downcast_ref::<Failure>()
                    .map(|f| f.0)
                    .unwrap_or(Reason::PeerBadData),
            ),
        };
        let bytes = postcard::to_allocvec(&reply)?;
        ensure!(bytes.len() <= IPC_LIMIT, "reply cap");
        output.write_all(&(bytes.len() as u32).to_be_bytes())?;
        output.write_all(&bytes)?;
        output.flush()?;
        if matches!(reply, Reply::Rejected(_)) {
            return Ok(());
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::OsStr,
        fs::File,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation},
        Security::SECURITY_ATTRIBUTES,
        System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
    };
    pub struct Job(OwnedHandle);
    const UI_LIMITS: u32 = JOB_OBJECT_UILIMIT_DESKTOP
        | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
        | JOB_OBJECT_UILIMIT_EXITWINDOWS
        | JOB_OBJECT_UILIMIT_GLOBALATOMS
        | JOB_OBJECT_UILIMIT_HANDLES
        | JOB_OBJECT_UILIMIT_READCLIPBOARD
        | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
        | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
    impl Job {
        pub fn new() -> Result<Self> {
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            ensure!(!handle.is_null(), Failure(Reason::WorkerUnavailable));
            let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
            limits.ProcessMemoryLimit = MEMORY_LIMIT;
            limits.JobMemoryLimit = MEMORY_LIMIT;
            ensure!(
                unsafe {
                    SetInformationJobObject(
                        job.0.as_raw_handle(),
                        JobObjectExtendedLimitInformation,
                        (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                        std::mem::size_of_val(&limits) as u32,
                    )
                } != 0,
                Failure(Reason::WorkerUnavailable)
            );
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: UI_LIMITS,
            };
            ensure!(
                unsafe {
                    SetInformationJobObject(
                        job.0.as_raw_handle(),
                        JobObjectBasicUIRestrictions,
                        (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                        std::mem::size_of_val(&ui) as u32,
                    )
                } != 0,
                Failure(Reason::WorkerUnavailable)
            );
            Ok(job)
        }
    }
    fn pipe() -> Result<(OwnedHandle, OwnedHandle)> {
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut read: HANDLE = ptr::null_mut();
        let mut write: HANDLE = ptr::null_mut();
        ensure!(
            unsafe { CreatePipe(&mut read, &mut write, &security, 0) } != 0,
            Failure(Reason::WorkerUnavailable)
        );
        Ok(unsafe {
            (
                OwnedHandle::from_raw_handle(read),
                OwnedHandle::from_raw_handle(write),
            )
        })
    }
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    struct Attributes(Vec<usize>);
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe {
                DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast());
            }
        }
    }
    pub fn spawn(
        path: &Path,
        job: Arc<Job>,
        slot: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Worker> {
        let (child_in, parent_in) = pipe()?;
        let (parent_out, child_out) = pipe()?;
        let null = File::options().write(true).open("NUL")?;
        unsafe {
            ensure!(
                SetHandleInformation(parent_in.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) != 0,
                Failure(Reason::WorkerUnavailable)
            );
            ensure!(
                SetHandleInformation(parent_out.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) != 0,
                Failure(Reason::WorkerUnavailable)
            );
            ensure!(
                SetHandleInformation(
                    null.as_raw_handle(),
                    HANDLE_FLAG_INHERIT,
                    HANDLE_FLAG_INHERIT
                ) != 0,
                Failure(Reason::WorkerUnavailable)
            );
        }
        let handles = [
            child_in.as_raw_handle(),
            child_out.as_raw_handle(),
            null.as_raw_handle(),
        ];
        let mut size = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
        }
        ensure!(size > 0, Failure(Reason::WorkerUnavailable));
        let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        ensure!(
            unsafe {
                InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut size)
            } != 0,
            Failure(Reason::WorkerUnavailable)
        );
        let mut attributes = Attributes(storage);
        let list = attributes.0.as_mut_ptr().cast();
        ensure!(
            unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    std::mem::size_of_val(&handles),
                    ptr::null_mut(),
                    ptr::null(),
                )
            } != 0,
            Failure(Reason::WorkerUnavailable)
        );
        let startup = STARTUPINFOEXW {
            StartupInfo: STARTUPINFOW {
                cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdInput: child_in.as_raw_handle(),
                hStdOutput: child_out.as_raw_handle(),
                hStdError: null.as_raw_handle(),
                ..Default::default()
            },
            lpAttributeList: list,
        };
        let mut process = PROCESS_INFORMATION::default();
        let app = wide(path.as_os_str());
        let mut command = wide(OsStr::new(&format!("\"{}\" --stdio", path.display())));
        // Do not copy the parent's credentials/environment into a parser process.
        let system = std::env::var_os("SystemRoot").ok_or(Failure(Reason::WorkerUnavailable))?;
        let mut environment = wide(OsStr::new(&format!(
            "SystemRoot={}",
            system.to_string_lossy()
        )));
        environment.push(0);
        ensure!(
            unsafe {
                CreateProcessW(
                    app.as_ptr(),
                    command.as_mut_ptr(),
                    ptr::null(),
                    ptr::null(),
                    1,
                    CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                    environment.as_ptr().cast(),
                    ptr::null(),
                    &startup.StartupInfo,
                    &mut process,
                )
            } != 0,
            Failure(Reason::WorkerUnavailable)
        );
        let handle = unsafe { OwnedHandle::from_raw_handle(process.hProcess) };
        let assigned =
            unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), handle.as_raw_handle()) };
        if assigned == 0 {
            unsafe {
                TerminateProcess(handle.as_raw_handle(), 1);
                CloseHandle(process.hThread);
            }
            return Err(Failure(Reason::WorkerCrashed).into());
        }
        let resumed = unsafe { ResumeThread(process.hThread) };
        unsafe {
            CloseHandle(process.hThread);
        }
        if resumed == u32::MAX {
            unsafe {
                TerminateProcess(handle.as_raw_handle(), 1);
            }
            return Err(Failure(Reason::WorkerUnavailable).into());
        }
        drop(child_in);
        drop(child_out);
        drop(null);
        Ok(Worker {
            input: tokio::fs::File::from_std(File::from(parent_in)),
            output: tokio::fs::File::from_std(File::from(parent_out)),
            process: handle,
            _job: job,
            _slot: slot,
        })
    }
    pub fn require_job() -> Result<()> {
        let mut ui = JOBOBJECT_BASIC_UI_RESTRICTIONS::default();
        ensure!(
            unsafe {
                QueryInformationJobObject(
                    ptr::null_mut(),
                    JobObjectBasicUIRestrictions,
                    (&mut ui as *mut JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                    std::mem::size_of_val(&ui) as u32,
                    ptr::null_mut(),
                )
            } != 0
                && ui.UIRestrictionsClass & UI_LIMITS == UI_LIMITS,
            Failure(Reason::WorkerUnavailable)
        );
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        ensure!(
            unsafe {
                QueryInformationJobObject(
                    ptr::null_mut(),
                    JobObjectExtendedLimitInformation,
                    (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                    ptr::null_mut(),
                )
            } != 0,
            Failure(Reason::WorkerUnavailable)
        );
        ensure!(
            limits.JobMemoryLimit > 0
                && limits.JobMemoryLimit <= MEMORY_LIMIT
                && limits.ProcessMemoryLimit > 0
                && limits.ProcessMemoryLimit <= MEMORY_LIMIT,
            Failure(Reason::WorkerUnavailable)
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model,
        store::{Commit, Store},
    };
    fn fixture() -> Result<Worker> {
        let executable = std::env::current_exe()?
            .parent()
            .ok_or(Failure(Reason::WorkerUnavailable))?
            .parent()
            .ok_or(Failure(Reason::WorkerUnavailable))?
            .join("examples/worker_fault.exe");
        Pool::new(executable)?.spawn()
    }
    fn empty() -> Request {
        Request {
            domain: "tasks".into(),
            actor: [1; 32],
            snapshot: Snapshot {
                version: 0,
                chunks: Vec::new(),
                reset: true,
                heads: Vec::new(),
            },
            incoming: Vec::new(),
            ack_previous: false,
        }
    }
    #[test]
    fn creator_is_root_change_author_instead_of_worker_or_relay_actor() -> Result<()> {
        let author = [2; 32];
        let mut source = Automerge::new().with_actor(ActorId::from(author.as_slice()));
        model::create(&mut source, "one", "synthetic")?;
        let mut engine = Engine::default();
        let reply = engine.process(Request {
            incoming: message(source.get_changes(&[])[0].raw_bytes()),
            ..empty()
        })?;
        assert_eq!(reply.creator, Some(author));
        assert_ne!(reply.creator, Some(empty().actor));
        let reply = Engine::default().process(Request {
            snapshot: Snapshot {
                version: 1,
                chunks: vec![source.save()],
                reset: true,
                heads: source.get_heads(),
            },
            ..empty()
        })?;
        assert_eq!(reply.creator, Some(author));
        Ok(())
    }
    #[test]
    fn f1_compaction_during_uncommitted_receive_retains_peer_changes() -> Result<()> {
        let mut source = Automerge::new();
        model::create(&mut source, "peer", "uncommitted")?;
        let change = source.get_changes(&[])[0].raw_bytes().to_vec();
        let peer_heads = source.get_heads();
        let mut engine = Engine::default();
        let first = engine.process(Request {
            incoming: message(&change),
            ..empty()
        })?;
        assert_eq!(first.heads, peer_heads);
        let dir = tempfile::tempdir()?;
        let mut store = Store::open(&dir.path().join("concurrent.sqlite"), b"destination")?;
        let mut rng = 0x9e3779b9u32;
        let value: String = (0..2 * 1024 * 1024)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                (b'!' + (rng % 90) as u8) as char
            })
            .collect();
        store.edit("tasks", |d| {
            model::create(d, "local", "concurrent")?;
            model::put(d, "local", "large", &value)
        })?;
        let snapshot = store.snapshot("tasks", Some(0))?;
        assert!(snapshot.reset);
        assert_eq!(snapshot.chunks.len(), 1);
        let version = snapshot.version;
        let retry = engine.process(Request {
            snapshot,
            ..empty()
        })?;
        assert!(peer_heads.iter().all(|h| retry.heads.contains(h)));
        store.append(
            "tasks",
            version,
            Commit {
                creator: retry.creator,
                chunks: retry.delta,
                heads: retry.heads,
                bytes: retry.bytes,
                compact: retry.compact,
                orphans: retry.orphans,
            },
            b"peer",
        )?;
        let loaded = store.load("tasks")?;
        assert!(
            peer_heads
                .iter()
                .all(|h| loaded.get_change_by_hash(h).is_some())
        );
        Ok(())
    }
    #[tokio::test]
    async fn f5_operation_dense_near_cap_document_cold_loads_in_one_chunk() -> Result<()> {
        use automerge::{ROOT, transaction::Transactable};
        let dir = tempfile::tempdir()?;
        let mut store = Store::open(&dir.path().join("dense.sqlite"), b"dense")?;
        let mut rng = 0x9e3779b9u32;
        store.edit("tasks", |doc| {
            let mut tx = doc.transaction();
            for i in 0..105_000 {
                let value: String = (0..256)
                    .map(|_| {
                        rng ^= rng << 13;
                        rng ^= rng >> 17;
                        rng ^= rng << 5;
                        (b'!' + (rng % 90) as u8) as char
                    })
                    .collect();
                tx.put(ROOT, format!("k{i}"), value)?;
            }
            tx.commit();
            Ok(())
        })?;
        let snapshot = store.snapshot("tasks", None)?;
        assert_eq!(snapshot.chunks.len(), 1);
        let bytes = store.load("tasks")?.save_nocompress().len();
        assert!((MAX_DOC * 8 / 10..MAX_DOC).contains(&bytes));
        let deadline = load_timeout(snapshot.chunks[0].len());
        assert!(deadline > MESSAGE_TIMEOUT && deadline <= Duration::from_secs(120));
        let mut worker = Pool::adjacent()?.spawn()?;
        let reply = worker
            .request(&Request {
                snapshot,
                ..empty()
            })
            .await?;
        assert_eq!(reply.heads, store.heads("tasks")?);
        assert!(reply.bytes >= MAX_DOC * 8 / 10);
        Ok(())
    }
    // Contained workers exist only on Windows (job objects); elsewhere Pool::new refuses.
    #[cfg(windows)]
    #[tokio::test]
    async fn f4_workers_have_independent_memory_and_pool_slots_are_released() -> Result<()> {
        let path = std::env::current_exe()?
            .parent()
            .and_then(|p| p.parent())
            .ok_or(Failure(Reason::WorkerUnavailable))?
            .join("examples/worker_fault.exe");
        let pool = Pool::new(path)?;
        let mut first = pool.spawn()?;
        let mut second = pool.spawn()?;
        for worker in [&mut first, &mut second] {
            worker.input.write_all(&[3]).await?;
            worker.input.flush().await?;
            let mut report = [0; 9];
            tokio::time::timeout(
                Duration::from_secs(10),
                worker.output.read_exact(&mut report),
            )
            .await??;
            assert_eq!(&report, b"allocated");
        }
        let third = pool.spawn()?;
        let fourth = pool.spawn()?;
        let error = pool.spawn().err().ok_or(Failure(Reason::Protocol))?;
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::WorkerUnavailable)
        );
        drop(third);
        let replacement = pool.spawn()?;
        drop((first, second, fourth, replacement));
        assert_eq!(pool.slots.available_permits(), 4);
        Ok(())
    }
    #[tokio::test]
    async fn c1_worker_crash_timeout_and_memory_limit_do_not_kill_the_caller() -> Result<()> {
        for mode in [0u8, 1, 2] {
            let mut worker = fixture()?;
            worker.input.write_all(&[mode]).await?;
            worker.input.flush().await?;
            if mode == 2 {
                let mut report = [0; 9];
                let read =
                    tokio::time::timeout(MESSAGE_TIMEOUT, worker.output.read_exact(&mut report))
                        .await?;
                assert_eq!(
                    read.expect_err("oversized allocation must never report success")
                        .kind(),
                    std::io::ErrorKind::UnexpectedEof
                );
            }
            let result = worker.request(&empty()).await;
            assert!(result.is_err());
            let code = result
                .err()
                .and_then(|e| e.downcast_ref::<Failure>().map(|f| f.0))
                .ok_or(Failure(Reason::Protocol))?;
            assert_eq!(
                code,
                if mode == 1 {
                    Reason::WorkerTimedOut
                } else {
                    Reason::WorkerCrashed
                }
            );
            let dir = tempfile::tempdir()?;
            let mut store = Store::open(&dir.path().join("caller.sqlite"), b"caller")?;
            store.edit("tasks", |d| {
                model::create(d, "still-alive", "caller survived")
            })?;
            assert_eq!(
                model::text(&store.load("tasks")?, "still-alive")?,
                "caller survived"
            );
        }
        Ok(())
    }
    fn leb(out: &mut Vec<u8>, mut n: usize) {
        loop {
            let mut b = (n & 127) as u8;
            n >>= 7;
            if n != 0 {
                b |= 128;
            }
            out.push(b);
            if n == 0 {
                break;
            }
        }
    }
    fn message(change: &[u8]) -> Vec<u8> {
        let mut out = vec![0x42, 0, 0, 0, 1];
        leb(&mut out, change.len());
        out.extend_from_slice(change);
        out
    }
    #[test]
    fn c5_orphan_change_survives_worker_and_store_restart() -> Result<()> {
        let mut source = Automerge::new();
        model::create(&mut source, "one", "parent")?;
        let parent = source.get_changes(&[])[0].raw_bytes().to_vec();
        model::put(&mut source, "one", "owner", "child")?;
        let child = source
            .get_changes(&[])
            .last()
            .ok_or(Failure(Reason::Protocol))?
            .raw_bytes()
            .to_vec();
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("orphans.sqlite");
        let mut store = Store::open(&path, b"destination")?;
        let mut engine = Engine::default();
        let first = engine.process(Request {
            incoming: message(&child),
            ..empty()
        })?;
        assert!(first.received);
        assert!(first.heads.is_empty());
        assert!(!first.delta.is_empty());
        store.append(
            "tasks",
            0,
            Commit {
                creator: first.creator,
                chunks: first.delta,
                heads: first.heads,
                bytes: first.bytes,
                compact: first.compact,
                orphans: first.orphans,
            },
            b"peer",
        )?;
        drop(engine);
        drop(store);
        let mut store = Store::open(&path, b"destination")?;
        let mut restarted = Engine::default();
        let snapshot = store.snapshot("tasks", None)?;
        let version = snapshot.version;
        let reply = restarted.process(Request {
            snapshot,
            incoming: message(&parent),
            ..empty()
        })?;
        store.append(
            "tasks",
            version,
            Commit {
                creator: reply.creator,
                chunks: reply.delta,
                heads: reply.heads,
                bytes: reply.bytes,
                compact: reply.compact,
                orphans: reply.orphans,
            },
            b"peer",
        )?;
        assert_eq!(
            model::values(&store.load("tasks")?, "one", "owner")?,
            ["child"]
        );
        assert_eq!(store.heads("tasks")?, source.get_heads());
        Ok(())
    }
    #[test]
    fn c1_preflight_is_used_by_the_actual_worker_engine() {
        let mut engine = Engine::default();
        let request = Request {
            incoming: vec![0x42, 0, 0, 1, 0, 3, 1, 0, 7, 0],
            ..empty()
        };
        assert_eq!(
            engine
                .process(request)
                .err()
                .expect("zero-bit bloom")
                .to_string(),
            "zero-bit bloom with entries"
        );
    }
}
