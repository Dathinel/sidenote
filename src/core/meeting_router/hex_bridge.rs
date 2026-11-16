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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HexFailure {
    Unsupported,
    Busy,
    Timeout,
    Failed,
    NoSpeech,
    HelperFailed,
    InvalidRange,
    StatusUnknown { timeouts: u8, last_idle: bool },
    Child(Box<ChildFailure>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildFailure {
    pub(super) kind: HexFailure,
    pub(super) exit_code: Option<i32>,
    pub(super) code: Option<String>,
    pub(super) reason: Option<String>,
    pub(super) stderr: String,
}
impl HexFailure {
    pub fn kind(&self) -> &Self {
        match self {
            Child(detail) => detail.kind.kind(),
            kind => kind,
        }
    }
    fn classify_as(self, kind: Self) -> Self {
        match self {
            Child(mut detail) => {
                detail.kind = kind;
                Child(detail)
            }
            _ => kind,
        }
    }
}
impl std::fmt::Display for HexFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported => f.write_str("Hex command is unsupported"),
            Busy => f.write_str("Hex is busy"),
            Timeout => f.write_str("Hex request timed out"),
            Failed => f.write_str("Hex recognition failed"),
            NoSpeech => f.write_str("Hex recognized no speech"),
            HelperFailed => f.write_str("Private Hex helper failed; captured baseline required"),
            InvalidRange => f.write_str("Invalid Hex audio range"),
            StatusUnknown {
                timeouts,
                last_idle,
            } => write!(
                f,
                "Hex status unknown after {timeouts} consecutive probe timeouts; last known idle={last_idle}"
            ),
            Child(detail) => {
                write!(
                    f,
                    "{}; exit={}",
                    detail.kind,
                    detail
                        .exit_code
                        .map_or_else(|| "unavailable".into(), |c| c.to_string())
                )?;
                if let Some(code) = &detail.code {
                    write!(f, "; code={code}")?;
                }
                if let Some(reason) = &detail.reason {
                    write!(f, "; reason={reason}")?;
                }
                write!(f, "; stderr={}", detail.stderr)
            }
        }
    }
}
impl std::error::Error for HexFailure {}
type Result<T> = std::result::Result<T, HexFailure>;
#[derive(Debug, PartialEq)]
pub struct HexReading {
    pub request: HexRequest,
    pub text: String,
    pub json: Option<Value>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HexRequest {
    pub range_id: String,
    pub model_id: String,
    pub language: String,
}
pub trait HexCommand {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String>;
    fn begin(&mut self, _request: &HexRequest) {}
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}
impl HexCommand for HexSpeech {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String> {
        #[cfg(windows)]
        {
            self.run_for_bridge(args, timeout)
                .map_err(|e| e.downcast_ref::<HexFailure>().cloned().unwrap_or(Failed))
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
        "no-speech" => Some(NoSpeech),
        "unsupported" | "unknown-command" | "version-mismatch" => Some(Unsupported),
        _ => Some(Failed),
    }
}
// Hex supplies no request id: hold this lock through dispatch, parsing and attribution.
static SERIAL: Mutex<()> = Mutex::new(());
pub struct HexBridge<C> {
    command: C,
    json: Option<bool>,
    busy_wait: Duration,
    request_timeout: Duration,
    deadline: Option<Instant>,
    last_idle: bool,
    status_timeouts: u8,
}
impl<C: HexCommand> HexBridge<C> {
    pub fn finish(&mut self) -> Result<()> {
        self.command.finish()
    }
    pub fn new(command: C) -> Self {
        Self {
            command,
            json: None,
            busy_wait: Duration::from_secs(15),
            request_timeout: Duration::from_secs(120),
            deadline: None,
            last_idle: false,
            status_timeouts: 0,
        }
    }
    fn call(&mut self, args: &[&str]) -> Result<String> {
        let control = args.contains(&"--help") || args[0] == "status";
        let mut timeout = if control {
            Duration::from_secs(2).min(self.request_timeout)
        } else {
            self.request_timeout
        };
        if let Some(deadline) = self.deadline {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or(Timeout)?;
            if left.is_zero() {
                return Err(Timeout);
            }
            timeout = timeout.min(left);
        }
        let out = self.command.call(args, timeout)?;
        reply_error(&out).map_or(Ok(out), Err)
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }
    pub fn limit(&mut self, remaining: Duration) {
        self.request_timeout = self.request_timeout.min(remaining);
        self.busy_wait = self.busy_wait.min(remaining);
        self.deadline = Some(Instant::now() + remaining);
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
    pub(super) fn probe_idle(&mut self) -> Result<bool> {
        match self.status() {
            Ok(status) => {
                self.status_timeouts = 0;
                self.last_idle = status["meeting"]["state"] == "idle"
                    && status["capture"]["state"] == "idle"
                    && status["capture"]["pending_jobs"] == 0
                    && status["model"]["preparing"].is_null();
                Ok(self.last_idle)
            }
            Err(error) if error.kind() == &Timeout => {
                self.status_timeouts = self.status_timeouts.saturating_add(1);
                crate::listener::log(&format!(
                    "Hex status probe timed out; consecutive={}; last_known_idle={}",
                    self.status_timeouts, self.last_idle
                ));
                Err(StatusUnknown {
                    timeouts: self.status_timeouts,
                    last_idle: self.last_idle,
                })
            }
            Err(error) => Err(error),
        }
    }
    pub fn transcribe(&mut self, wav: &Path, request: &HexRequest) -> Result<HexReading> {
        let reader = hound::WavReader::open(wav).map_err(|_| InvalidRange)?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate != 16000
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || !(1..=480_000).contains(&reader.duration())
            || [&request.range_id, &request.model_id, &request.language]
                .iter()
                .any(|s| s.is_empty())
        {
            return Err(InvalidRange);
        }
        let wav = wav.to_str().ok_or(InvalidRange)?;
        let _serial = SERIAL.try_lock().map_err(|_| Busy)?;
        self.command.begin(request);
        if self.json.is_none() {
            // ADR 0009:12; pinned Hex f5681bc control/cli.rs:593-601,638-651.
            let help = self.call(&["model", "transcribe", "--help"]).map_err(|e| {
                if e.kind() == &Timeout {
                    e
                } else {
                    e.classify_as(Unsupported)
                }
            })?;
            if !["--model", "--language"]
                .iter()
                .all(|flag| help.split_whitespace().any(|word| word == *flag))
            {
                return Err(Unsupported);
            }
            self.json = Some(help.split_whitespace().any(|word| word == "--json"));
        }
        let started = Instant::now();
        let out = loop {
            if started.elapsed() >= self.busy_wait {
                return Err(Busy);
            }
            // Only a current successful idle probe authorizes dispatch.
            if self.probe_idle()? {
                let mut args = vec![
                    "model",
                    "transcribe",
                    wav,
                    "--model",
                    &request.model_id,
                    "--language",
                    &request.language,
                ];
                if self.json == Some(true) {
                    args.push("--json");
                }
                if started.elapsed() >= self.busy_wait {
                    return Err(Busy);
                }
                break self.call(&args)?;
            }
            if started.elapsed() >= self.busy_wait {
                return Err(Busy);
            }
            thread::sleep(
                Duration::from_millis(100).min(self.busy_wait.saturating_sub(started.elapsed())),
            );
        };
        if out.trim().is_empty() {
            return Err(NoSpeech);
        }
        let json = if self.json == Some(true) {
            Some(serde_json::from_str::<Value>(&out).map_err(|_| Failed)?)
        } else {
            None
        };
        if let Some(v) = &json {
            for model in ["model", "model_id"].iter().filter_map(|key| v.get(key)) {
                if model.as_str() != Some(request.model_id.as_str()) {
                    return Err(Failed);
                }
            }
            let mut previous = 0.0;
            if let Some(segments) = v.get("segments") {
                for segment in segments.as_array().ok_or(Failed)? {
                    let start = segment["start"].as_f64().ok_or(Failed)?;
                    let end = segment["end"].as_f64().ok_or(Failed)?;
                    if !start.is_finite()
                        || !end.is_finite()
                        || start < previous
                        || end < start
                        || end > f64::from(reader.duration()) / 16000.0
                    {
                        return Err(Failed);
                    }
                    previous = end;
                }
            }
        }
        let text = match &json {
            Some(v) => v["transcript"]
                .as_str()
                .or_else(|| v["text"].as_str())
                .ok_or(Unsupported)?,
            None => out.trim(),
        };
        if text.trim().is_empty() {
            return Err(NoSpeech);
        }
        Ok(HexReading {
            request: request.clone(),
            text: text.to_owned(),
            json,
        })
    }
}
#[cfg(test)]
mod tests;
