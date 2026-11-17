//! Completion discovery only. Consumers must enqueue idempotently by meeting ID: a crash
//! after enqueue but before cursor persistence deliberately replays the completion.
use super::{
    meeting_router::inputs::{MeetingScan, Snapshot},
    store_io,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Offline,
    Disabled,
    Busy,
    InvalidInput,
    Incomplete,
    Input(io::ErrorKind),
    Cursor(io::ErrorKind),
    Dispatch(io::ErrorKind),
    Stopped,
}

/// Carries identity, never paths or audio. The queue independently captures its inputs.
pub struct CompletedMeeting(String, String);
impl CompletedMeeting {
    pub fn id(&self) -> &str {
        &self.0
    }

    pub fn capture(meetings: &Path, id: &str) -> Result<Self, Error> {
        if !valid_id(id) {
            return Err(Error::InvalidInput);
        }
        let folder = meetings.join(id);
        let before = revision::input(&folder).map_err(|e| Error::Input(e.kind()))?;
        let mut meeting = Self::capture_using(id, || Snapshot::capture(&folder))?;
        if revision::input(&folder).map_err(|e| Error::Input(e.kind()))? != before {
            return Err(Error::Incomplete);
        }
        meeting.1 = before;
        Ok(meeting)
    }
    pub(crate) fn capture_in(scan: &MeetingScan, id: &str) -> Result<Self, Error> {
        Self::capture_using(id, || scan.capture(id))
    }
    fn capture_using(
        id: &str,
        capture: impl FnOnce() -> io::Result<Snapshot>,
    ) -> Result<Self, Error> {
        if !valid_id(id) {
            return Err(Error::InvalidInput);
        }
        let input = capture().map_err(|e| Error::Input(e.kind()))?;
        let manifest: serde_json::Value = serde_json::from_slice(
            input
                .manifest
                .strip_prefix(b"\xEF\xBB\xBF")
                .unwrap_or(&input.manifest),
        )
        .map_err(|_| Error::InvalidInput)?;
        if manifest["status"] != "complete" {
            return Err(Error::Incomplete);
        }
        // Validate every segment, including a partially written final line.
        #[derive(Deserialize)]
        struct Segment {
            #[serde(rename = "text")]
            _text: String,
        }
        let text = std::str::from_utf8(&input.transcript).map_err(|_| Error::Incomplete)?;
        for line in text
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|s| !s.trim().is_empty())
        {
            serde_json::from_str::<Segment>(line).map_err(|_| Error::Incomplete)?;
        }
        Ok(Self(id.into(), String::new()))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// Completed inputs and an optional full listing, including incomplete inputs, for receipt retention.
pub type CatchUp = (Vec<CompletedMeeting>, Option<BTreeSet<String>>);

/// Adapters subscribe before catch-up here. Poll must replay unacknowledged IDs;
/// neither a timestamp high-water mark nor a transient event stream is sufficient.
pub trait Source {
    fn start(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn poll(&mut self) -> Result<Vec<CompletedMeeting>, Error>;
    fn catch_up(&mut self) -> Result<CatchUp, Error> {
        Ok((self.poll()?, None))
    }
    /// Adapters may avoid rereading inputs the session has already acknowledged.
    fn catch_up_excluding(
        &mut self,
        _delivered: &BTreeMap<String, String>,
    ) -> Result<CatchUp, Error> {
        self.catch_up()
    }
    fn stop(&mut self) {}
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    delivered: BTreeSet<String>,
    #[serde(default)]
    revisions: BTreeMap<String, String>,
}

#[cfg(windows)]
pub(crate) fn delivered(cursor: &Path) -> io::Result<BTreeMap<String, String>> {
    let _read = super::idle_pass::Span::begin("cursor_reads");
    Ok(store_io::read::<Cursor>(cursor)?.revisions)
}

pub(crate) mod revision;

pub(crate) fn input_changed(root: &Path, id: &str, revision: &str) -> bool {
    store_io::read::<Cursor>(&root.join("local/recording-source.json"))
        .map(|cursor| {
            cursor.delivered.contains(id)
                && cursor.revisions.get(id).map(String::as_str) != Some(revision)
        })
        .unwrap_or(true)
}

pub struct Session<S: Source> {
    source: S,
    cursor: PathBuf,
    stopped: bool,
}
impl<S: Source> Session<S> {
    pub fn open(mut source: S, cursor: PathBuf) -> Result<Self, Error> {
        // A corrupt cursor must fail closed, never silently reset deduplication.
        let _: Cursor = store_io::read(&cursor).map_err(|e| Error::Cursor(e.kind()))?;
        source.start()?;
        Ok(Self {
            source,
            cursor,
            stopped: false,
        })
    }

    /// Subscription/control availability remains observable while folder fallback runs.
    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn drain(
        &mut self,
        mut enqueue: impl FnMut(&CompletedMeeting) -> io::Result<()>,
    ) -> Result<(), Error> {
        if self.stopped {
            return Err(Error::Stopped);
        }
        let _lock = store_io::lock(&self.cursor.with_extension("lock"))
            .map_err(|e| Error::Cursor(e.kind()))?;
        let mut cursor: Cursor = {
            let _read = super::idle_pass::Span::begin("cursor_reads");
            store_io::read(&self.cursor).map_err(|e| Error::Cursor(e.kind()))?
        };
        let (meetings, retained) = self.source.catch_up_excluding(&cursor.revisions)?;
        if let Some(ids) = retained {
            let before = cursor.delivered.len();
            cursor.delivered.retain(|id| ids.contains(id));
            cursor.revisions.retain(|id, _| ids.contains(id));
            if cursor.delivered.len() != before {
                store_io::write(&self.cursor, &cursor).map_err(|e| Error::Cursor(e.kind()))?;
            }
        }
        for meeting in meetings {
            if cursor.revisions.get(meeting.id()) == Some(&meeting.1) {
                continue;
            }
            match enqueue(&meeting) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(Error::Dispatch(e.kind())),
            }
            cursor.revisions.insert(meeting.0.clone(), meeting.1);
            cursor.delivered.insert(meeting.0);
            store_io::write(&self.cursor, &cursor).map_err(|e| Error::Cursor(e.kind()))?;
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        if !self.stopped {
            self.source.stop();
            self.stopped = true;
        }
    }
}
impl<S: Source> Drop for Session<S> {
    fn drop(&mut self) {
        self.stop();
    }
}

pub mod events;
pub mod folders;
#[cfg(test)]
mod tests;
