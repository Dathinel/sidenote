use super::tests::range;
use super::*;
use serde_json::json;
#[test]
fn untimed_neighbouring_context_is_evidence_not_selected_speech() {
    use super::super::{
        batches::{self, Range},
        pieces::{Piece, PieceManifest, TrackManifest},
        ranges::AsrRange,
    };
    struct Fake;
    impl batches::Recognizer for Fake {
        fn recognize(&mut self, _: &Range) -> Result<serde_json::Value> {
            Ok(json!({"text":"neighbour"}))
        }
    }
    impl batches::NativeLoader for Fake {
        fn load(
            &mut self,
            _: batches::NativeLanguage,
            _: &Range,
        ) -> Result<Box<dyn batches::Recognizer>> {
            Ok(Box::new(Fake))
        }
    }
    impl batches::EnglishBridge for Fake {
        fn transcribe(&mut self, r: &Range) -> Result<serde_json::Value> {
            batches::Recognizer::recognize(self, r)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let wav = root.path().join("clip.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 32_000]).unwrap();
    let ranges = [
        ("en", (8_000, 40_000), (16_000, 32_000)),
        ("hi", (24_000, 56_000), (32_000, 48_000)),
    ]
    .into_iter()
    .map(|(lang, context, owned)| Range {
        track: "system".into(),
        piece: "neighbour".into(),
        wav: wav.clone(),
        range: AsrRange {
            context,
            members: vec![(0, owned)],
            language: Some(lang.into()),
        },
        model_id: format!("fake-{lang}"),
        model_sha256: "a".repeat(64),
    })
    .collect();
    let plan = batches::spool(root.path(), ranges).unwrap();
    batches::run(root.path(), &plan, &mut Fake, &mut Fake).unwrap();
    let manifest = PieceManifest {
        source_revision: "capture".into(),
        config_checksum: "config".into(),
        tracks: vec![TrackManifest {
            track: "system".into(),
            origin_sample: 0,
            sample_count: 56_000,
            pieces: vec![Piece {
                id: "neighbour".into(),
                start_sample: 0,
                end_sample: 56_000,
            }],
        }],
    };
    let baseline: Vec<Chunk> = [
        (1.0, 2.0, "First owned sentence"),
        (2.0, 3.0, "Second owned neighbour"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (start, end, text))| {
        serde_json::from_value(
            json!({"id":format!("b-{i}"),"start":start,"end":end,"track":"system","english":text}),
        )
        .unwrap()
    })
    .collect();
    let data = assemble(root.path(), &plan, &manifest, &baseline, 7).unwrap();
    let text = data
        .transcript
        .segments
        .iter()
        .map(|s| s.text_original.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(text.matches("neighbour").count(), 1);
    assert!(data.transcript.segments.iter().all(|s| s.lang == "en"));
    assert_eq!(data.alternatives.len(), 2);
    assert!(
        data.alternatives
            .iter()
            .all(|s| s.status == "unresolved-context" && s.text_original == "neighbour")
    );
    assert_eq!(data.evidence.len(), 2);
}
#[cfg(windows)]
#[test]
fn receipts_out_of_order_cross_track_and_partial_failure_keep_coverage() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let wav = root.join("clip.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 32_000]).unwrap();
    let mut a = range();
    a.wav = wav.clone();
    a.range.context = (16_000, 32_000);
    let mut b = range();
    b.wav = wav.clone();
    b.range.context = (16_000, 32_000);
    b.track = "system".into();
    b.range.language = Some("hi".into());
    b.model_id = "indicconformer-hi-ctc".into();
    let mut failed = range();
    failed.wav = wav;
    failed.piece = "failed".into();
    failed.range.context = (48_000, 64_000);
    failed.range.members = vec![(1, (48_000, 64_000))];
    let plan = super::super::batches::spool(root, vec![failed, b, a]).unwrap();
    let mut baseline = Vec::new();
    for (id, r) in plan.batches.values().flat_map(|b| b.iter()).rev() {
        let start = r.range.members[0].1.0 as f64 / 16_000.0;
        baseline.push(Chunk {
            id: id.clone(),
            start,
            end: r.range.members[0].1.1 as f64 / 16_000.0,
            track: r.track.clone(),
            speaker: None,
            turn: None,
            english: format!("baseline {}", r.piece),
            audio_start: None,
            words: Vec::new(),
        });
        if r.piece == "failed" {
            continue;
        }
        let bytes =
            serde_json::to_vec(&json!({"text":if r.track=="system" {"हिंदी"} else {"తెలుగు"}}))
                .unwrap();
        std::fs::write(root.join(format!("{}-{id}.result", plan.revision)), &bytes).unwrap();
        std::fs::write(root.join(format!("{}-{id}.receipt",plan.revision)),json!({"revision":plan.revision,"range":id,"sha256":hex::encode(Sha256::digest(&bytes))}).to_string()).unwrap();
    }
    use super::super::pieces::{Piece, PieceManifest, TrackManifest};
    let mut manifest = PieceManifest {
        source_revision: "source".into(),
        config_checksum: "config".into(),
        tracks: ["system", "microphone"]
            .into_iter()
            .map(|track| TrackManifest {
                track: track.into(),
                origin_sample: 0,
                sample_count: 64_000,
                pieces: ["p", "failed"]
                    .into_iter()
                    .map(|id| Piece {
                        id: id.into(),
                        start_sample: 0,
                        end_sample: 64_000,
                    })
                    .collect(),
            })
            .collect(),
    };
    let merged = assemble(root, &plan, &manifest, &baseline, 1).unwrap();
    assert_eq!(merged.transcript.segments.len(), 3);
    assert_eq!(
        merged
            .transcript
            .segments
            .iter()
            .map(|s| s.lang.as_str())
            .collect::<Vec<_>>(),
        ["te", "hi", "en"]
    );
    assert_eq!(merged.evidence.len(), 2);
    assert_eq!(
        merged.transcript.segments[2].text_original,
        "baseline failed"
    );
    let incomplete: Vec<_> = baseline
        .iter()
        .filter(|b| b.english != "baseline failed")
        .cloned()
        .collect();
    assert!(assemble(root, &plan, &manifest, &incomplete, 1).is_err());
    manifest.tracks.clear();
    assert!(assemble(root, &plan, &manifest, &baseline, 1).is_err());
}
