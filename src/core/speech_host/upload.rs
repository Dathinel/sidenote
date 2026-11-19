mod events;

use super::{
    Asr, Host, Reply, Request,
    journal::{Change, Journal, Piece, key},
    store_io,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

pub(in crate::core) const MAX_WAV: u64 = 10_000_000;

#[derive(Serialize)]
struct Session<'a> {
    client_session: String,
    meeting_id: &'a str,
    languages: &'a [String],
    language_mode: &'static str,
    config_sha256: &'a str,
}

pub(in crate::core) struct Upload<'a> {
    host: Host,
    settings: &'a Path,
    root: PathBuf,
    languages: Vec<String>,
    config: String,
    ready_at: std::time::Instant,
}

pub(in crate::core) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn remote_error() -> io::Error {
    io::Error::other("Speech host request did not complete")
}

impl<'a> Upload<'a> {
    pub(in crate::core) fn ready(
        host: Host,
        settings: &'a Path,
        root: PathBuf,
        languages: Vec<String>,
    ) -> Option<Self> {
        host.upload_since?;
        let (200, bytes) = host.get("/v1/host")? else {
            return None;
        };
        let reply: Reply = serde_json::from_slice(&bytes).ok()?;
        if reply.api != 1
            || reply.role != "server"
            || !matches!(reply.asr.state, Asr::Ready)
            || !reply.speech_jobs
        {
            return None;
        }
        let native: Vec<_> = languages
            .iter()
            .filter(|l| matches!(l.as_str(), "hi" | "te"))
            .collect();
        if native.is_empty() || native.iter().any(|l| !reply.asr.languages.contains(l)) {
            return None;
        }
        let config = digest(&serde_json::to_vec(&(&languages, reply.asr.models)).ok()?);
        Some(Self {
            host,
            settings,
            root,
            languages,
            config,
            ready_at: std::time::Instant::now(),
        })
    }

    pub(in crate::core) fn since(&self) -> i64 {
        self.host.upload_since.expect("only consented uploads")
    }

    fn still_allowed(&self) -> bool {
        if self.ready_at.elapsed() >= super::INTERVAL || crate::updates::listener_stopping() {
            return false;
        }
        Host::load(self.settings)
            .ok()
            .flatten()
            .is_some_and(|loaded| {
                self.host.recipient() == loaded.recipient()
                    && self.host.upload_since == loaded.upload_since
                    && self.host.token.as_slice() == loaded.token.as_slice()
            })
    }

    fn send(
        &self,
        path: &str,
        method: &str,
        headers: &str,
        body: &[u8],
    ) -> io::Result<(u32, Value)> {
        if !self.still_allowed() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let (status, data) = self
            .host
            .bytes(
                Request {
                    method,
                    path,
                    headers,
                    body,
                },
                16 << 20,
            )
            .ok_or_else(remote_error)?;
        Ok((status, serde_json::from_slice(&data).unwrap_or(Value::Null)))
    }

    pub(in crate::core) fn directory(&self, meeting: &str) -> PathBuf {
        self.root
            .join(meeting)
            .join(&self.config)
            .join(self.since().to_string())
    }

    pub(in crate::core) fn stage(
        &self,
        meeting: &str,
        journals: &[Journal],
        transcript: &[u8],
    ) -> io::Result<()> {
        let source = digest(transcript);
        let destination = self
            .directory(meeting)
            .join(format!("revision-{source}.dpapi"));
        if destination.exists() {
            return Ok(());
        }
        let baseline = crate::core::meeting_router::jobs::read_baseline(transcript)
            .map_err(|_| remote_error())?;
        let revision = super::journal::assemble(journals, &self.config, &source, &baseline, 0)
            .map_err(|_| remote_error())?;
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&revision)?);
        let encrypted = sidenote_sync::identity::protect(&bytes).map_err(|_| remote_error())?;
        store_io::atomic_write(&destination, &encrypted)
    }

    pub(in crate::core) fn piece(
        &self,
        meeting: &str,
        mut piece: Piece,
        wav: &Path,
    ) -> io::Result<Journal> {
        let _lock = store_io::lock(&self.root.join("upload.lock"))?;
        if !matches!(piece.track.as_str(), "microphone" | "system") {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let journal_path = self
            .directory(meeting)
            .join(format!("{}-{}.dpapi", piece.track, piece.index));
        let mut audio = None;
        let mut journal = match Journal::load(journal_path.clone())? {
            Some(journal) => {
                piece.sha256.clone_from(&journal.piece().sha256);
                if journal.piece() != &piece {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                journal
            }
            None => {
                let bytes = store_io::read_bounded(wav, MAX_WAV)?;
                piece.sha256 = digest(&bytes);
                audio = Some(bytes);
                Journal::open(journal_path, piece)?
            }
        };
        if journal.done() {
            return Ok(journal);
        }
        let piece = journal.piece();
        let id = digest(&serde_json::to_vec(&(
            meeting,
            &piece.track,
            piece.index,
            piece.start_sample,
            piece.end_sample,
            &piece.sha256,
            &self.config,
        ))?);
        let client_session = digest(&serde_json::to_vec(&(
            self.since(),
            &id,
            journal.session_generation(),
        ))?);
        let (status, reply) = self.send(
            "/v1/sessions",
            "POST",
            "Content-Type: application/json\r\n",
            &serde_json::to_vec(&Session {
                client_session,
                meeting_id: meeting,
                languages: &self.languages,
                language_mode: "auto",
                config_sha256: &self.config,
            })?,
        )?;
        let session = reply["session"]
            .as_str()
            .filter(|s| key(s))
            .ok_or_else(remote_error)?;
        if !matches!(status, 200 | 201) {
            return Err(remote_error());
        }
        let path = format!("/v1/sessions/{session}");
        let latest = reply["next_seq"].as_u64().ok_or_else(remote_error)?;
        let piece_path = format!("{path}/pieces/{id}");
        if status == 201 && journal.acknowledged() {
            journal.record(vec![Change::Ack(false), Change::Cursor(0)])?;
        }
        if !journal.acknowledged() {
            let headers = format!(
                "Content-Type: audio/wav\r\nX-Piece: {}\r\n",
                serde_json::to_string(journal.piece())?
            );
            let audio = match audio {
                Some(bytes) => bytes,
                None => store_io::read_bounded(wav, MAX_WAV)?,
            };
            if digest(&audio) != journal.piece().sha256 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let (code, result) = self.send(&piece_path, "PUT", &headers, &audio)?;
            if code == 429 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            if result["piece"] != id
                || !matches!(
                    (code, result["state"].as_str()),
                    (201, Some("accepted")) | (200, Some("have" | "done"))
                )
            {
                return Err(remote_error());
            }
            journal.record(vec![Change::Ack(true)])?;
        }
        let (code, _) = self.send(
            &format!("{path}/finish"),
            "POST",
            "Content-Type: application/json\r\n",
            br#"{"pieces_total":1}"#,
        )?;
        if code != 200 {
            return Err(remote_error());
        }
        self.consume(&path, &piece_path, &id, &mut journal, latest)?;
        Ok(journal)
    }
}
