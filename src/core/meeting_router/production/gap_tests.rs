use super::no_speech_tests::Fixture;
use super::*;

#[test]
fn production_saved_gap_requires_both_dispositions_and_survives_load() {
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
    assert!(pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).is_err());
    store_io::write(&f.work.join("baseline-no-speech.json"), &replies).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    assert!(pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).is_err());
}
