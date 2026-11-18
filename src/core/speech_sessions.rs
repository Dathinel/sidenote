//! Durable intake only. Accepted pieces stay in flight until the speech worker handles them.
use super::{meeting_watch::wav_header, store_io};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io, path::PathBuf};
pub const MAX_PIECE_BYTES: usize = 10_000_000;
const MAX_IN_FLIGHT: usize = 4;
const SESSION_BYTES: u64 = 40_000_000;
#[derive(Debug)]
pub enum Error {
    Invalid,
    Missing,
    Conflict,
    Full(usize),
    Storage,
}
impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::Storage
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Invalid
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub client_session: String,
    pub meeting_id: String,
    pub languages: Vec<String>,
    pub language_mode: String,
    pub config_sha256: String,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceRequest {
    pub track: String,
    pub index: u32,
    pub origin_sample: i64,
    pub start_sample: u64,
    pub end_sample: u64,
    pub context_start_sample: u64,
    pub sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Accepted {
    pub metadata: PieceRequest,
    pub bytes: u64,
    #[serde(default)]
    pub done: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub owner: String,
    pub request: SessionRequest,
    pub pieces: BTreeMap<String, Accepted>,
    pub finished: Option<usize>,
}
type Sessions = BTreeMap<String, Session>;
pub struct Store(pub PathBuf);
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
impl PieceRequest {
    /// Canonical wire key: SHA-256 of this JSON tuple, in the design's field order.
    pub fn id(&self, session: &SessionRequest) -> String {
        digest(
            &serde_json::to_vec(&(
                &session.meeting_id,
                &self.track,
                self.index,
                self.start_sample,
                self.end_sample,
                &self.sha256,
                &session.config_sha256,
            ))
            .expect("scalar tuple"),
        )
    }
    fn validate(&self, bytes: &[u8]) -> Result<(), Error> {
        let header = wav_header(&mut io::Cursor::new(bytes), bytes.len() as u64)
            .map_err(|_| Error::Invalid)?;
        let samples = u64::from(header.declared_data_bytes) / 2;
        if bytes.len() > MAX_PIECE_BYTES
            || !matches!(self.track.as_str(), "system" | "microphone")
            || self.end_sample <= self.start_sample
            || self.context_start_sample > self.start_sample
            || self.start_sample - self.context_start_sample > 32_000
            || self.end_sample - self.start_sample > 4_800_000
            || self.end_sample - self.context_start_sample != samples
            || i64::try_from(self.end_sample)
                .ok()
                .and_then(|n| self.origin_sample.checked_add(n))
                .is_none()
            || header.encoding_tag != 1
            || header.channels != 1
            || header.sample_rate != 16_000
            || header.bits_per_sample != 16
            || header.block_align != 2
            || header.byte_rate != 32_000
            || u64::from(header.declared_data_bytes) > header.visible_data_bytes
            || header.declared_data_bytes % 2 != 0
            || u64::from(header.riff_bytes) + 8 != bytes.len() as u64
            || !hex(&self.sha256)
            || digest(bytes) != self.sha256
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Store {
    pub(crate) fn load(&self) -> Result<(std::fs::File, Sessions), Error> {
        let lock = store_io::lock(&self.0.join("intake.lock"))?;
        let data = match store_io::read_bounded(&self.0.join("sessions.json"), 1_048_576) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        self.recover(&data)?;
        Ok((lock, data))
    }
    fn recover(&self, sessions: &Sessions) -> Result<(), Error> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let root = store_io::checked_parent(&self.0.join("sessions.json"))?;
        for entry in std::fs::read_dir(&root.path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if hex(&name) {
                let pieces = sessions.get(&name).map(|s| &s.pieces);
                let dir = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                    .open(entry.path())?;
                for file in std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
                    let file = file?;
                    let filename = file.file_name();
                    let filename = filename.to_string_lossy();
                    if filename.ends_with(".tmp")
                        || filename
                            .strip_suffix(".wav")
                            .is_some_and(|p| !pieces.is_some_and(|s| s.contains_key(p)))
                    {
                        std::fs::remove_file(file.path())?;
                    }
                }
            } else if name.ends_with(".tmp") {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
    pub(crate) fn save(&self, sessions: &Sessions) -> Result<(), Error> {
        let bytes = serde_json::to_vec(sessions)?;
        if bytes.len() > 1_048_576 {
            return Err(Error::Full(MAX_IN_FLIGHT + 1));
        }
        store_io::atomic_write(&self.0.join("sessions.json"), &bytes)?;
        Ok(())
    }
    pub fn create(&self, owner: &str, request: SessionRequest) -> Result<(String, bool), Error> {
        if !label(&request.client_session)
            || !label(&request.meeting_id)
            || !hex(&request.config_sha256)
            || request.language_mode != "auto"
            || request.languages.is_empty()
            || request.languages.len() > 3
            || request
                .languages
                .iter()
                .any(|l| !matches!(l.as_str(), "en" | "hi" | "te"))
        {
            return Err(Error::Invalid);
        }
        let id = digest(&serde_json::to_vec(&(owner, &request.client_session))?);
        let (_lock, mut sessions) = self.load()?;
        if let Some(session) = sessions.get(&id) {
            return if session.request == request {
                Ok((id, false))
            } else {
                Err(Error::Conflict)
            };
        }
        if sessions.len() >= 128 {
            return Err(Error::Full(MAX_IN_FLIGHT + 1));
        }
        sessions.insert(
            id.clone(),
            Session {
                owner: owner.into(),
                request,
                pieces: BTreeMap::new(),
                finished: None,
            },
        );
        self.save(&sessions)?;
        Ok((id, true))
    }
    pub fn put(
        &self,
        owner: &str,
        id: &str,
        piece: &str,
        metadata: PieceRequest,
        bytes: &[u8],
    ) -> Result<(bool, usize), Error> {
        metadata.validate(bytes)?;
        let (_lock, mut sessions) = self.load()?;
        let session = sessions
            .get(id)
            .filter(|s| s.owner == owner)
            .ok_or(Error::Missing)?;
        if !hex(piece) || metadata.id(&session.request) != piece {
            return Err(Error::Invalid);
        }
        if let Some(accepted) = session.pieces.get(piece) {
            return if accepted.metadata == metadata {
                Ok((false, 0))
            } else {
                Err(Error::Conflict)
            };
        }
        if session.finished.is_some()
            || session
                .pieces
                .values()
                .any(|p| p.metadata.track == metadata.track && p.metadata.index == metadata.index)
        {
            return Err(Error::Conflict);
        }
        let queued: usize = sessions
            .values()
            .map(|s| s.pieces.values().filter(|p| !p.done).count())
            .sum();
        let owned: usize = sessions
            .values()
            .filter(|s| s.owner == owner)
            .map(|s| s.pieces.values().filter(|p| !p.done).count())
            .sum();
        if queued >= MAX_IN_FLIGHT
            || owned >= 2
            || session.pieces.values().map(|p| p.bytes).sum::<u64>() + bytes.len() as u64
                > SESSION_BYTES
        {
            return Err(Error::Full(queued + 1));
        }
        let path = self.0.join(id).join(format!("{piece}.wav"));
        store_io::atomic_write(&path, bytes)?;
        sessions.get_mut(id).ok_or(Error::Missing)?.pieces.insert(
            piece.into(),
            Accepted {
                metadata,
                bytes: bytes.len() as u64,
                done: false,
            },
        );
        self.save(&sessions)?;
        Ok((true, queued + 1))
    }
    pub fn finish(&self, owner: &str, id: &str, total: usize) -> Result<(), Error> {
        let (_lock, mut sessions) = self.load()?;
        let session = sessions
            .get_mut(id)
            .filter(|s| s.owner == owner)
            .ok_or(Error::Missing)?;
        if total != session.pieces.len() || session.finished.is_some_and(|n| n != total) {
            return Err(Error::Conflict);
        }
        session.finished = Some(total);
        self.save(&sessions)
    }
}
#[cfg(test)]
pub(super) mod tests {
    use super::*;
    thread_local! { static FAULT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
    pub(crate) fn interrupt(
        path: &std::path::Path,
        file: Option<&mut std::fs::File>,
    ) -> io::Result<()> {
        use std::io::Write;
        let ledger = path.file_name().unwrap() == "sessions.json";
        let stage = FAULT.get();
        if matches!(
            (stage, ledger, file.is_some()),
            (1, false, true) | (2, true, false) | (3 | 4, true, true)
        ) {
            if let Some(file) = file {
                file.write_all(b"partial")?;
            }
            if stage == 4 {
                return Err(io::Error::other("interrupted ledger write"));
            }
            panic!("simulated interruption {stage}");
        }
        Ok(())
    }
    #[test]
    fn durable_intake_validates_before_acceptance() {
        let root = tempfile::tempdir().unwrap();
        let store = Store(root.path().join("speech"));
        let request = SessionRequest {
            client_session: "synthetic".into(),
            meeting_id: "public".into(),
            languages: vec!["hi".into()],
            language_mode: "auto".into(),
            config_sha256: "a".repeat(64),
        };
        let (id, fresh) = store.create("owner", request.clone()).unwrap();
        assert!(fresh);
        assert_eq!(
            store.create("owner", request.clone()).unwrap(),
            (id.clone(), false)
        );
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data\x02\0\0\0\0\0".to_vec();
        let metadata = PieceRequest {
            track: "system".into(),
            index: 0,
            origin_sample: 0,
            start_sample: 0,
            end_sample: 1,
            context_start_sample: 0,
            sha256: digest(&wav),
        };
        let piece = metadata.id(&request);
        assert!(
            store
                .put("owner", &id, &piece, metadata.clone(), &wav)
                .unwrap()
                .0
        );
        let restarted = Store(store.0.clone());
        assert!(
            !restarted
                .put("owner", &id, &piece, metadata.clone(), &wav)
                .unwrap()
                .0
        );
        let mut leftovers = Vec::new();
        for stage in 1..=4 {
            let mut next = metadata.clone();
            next.index = stage;
            let next_id = next.id(&request);
            FAULT.set(stage);
            let result = std::panic::catch_unwind(|| store.put("owner", &id, &next_id, next, &wav));
            FAULT.set(0);
            assert!(result.is_err() || matches!(result, Ok(Err(Error::Storage))));
            let restarted = Store(store.0.clone());
            restarted.create("owner", request.clone()).unwrap();
            let files = std::fs::read_dir(store.0.join(&id)).unwrap().count();
            leftovers.push((stage, files, std::fs::read_dir(&store.0).unwrap().count()));
            assert_eq!(
                std::fs::read(store.0.join(&id).join(format!("{piece}.wav"))).unwrap(),
                wav
            );
        }
        assert_eq!(leftovers, [(1, 1, 3), (2, 1, 3), (3, 1, 3), (4, 1, 3)]);
        wav[44] ^= 1;
        assert!(matches!(
            restarted.put("owner", &id, &piece, metadata.clone(), &wav),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            restarted.finish("other", &id, 1),
            Err(Error::Missing)
        ));
        assert!(matches!(
            restarted.finish("owner", &id, 2),
            Err(Error::Conflict)
        ));
        restarted.finish("owner", &id, 1).unwrap();
    }
}
