use super::super::hex_bridge::HexFailure as Failure;
use super::*;
use std::{
    io::Write,
    num::NonZeroUsize,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub struct Worker {
    child: crate::platform::JobProcess,
    job: Option<crate::platform::ProcessJob>,
    output: tempfile::TempDir,
    pub cancel: Arc<AtomicBool>,
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
        let job = crate::platform::ProcessJob::new()?;
        let child = job.spawn(&command)?;
        let mut worker = Self {
            child,
            job: Some(job),
            output,
            cancel,
            timeout,
        };
        worker.input().write_all(b"start\n")?;
        Ok(worker)
    }
    fn input(&mut self) -> &mut std::io::PipeWriter {
        &mut self.child.input
    }
    fn stop(&mut self) {
        drop(self.job.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Recognizer for Worker {
    fn recognize(&mut self, range: &Range) -> Result<Value> {
        let result = (|| {
            let path = self.output.path().join("result.json");
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let started = Instant::now();
            store_io::write(&self.output.path().join("range.json"), range)?;
            self.input().write_all(b"range\n")?;
            loop {
                if self.cancel.load(Ordering::Acquire) {
                    return Err("Worker cancelled".into());
                }
                if started.elapsed() >= self.timeout {
                    return Err(Failure::Timeout.into());
                }
                if self.child.try_wait()?.is_some() {
                    return Err("Worker exited".into());
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
            self.stop();
        }
        result
    }
}
#[cfg(test)]
mod tests;
