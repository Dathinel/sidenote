//! Device-local meeting notes. Commands and consent are independent of editable runners.
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf, time::Duration};
pub mod draft;
mod helper;
mod prompt;
pub mod queue;
mod schema;
mod transcript;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Helper {
    Claude,
    Codex,
    OpenCode,
}

pub const HELPER_NOT_OFFERED: &str = "helper_not_offered";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    App,
    Scripts(PathBuf),
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done {
        fallback: bool,
        tasks_failed: bool,
    },
    NoHelper,
    NeedsClick,
    HelperMissing,
    BackendError {
        next_try_ms: Option<i64>,
        exit: Option<i32>,
    },
    #[serde(rename = "HELPER_LIMIT")]
    HelperLimit {
        exit: Option<i32>,
    },
    #[serde(rename = "HELPER_AUTH")]
    HelperAuth {
        exit: Option<i32>,
    },
    TimedOut,
    BadOutput,
    TooLong {
        minutes: u64,
    },
    ContextTooLong {
        typed_notes: bool,
        glossary: bool,
    },
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    MakeNotes,
    TryAgain,
    FirstPart,
    SortAgain,
    RedoTasks,
}

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
    #[serde(skip)]
    pub relay_name_base: Option<String>,
}
impl Default for NotesSettings {
    fn default() -> Self {
        Self {
            engine: "app".into(),
            helper: None,
            model: String::new(),
            auto: true,
            enabled_at_ms: None,
            script: None,
            your_name: String::new(),
            relay_name_base: None,
        }
    }
}
impl NotesSettings {
    pub fn effective_helper(&self) -> Option<Helper> {
        self.helper.filter(|helper| helper.offered())
    }
    /// Fixed code for UI-owned wording; stored choices remain available for migration.
    pub fn helper_reason(&self) -> Option<&'static str> {
        self.helper
            .filter(|helper| !helper.offered())
            .map(|_| HELPER_NOT_OFFERED)
    }
    pub fn engine(&self) -> Engine {
        match self.engine.as_str() {
            "app" => Engine::App,
            "scripts" => self
                .script
                .clone()
                .map(Engine::Scripts)
                .unwrap_or(Engine::Off),
            _ => Engine::Off,
        }
    }
}

pub(crate) fn migrated_settings(legacy: &std::path::Path) -> NotesSettings {
    if legacy.is_file() {
        NotesSettings {
            engine: "scripts".into(),
            script: Some(legacy.to_owned()),
            ..NotesSettings::default()
        }
    } else {
        NotesSettings::default()
    }
}
pub fn settings() -> io::Result<NotesSettings> {
    super::Config::load()?
        .notes
        .ok_or_else(|| io::Error::other("Missing notes settings"))
}
pub fn save_settings(baseline: &NotesSettings, s: &NotesSettings) -> io::Result<()> {
    if !["app", "scripts", "off"].contains(&s.engine.as_str()) {
        return Err(super::store_io::invalid("Unknown notes engine"));
    }
    if s.engine == "scripts"
        && s.script
            .as_ref()
            .is_none_or(|p| !p.is_absolute() || !p.is_file())
    {
        return Err(super::store_io::invalid(
            "Choose an existing script with an absolute path",
        ));
    }
    if s.model.chars().count() > 200
        || s.model.chars().any(char::is_control)
        || s.your_name.chars().count() > 200
        || s.your_name.contains('\0')
    {
        return Err(super::store_io::invalid("Invalid notes model or name"));
    }
    super::Config::save_notes(&super::state_dir(), baseline, s)
}
pub fn detect() -> Vec<(Helper, bool)> {
    [Helper::Claude, Helper::Codex, Helper::OpenCode]
        .into_iter()
        .map(|h| (h, super::runners::installed(&helper::runner(h, ""))))
        .collect()
}
pub fn status(meeting_id: &str) -> Option<JobState> {
    queue::read(&super::state_dir(), meeting_id)
        .ok()
        .flatten()
        .map(|j| j.state)
}
pub fn request(meeting_id: &str, mode: Request) -> io::Result<()> {
    let settings = settings()?;
    if settings.engine() != Engine::App {
        return Err(super::store_io::invalid(
            "Choose the app notes engine first",
        ));
    }
    queue::enqueue_in(
        &super::state_dir(),
        &super::meetings_dir(),
        meeting_id,
        Some(mode),
        &settings,
    )?;
    queue::start_worker()
}
pub fn test_helper(h: Helper, model: &str) -> Result<Duration, String> {
    use helper::Run;
    // Public-domain words, fixed in code; never reads a meeting or the glossary.
    let transcript = "[00:00] Speaker: Four score and seven years ago our fathers brought forth on this continent, a new nation, conceived in Liberty, and dedicated to the proposition that all men are created equal.";
    let date = chrono::NaiveDate::from_ymd_opt(1863, 11, 19).unwrap();
    let sample = format!(
        "{}\n{}\n{}\n<meeting>\nTitle: Public-domain sample\nDate: 1863-11-19\nTranscript: {transcript}\n</meeting>",
        super::agent_instructions::core_head(super::agent_instructions::Agent::Notes),
        super::agent_instructions::default_base(super::agent_instructions::Agent::Notes),
        super::agent_instructions::core_tail(super::agent_instructions::Agent::Notes)
    );
    let start = std::time::Instant::now();
    let raw = helper::Cli { root: None }
        .run(h, model, sample, helper::NOTES_TIMEOUT)
        .map_err(|e| {
            match e {
                #[cfg(target_os = "linux")]
                helper::Failure::NetworkUnavailable => crate::core::runners::NETWORK_UNAVAILABLE,
                helper::Failure::Missing => "The agent was not found on this PC",
                helper::Failure::TimedOut => "The agent test timed out",
                _ => "The agent test failed; check its sign-in",
            }
            .to_string()
        })?;
    schema::parse(&raw, "1-1", date, transcript, &[])
        .map_err(|_| "The agent test returned invalid JSON".to_string())?;
    Ok(start.elapsed())
}

#[cfg(test)]
mod tests;
