use super::super::{meeting_router, store_io};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sidenote_sync::identity::{protect, unprotect};
use std::{collections::BTreeMap, io, path::PathBuf};
use zeroize::Zeroizing;

const LIMIT: u64 = 16 << 20;

#[derive(Deserialize, Serialize, PartialEq, Eq)]
pub struct Piece {
    pub track: String,
    pub index: u32,
    pub origin_sample: i64,
    pub start_sample: u64,
    pub end_sample: u64,
    pub context_start_sample: u64,
    pub sha256: String,
}

#[derive(Deserialize, Serialize)]
pub enum Change {
    Ack(bool),
    Range(Value),
    Done(Vec<Value>),
    Cursor(u64),
    NewSession,
}

#[derive(Deserialize, Serialize)]
struct Log {
    cut: Piece,
    changes: Vec<Change>,
}

/// A private, flushed sequence of receipts. The collector owns the WAV throughout.
pub struct Journal {
    path: PathBuf,
    log: Log,
}

fn invalid() -> io::Error {
    io::ErrorKind::InvalidData.into()
}

pub fn key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl Journal {
    pub fn load(path: PathBuf) -> io::Result<Option<Self>> {
        match store_io::read_bounded(&path, LIMIT) {
            Ok(bytes) => {
                let log = serde_json::from_slice(&unprotect(&bytes).map_err(|_| invalid())?)?;
                Ok(Some(Self { path, log }))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn open(path: PathBuf, piece: Piece) -> io::Result<Self> {
        if let Some(journal) = Self::load(path.clone())? {
            if journal.log.cut != piece {
                return Err(invalid());
            }
            return Ok(journal);
        }
        let journal = Self {
            path,
            log: Log {
                cut: piece,
                changes: Vec::new(),
            },
        };
        journal.persist()?;
        Ok(journal)
    }

    fn persist(&self) -> io::Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec(&self.log)?);
        let encrypted = protect(&bytes).map_err(|_| invalid())?;
        if encrypted.len() as u64 > LIMIT {
            return Err(invalid());
        }
        store_io::atomic_write(&self.path, &encrypted)
    }

    pub fn record(&mut self, changes: Vec<Change>) -> io::Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let before = self.log.changes.len();
        self.log.changes.extend(changes);
        let written = self.persist();
        if written.is_err() {
            self.log.changes.truncate(before);
        }
        written
    }

    pub fn piece(&self) -> &Piece {
        &self.log.cut
    }

    pub fn acknowledged(&self) -> bool {
        self.log
            .changes
            .iter()
            .rev()
            .find_map(|c| match c {
                Change::Ack(acked) => Some(*acked),
                _ => None,
            })
            .unwrap_or(false)
    }

    pub fn cursor(&self) -> u64 {
        self.log
            .changes
            .iter()
            .rev()
            .find_map(|c| match c {
                Change::Cursor(id) => Some(*id),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn session_generation(&self) -> usize {
        self.log
            .changes
            .iter()
            .filter(|c| matches!(c, Change::NewSession))
            .count()
    }

    pub fn done(&self) -> bool {
        self.log
            .changes
            .iter()
            .any(|c| matches!(c, Change::Done(_)))
    }

    pub fn range<'a>(&self, value: &'a Value) -> io::Result<&'a str> {
        let piece = &self.log.cut;
        let id = value["range"]
            .as_str()
            .filter(|s| key(s))
            .ok_or_else(invalid)?;
        let start = value["start_sample"].as_u64().ok_or_else(invalid)?;
        let end = value["end_sample"].as_u64().ok_or_else(invalid)?;
        if !matches!(value["language"].as_str(), Some("en" | "hi" | "te"))
            || value["route"] != value["language"]
            || !value["model_sha256"].as_str().is_some_and(key)
            || value["track"] != piece.track
            || start < piece.start_sample
            || start >= end
            || end > piece.end_sample
        {
            return Err(invalid());
        }
        Ok(id)
    }

    pub fn ranges(&self) -> BTreeMap<&str, &Value> {
        let mut ranges = BTreeMap::new();
        for rec in &self.log.changes {
            if let Change::Range(value) = rec
                && let Ok(id) = self.range(value)
            {
                ranges.entry(id).or_insert(value);
            }
        }
        ranges
    }
}

pub fn assemble<'a>(
    journals: impl IntoIterator<Item = &'a Journal>,
    origins: &BTreeMap<String, i64>,
    config: &str,
    source: &str,
    baseline: &[meeting_router::Chunk],
    generation: u128,
) -> super::super::asr::Result<meeting_router::revision::Revision> {
    let mut ranges = BTreeMap::new();
    for journal in journals {
        let origin = *origins
            .get(&journal.piece().track)
            .ok_or("Missing final track origin")?;
        for (id, value) in journal.ranges() {
            ranges.entry(id).or_insert((origin, value));
        }
    }
    meeting_router::revision::assemble_host(ranges, config, source, baseline, generation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn receipts_are_reopened_and_first_range_wins() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal.dpapi");
        let piece = Piece {
            track: "system".into(),
            index: 0,
            origin_sample: 0,
            start_sample: 0,
            end_sample: 16000,
            context_start_sample: 0,
            sha256: "a".repeat(64),
        };
        let mut journal = Journal::open(path.clone(), piece).unwrap();
        let key = "b".repeat(64);
        let mut range = json!({"range":key,"track":"system","start_sample":0,"end_sample":16000,
            "model_sha256":"c".repeat(64),"route":"hi","language":"hi","timing":"range","text":"first reading"});
        journal
            .record(vec![
                Change::Ack(true),
                Change::Range(range.clone()),
                Change::Cursor(1),
            ])
            .unwrap();
        range["text"] = "replayed reading".into();
        journal
            .record(vec![Change::Range(range), Change::Done(Vec::new())])
            .unwrap();
        let reopened = Journal::load(path.clone()).unwrap().unwrap();
        assert!(reopened.acknowledged());
        assert!(reopened.done());
        assert_eq!(reopened.cursor(), 1);
        assert_eq!(reopened.ranges().len(), 1);
        let revision = assemble(
            [&reopened],
            &[("system".into(), 0)].into(),
            "config",
            "source",
            &[],
            1,
        )
        .unwrap();
        assert_eq!(
            revision.transcript.segments[0].text_original,
            "first reading"
        );
        assert!(
            !std::fs::read(&path)
                .unwrap()
                .windows(13)
                .any(|part| part == b"first reading")
        );
    }
}
