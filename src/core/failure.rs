//! Content-free diagnostics: never retain error messages or paths.
use std::{fmt, io};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Stage {
    KbConfigEmpty,
    KbConfigInvalid,
    KbGit,
    KbSnapshot,
    CheckLock,
    CheckInput,
    CheckStorage,
    CheckRunner,
    CheckOutput,
    NotesSettingsRead,
    NotesExePath,
    NotesWorkerSpawn,
    CatchUpActivity,
    CatchUpSettings,
    CatchUpMeetings,
    CatchUpEnqueue,
    CatchUpScriptLock,
    CatchUpScriptCommand,
    CatchUpRouting,
}
impl Stage {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::KbConfigEmpty => "KB_CONFIG_EMPTY",
            Self::KbConfigInvalid => "KB_CONFIG_INVALID",
            Self::KbGit => "KB_GIT",
            Self::KbSnapshot => "KB_SNAPSHOT",
            Self::CheckLock => "CHECK_LOCK",
            Self::CheckInput => "CHECK_INPUT",
            Self::CheckStorage => "CHECK_STORAGE",
            Self::CheckRunner => "CHECK_RUNNER",
            Self::CheckOutput => "CHECK_OUTPUT",
            Self::NotesSettingsRead => "NOTES_SETTINGS_READ",
            Self::NotesExePath => "NOTES_EXE_PATH",
            Self::NotesWorkerSpawn => "NOTES_WORKER_SPAWN",
            Self::CatchUpActivity => "CATCH_UP_ACTIVITY",
            Self::CatchUpSettings => "CATCH_UP_SETTINGS",
            Self::CatchUpMeetings => "CATCH_UP_MEETINGS",
            Self::CatchUpEnqueue => "CATCH_UP_ENQUEUE",
            Self::CatchUpScriptLock => "CATCH_UP_SCRIPT_LOCK",
            Self::CatchUpScriptCommand => "CATCH_UP_SCRIPT_COMMAND",
            Self::CatchUpRouting => "CATCH_UP_ROUTING",
        }
    }
    pub(crate) fn error(self, error: io::Error) -> io::Error {
        if error.get_ref().is_some_and(|e| e.is::<Failure>()) {
            return error;
        }
        io::Error::new(
            error.kind(),
            Failure {
                stage: self,
                kind: error.kind(),
                os: error.raw_os_error(),
            },
        )
    }
}
#[derive(Debug)]
struct Failure {
    stage: Stage,
    kind: io::ErrorKind,
    /// The OS error number, kept so an `Uncategorized` kind (for example a
    /// Windows sharing or lock violation) can still be told apart in the log.
    os: Option<i32>,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.os {
            Some(code) => write!(f, "{} ({:?}, os {code})", self.stage.code(), self.kind),
            None => write!(f, "{} ({:?})", self.stage.code(), self.kind),
        }
    }
}
impl std::error::Error for Failure {}

pub(crate) trait AtStage<T> {
    fn at(self, stage: Stage) -> io::Result<T>;
}
impl<T> AtStage<T> for io::Result<T> {
    fn at(self, stage: Stage) -> io::Result<T> {
        self.map_err(|error| stage.error(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_drops_raw_content_and_preserves_the_original_boundary() {
        let error = Stage::KbGit.error(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "synthetic private text",
        ));
        let error = Stage::CheckStorage.error(error);
        assert_eq!(error.to_string(), "KB_GIT (PermissionDenied)");
    }
    #[test]
    fn stage_keeps_the_os_error_number() {
        let error = Stage::NotesSettingsRead.error(io::Error::from_raw_os_error(32));
        let text = error.to_string();
        assert!(text.starts_with("NOTES_SETTINGS_READ ("), "{text}");
        assert!(text.ends_with(", os 32)"), "{text}");
    }
}
