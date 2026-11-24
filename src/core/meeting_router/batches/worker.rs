use super::super::hex_bridge::HexFailure as Failure;
use super::*;
use std::{
    io::{self, Write},
    num::NonZeroUsize,
    process::{Command, ExitStatus},
    sync::{
        Arc,
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
const SHUTDOWN_BOUND: Duration = Duration::from_millis(500);
#[cfg(test)]
type DeadlineHook = Box<dyn FnOnce(&Worker)>;
#[cfg(test)]
std::thread_local! {
    static CONTROLLER_GATE: std::cell::RefCell<Option<Receiver<()>>> = const { std::cell::RefCell::new(None) };
    static CONTROLLER_REPORTED: std::cell::RefCell<Option<Sender<()>>> = const { std::cell::RefCell::new(None) };
    static AT_RESPONSE_DEADLINE: std::cell::RefCell<Option<DeadlineHook>> = const { std::cell::RefCell::new(None) };
}
fn controller(
    job: Arc<crate::platform::ProcessJob>,
    process: std::os::windows::io::OwnedHandle,
    cancel: Arc<AtomicBool>,
) -> std::io::Result<(Sender<()>, Receiver<Cleanup>)> {
    #[cfg(test)]
    let gate = CONTROLLER_GATE.with(|slot| slot.borrow_mut().take());
    #[cfg(test)]
    let reported = CONTROLLER_REPORTED.with(|slot| slot.borrow_mut().take());
    let (stop, requests) = mpsc::channel();
    let (finished, cleanup) = mpsc::channel();
    drop(
        std::thread::Builder::new()
            .name("batch-owner".into())
            .spawn(move || {
                use crate::platform::JobProcess;
                #[cfg(test)]
                if let Some(gate) = gate {
                    let _ = gate.recv();
                }
                while !cancel.load(Ordering::Acquire)
                    && matches!(JobProcess::wait_handle(&process, 0), Ok(None))
                {
                    if requests.recv_timeout(Duration::from_millis(10))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
                let drained = || -> std::io::Result<bool> {
                    Ok(JobProcess::wait_handle(&process, 0)?.is_some() && job.empty()?)
                };
                let deadline = Instant::now() + Duration::from_millis(250);
                let result = (|| {
                    job.terminate().map_err(|_| WorkerFailure::CleanupFailed)?;
                    loop {
                        match drained() {
                            Ok(true) => return Ok(()),
                            Err(_) => return Err(WorkerFailure::CleanupFailed),
                            _ if Instant::now() >= deadline => {
                                return Err(WorkerFailure::CleanupTimeout);
                            }
                            _ => std::thread::sleep(Duration::from_millis(5)),
                        }
                    }
                })();
                let _ = finished.send(result);
                #[cfg(test)]
                if let Some(reported) = reported {
                    let _ = reported.send(());
                }
                // Retain both job and process handles until every descendant has terminated.
                if result.is_err() {
                    while !matches!(drained(), Ok(true)) {
                        let _ = job.terminate();
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            })?,
    );
    Ok((stop, cleanup))
}
pub struct Worker {
    job: Arc<crate::platform::ProcessJob>,
    child: crate::platform::JobProcess,
    stop: Sender<()>,
    cleanup: Receiver<Cleanup>,
    cleaned: Option<Cleanup>,
    output: tempfile::TempDir,
    cancel: Arc<AtomicBool>,
    pub timeout: Duration,
}
impl Worker {
    /// Command waits for `start\n` before model loads/spawns, then `range\n` per request.
    /// SIDENOTE_BATCH_OUTPUT contains range.json; publish result.json atomically.
    pub fn spawn(command: Command, cancel: Arc<AtomicBool>, timeout: Duration) -> Result<Self> {
        Self::spawn_with_threads(command, cancel, timeout, NonZeroUsize::new(4).unwrap())
    }
    /// Child adapters must use SIDENOTE_BATCH_THREADS at session creation.
    pub fn spawn_with_threads(
        mut command: Command,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
        threads: NonZeroUsize,
    ) -> Result<Self> {
        if cancel.load(Ordering::Acquire) {
            return Err(WorkerFailure::Cancelled.into());
        }
        let output = tempfile::tempdir()?;
        command.env_clear();
        // The runtime uses an absolute DLL path; PATH and owner state are unnecessary.
        for key in ["SystemRoot", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("SIDENOTE_BATCH_OUTPUT", output.path())
            .env("SIDENOTE_BATCH_THREADS", threads.to_string());
        let job = Arc::new(crate::platform::ProcessJob::new()?);
        let child = job.spawn(&command)?;
        let (stop, cleanup) = controller(job.clone(), child.handle.try_clone()?, cancel.clone())?;
        let mut worker = Self {
            job,
            child,
            stop,
            cleanup,
            cleaned: None,
            output,
            cancel,
            timeout,
        };
        if let Err(error) = worker.send(b"start\n", Instant::now()) {
            worker.shutdown()?;
            return Err(error);
        }
        Ok(worker)
    }
    fn send(&mut self, command: &[u8], started: Instant) -> Result<()> {
        let error = match self.child.input.write_all(command) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        self.running()?;
        if error.kind() != io::ErrorKind::BrokenPipe {
            return Err(error.into());
        }
        // Windows can close stdin before it signals the process handle.
        loop {
            let remaining = self.timeout.saturating_sub(started.elapsed());
            let wait_ms = remaining.as_millis().min(10) as u32;
            if wait_ms == 0 {
                #[cfg(test)]
                self.at_response_deadline();
                if let Some(status) = self.child.try_wait()? {
                    return Err(WorkerFailure::Exited(status).into());
                }
                return Err(error.into());
            }
            let status = self.child.wait_for(wait_ms)?;
            if self.cancel.load(Ordering::Acquire) {
                return Err(WorkerFailure::Cancelled.into());
            }
            if let Some(status) = status {
                return Err(WorkerFailure::Exited(status).into());
            }
        }
    }
    fn running(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(WorkerFailure::Cancelled.into());
        }
        if let Some(status) = self.child.try_wait()? {
            return Err(WorkerFailure::Exited(status).into());
        }
        Ok(())
    }
    #[cfg(test)]
    fn at_response_deadline(&self) {
        if let Some(hook) = AT_RESPONSE_DEADLINE.with(|slot| slot.borrow_mut().take()) {
            hook(self);
        }
    }
    /// Wait at most 500 ms; a cleanup failure leaves the controller owning deferred reaping.
    pub fn shutdown(&mut self) -> Result<()> {
        let result = *self.cleaned.get_or_insert_with(|| {
            let deadline = Instant::now() + SHUTDOWN_BOUND;
            // Request termination before waiting for a potentially unscheduled controller.
            let terminated = self
                .job
                .terminate()
                .map_err(|_| WorkerFailure::CleanupFailed);
            let _ = self.stop.send(());
            terminated?;
            match self
                .cleanup
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
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
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let started = Instant::now();
            store_io::write(&self.output.path().join("range.json"), range)?;
            self.send(b"range\n", started)?;
            loop {
                self.running()?;
                if started.elapsed() >= self.timeout {
                    #[cfg(test)]
                    self.at_response_deadline();
                    if let Some(status) = self.child.try_wait()? {
                        return Err(WorkerFailure::Exited(status).into());
                    }
                    return Err(Failure::Timeout.into());
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
            // Callers classify the cause (Timeout, Cancelled, Exited); a slow reap must not
            // replace it. The cleanup result stays cached for finish() and Drop.
            let _ = self.shutdown();
        }
        result
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod lifetime_tests;

#[cfg(test)]
mod signals;
