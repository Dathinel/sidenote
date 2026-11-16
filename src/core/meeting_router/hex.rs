use super::super::asr::{Result, write_wav};
use super::hex_bridge::{ChildFailure, HexFailure, reply_error};
use crate::platform::CommandExt;
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

pub struct HexSpeech {
    pub executable: PathBuf,
}
impl Default for HexSpeech {
    fn default() -> Self {
        Self {
            executable: default_executable(),
        }
    }
}
impl HexSpeech {
    fn run(&self, args: &[&str], wav: Option<&Path>) -> Result<String> {
        self.run_for(args, wav, Duration::from_secs(7200))
    }
    pub(super) fn run_for(
        &self,
        args: &[&str],
        wav: Option<&Path>,
        timeout: Duration,
    ) -> Result<String> {
        self.run_command(args, wav, timeout, false)
    }
    pub(super) fn run_for_bridge(&self, args: &[&str], timeout: Duration) -> Result<String> {
        self.run_command(args, None, timeout, true)
    }
    fn run_command(
        &self,
        args: &[&str],
        wav: Option<&Path>,
        timeout: Duration,
        structured_errors: bool,
    ) -> Result<String> {
        crate::platform::require_process_support()?;
        let mut command = crate::platform::worker_command(&self.executable)?;
        command
            .args(args)
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = wav {
            command.arg(path);
        }
        let mut child = command.spawn()?;
        let job = match crate::platform::ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(_) => {
                if !stop_child(&mut child) {
                    return Err(HexFailure::Timeout.into());
                }
                return Err("Cannot contain speech process".into());
            }
        };
        let stdout = child.stdout.take().ok_or("Speech output unavailable")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Speech diagnostics unavailable")?;
        let (status, bytes, stderr, truncated) =
            collect_child(&mut child, stdout, stderr, timeout, || drop(job))?;
        let output = String::from_utf8_lossy(&bytes);
        let failure = reply_error(&output);
        if !status.success() || (structured_errors && failure.is_some()) {
            let value: Value = serde_json::from_str(&output).unwrap_or_default();
            let code = value["code"].as_str().map(|code| {
                if code.len() <= 64
                    && code
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_".contains(&c))
                {
                    code.to_owned()
                } else {
                    "[redacted]".into()
                }
            });
            let reason = value["reason"]
                .as_str()
                .or_else(|| value["message"].as_str())
                .map(diagnostic_excerpt);
            let mut stderr = diagnostic_excerpt(&String::from_utf8_lossy(&stderr));
            if truncated {
                stderr.push_str(" [truncated]");
            }
            return Err(HexFailure::Child(Box::new(ChildFailure {
                kind: failure.unwrap_or(HexFailure::Failed),
                exit_code: status.code(),
                code,
                reason,
                stderr,
            }))
            .into());
        }
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Hex transcription failed; raw diagnostics suppressed".into());
        }
        Ok(String::from_utf8(bytes)?.trim().to_owned())
    }
    pub fn selection(&self) -> Result<String> {
        let value: Value = serde_json::from_str(&self.run(&["model", "list", "--json"], None)?)?;
        let selected: Vec<_> = value["models"]
            .as_array()
            .ok_or("Hex model list unavailable")?
            .iter()
            .filter(|m| m["selected"] == true)
            .filter_map(|m| m["id"].as_str())
            .collect();
        if selected.len() != 1 {
            return Err("Hex selection is ambiguous".into());
        }
        Ok(selected[0].to_owned())
    }
    pub fn transcribe(&self, wav: &Path, model: &str) -> Result<String> {
        let before = self.selection()?;
        if before != "parakeet_unified_en" {
            return Err(
                "FATAL: Hex selection must remain parakeet_unified_en; no settings were changed"
                    .into(),
            );
        }
        let result = self.run(&["model", "transcribe", "--model", model], Some(wav));
        let after = self.selection()?;
        if after != before {
            return Err("FATAL: Hex dictation selection changed during transcription; check Hex before continuing".into());
        }
        result
    }
    pub fn samples(&self, path: &Path, samples: &[f32], model: &str) -> Result<String> {
        write_wav(path, samples)?;
        self.transcribe(path, model)
    }
}

fn diagnostic_excerpt(text: &str) -> String {
    // Child diagnostics may contain transcript text or private paths; retain only diagnostic vocabulary.
    let mut out = String::new();
    for word in text.split_whitespace() {
        let word = word
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_ascii_lowercase();
        let safe = match word.as_str() {
            "recognizer" | "decoder" | "allocation" | "failed" | "failure" | "error" | "out"
            | "of" | "memory" | "model" | "runtime" | "onnx" | "invalid" | "argument"
            | "unsupported" | "busy" | "timeout" | "timed" | "no" | "speech" | "was" | "heard"
            | "in" | "the" | "file" | "nothing" | "to" | "print" | "cannot" | "load" | "open"
            | "read" | "write" | "device" | "unavailable" | "denied" | "permission" | "access"
            | "not" | "found" | "status" | "code" | "assertion" | "panic" => word.as_str(),
            _ => "[redacted]",
        };
        if out.len() + safe.len() + 1 > 512 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(safe);
    }
    out
}

fn default_executable() -> PathBuf {
    #[cfg(windows)]
    {
        crate::core::meetings_dir()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("bin/hex.com")
    }
    #[cfg(not(windows))]
    {
        PathBuf::new()
    }
}

pub(super) trait SpeechChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn kill(&mut self) -> std::io::Result<()>;
}
impl SpeechChild for std::process::Child {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.try_wait()
    }
    fn kill(&mut self) -> std::io::Result<()> {
        self.kill()
    }
}
pub(super) fn collect_child(
    child: &mut impl SpeechChild,
    stdout: impl Read + Send + 'static,
    mut stderr: impl Read + Send + 'static,
    timeout: Duration,
    cleanup: impl FnOnce(),
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>, bool)> {
    let diagnostics = thread::spawn(move || -> std::io::Result<_> {
        let mut bytes = Vec::new();
        stderr.by_ref().take(4096).read_to_end(&mut bytes)?;
        let truncated = std::io::copy(&mut stderr, &mut std::io::sink())? > 0;
        Ok((bytes, truncated))
    });
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(50)),
            _ => {
                stop_child(child);
                break Err(HexFailure::Timeout);
            }
        }
    };
    cleanup();
    let status = status?;
    let deadline = Instant::now() + PIPE_DRAIN_LIMIT;
    let (stderr, truncated) = finish_thread(diagnostics, deadline)??;
    let bytes = finish_thread(reader, deadline)??;
    Ok((status, bytes, stderr, truncated))
}

const CHILD_CLEANUP_LIMIT: Duration = Duration::from_millis(250);
const PIPE_DRAIN_LIMIT: Duration = Duration::from_millis(250);
fn stop_child(child: &mut impl SpeechChild) -> bool {
    let _ = child.kill();
    let deadline = Instant::now() + CHILD_CLEANUP_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => return false,
        }
    }
}
pub(super) fn finish_thread<T>(thread: thread::JoinHandle<T>, deadline: Instant) -> Result<T> {
    while !thread.is_finished() {
        if Instant::now() >= deadline {
            return Err(HexFailure::Timeout.into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    thread.join().map_err(|_| HexFailure::Failed.into())
}
