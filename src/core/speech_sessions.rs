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
#[derive(Serialize, Deserialize)]
struct Accepted {
    metadata: PieceRequest,
    bytes: u64,
}
#[derive(Serialize, Deserialize)]
struct Session {
    owner: String,
    request: SessionRequest,
    pieces: BTreeMap<String, Accepted>,
    finished: Option<usize>,
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
    fn load(&self) -> Result<(std::fs::File, Sessions), Error> {
        let lock = store_io::lock(&self.0.join("intake.lock"))?;
        let data = match store_io::read_bounded(&self.0.join("sessions.json"), 1_048_576) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok((lock, data))
    }
    fn save(&self, sessions: &Sessions) -> Result<(), Error> {
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
        let queued: usize = sessions.values().map(|s| s.pieces.len()).sum();
        let owned: usize = sessions
            .values()
            .filter(|s| s.owner == owner)
            .map(|s| s.pieces.len())
            .sum();
        if queued >= MAX_IN_FLIGHT
            || owned >= 2
            || session.pieces.values().map(|p| p.bytes).sum::<u64>() + bytes.len() as u64
                > SESSION_BYTES
        {
            return Err(Error::Full(queued + 1));
        }
        let path = self.0.join(id).join(format!("{piece}.wav"));
        // Replaces only an uncommitted orphan from a crash before the ledger publish.
        store_io::atomic_write(&path, bytes)?;
        sessions.get_mut(id).ok_or(Error::Missing)?.pieces.insert(
            piece.into(),
            Accepted {
                metadata,
                bytes: bytes.len() as u64,
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
mod tests {
    use super::*;
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
        let mut wav = b"RIFF".to_vec();
        wav.extend(38u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]);
        wav.extend(16_000u32.to_le_bytes());
        wav.extend(32_000u32.to_le_bytes());
        wav.extend([2, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(2u32.to_le_bytes());
        wav.extend([0, 0]);
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
