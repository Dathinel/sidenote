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

#[test]
fn standalone_sidecars_preserve_audio_and_cover_bypass_silence_and_missing_model() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("public-synthetic.wav");
    crate::core::asr::write_wav(&wav, &vec![0.1; 3 * 16000]).unwrap();
    let before = std::fs::read(&wav).unwrap();
    let request = Request {
        meeting_languages: pair(),
        jobs: vec![Job {
            id: "one".into(),
            wav: wav.clone(),
            chunks: vec![chunk("c", 0.0, 3.0)],
        }],
    };
    let run = |request: Request, enabled: bool, mode: &str| {
        stage::start(
            request,
            Settings::default(),
            enabled,
            mode.into(),
            dir.path().into(),
        )
        .unwrap()
        .join()
        .unwrap()
    };
    for (enabled, mode, status) in [
        (false, "auto", "disabled"),
        (true, "off", "off"),
        (true, "forced:hi", "forced"),
        (true, "auto", "detection_failed_unchecked"),
    ] {
        run(request.clone(), enabled, mode).unwrap();
        let sidecar: Sidecar = serde_json::from_slice(
            &std::fs::read(dir.path().join("one/language-detect.v1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(sidecar.chunks[0].guess.status, status);
        assert_eq!(sidecar.chunks[0].guess.confidence, None);
        assert!(!sidecar.thresholds_calibrated);
        let row = &sidecar.chunks[0];
        assert_eq!((row.start, row.end), (0.0, 3.0));
        assert_eq!(row.guess.chunk, "c");
        assert_eq!(row.track, "mic");
        assert_eq!(row.speaker.as_deref(), Some("a"));
        assert_eq!(row.turn.as_deref(), Some("one"));
        assert_eq!(row.pcm_f32le_sha256.is_some(), enabled && mode == "auto");
    }
    assert_eq!(std::fs::read(&wav).unwrap(), before);
    assert!(run(request.clone(), true, "forced:te").is_err());
    crate::core::asr::write_wav(&wav, &vec![0.0; 3 * 16000]).unwrap();
    run(request, true, "auto").unwrap();
    let sidecar: Sidecar = serde_json::from_slice(
        &std::fs::read(dir.path().join("one/language-detect.v1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(sidecar.chunks[0].guess.status, "silence_unchecked");
}

#[test]
fn labs_settings_preserve_existing_asr_configuration_identity() {
    let mut config = crate::core::Config::default();
    let serialize = |c: &crate::core::Config| serde_json::to_string(&c.pipeline).unwrap();
    let expected = r#"{"schema":1,"profile":"compat","language_mode":"auto","model_state_dir":"","runtime_dll":""}"#;
    assert_eq!(serialize(&config), expected);
    config.labs.language_detection = true;
    config.labs.language_detection_settings.confidence_threshold = 0.75;
    assert_eq!(serialize(&config), expected);
}
