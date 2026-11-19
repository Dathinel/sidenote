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
    Session(Option<String>),
    Lost,
    HostFailed,
    Local((u64, u64), Option<Vec<Value>>),
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

    pub fn create(path: PathBuf, piece: Piece) -> io::Result<Self> {
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

    pub fn session(&self) -> Option<&str> {
        self.log
            .changes
            .iter()
            .rev()
            .find_map(|change| match change {
                Change::Session(id) => Some(id.as_deref()),
                _ => None,
            })
            .flatten()
    }

    fn state(&self) -> Option<&Change> {
        self.log.changes.iter().rev().find(|change| {
            matches!(
                change,
                Change::Lost | Change::NewSession | Change::Done(_) | Change::HostFailed
            )
        })
    }

    pub fn lost(&self) -> bool {
        matches!(self.state(), Some(Change::Lost))
    }

    pub fn replacement(&self) -> bool {
        self.log
            .changes
            .iter()
            .any(|c| matches!(c, Change::NewSession))
    }

    pub fn host_failed(&self) -> bool {
        matches!(self.state(), Some(Change::HostFailed))
    }

    pub fn done(&self) -> bool {
        matches!(self.state(), Some(Change::Done(_) | Change::HostFailed))
    }

    pub fn range(&self, value: &Value) -> io::Result<String> {
        let piece = &self.log.cut;
        value["range"]
            .as_str()
            .filter(|s| key(s))
            .ok_or_else(invalid)?;
        let piece_id = value["piece"]
            .as_str()
            .filter(|s| key(s))
            .ok_or_else(invalid)?;
        let model = value["model_sha256"]
            .as_str()
            .filter(|s| key(s))
            .ok_or_else(invalid)?;
        let start = value["start_sample"].as_u64().ok_or_else(invalid)?;
        let end = value["end_sample"].as_u64().ok_or_else(invalid)?;
        if !matches!(value["language"].as_str(), Some("en" | "hi" | "te"))
            || value["route"] != value["language"]
            || value["track"] != piece.track
            || start < piece.start_sample
            || start >= end
            || end > piece.end_sample
        {
            return Err(invalid());
        }
        // L7 hashes a batch WAV path into its receipt key. Ownership survives a new session.
        Ok(super::upload::digest(&serde_json::to_vec(&(
            piece_id,
            &piece.track,
            start,
            end,
            &value["language"],
            model,
        ))?))
    }

    pub fn local(&self, span: (u64, u64)) -> bool {
        self.log
            .changes
            .iter()
            .any(|c| matches!(c, Change::Local(s, _) if *s == span))
    }

    pub fn finished_here(&self) -> bool {
        self.log
            .changes
            .iter()
            .any(|c| matches!(c, Change::Local(_, Some(_))))
    }

    pub fn missing(&self) -> Vec<(u64, u64)> {
        if !self.host_failed() {
            return Vec::new();
        }
        let piece = self.piece();
        let mut covered: Vec<_> = self
            .ranges()
            .values()
            .filter_map(|v| v["start_sample"].as_u64().zip(v["end_sample"].as_u64()))
            .chain(self.log.changes.iter().filter_map(|c| match c {
                Change::Local(span, Some(_)) => Some(*span),
                _ => None,
            }))
            .collect();
        covered.sort_unstable();
        let mut until = piece.start_sample;
        let mut missing = Vec::new();
        for (start, end) in covered
            .into_iter()
            .chain([(piece.end_sample, piece.end_sample)])
        {
            if start > until {
                missing.push((until, start));
            }
            until = until.max(end);
        }
        missing
    }

    pub(super) fn staged_receipts(&self) -> impl Serialize + '_ {
        let local: Vec<_> = self
            .log
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::Local(span, Some(readings)) => Some((span, readings)),
                _ => None,
            })
            .collect();
        (&self.log.cut, self.host_failed(), self.ranges(), local)
    }

    pub fn ranges(&self) -> BTreeMap<String, &Value> {
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
    let mut missing = Vec::new();
    for journal in journals {
        let origin = *origins
            .get(&journal.piece().track)
            .ok_or("Missing final track origin")?;
        let readings = journal.ranges();
        for (start, end) in journal.missing() {
            missing.push(
                serde_json::json!({"track":journal.piece().track,"start_sample":start,
                "end_sample":end,"origin_sample":origin}),
            );
        }
        for (id, value) in readings {
            ranges.entry(id).or_insert((origin, value, false));
        }
        for change in &journal.log.changes {
            if let Change::Local(_, Some(values)) = change {
                for value in values {
                    let id = journal.range(value)?;
                    ranges.entry(id).or_insert((origin, value, true));
                }
            }
        }
    }
    let mut revision =
        meeting_router::revision::assemble_host(ranges, config, source, baseline, generation)?;
    if !missing.is_empty() {
        revision.provenance["missing_ranges"] = missing.into();
    }
    Ok(revision)
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
        let mut journal = Journal::create(path.clone(), piece).unwrap();
        let key = "b".repeat(64);
        let mut range = json!({"piece":"d".repeat(64),"range":key,"track":"system","start_sample":0,"end_sample":16000,
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
