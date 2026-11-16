//! Bounded children and one compiled executable per process fixture.
use super::*;
use crate::platform::{CommandExt, ProcessJob};
use std::{io::Read, process::{Output, Stdio}, sync::{OnceLock, mpsc}};

pub(super) fn output(mut command: Command, timeout: Duration) -> std::io::Result<Output> {
    command.creation_flags(0x08000000).stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let job = match ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let deadline = Instant::now() + Duration::from_secs(5);
            while child.try_wait()?.is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            return Err(error);
        }
    };
    fn drain(pipe: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe.take(1 << 20).read_to_end(&mut bytes).map(|_| bytes);
            let _ = tx.send(result);
        });
        rx
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => break Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "fixture deadline")),
            Err(error) => break Err(error),
        }
    };
    job.terminate()?;
    let cleanup = Instant::now() + Duration::from_secs(5);
    while !job.empty()? && Instant::now() < cleanup {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !job.empty()? { return Err(std::io::Error::other("fixture cleanup deadline")); }
    let status = result?;
    let read = |rx: mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| std::io::Error::other("fixture pipe deadline"))?
    };
    Ok(Output { status, stdout: read(stdout)?, stderr: read(stderr)? })
}

pub(super) fn executable(http: bool) -> PathBuf {
    static CLI: OnceLock<tempfile::TempDir> = OnceLock::new();
    static HTTP: OnceLock<tempfile::TempDir> = OnceLock::new();
    let directory = if http { &HTTP } else { &CLI }.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("fixture.c");
        fs::write(&source, if http { include_str!("http_fake.c") } else { include_str!("fake_hex.c") }).unwrap();
        let mut command = Command::new("gcc.exe");
        command.args(["-Os", "-static", "-s"]).arg(source).arg("-o").arg(dir.path().join("fixture.exe"));
        if http { command.arg("-lws2_32"); }
        let result = output(command, Duration::from_secs(30)).unwrap();
        assert!(result.status.success(), "fixture compiler failed: {}", String::from_utf8_lossy(&result.stderr));
        dir
    });
    directory.path().join("fixture.exe")
}

#[test]
fn fixture_child_deadline_is_enforced() {
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
    let start = Instant::now();
    assert_eq!(output(command, Duration::from_millis(100)).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(6));
}
