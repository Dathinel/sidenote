use super::super::{batches::Range, ranges::AsrRange};
use super::*;
use serde_json::json;
pub(super) fn range() -> Range {
    Range {
        track: "microphone".into(),
        piece: "p".into(),
        wav: "unused.wav".into(),
        range: AsrRange {
            context: (8_000, 40_000),
            members: vec![(0, (16_000, 32_000))],
            language: Some("te".into()),
        },
        model_id: "indicconformer-te-ctc".into(),
        model_sha256: "a".repeat(64),
    }
}
#[test]
fn new_english_coverage_outside_baseline_is_selected() {
    let mut r = range();
    r.range.language = Some("en".into());
    r.range.context = (16_000, 32_000);
    let b = Chunk {
        id: "other".into(),
        start: 3.0,
        end: 4.0,
        track: r.track.clone(),
        speaker: None,
        turn: None,
        english: "Existing".into(),
        audio_start: None,
    };
    let mut t = english_fallback(
        std::slice::from_ref(&b),
        &Default::default(),
        "input".into(),
        "failed",
    );
    t.segments = vec![segment("r", &r, 0, &json!({"text":"New English"})).unwrap()];
    reconcile(&mut t, &[b]);
    assert_eq!(
        t.segments
            .iter()
            .map(|s| s.text_original.as_str())
            .collect::<Vec<_>>(),
        ["New English", "Existing"]
    );
}
#[test]
fn mapping_owns_tokens_and_keeps_segment_level_times() {
    let r = range();
    let text = segment("r", &r, 80_000, &json!({"text":"నమస్కారం"})).unwrap();
    assert_eq!((text.start, text.end), (6.0, 7.0));
    assert_eq!(
        (text.lang.as_str(), text.status.as_str()),
        ("te", "unresolved-context")
    );
    assert!(text.text_en.is_none());
    let mut owned = range();
    owned.range.context = (16_000, 32_000);
    assert_eq!(
        segment("r", &owned, 80_000, &json!({"text":"owned"}))
            .unwrap()
            .status,
        "segment-level"
    );
    let timed = json!({"text":"context native context", "timing":"validated-samples", "tokens":[
        {"start_sample":0,"end_sample":8000,"text":"context"},
        {"start_sample":8000,"end_sample":24000,"text":"నమస్కారం"},
        {"start_sample":24000,"end_sample":32000,"text":"context"}]});
    assert_eq!(
        segment("r", &r, 80_000, &timed).unwrap().text_original,
        "నమస్కారం"
    );
    let mut crossing = timed.clone();
    crossing["tokens"][0]["end_sample"] = 9000.into();
    assert!(segment("r", &r, 0, &crossing).is_err());
    assert!(segment("r", &r, -80_000, &timed).is_err());
}
#[test]
fn partial_and_one_sample_gaps_preserve_whole_baseline() {
    let r = range();
    let baseline = Chunk {
        id: "b".into(),
        start: 1.0,
        end: 3.0,
        track: r.track.clone(),
        speaker: None,
        turn: None,
        english: "Whole baseline".into(),
        audio_start: None,
    };
    let mut t = english_fallback(
        std::slice::from_ref(&baseline),
        &Default::default(),
        "source".into(),
        "failed",
    );
    t.segments = vec![segment("a", &r, 0, &json!({"text":"native"})).unwrap()];
    reconcile(&mut t, std::slice::from_ref(&baseline));
    assert_eq!(t.segments[0].text_original, "Whole baseline");
    let mut b = range();
    b.range.context = (32_001, 48_000);
    b.range.members = vec![(1, (32_001, 48_000))];
    t.segments = vec![
        segment("a", &r, 0, &json!({"text":"first"})).unwrap(),
        segment("b", &b, 0, &json!({"text":"second"})).unwrap(),
    ];
    reconcile(&mut t, &[baseline]);
    assert_eq!(t.segments.len(), 1);
    assert_eq!(t.segments[0].text_original, "Whole baseline");
}
