use super::no_speech_tests::Fixture;
use super::*;

fn record_coverage(f: &Fixture, start: f64, end: f64, lid_uncertain: bool) {
    let interval = json!({"track":"system","start":start,"end":end});
    let reasons = if lid_uncertain {
        json!([{"reason":"guard_rejected","interval":interval}])
    } else {
        json!([])
    };
    let mut speech = interval;
    speech["id"] = "uncertain".into();
    store_io::write(&f.work.join("coverage.json"), &[speech]).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &reasons).unwrap();
    store_io::write(
        &f.work.join("english-transport.json"),
        &json!({"ranges":{}}),
    )
    .unwrap();
}

fn uncertain_capture(transcript: &[u8]) -> Fixture {
    let mut f = Fixture::new(b"public", 0, b"");
    f.plan = batches::spool(&f.work, vec![]).unwrap();
    f.baseline = jobs::read_baseline(transcript).unwrap();
    store_io::atomic_write(&f.work.join("transcript.ndjson"), transcript).unwrap();
    record_coverage(&f, 1.0, 2.0, true);
    f
}

fn publish_capture(f: &Fixture) -> revision::Revision {
    let data = pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let dest = f.root.path().join("published");
    revision::begin(&dest, 1).unwrap();
    let chosen = revision::commit(&dest, &data).unwrap();
    revision::load(&dest, Some(&chosen)).unwrap().unwrap().1
}

#[test]
fn uncertain_capture_keeps_baseline_words_and_marks_only_the_uncovered_tail() {
    let f = uncertain_capture(
        br#"{"source":"system","start_ms":700,"end_ms":1500,"text":"public captured words"}"#,
    );
    let saved = publish_capture(&f);
    let [english, gap] = saved.transcript.segments.as_slice() else {
        panic!("expected the whole captured sentence and a gap");
    };
    assert_eq!((english.start, english.end), (0.7, 1.5));
    assert_eq!(english.text_original, "public captured words");
    assert!(english.status.starts_with("captured Hex retained:"));
    assert!(english.status.contains("Language confidence was too low"));
    assert_eq!(
        (gap.start, gap.end, gap.status.as_str()),
        (1.5, 2.0, "marked-gap")
    );
    assert_eq!(
        saved.provenance["marked_gaps"][0]["reasons"][0]["reason"],
        "guard_rejected"
    );
}

#[test]
fn uncertain_capture_over_silence_marks_a_gap_without_a_baseline_reply() {
    let f = uncertain_capture(
        br#"{"source":"system","start_ms":3000,"end_ms":3500,"text":"public later words"}"#,
    );
    let saved = publish_capture(&f);
    let gap = &saved.transcript.segments[0];
    assert_eq!(
        (gap.start, gap.end, gap.status.as_str()),
        (1.0, 2.0, "marked-gap")
    );
    assert_eq!(
        saved.transcript.segments[1].text_original,
        "public later words"
    );
    assert_eq!(
        saved.provenance["marked_gaps"][0]["baseline"]["code"],
        "no-overlapping-baseline-text"
    );
}

#[test]
fn production_saved_gap_preserves_explicit_baseline_reply_and_survives_load() {
    let mut f = Fixture::new(b"public", 0, b"");
    f.plan = batches::spool(&f.work, vec![]).unwrap();
    f.baseline.retain(|b| b.start != 1.0);
    let c: Chunk =
        serde_json::from_value(json!({"id":"uncertain","track":"system","start":1.0,"end":2.0}))
            .unwrap();
    store_io::write(&f.work.join("coverage.json"), &[c]).unwrap();
    store_io::write(
        &f.work.join("uncertainty.json"),
        &json!([
            {"reason":"guard_rejected","interval":{"track":"system","start":1.0,"end":2.0}}
        ]),
    )
    .unwrap();
    let mut rows: Value = serde_json::from_str(
        include_str!("../revision/baseline_no_speech_fixture.ndjson")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    rows["start_ms"] = 1000.into();
    rows["end_ms"] = 2000.into();
    let replies = revision::gaps::capture(&serde_json::to_vec(&rows).unwrap()).unwrap();
    store_io::write(&f.work.join("baseline-no-speech.json"), &replies).unwrap();
    store_io::write(
        &f.work.join("english-transport.json"),
        &json!({"ranges":{}}),
    )
    .unwrap();
    let data = pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    assert_eq!(data.provenance["marked_gaps"].as_array().unwrap().len(), 1);
    let lines = data.transcript.original_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.text.starts_with("[Speech unavailable: 1.000-2.000s;"))
    );
    let dest = f.root.path().join("published");
    revision::begin(&dest, 1).unwrap();
    let chosen = revision::commit(&dest, &data).unwrap();
    let saved = revision::load(&dest, Some(&chosen)).unwrap().unwrap().1;
    assert_eq!(
        saved.provenance["marked_gaps"],
        data.provenance["marked_gaps"]
    );
    let mut changed = data.clone();
    changed
        .transcript
        .segments
        .iter_mut()
        .find(|s| s.status == "marked-gap")
        .unwrap()
        .text_original = "invented speech".into();
    assert!(revision::commit(&dest, &changed).is_err());
    fs::remove_file(f.work.join("baseline-no-speech.json")).unwrap();
    let inferred = pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    assert_eq!(
        inferred.provenance["marked_gaps"][0]["baseline"]["code"],
        "no-overlapping-baseline-text"
    );
    store_io::write(&f.work.join("baseline-no-speech.json"), &replies).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    assert!(pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).is_err());
}

#[test]
fn production_gap_requires_a_verified_recognizer_no_speech_record() {
    let mut f = Fixture::new(b"\n", 0, b"");
    f.run(&mut super::no_speech_tests::NativeFake::default())
        .unwrap();
    let (id, range) = f.plan.batches["en"]
        .iter()
        .find(|(id, _)| {
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_none()
        })
        .unwrap();
    let (start, end) = range.range.context;
    let (start, end) = (start as f64 / 16000.0, end as f64 / 16000.0);
    f.baseline.retain(|b| b.end <= start || end <= b.start);
    record_coverage(&f, start, end, false);
    let saved = publish_capture(&f);
    assert_eq!(
        saved.provenance["marked_gaps"][0]["reasons"][0]["reason"],
        "no-speech"
    );
    fs::remove_file(f.work.join(format!("{}-{id}.fallback", f.plan.revision))).unwrap();
    assert!(pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).is_err());
}

#[test]
fn captured_sentence_can_straddle_an_independently_pinned_english_range() {
    let mut f = Fixture::new(b"public routed words", 0, b"");
    f.baseline = jobs::read_baseline(
        br#"{"source":"system","start_ms":1000,"end_ms":1500,"text":"public captured words"}"#,
    )
    .unwrap();
    record_coverage(&f, 1.1, 2.0, true);
    f.run(&mut super::no_speech_tests::NativeFake::default())
        .unwrap();
    let mut saved = publish_capture(&f);
    let overlaps = saved.provenance["assembly_coverage"]["overlaps"]
        .as_array()
        .unwrap();
    assert_eq!(overlaps.len(), 1);
    assert!(overlaps[0]["english_source"].is_string());
    for segment in saved
        .transcript
        .segments
        .iter_mut()
        .chain(&mut saved.alternatives)
    {
        if (segment.start, segment.end) == (0.9, 1.1) {
            segment.text_original = "changed after pinning".into();
        }
    }
    assert_eq!(
        revision::commit(&f.root.path().join("published"), &saved)
            .unwrap_err()
            .to_string(),
        "Invalid recorded fallback overlap evidence"
    );
}

fn notes_at_routed_boundary(capture: &[u8]) -> String {
    let mut f = Fixture::new(b"public routed words", 0, b"");
    f.baseline = jobs::read_baseline(capture).unwrap();
    record_coverage(&f, 0.9, 1.1, false);
    f.run(&mut super::no_speech_tests::NativeFake::default())
        .unwrap();
    let root = f.root.path();
    store_io::write(
        &root.join("config.json"),
        &json!({"version":1,"features":{},"pipeline":{"profile":"experimental"}}),
    )
    .unwrap();
    let data = pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let dest = root.join("local/transcripts/123-4");
    revision::begin(&dest, 1).unwrap();
    let chosen = revision::commit(&dest, &data).unwrap();
    crate::core::notes::render_routed_fixture(root, &chosen)
}

#[test]
fn routed_boundary_preserves_captured_words_in_notes_input() {
    let rendered = notes_at_routed_boundary(
        br#"{"source":"system","start_ms":750,"end_ms":1050,"text":"keep the first words"}"#,
    );
    assert!(
        rendered.contains("[00:00:00.750] Them (English): keep the first words\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains("[00:00:00.900] Them (English): public routed words"),
        "{rendered}"
    );
}
