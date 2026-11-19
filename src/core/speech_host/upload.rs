mod events;

use super::{
    Asr, Host, Reply, Request, WorkMode,
    journal::{Change, Journal, Piece, key},
    store_io,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

pub(in crate::core) const MAX_WAV: u64 = 10_000_000;

pub(in crate::core) struct Upload<'a> {
    host: Option<Host>,
    settings: &'a Path,
    root: PathBuf,
    languages: Vec<String>,
    online: Option<String>,
    offline: String,
    mode: WorkMode,
    since: i64,
    blocked: bool,
    ready_at: std::time::Instant,
}

pub(in crate::core) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn remote_error() -> io::Error {
    io::ErrorKind::ConnectionAborted.into()
}

impl<'a> Upload<'a> {
    pub(in crate::core) fn new(
        host: Option<Host>,
        settings: &'a Path,
        root: PathBuf,
        languages: Vec<String>,
    ) -> Option<Self> {
        let (mode, selected_at) = WorkMode::load(settings).ok()?;
        let (since, online) = if mode == WorkMode::ThisDevice {
            (selected_at, None)
        } else {
            let host = host.as_ref()?;
            (host.upload_since?, Self::probe_config(host, &languages))
        };
        let offline = digest(&serde_json::to_vec(&("offline", &languages)).ok()?);
        Some(Self {
            host,
            settings,
            root,
            languages,
            online,
            offline,
            mode,
            since,
            blocked: false,
            ready_at: std::time::Instant::now(),
        })
    }

    fn probe_config(host: &Host, languages: &[String]) -> Option<String> {
        let (200, bytes) = host.get("/v1/host")? else {
            return None;
        };
        let reply: Reply = serde_json::from_slice(&bytes).ok()?;
        if reply.api != 1
            || reply.role != "server"
            || !matches!(reply.asr.state, Asr::Ready)
            || !reply.speech_jobs
            || !reply.session_retirement
        {
            return None;
        }
        let mut native = languages
            .iter()
            .filter(|l| matches!(l.as_str(), "hi" | "te"))
            .peekable();
        if native.peek().is_none() || native.any(|l| !reply.asr.languages.contains(l)) {
            return None;
        }
        Some(digest(
            &serde_json::to_vec(&(languages, reply.asr.models)).ok()?,
        ))
    }

    pub(in crate::core) fn since(&self) -> i64 {
        self.since
    }

    fn host(&self) -> io::Result<&Host> {
        self.host.as_ref().ok_or_else(remote_error)
    }

    fn still_allowed(&self) -> bool {
        if self.ready_at.elapsed() >= super::INTERVAL || crate::updates::listener_stopping() {
            return false;
        }
        let Some(host) = &self.host else { return false };
        if !WorkMode::load(self.settings).is_ok_and(|(mode, _)| mode != WorkMode::ThisDevice) {
            return false;
        }
        Host::load(self.settings)
            .ok()
            .flatten()
            .is_some_and(|loaded| {
                host.recipient() == loaded.recipient()
                    && host.upload_since == loaded.upload_since
                    && host.token.as_slice() == loaded.token.as_slice()
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
            .host()?
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
        if status == 429 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        Ok((status, serde_json::from_slice(&data).unwrap_or(Value::Null)))
    }

    pub(in crate::core) fn directory(&self, meeting: &str) -> io::Result<PathBuf> {
        let root = self.root.join(meeting);
        let since = self.since.to_string();
        let offline = root.join(&self.offline).join(&since);
        if let Some(config) = &self.online {
            let target = root.join(config).join(&since);
            if offline.is_dir() && !target.exists() {
                let _lock = store_io::lock(&self.root.join("upload.lock"))?;
                std::fs::create_dir_all(root.join(config))?;
                std::fs::rename(&offline, &target)?;
                let _ = std::fs::remove_dir(root.join(&self.offline));
            }
            return Ok(target);
        }
        if self.mode != WorkMode::ThisDevice {
            let mut prior = std::fs::read_dir(&root)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_name().to_str().is_some_and(key))
                .map(|e| e.path().join(&since))
                .filter(|p| p.is_dir());
            if let Some(dir) = prior.next() {
                return if prior.next().is_none() {
                    Ok(dir)
                } else {
                    Err(remote_error())
                };
            }
        }
        Ok(offline)
    }

    pub(in crate::core) fn stage(
        &self,
        directory: &Path,
        journals: &[Journal],
        origins: &std::collections::BTreeMap<String, i64>,
        transcript: &[u8],
    ) -> io::Result<()> {
        let source = digest(transcript);
        let destination = directory.join(format!("revision-{source}.dpapi"));
        let config = directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .ok_or_else(remote_error)?;
        if destination.exists() {
            return Ok(());
        }
        let baseline = crate::core::meeting_router::jobs::read_baseline(transcript)
            .map_err(|_| remote_error())?;
        let revision = super::journal::assemble(journals, origins, config, &source, &baseline, 0)
            .map_err(|_| remote_error())?;
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&revision)?);
        let encrypted = sidenote_sync::identity::protect(&bytes).map_err(|_| remote_error())?;
        store_io::atomic_write(&destination, &encrypted)
    }

    pub(in crate::core) fn piece(
        &mut self,
        meeting: &str,
        directory: &Path,
        mut piece: Piece,
        wav: &Path,
        expired: impl Fn() -> bool,
        mut local: impl FnMut(&Journal, (u64, u64)) -> Option<Vec<Value>>,
    ) -> io::Result<Option<Journal>> {
        if !matches!(piece.track.as_str(), "microphone" | "system") {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let online = self.online.is_some() && !self.blocked;
        let journal_path = directory.join(format!("{}-{}.dpapi", piece.track, piece.index));
        if !online && self.mode != WorkMode::ThisDevice && !expired() && !journal_path.is_file() {
            return Ok(None);
        }
        let _lock = store_io::lock(&self.root.join("upload.lock"))?;
        let mut audio = None;
        let mut journal = match Journal::load(journal_path.clone())? {
            Some(journal) => {
                // A pre-existing cut keeps exactly the metadata previously sent.
                piece.origin_sample = journal.piece().origin_sample;
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
                Journal::create(journal_path, piece)?
            }
        };
        if !journal.done() && (expired() || self.mode == WorkMode::ThisDevice) {
            journal.record(vec![Change::HostFailed])?;
        }
        if online && journal.lost() {
            self.retire(&mut journal)?;
        }
        if !journal.done()
            && online
            && let Err(error) = self.transfer(meeting, wav, audio, &mut journal)
        {
            self.blocked = error.kind() == io::ErrorKind::WouldBlock;
            if journal.replacement()
                && matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionAborted | io::ErrorKind::WouldBlock
                )
            {
                journal.record(vec![Change::HostFailed])?;
            } else if !expired() {
                return Err(error);
            }
        }
        if !journal.done() && expired() {
            journal.record(vec![Change::HostFailed])?;
        }
        if self.mode != WorkMode::HostOnly {
            for span in journal.missing() {
                if !journal.local(span)
                    && !crate::updates::listener_stopping()
                    && WorkMode::load(self.settings).is_ok_and(|(mode, _)| mode == self.mode)
                {
                    journal.record(vec![Change::Local(span, None)])?;
                    if let Some(readings) = local(&journal, span) {
                        journal.record(vec![Change::Local(span, Some(readings))])?;
                    }
                }
            }
        }
        if online && !self.blocked {
            self.retire(&mut journal)?;
        }
        Ok(Some(journal))
    }

    fn transfer(
        &self,
        meeting: &str,
        wav: &Path,
        audio: Option<Vec<u8>>,
        journal: &mut Journal,
    ) -> io::Result<()> {
        let piece = journal.piece();
        let config = self.online.as_deref().ok_or_else(remote_error)?;
        let id = digest(&serde_json::to_vec(&(
            meeting,
            &piece.track,
            piece.index,
            piece.start_sample,
            piece.end_sample,
            &piece.sha256,
            config,
        ))?);
        let client_session = digest(&serde_json::to_vec(&(
            self.since(),
            &id,
            u8::from(journal.replacement()),
        ))?);
        let (status, reply) = self.send(
            "/v1/sessions",
            "POST",
            "Content-Type: application/json\r\n",
            &serde_json::to_vec(&json!({
                "client_session":client_session,"meeting_id":meeting,"languages":self.languages,
                "language_mode":"auto","config_sha256":config
            }))?,
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
        let mut changes = Vec::new();
        match journal.session() {
            Some(saved) if saved != session => return Err(remote_error()),
            None => changes.push(Change::Session(Some(session.to_owned()))),
            _ => {}
        }
        if status == 201 && journal.acknowledged() {
            if journal.replacement() {
                return Err(remote_error());
            }
            changes.extend([Change::Ack(false), Change::Cursor(0)]);
        }
        journal.record(changes)?;
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
        self.consume(&path, &piece_path, &id, journal, latest)
    }

    fn retire(&self, journal: &mut Journal) -> io::Result<()> {
        if (journal.done() || journal.lost())
            && let Some(id) = journal.session()
        {
            match self.send(&format!("/v1/sessions/{id}"), "DELETE", "", &[]) {
                Ok((204, _)) => {}
                // Keep the session receipt for cleanup on a later poll.
                _ if journal.host_failed() => return Ok(()),
                Err(error) => return Err(error),
                _ => return Err(remote_error()),
            }
            let mut changes = vec![Change::Session(None)];
            if journal.lost() {
                changes.extend([Change::Ack(false), Change::Cursor(0), Change::NewSession]);
            }
            journal.record(changes)?;
        }
        Ok(())
    }
}
