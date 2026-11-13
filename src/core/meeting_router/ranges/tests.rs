use super::*;
use crate::core::lid::{self, Unsure};
fn accepted(language: &str) -> Decision {
    let mut logits = [-20.0; 23];
    logits[lid::LANGUAGES.iter().position(|l| *l == language).unwrap()] = 20.0;
    lid::decide_logits(
        &logits,
        &["en".into(), "hi".into(), "te".into()],
        lid::MODEL_SHA256,
        lid::CALIBRATION_VERSION,
        &lid::Settings::default(),
    )
}
fn run(bounds: Span, spans: &[Span], decisions: &[Decision]) -> Vec<AsrRange> {
    let speech: Vec<_> = spans
        .iter()
        .zip(decisions)
        .map(|(&interval, decision)| Speech {
            interval,
            decision,
            speaker_turn: Some(("public", "turn")),
        })
        .collect();
    let out = plan(bounds, &speech).unwrap();
    assert!(out.windows(2).all(|w| w[0].context.1 <= w[1].context.0));
    for r in &out {
        assert!(bounds.0 <= r.context.0 && r.context.0 < r.context.1);
        assert!(r.context.1 <= bounds.1 && r.context.1 - r.context.0 <= CAP);
    }
    for (i, &span) in spans.iter().enumerate() {
        let mut cursor = span.0;
        for (_, owned) in out.iter().flat_map(|r| &r.members).filter(|m| m.0 == i) {
            assert_eq!(owned.0, cursor);
            cursor = owned.1;
        }
        assert_eq!(cursor, span.1);
    }
    out
}
#[test]
fn s45d_heldout_replay() {
    // All 50 clips: raw spans from saved probabilities via pinned Silero pad=0;
    // decisions and expected ranges from heldout-f/{segments,chunks}.json.
    type Clip = (String, u64, Vec<(u64, u64, String)>, serde_json::Value);
    let mut count = (0, 0);
    for line in include_str!("s45d.jsonl").lines() {
        let (id, samples, raw, expected): Clip = serde_json::from_str(line).unwrap();
        let spans: Vec<_> = raw.iter().map(|s| (s.0, s.1)).collect();
        let decisions: Vec<_> = raw.iter().map(|s| accepted(&s.2)).collect();
        let actual: Vec<_> = run((0, samples), &spans, &decisions)
            .into_iter()
            .map(|r| {
                let members: Vec<_> = r.members.iter().map(|m| m.0).collect();
                serde_json::json!([r.context.0, r.context.1, r.language, members])
            })
            .collect();
        assert_eq!(serde_json::json!(actual), expected, "{id}");
        count.0 += 1;
        count.1 += actual.len();
    }
    assert_eq!(count, (50, 98));
}
#[test]
fn edges_seams_changes_unsure_and_long_speech() {
    let en = accepted("en");
    let unsure = Decision::Unsure(Unsure::GuardRejected);
    let spans = [(0, 10), (21, 30), (40, 100)];
    assert!(run((0, 0), &[], &[]).is_empty());
    assert_eq!(
        padded((0, 100), &spans).unwrap(),
        [(0, 15), (16, 35), (35, 100)]
    );
    for middle in [accepted("hi"), unsure] {
        let out = run((0, 100), &spans, &[en.clone(), middle.clone(), en.clone()]);
        assert_eq!(out.len(), 3);
        assert_eq!(
            out[1].language.is_none(),
            matches!(middle, Decision::Unsure(_))
        );
    }
    for bounds in [(0, 4_800_000), (4_800_000, 9_600_000)] {
        let out = run(bounds, &[bounds], &[en.clone()]);
        assert_eq!(out.len(), 10);
    }
    for next in [None, Some(("b", "1")), Some(("a", "2"))] {
        let turns = [Some(("a", "1")), next];
        let speech: Vec<_> = spans[..2]
            .iter()
            .zip(turns)
            .map(|(&interval, speaker_turn)| Speech {
                interval,
                decision: &en,
                speaker_turn,
            })
            .collect();
        assert_eq!(plan((0, 100), &speech).unwrap().len(), 2);
    }
    assert!(padded((0, 100), &[(0, 20), (10, 30)]).is_err());
    assert!(padded((0, 100), &[(0, 101)]).is_err());
}
