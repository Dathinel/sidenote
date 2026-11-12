use super::*;
use crate::core::meeting_router::Chunk;

fn probabilities(en: f32, hi: f32, te: f32) -> Vec<f32> {
    let mut scores = vec![0.0; 23];
    scores[4] = en;
    scores[6] = hi;
    scores[21] = te;
    scores
}
fn pair() -> Vec<String> {
    vec!["en".into(), "hi".into()]
}

#[test]
fn declared_pair_keeps_top_two_and_flags_unrestricted_winner() {
    let guess = decide(
        &probabilities(0.02, 0.08, 0.9),
        &pair(),
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(guess.language, "hi");
    assert!(guess.outside_meeting_languages);
    assert_eq!(guess.unrestricted_language.as_deref(), Some("te"));
    assert_eq!(guess.top_two.len(), 2);
    assert!((guess.confidence.unwrap() - 0.8).abs() < 0.001);
    assert!(guess.top_two.iter().all(|s| pair().contains(&s.language)));
}

#[test]
fn low_margin_is_uncertainty_not_a_mixed_label() {
    let guess = decide(
        &probabilities(0.51, 0.49, 0.0),
        &pair(),
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(guess.status, "uncertain");
    assert_eq!(guess.language, "en");
    assert!(guess.observed_languages.is_empty());
}

#[test]
fn malformed_scores_and_thresholds_are_rejected() {
    for scores in [vec![f32::NAN; 23], vec![0.0; 23], vec![1.0; 22]] {
        assert!(decide(&scores, &pair(), &Settings::default()).is_err());
    }
    assert!(
        Settings {
            margin_threshold: -1.0,
            ..Settings::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn missing_config_is_off_and_settings_round_trip() {
    let labs: crate::core::meeting_watch::Labs = serde_json::from_str("{}").unwrap();
    assert!(!labs.language_detection);
    let settings = Settings {
        confidence_threshold: 0.8,
        ..Settings::default()
    };
    let decoded: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(decoded.confidence_threshold, 0.8);
}

fn chunk(id: &str, start: f64, end: f64) -> Chunk {
    Chunk {
        id: id.into(),
        start,
        end,
        track: "mic".into(),
        speaker: Some("a".into()),
        turn: Some("one".into()),
        english: String::new(),
        audio_start: None,
    }
}

#[test]
fn short_chunks_never_inherit_across_speaker_or_turn_boundaries() {
    let prior = chunk("a", 0.0, 3.0);
    let mut next = chunk("b", 3.0, 4.0);
    let guess = decide(
        &probabilities(0.01, 0.99, 0.0),
        &pair(),
        &Settings::default(),
    )
    .unwrap();
    let previous = (&prior, guess);
    assert_eq!(stage::inherit(&next, Some(&previous), "en").language, "hi");
    next.speaker = Some("b".into());
    assert_eq!(
        stage::inherit(&next, Some(&previous), "en").confidence,
        None
    );
    next.speaker = Some("a".into());
    next.turn = None;
    assert_eq!(
        stage::inherit(&next, Some(&previous), "en").confidence,
        None
    );
    assert_eq!(stage::inherit(&next, None, "en").language, "en");
}

#[test]
fn mixed_window_preserves_observed_labels_without_claiming_a_classifier() {
    let guess = worker::aggregate(
        &[
            probabilities(0.99, 0.01, 0.0),
            probabilities(0.01, 0.99, 0.0),
        ],
        &pair(),
        &Settings::default(),
    )
    .unwrap();
    assert_eq!(guess.observed_languages, pair());
    assert_eq!(guess.status, "uncertain");
    assert_eq!(guess.top_two.len(), 2);
}
