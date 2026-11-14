//! Explicit bounded calls only; baseline coverage selection belongs to the caller.
use super::HexSpeech;
use HexFailure::*;
use serde_json::Value;
use std::{
    path::Path,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HexFailure {
    Unsupported,
    Busy,
    Timeout,
    Failed,
    InvalidRange,
}
impl std::fmt::Display for HexFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HexFailure {}
type Result<T> = std::result::Result<T, HexFailure>;
#[derive(Debug, PartialEq)]
pub struct HexReading {
    pub text: String,
    pub json: Option<Value>,
}
pub trait HexCommand {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String>;
}
impl HexCommand for HexSpeech {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String> {
        #[cfg(windows)]
        {
            self.run_for(args, None, timeout)
                .map_err(|e| e.downcast_ref::<HexFailure>().copied().unwrap_or(Failed))
        }
        #[cfg(not(windows))]
        {
            let _ = (args, timeout);
            Err(Unsupported)
        }
    }
}
pub(super) fn reply_error(output: &str) -> Option<HexFailure> {
    let v: Value = serde_json::from_str(output).ok()?;
    match v["code"].as_str()? {
        "busy" => Some(Busy),
        "unsupported" | "unknown-command" | "version-mismatch" => Some(Unsupported),
        _ => Some(Failed),
    }
}
static SERIAL: Mutex<()> = Mutex::new(());
pub struct HexBridge<C> {
    command: C,
    probed: bool,
    busy_wait: Duration,
}
impl<C: HexCommand> HexBridge<C> {
    pub fn new(command: C) -> Self {
        Self {
            command,
            probed: false,
            busy_wait: Duration::from_secs(15),
        }
    }
    fn call(&mut self, args: &[&str]) -> Result<String> {
        let timeout = if args.len() > 2 {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(2)
        };
        let out = self.command.call(args, timeout)?;
        reply_error(&out).map_or(Ok(out), Err)
    }
    fn status(&mut self) -> Result<Value> {
        let v: Value =
            serde_json::from_str(&self.call(&["status", "--json"])?).map_err(|_| Unsupported)?;
        if v["version"] != 4
            || v["model"].get("preparing").is_none()
            || !v["meeting"]["state"].is_string()
            || !v["capture"]["state"].is_string()
            || v["capture"]["pending_jobs"].as_u64().is_none()
        {
            return Err(Unsupported);
        }
        Ok(v)
    }
    pub fn transcribe(&mut self, wav: &Path, model: &str, lang: &str) -> Result<HexReading> {
        let reader = hound::WavReader::open(wav).map_err(|_| InvalidRange)?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate != 16000
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || !(1..=480_000).contains(&reader.duration())
            || model.is_empty()
            || lang.is_empty()
        {
            return Err(InvalidRange);
        }
        let wav = wav.to_str().ok_or(InvalidRange)?;
        let _serial = SERIAL.try_lock().map_err(|_| Busy)?;
        if !self.probed {
            let help = self
                .call(&["transcribe", "--help"])
                .map_err(|e| if e == Timeout { e } else { Unsupported })?;
            if !["--model", "--language", "--json"]
                .iter()
                .all(|flag| help.split_whitespace().any(|word| word == *flag))
            {
                return Err(Unsupported);
            }
            self.probed = true;
        }
        let started = Instant::now();
        let out = loop {
            let status = self.status()?;
            let idle = status["meeting"]["state"] == "idle"
                && status["capture"]["state"] == "idle"
                && status["capture"]["pending_jobs"] == 0
                && status["model"]["preparing"].is_null();
            if idle {
                let result = self.call(&[
                    "transcribe",
                    wav,
                    "--model",
                    model,
                    "--language",
                    lang,
                    "--json",
                ]);
                match result {
                    Err(Busy) => {}
                    Err(e) => return Err(e),
                    Ok(out) => break out,
                }
            }
            if started.elapsed() >= self.busy_wait {
                return Err(Busy);
            }
            thread::sleep(
                Duration::from_millis(100).min(self.busy_wait.saturating_sub(started.elapsed())),
            );
        };
        let json = serde_json::from_str::<Value>(&out).ok();
        let text = match &json {
            Some(v) => v["transcript"]
                .as_str()
                .or_else(|| v["text"].as_str())
                .ok_or(Unsupported)?,
            None => out.trim(),
        };
        if text.trim().is_empty() {
            return Err(Failed);
        }
        Ok(HexReading {
            text: text.to_owned(),
            json,
        })
    }
}
#[cfg(test)]
mod tests;
