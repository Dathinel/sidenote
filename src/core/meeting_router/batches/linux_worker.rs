//! Linux native ASR stays behind the reviewed namespace launcher.
use super::*;
use std::{
    io::{self, Write},
    num::NonZeroUsize,
    process::{Child, ChildStdin, Command, ExitStatus},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerFailure {
    Cancelled,
    Exited(ExitStatus),
    CleanupTimeout,
    CleanupFailed,
}
impl std::fmt::Display for WorkerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for WorkerFailure {}
type Cleanup = std::result::Result<(), WorkerFailure>;
pub struct Worker {
    child: Arc<Mutex<Child>>,
    input: ChildStdin,
    stop: Sender<()>,
    cleanup: Receiver<Cleanup>,
    cleaned: Option<Cleanup>,
    output: Arc<tempfile::TempDir>,
    cancel: Arc<AtomicBool>,
    pub timeout: Duration,
}
impl Worker {
    pub fn spawn(command: Command, cancel: Arc<AtomicBool>, timeout: Duration) -> Result<Self> {
        Self::spawn_with_threads(command, cancel, timeout, NonZeroUsize::new(4).unwrap())
    }
    pub fn spawn_with_threads(
        mut command: Command,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        threads: NonZeroUsize,
    ) -> Result<Self> {
        if cancel.load(Ordering::Acquire) {
            return Err(WorkerFailure::Cancelled.into());
        }
        let output = Arc::new(tempfile::tempdir()?);
        command
            .env_clear()
            .env("SIDENOTE_BATCH_OUTPUT", output.path())
            .env("SIDENOTE_BATCH_THREADS", threads.to_string());
        let mut child = sidenote_sync::tree_worker::spawn_speech(&mut command)?;
        let input = child.stdin.take().ok_or("Missing native stdin")?;
        let child = Arc::new(Mutex::new(child));
        let (stop, requests) = mpsc::channel();
        let (finished, cleanup) = mpsc::channel();
        let owned = child.clone();
        let cancelled = cancel.clone();
        // Retain the output directory until the launcher and its namespace are reaped.
        let owned_output = output.clone();
        let controller = std::thread::Builder::new()
            .name("batch-owner".into())
            .spawn(move || {
                loop {
                    if cancelled.load(Ordering::Acquire)
                        || !matches!(owned.lock().unwrap().try_wait(), Ok(None))
                        || requests.recv_timeout(Duration::from_millis(10))
                            != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
                let mut child = owned.lock().unwrap();
                // The launcher owns namespace PID 1. Killing its group tears down descendants too.
                if matches!(child.try_wait(), Ok(None)) {
                    sidenote_sync::containment::kill_group(child.id());
                }
                let result = child
                    .wait()
                    .map(|_| ())
                    .map_err(|_| WorkerFailure::CleanupFailed);
                drop(owned_output);
                let _ = finished.send(result);
            });
        if let Err(error) = controller {
            let mut child = child.lock().unwrap();
            sidenote_sync::containment::kill_group(child.id());
            let _ = child.wait();
            return Err(error.into());
        }
        let mut worker = Self {
            child,
            input,
            stop,
            cleanup,
            cleaned: None,
            output,
            cancel,
            timeout,
        };
        if let Err(error) = worker.input.write_all(b"start\n") {
            let error = worker.running().err().unwrap_or_else(|| error.into());
            let _ = worker.shutdown();
            return Err(error);
        }
        Ok(worker)
    }
    pub fn pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }
    fn running(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(WorkerFailure::Cancelled.into());
        }
        if let Some(status) = self.child.lock().unwrap().try_wait()? {
            return Err(WorkerFailure::Exited(status).into());
        }
        Ok(())
    }
    pub fn shutdown(&mut self) -> Result<()> {
        let result = *self.cleaned.get_or_insert_with(|| {
            let _ = self.stop.send(());
            match self.cleanup.recv_timeout(Duration::from_millis(500)) {
                Ok(result) => result,
                Err(mpsc::RecvTimeoutError::Timeout) => Err(WorkerFailure::CleanupTimeout),
                Err(_) => Err(WorkerFailure::CleanupFailed),
            }
        });
        result.map_err(Into::into)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
impl Recognizer for Worker {
    fn finish(&mut self) -> Result<()> {
        self.shutdown()
    }
    fn limit(&mut self, remaining: Duration) {
        self.timeout = self.timeout.min(remaining);
    }
    fn recognize(&mut self, range: &Range) -> Result<Value> {
        let result = (|| {
            self.running()?;
            let path = self.output.path().join("result.json");
            match std::fs::remove_file(&path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            let started = Instant::now();
            store_io::write(&self.output.path().join("range.json"), range)?;
            if let Err(error) = self.input.write_all(b"range\n") {
                self.running()?;
                return Err(error.into());
            }
            loop {
                self.running()?;
                if started.elapsed() >= self.timeout {
                    return Err(super::super::hex_bridge::HexFailure::Timeout.into());
                }
                if path.exists() {
                    return Ok(serde_json::from_slice(&store_io::read_bounded(
                        &path,
                        1 << 20,
                    )?)?);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if result.is_err() {
            let _ = self.shutdown();
        }
        result
    }
}
