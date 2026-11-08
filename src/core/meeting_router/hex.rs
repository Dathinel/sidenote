use super::super::asr::{Result, write_wav};
use serde_json::Value;
use std::{
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct HexSpeech {
    pub executable: PathBuf,
}
impl Default for HexSpeech {
    fn default() -> Self {
        Self {
            executable: PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                .join("hex/bin/hex.com"),
        }
    }
}
impl HexSpeech {
    fn run(&self, args: &[&str], wav: Option<&Path>) -> Result<String> {
        let mut command = Command::new(&self.executable);
        command
            .args(args)
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = wav {
            command.arg(path);
        }
        let mut child = command.spawn()?;
        let job = match crate::platform::ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cannot contain speech process".into());
            }
        };
        let stdout = child.stdout.take().ok_or("Speech output unavailable")?;
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
                Ok(None) if start.elapsed() < Duration::from_secs(7200) => {
                    thread::sleep(Duration::from_millis(50))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("Speech process timed out or failed");
                }
            }
        };
        drop(job);
        let bytes = reader.join().map_err(|_| "Speech reader failed")??;
        if !status?.success() || bytes.len() > 8 * 1024 * 1024 {
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
