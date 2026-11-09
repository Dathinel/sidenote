//! Device-local meeting notes. Commands and consent are independent of editable runners.
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Helper { Claude, Codex, OpenCode }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine { App, Scripts(PathBuf), Off }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Queued, Running, Done { fallback: bool, tasks_failed: bool }, NoHelper,
    NeedsClick, HelperMissing, BackendError { next_try_ms: Option<i64>, exit: Option<i32> },
    TimedOut, BadOutput, TooLong { minutes: u64 }, Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request { MakeNotes, TryAgain, FirstPart, SortAgain }

/// `engine` is app/scripts/off on disk; `script` is retained when switching engines.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesSettings {
    pub engine: String,
    pub helper: Option<Helper>,
    pub model: String,
    pub auto: bool,
    pub enabled_at_ms: Option<i64>,
    pub script: Option<PathBuf>,
    pub your_name: String,
}
impl Default for NotesSettings {
    fn default() -> Self {
        Self { engine: "app".into(), helper: None, model: String::new(), auto: true,
            enabled_at_ms: None, script: None, your_name: String::new() }
    }
}
impl NotesSettings {
    pub fn engine(&self) -> Engine {
        match self.engine.as_str() {
            "app" => Engine::App,
            "scripts" => self.script.clone().map(Engine::Scripts).unwrap_or(Engine::Off),
            _ => Engine::Off,
        }
    }
}

pub fn settings() -> io::Result<NotesSettings> { Err(io::Error::other("Notes engine not yet wired")) }
pub fn save_settings(_: &NotesSettings) -> io::Result<()> { Err(io::Error::other("Notes engine not yet wired")) }
pub fn detect() -> Vec<(Helper, bool)> { Vec::new() }
pub fn status(_: &str) -> Option<JobState> { None }
pub fn request(_: &str, _: Request) -> io::Result<()> { Err(io::Error::other("Notes engine not yet wired")) }
pub fn test_helper(_: Helper, _: &str) -> Result<Duration, String> { Err("Notes engine not yet wired".into()) }
