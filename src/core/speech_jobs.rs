//! Durable per-session event sequence. Publication shares intake recovery's lock.
use super::{
    speech_sessions::{Error, Store},
    store_io,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io;
pub(crate) static STOP: std::sync::LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    std::sync::LazyLock::new(|| Default::default());
#[cfg(any(feature = "asr-runtime", test))]
mod runner;
#[cfg(feature = "asr-runtime")]
pub(crate) use runner::process;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub event: String,
    pub data: Value,
}
impl Store {
    pub fn events(&self, owner: &str, session: &str, after: u64) -> Result<Vec<Event>, Error> {
        let (_lock, sessions) = self.load()?;
        sessions
            .get(session)
            .filter(|s| s.owner == owner)
            .ok_or(Error::Missing)?;
        Ok(self
            .log(session)?
            .into_iter()
            .filter(|e| e.seq > after)
            .collect())
    }
    fn log(&self, session: &str) -> Result<Vec<Event>, Error> {
        let bytes =
            match store_io::read_bounded(&self.0.join(session).join("events.ndjson"), 16 << 20) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e.into()),
            };
        let events: Vec<Event> = bytes
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .map(serde_json::from_slice)
            .collect::<Result<_, _>>()
            .map_err(|_| Error::Storage)?;
        if events
            .iter()
            .enumerate()
            .any(|(i, e)| e.seq != i as u64 + 1)
        {
            return Err(Error::Storage);
        }
        Ok(events)
    }
    pub(crate) fn emit(&self, session: &str, event: &str, data: Value) -> Result<(), Error> {
        // Intake recovery removes temporary files, so publication holds its lock too.
        let (_lock, sessions) = self.load()?;
        sessions.get(session).ok_or(Error::Missing)?;
        let mut events = self.log(session)?;
        if event != "host.status"
            && events.iter().any(|e| {
                e.event == event
                    && e.data["piece"] == data["piece"]
                    && e.data["range"] == data["range"]
            })
        {
            return Ok(());
        }
        events.push(Event {
            seq: events.len() as u64 + 1,
            event: event.into(),
            data,
        });
        let mut bytes = Vec::new();
        for event in &events {
            serde_json::to_writer(&mut bytes, event)?;
            bytes.push(b'\n');
        }
        if bytes.len() > 16 << 20 {
            return Err(Error::Storage);
        }
        // Atomic replacement preserves the complete append-only sequence after interruption.
        store_io::atomic_write(&self.0.join(session).join("events.ndjson"), &bytes)?;
        Ok(())
    }
    pub fn snapshot(&self, owner: &str, session: &str, piece: &str) -> Result<Value, Error> {
        let (_lock, sessions) = self.load()?;
        sessions
            .get(session)
            .filter(|s| s.owner == owner && s.pieces.contains_key(piece))
            .ok_or(Error::Missing)?;
        let events = self.log(session)?;
        let mut state = "queued";
        let mut ranges = Vec::new();
        let mut failed = json!([]);
        for e in events.iter().filter(|e| e.data["piece"] == piece) {
            match e.event.as_str() {
                "piece.progress" => state = "running",
                "range.result" => ranges.push(e.data.clone()),
                "piece.done" => {
                    state = "done";
                    failed = e.data["failed"].clone();
                }
                "piece.failed" => {
                    state = "failed";
                    failed = json!([e.data]);
                }
                _ => {}
            }
        }
        Ok(json!({"state":state,"ranges":ranges,"failed":failed}))
    }
    pub(crate) fn tick(
        &self,
        mut process: impl FnMut(&str, &str, &super::speech_sessions::Session) -> super::asr::Result<()>,
    ) -> Result<(), Error> {
        let sessions = {
            let (_lock, sessions) = self.load()?;
            sessions
        };
        for (id, session) in &sessions {
            for (piece, accepted) in session.pieces.iter().filter(|(_, p)| !p.done) {
                if STOP.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                self.emit(
                    id,
                    "piece.accepted",
                    json!({"piece":piece,"track":accepted.metadata.track,
                    "index":accepted.metadata.index,"bytes":accepted.bytes,"queue_pos":1}),
                )?;
                let snapshot = self.snapshot(&session.owner, id, piece)?;
                let result = if snapshot["state"] != "done" && snapshot["state"] != "failed" {
                    process(id, piece, session)
                } else {
                    Ok(())
                };
                if STOP.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                if result.is_err() {
                    self.emit(
                        id,
                        "piece.failed",
                        json!({"piece":piece,"code":"ASR_UNAVAILABLE","retryable":false}),
                    )?;
                }
                let (_lock, mut current) = self.load()?;
                current
                    .get_mut(id)
                    .ok_or(Error::Missing)?
                    .pieces
                    .get_mut(piece)
                    .ok_or(Error::Missing)?
                    .done = true;
                self.save(&current)?;
            }
            if session.finished.is_some()
                && session.pieces.keys().all(|p| {
                    self.snapshot(&session.owner, id, p)
                        .is_ok_and(|v| v["state"] == "done" || v["state"] == "failed")
                })
            {
                self.emit(id, "session.done", json!({"pieces_total":session.finished}))?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speech_log_replays_deduplicates_and_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let store = Store(root.path().join("speech"));
        let request = super::super::speech_sessions::SessionRequest {
            client_session: "synthetic".into(),
            meeting_id: "public".into(),
            languages: vec!["hi".into()],
            language_mode: "auto".into(),
            config_sha256: "a".repeat(64),
        };
        let (session, _) = store.create("owner", request).unwrap();
        for event in [
            "piece.accepted",
            "piece.progress",
            "range.result",
            "piece.done",
        ] {
            store
                .emit(&session, event, json!({"piece":"public","range":"key"}))
                .unwrap();
        }
        store
            .emit(
                &session,
                "range.result",
                json!({"piece":"public","range":"key"}),
            )
            .unwrap();
        let restarted = Store(store.0.clone());
        let events = restarted.events("owner", &session, 2).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4]);
        assert!(matches!(
            store.events("other", &session, 0),
            Err(Error::Missing)
        ));
        store_io::atomic_write(&store.0.join(&session).join("events.ndjson"), b"partial").unwrap();
        assert!(matches!(
            store.events("owner", &session, 0),
            Err(Error::Storage)
        ));
    }
}
