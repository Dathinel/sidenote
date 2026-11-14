use super::*;
use crate::core::meeting_router::{
    batches::{self, EnglishBridge, NativeLanguage, NativeLoader, Range, Recognizer},
    pieces::{Piece, TrackManifest},
    ranges::AsrRange,
};
pub(crate) struct Fake;
impl Recognizer for Fake {
    fn recognize(&mut self, r: &Range) -> Result<serde_json::Value> {
        Ok(serde_json::json!({"text":r.piece}))
    }
}
impl NativeLoader for Fake {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(Fake))
    }
}
impl EnglishBridge for Fake {
    fn transcribe(&mut self, r: &Range) -> Result<serde_json::Value> {
        self.recognize(r)
    }
}

/// Produces real spooled plans/receipts, then invokes the same parent hand-off as run_pending.
pub(crate) fn complete(
    root: &Path,
    id: &str,
    baseline: &[Chunk],
    language: &str,
) -> Result<String> {
    produce(root, id, baseline, language)?;
    commit_batches(root, id, 7, "fixture-input", baseline)
}
fn produce(root: &Path, id: &str, baseline: &[Chunk], language: &str) -> Result<()> {
    let generation = 7;
    store_io::write(
        &queue(root).join(format!("{id}.json")),
        &Pending {
            id: id.into(),
            generation,
        },
    )?;
    revision::begin(destination(root, id).parent().unwrap(), generation)?;
    let dir = work(root, id).join("batches");
    fs::create_dir_all(&dir)?;
    let b = &baseline[0];
    let start = (b.start * 16000.0).round() as u64;
    let end = (b.end * 16000.0).round() as u64;
    let wav = dir.join("clip.wav");
    crate::core::asr::write_wav(&wav, &vec![0.0; (end - start) as usize])?;
    let text = match language {
        "te" => "తెలుగు selected revision",
        "hi" => "हिंदी later revision",
        _ => "English selected revision",
    };
    let plan = batches::spool(
        &dir,
        vec![Range {
            track: b.track.clone(),
            piece: text.into(),
            wav,
            range: AsrRange {
                context: (start, end),
                members: vec![(0, (start, end))],
                language: Some(language.into()),
            },
            model_id: format!("fake-{language}"),
            model_sha256: "a".repeat(64),
        }],
    )?;
    let manifest = PieceManifest {
        source_revision: "capture".into(),
        config_checksum: "config".into(),
        tracks: vec![TrackManifest {
            track: b.track.clone(),
            origin_sample: 0,
            sample_count: end,
            pieces: vec![Piece {
                id: text.into(),
                start_sample: 0,
                end_sample: end,
            }],
        }],
    };
    let output = BatchOutput {
        generation: generation.to_string(),
        source_revision: "fixture-input".into(),
        plan,
        manifest,
    };
    store_io::write(&dir.join("output.json"), &output)?;
    batches::run(&dir, &output.plan, &mut Fake, &mut Fake)
}
