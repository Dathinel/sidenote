use super::*;
use serde_json::Value;

fn decide(logits: &[f32], languages: &[String], guards: &Settings) -> Decision {
    decide_logits(logits, languages, MODEL_SHA256, CALIBRATION_VERSION, guards)
}

#[test]
fn s45b_frozen_replay_and_declared_order() {
    let fixture: Value = serde_json::from_str(include_str!("s45b.json")).unwrap();
    let guards = Settings {
        confidence_threshold: 0.0,
        margin_threshold: 0.0,
        ..Settings::default()
    };
    for (profile, spec) in fixture["profiles"].as_object().unwrap() {
        let mut declared: Vec<String> = profile.split('+').map(Into::into).collect();
        let mut counts = std::collections::BTreeMap::<(String, String), u64>::new();
        for row in fixture["rows"].as_array().unwrap() {
            let logits: Vec<f32> = serde_json::from_value(row["logits"].clone()).unwrap();
            let decision = decide(&logits, &declared, &guards);
            let Decision::Accepted {
                calibration,
                language,
                ..
            } = &decision
            else {
                panic!("{} / {profile}: {decision:?}", row["id"]);
            };
            assert_eq!(calibration.declared_set, *profile);
            assert_eq!(calibration.model_sha256, MODEL_SHA256);
            assert_eq!(calibration.version, CALIBRATION_VERSION);
            assert_eq!(*language, row["expected"][profile].as_str().unwrap());
            let class = row["language"].as_str().unwrap();
            *counts.entry((class.into(), language.clone())).or_default() += 1;
            // All six permutations for three labels; both orders for two.
            for _ in 0..declared.len() {
                declared.rotate_left(1);
                assert_eq!(decide(&logits, &declared, &guards), decision);
                declared.reverse();
                assert_eq!(decide(&logits, &declared, &guards), decision);
                declared.reverse();
            }
        }
        for (class, results) in spec["classes"].as_object().unwrap() {
            for language in &declared {
                let actual = counts
                    .get(&(class.clone(), language.clone()))
                    .copied()
                    .unwrap_or(0);
                assert_eq!(actual, results["confusion"][language].as_u64().unwrap());
            }
        }
        for (language, bias) in spec["biases"].as_object().unwrap() {
            assert_eq!(
                bias.as_f64().unwrap(),
                if language == "te" { -4.0 } else { 0.0 }
            );
        }
    }
}

#[test]
fn unknown_invalid_and_guarded_results_keep_baseline() {
    let pair = vec!["en".into(), "hi".into()];
    let guards = Settings::default();
    let logits = vec![0.0; 23];
    for declared in [
        vec![],
        vec!["hi".into(), "te".into()],
        vec!["en".into(), "hi+te".into()],
        vec!["en".into(), "en".into(), "hi".into()],
    ] {
        assert_eq!(
            decide(&logits, &declared, &guards),
            Decision::Unsure(Unsure::Uncalibrated)
        );
    }
    for (model, version) in [("unknown", CALIBRATION_VERSION), (MODEL_SHA256, "unknown")] {
        assert_eq!(
            decide_logits(&logits, &pair, model, version, &guards),
            Decision::Unsure(Unsure::Uncalibrated)
        );
    }
    for scores in [
        vec![],
        vec![0.0; 22],
        vec![f32::NAN; 23],
        vec![f32::INFINITY; 23],
        vec![f32::NEG_INFINITY; 23],
    ] {
        assert_eq!(
            decide(&scores, &pair, &guards),
            Decision::Unsure(Unsure::InvalidScores)
        );
    }
    let mut scores = logits;
    for (gap, confidence_threshold) in [(0.0, 0.0), (0.1, 0.0), (1.0, 0.9)] {
        let guards = Settings {
            confidence_threshold,
            ..guards.clone()
        };
        scores[4] = gap;
        let baseline = match decide(&scores, &pair, &guards) {
            Decision::Unsure(Unsure::GuardRejected) => "baseline",
            other => panic!("unexpected replacement: {other:?}"),
        };
        assert_eq!(baseline, "baseline");
    }
    scores[4] = f32::MAX;
    scores[6] = f32::MIN;
    assert!(matches!(
        decide(&scores, &pair, &guards),
        Decision::Accepted { .. }
    ));
    let invalid = Settings {
        margin_threshold: f32::NAN,
        ..guards
    };
    assert_eq!(
        decide(&scores, &pair, &invalid),
        Decision::Unsure(Unsure::GuardRejected)
    );
}
