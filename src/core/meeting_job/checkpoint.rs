//! Dormant engine contract; wiring adapters into the active queue is a later port.
use super::{Cause, Stage, claim};
use crate::core::{failure::AtStage, failure::Stage as Diagnostic, store_io};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

/// Order and key numbering are fixed for journal version 1. Legacy wire stages stay intact.
pub const STAGES: [Stage; 9] = [
    Stage::Discover,
    Stage::Prepare,
    Stage::Route,
    Stage::Transcribe,
    Stage::Translate,
    Stage::Notes,
    Stage::Tasks,
    Stage::Render,
    Stage::Publish,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Completed,
    NotAvailable,
    /// An attempted optional stage failed; retain the cause and continue required work.
    Failed {
        cause: Cause,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    /// Reconcile the stable key's durable receipt; never repeat an already committed effect.
    Recover,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    inputs: [u8; 32],
    completed: Vec<Availability>,
    started: bool,
    failure: Option<Cause>,
}
impl Journal {
    fn read(path: &Path, inputs: [u8; 32]) -> io::Result<Self> {
        // A refresh Scope caches old bytes across writes; decisions require current disk state.
        let journal: Self = match store_io::retry_sharing_violation(|| {
            crate::core::meeting_router::inputs::Snapshot::read_file(path, 8192)
        }) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::from)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self {
                version: 1,
                inputs,
                completed: Vec::new(),
                started: false,
                failure: None,
            },
            Err(e) => return Err(e),
        };
        if journal.version != 1
            || journal.inputs != inputs
            || journal.completed.len() > STAGES.len()
            || (journal.completed.len() == STAGES.len() && journal.started)
            || (journal.failure.is_some() && !journal.started)
            || journal
                .completed
                .iter()
                .zip(STAGES)
                .any(|(outcome, stage)| {
                    matches!(outcome, Availability::Failed { .. }) && !stage.is_optional()
                })
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(journal)
    }
}

/// Owns meeting-guards' claim through every stage and final publication.
/// Inputs must hash meeting identity, content, configuration, adapter versions and capabilities.
/// Keep old identities: changing A -> B -> A must reuse A's completed stages.
pub struct Job {
    _claim: File,
    path: PathBuf,
    journal: Journal,
}
impl Job {
    pub fn open(folder: &Path, inputs: [u8; 32]) -> io::Result<Self> {
        (|| {
            let claim = claim(folder).map_err(|e| {
                if matches!(e.raw_os_error(), Some(32 | 33)) {
                    io::ErrorKind::WouldBlock.into()
                } else {
                    e
                }
            })?;
            let path = folder
                .join(".sidenote-jobs")
                .join(format!("{}.json", hex::encode(inputs)));
            let journal = Journal::read(&path, inputs)?;
            Ok(Self {
                _claim: claim,
                path,
                journal,
            })
        })()
        .at(Diagnostic::MeetingJob)
    }

    pub fn next_stage(&self) -> Option<Stage> {
        STAGES.get(self.journal.completed.len()).copied()
    }

    pub fn completed(&self) -> &[Availability] {
        &self.journal.completed
    }

    pub fn failure(&self) -> Option<(Stage, Cause)> {
        self.next_stage().zip(self.journal.failure)
    }

    fn save(
        &mut self,
        journal: Journal,
        persist: &mut impl FnMut(&Path, &Journal) -> io::Result<()>,
    ) -> io::Result<()> {
        persist(&self.path, &journal)?;
        self.journal = journal;
        Ok(())
    }

    /// Returns true after settling one stage, false on completion or a required-stage failure.
    /// Missing adapters return NotAvailable, a durable nonfailure outcome.
    ///
    /// Start is issued at most once per stage/input identity. On Recover an adapter must
    /// inspect its key's receipt and finish interrupted work idempotently. It must not
    /// blindly repeat a nontransactional action. This coordinator cannot make external
    /// effects exactly-once; adapters must supply atomic publication or key deduplication.
    pub fn advance(
        &mut self,
        execute: impl FnOnce(Stage, &str, Action) -> Result<Availability, Cause>,
    ) -> io::Result<bool> {
        self.advance_inner(execute, store_io::write)
            .at(Diagnostic::MeetingJob)
    }

    fn advance_inner(
        &mut self,
        execute: impl FnOnce(Stage, &str, Action) -> Result<Availability, Cause>,
        mut persist: impl FnMut(&Path, &Journal) -> io::Result<()>,
    ) -> io::Result<bool> {
        // Reload even after a failed write: replacement may have succeeded before an IO error.
        self.journal = Journal::read(&self.path, self.journal.inputs)?;
        let Some(stage) = self.next_stage() else {
            return Ok(false);
        };
        let action = if self.journal.started {
            Action::Recover
        } else {
            Action::Start
        };
        if !self.journal.started {
            let mut journal = self.journal.clone();
            journal.started = true;
            self.save(journal, &mut persist)?;
        }
        let key = format!(
            "engine-1-{}-{}",
            hex::encode(self.journal.inputs),
            self.journal.completed.len()
        );
        let result = execute(stage, &key, action).and_then(|outcome| match outcome {
            Availability::Failed { cause } => Err(cause),
            outcome => Ok(outcome),
        });
        let mut journal = self.journal.clone();
        let outcome = match result {
            Ok(outcome) => Some(outcome),
            Err(cause) if stage.is_optional() => Some(Availability::Failed { cause }),
            Err(cause) => {
                journal.failure = Some(cause);
                None
            }
        };
        if let Some(outcome) = outcome {
            journal.completed.push(outcome);
            journal.started = false;
            journal.failure = None;
        }
        self.save(journal, &mut persist)?;
        Ok(outcome.is_some())
    }
}
