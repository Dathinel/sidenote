use super::super::hex_bridge::HexFailure as Failure;
use super::*;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub struct Worker {
    child: Child,
    job: Option<crate::platform::ProcessJob>,
    output: tempfile::TempDir,
    pub cancel: Arc<AtomicBool>,
    pub timeout: Duration,
}
impl Worker {
    /// Command waits for `start\n` before model loads/spawns, then `range\n` per request.
    /// SIDENOTE_BATCH_OUTPUT contains range.json; publish result.json atomically.
    pub fn spawn(mut command: Command, cancel: Arc<AtomicBool>, timeout: Duration) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        let output = tempfile::tempdir()?;
        command
            .env("SIDENOTE_BATCH_OUTPUT", output.path())
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn()?;
        let mut worker = Self {
            child,
            job: None,
            output,
            cancel,
            timeout,
        };
        worker.job = Some(crate::platform::ProcessJob::attach(&worker.child)?);
        worker.input().write_all(b"start\n")?;
        Ok(worker)
    }
    fn input(&mut self) -> &mut std::process::ChildStdin {
        self.child.stdin.as_mut().unwrap()
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
