//! Engine-neutral meeting job vocabulary. Discovery and retry scheduling belong to callers.
use serde::{Deserialize, Serialize};
use std::{fs::File, io, path::Path, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Pipeline,
    Prepare,
    Transcribe,
    Translate,
    Notes,
    Render,
    Publish,
    Tasks,
    Compare,
    Title,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cause {
    #[serde(rename = "HELPER_LIMIT")]
    Limit,
    #[serde(rename = "HELPER_AUTH")]
    Auth,
    #[serde(rename = "timeout")]
    TimedOut,
    #[serde(rename = "other")]
    Other,
}
impl Cause {
    pub fn message(self) -> &'static str {
        match self {
            Self::Limit => {
                "The summary agent has reached its usage limit. Your transcript is saved."
            }
            Self::Auth => "The summary agent needs you to sign in again. Your transcript is saved.",
            Self::TimedOut => {
                "Meeting processing took too long and was stopped. Your transcript is saved."
            }
            Self::Other => {
                "Meeting processing failed. Your transcript is saved. Check the notes setup before trying again."
            }
        }
    }
}

// This is the existing notes helper failure type, shared rather than translated into a rival type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Busy,
    LinuxUnavailable,
    Consent,
    Missing,
    TimedOut,
    Backend { exit: Option<i32> },
    BadOutput,
    Limit { exit: Option<i32> },
    Auth { exit: Option<i32> },
}

#[derive(Deserialize)]
pub struct Contract {
    pub version: u32,
    pub stages: Vec<Stage>,
    pub categories: Vec<Cause>,
    pub lock_file: String,
    pub summary_file: String,
    pub preserve_completed: bool,
}
pub fn contract() -> &'static Contract {
    static CONTRACT: OnceLock<Contract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../scripts/meeting-pipeline/job-contract.json"
        ))
        .expect("compiled meeting job contract")
    })
}

/// Hold through final publication. Never unlink the lock: the OS releases it on crash.
/// A busy claim is deferral, without consuming an attempt or replacing durable job state.
pub fn claim(folder: &Path) -> io::Result<File> {
    super::store_io::lock(&folder.join(&contract().lock_file))
}

/// Recheck under the claim after restart. Running is reclaimable, not proof of ownership.
/// Existing output completes automatic work; explicit replay alone may replace it.
pub fn preserve_summary(folder: &Path, explicit_replay: bool) -> bool {
    contract().preserve_completed
        && !explicit_replay
        && folder.join(&contract().summary_file).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contract_keeps_existing_vocabulary() {
        let c = contract();
        assert_eq!(c.version, 1);
        assert_eq!(c.stages.len(), 10);
        assert_eq!(
            c.categories,
            [Cause::Limit, Cause::Auth, Cause::TimedOut, Cause::Other]
        );
        assert_eq!(c.lock_file, ".notes-engine.lock");
        assert_eq!(c.summary_file, "english-summary.json");
        assert!(c.preserve_completed);
    }

    #[test]
    fn claim_serializes_engines_and_restart_preserves_completed_output() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path();
        let first = claim(folder).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                tx.send(claim(folder).is_err()).unwrap();
            });
        });
        assert!(rx.recv().unwrap());
        drop(first);
        let _restarted = claim(folder).unwrap();
        assert!(!preserve_summary(folder, false));
        std::fs::write(folder.join(&contract().summary_file), b"synthetic summary").unwrap();
        assert!(preserve_summary(folder, false));
        assert!(!preserve_summary(folder, true));
    }
}
