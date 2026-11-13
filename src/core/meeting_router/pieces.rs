//! Header-only planning for finalized, already normalized capture tracks.
use super::{RoutingConfig, cache, inputs::Snapshot};
use crate::core::asr::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const SAMPLE_RATE: u64 = 16_000;
pub const PIECE_SAMPLES: u64 = 300 * SAMPLE_RATE;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    pub id: String,
    /// Half-open ownership in track-relative 16 kHz samples.
    pub start_sample: u64,
    pub end_sample: u64,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackManifest {
    pub track: String,
    /// Meeting-relative origin, retained even for an empty track.
    pub origin_sample: i64,
    pub sample_count: u64,
    pub pieces: Vec<Piece>,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PieceManifest {
    pub source_revision: String,
    pub config_checksum: String,
    pub tracks: Vec<TrackManifest>,
}

/// Origins are supplied by the capture contract, never inferred from WAV length.
/// The caller must normalize first; this planner never decodes or rewrites audio.
pub fn plan(
    folder: &Path,
    origins: &BTreeMap<String, i64>,
    config: &RoutingConfig,
) -> Result<PieceManifest> {
    let mut input = Snapshot::capture(folder)?;
    let manifest: serde_json::Value = serde_json::from_slice(&input.manifest)?;
    if manifest["status"] != "complete" {
        return Err("Piece planning requires a finalized capture".into());
    }
    config.validate()?;
    let config_checksum = cache::key(
        &[],
        &serde_json::to_string(&(config, &config.meeting_languages))?,
        "",
        "piece-config-v1",
    );
    let source_revision = input.revision()?;
    let mut tracks = Vec::new();
    for track in ["system", "microphone"] {
        let Some(header) = input.wav_header(track)? else {
            continue;
        };
        if header.encoding_tag != 1
            || header.channels != 1
            || u64::from(header.sample_rate) != SAMPLE_RATE
            || header.bits_per_sample != 16
            || header.block_align != 2
            || header.byte_rate != 32_000
            || u64::from(header.declared_data_bytes) > header.visible_data_bytes
            || header.declared_data_bytes % 2 != 0
        {
            return Err("Pieces require complete mono 16 kHz PCM16 WAV".into());
        }
        // The data chunk owns samples; subsequent WAV metadata never owns audio.
        let sample_count = u64::from(header.declared_data_bytes) / 2;
        let origin_sample = *origins.get(track).ok_or("Missing track origin")?;
        origin_sample
            .checked_add(i64::try_from(sample_count)?)
            .ok_or("Meeting sample offset overflow")?;
        let mut pieces = Vec::new();
        for start_sample in (0..sample_count).step_by(PIECE_SAMPLES as usize) {
            let end_sample = (start_sample + PIECE_SAMPLES).min(sample_count);
            let range = serde_json::to_string(&(track, origin_sample, start_sample, end_sample))?;
            pieces.push(Piece {
                id: cache::key(&[], &source_revision, &config_checksum, &range),
                start_sample,
                end_sample,
            });
        }
        tracks.push(TrackManifest {
            track: track.into(),
            origin_sample,
            sample_count,
            pieces,
        });
    }
    if tracks.is_empty() || origins.len() != tracks.len() {
        return Err("Supply exactly one origin per captured track".into());
    }
    Ok(PieceManifest {
        source_revision,
        config_checksum,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, io::Write};

    fn wav(path: &Path, samples: u32, rate: u32) {
        let bytes = samples * 2;
        let mut file = fs::File::create(path).unwrap();
        for chunk in [
            b"RIFF".to_vec(),
            (36 + bytes).to_le_bytes().to_vec(),
            b"WAVEfmt ".to_vec(),
            16u32.to_le_bytes().to_vec(),
            vec![1, 0, 1, 0],
            rate.to_le_bytes().to_vec(),
            (rate * 2).to_le_bytes().to_vec(),
            vec![2, 0, 16, 0],
            b"data".to_vec(),
            bytes.to_le_bytes().to_vec(),
        ] {
            file.write_all(&chunk).unwrap();
        }
        file.set_len(44 + u64::from(bytes)).unwrap();
    }
    fn fixture(counts: [u32; 2]) -> (tempfile::TempDir, BTreeMap<String, i64>) {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("manifest.json"),
            br#"{"status":"complete"}"#,
        )
        .unwrap();
        fs::write(root.path().join("transcript.ndjson"), b"").unwrap();
        for (track, count) in ["system", "microphone"].into_iter().zip(counts) {
            wav(&root.path().join(format!("{track}.wav")), count, 16_000);
        }
        (
            root,
            BTreeMap::from([("system".into(), 123), ("microphone".into(), 80_007)]),
        )
    }
    #[test]
    fn two_track_601_seconds_has_exact_stable_ownership() {
        let count = 601 * SAMPLE_RATE;
        let (root, origins) = fixture([count as u32; 2]);
        let config = RoutingConfig::default();
        let first = plan(root.path(), &origins, &config).unwrap();
        assert_eq!(first, plan(root.path(), &origins, &config).unwrap());
        let saved = serde_json::to_vec(&first).unwrap();
        assert_eq!(first, serde_json::from_slice(&saved).unwrap());
        let mut identities = BTreeSet::new();
        for track in &first.tracks {
            assert_eq!(track.origin_sample, origins[&track.track]);
            assert_eq!(track.sample_count, count);
            assert_eq!(track.pieces.len(), 3);
            let mut cursor = 0;
            for piece in &track.pieces {
                assert_eq!(piece.start_sample, cursor);
                assert!(piece.end_sample > piece.start_sample);
                assert!(piece.end_sample - piece.start_sample <= PIECE_SAMPLES);
                assert!(identities.insert(&piece.id));
                cursor = piece.end_sample;
            }
            assert_eq!(cursor, count);
            assert_eq!(track.pieces[2].start_sample, 600 * SAMPLE_RATE);
            assert_eq!(
                track.pieces[2].end_sample - track.pieces[2].start_sample,
                SAMPLE_RATE
            );
        }
        let mut changed = config.clone();
        changed.language_mode = "off".into();
        let changed = plan(root.path(), &origins, &changed).unwrap();
        assert_ne!(first.tracks[0].pieces[0].id, changed.tracks[0].pieces[0].id);
        let mut shifted = origins.clone();
        *shifted.get_mut("system").unwrap() += 1;
        let shifted = plan(root.path(), &shifted, &config).unwrap();
        assert_ne!(first.tracks[0].pieces[0].id, shifted.tracks[0].pieces[0].id);
        fs::write(
            root.path().join("transcript.ndjson"),
            b"{\"text\":\"synthetic\"}",
        )
        .unwrap();
        let revised = plan(root.path(), &origins, &config).unwrap();
        assert_ne!(first.tracks[0].pieces[0].id, revised.tracks[0].pieces[0].id);
    }
    #[test]
    fn empty_and_short_tracks_keep_origins_and_exact_samples() {
        let (root, origins) = fixture([0, 12_345]);
        let result = plan(root.path(), &origins, &RoutingConfig::default()).unwrap();
        assert_eq!(result.tracks[0].sample_count, 0);
        assert_eq!(result.tracks[0].origin_sample, 123);
        assert!(result.tracks[0].pieces.is_empty());
        let short = &result.tracks[1];
        assert_eq!(short.origin_sample, 80_007);
        assert_eq!(short.pieces.len(), 1);
        assert_eq!(
            (short.pieces[0].start_sample, short.pieces[0].end_sample),
            (0, 12_345)
        );
    }
    #[test]
    fn rejects_unfinalized_unnormalized_truncated_or_unlocated_tracks() {
        let (root, mut origins) = fixture([1, 2]);
        let config = RoutingConfig::default();
        origins.remove("system");
        assert!(plan(root.path(), &origins, &config).is_err());
        origins.insert("system".into(), 0);
        wav(&root.path().join("system.wav"), 1, 48_000);
        assert!(plan(root.path(), &origins, &config).is_err());
        wav(&root.path().join("system.wav"), 1, 16_000);
        fs::OpenOptions::new()
            .write(true)
            .open(root.path().join("system.wav"))
            .unwrap()
            .set_len(45)
            .unwrap();
        assert!(plan(root.path(), &origins, &config).is_err());
        wav(&root.path().join("system.wav"), 1, 16_000);
        fs::write(
            root.path().join("manifest.json"),
            br#"{"status":"recording"}"#,
        )
        .unwrap();
        assert!(plan(root.path(), &origins, &config).is_err());
    }
}
