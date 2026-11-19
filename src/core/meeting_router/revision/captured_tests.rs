//! Frozen public replay enters through the production receipt reader and assembler.
use super::*;

fn numeric_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => a
            .iter()
            .find_map(|(k, v)| numeric_difference(v, b.get(k)?, &format!("{path}.{k}"))),
        (Value::Array(a), Value::Array(b)) => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (a, b))| numeric_difference(a, b, &format!("{path}[{i}]"))),
        (Value::Number(a), Value::Number(b)) if a != b => Some(format!("{path}: {a} -> {b}")),
        _ => None,
    }
}

fn fixture() -> (
    tempfile::TempDir,
    Plan,
    super::super::pieces::PieceManifest,
    Vec<Chunk>,
) {
    let saved = std::path::PathBuf::from(std::env::var_os("SIDENOTE_ASSEMBLY_REPRO").unwrap());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for file in std::fs::read_dir(&saved).unwrap() {
        let file = file.unwrap();
        std::fs::copy(file.path(), root.join(file.file_name())).unwrap();
    }
    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    let plan: Plan = serde_json::from_slice(&read("batch-plan.json")).unwrap();
    // The saved bundle omitted the canonical file; restore the same captured Plan bytes.
    std::fs::write(
        root.join(format!("{}.plan.json", plan.revision)),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    let pieces = serde_json::from_slice(&read("pieces.json")).unwrap();
    let baseline: Vec<Chunk> = serde_json::from_slice(&read("baseline.json")).unwrap();
    let replies = gaps::capture(include_bytes!("baseline_no_speech_fixture.ndjson")).unwrap();
    std::fs::write(
        root.join("baseline-no-speech.json"),
        serde_json::to_vec(&replies).unwrap(),
    )
    .unwrap();
    (temp, plan, pieces, baseline)
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_assembly_keeps_owned_coverage_and_publishes() {
    let (temp, plan, pieces, baseline) = fixture();
    let root = temp.path();
    let revision = assemble(root, &plan, &pieces, &baseline, 1).unwrap();
    assert_eq!(revision.evidence.len(), 291);
    assert_eq!(
        revision.provenance["no_speech"].as_object().unwrap().len(),
        3
    );
    let dest = root.join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let bytes = std::fs::read(dest.join("revisions").join(format!("{chosen}.json"))).unwrap();
    let decoded: Revision = serde_json::from_slice(&bytes).unwrap();
    let difference = numeric_difference(
        &serde_json::to_value(&revision).unwrap(),
        &serde_json::to_value(&decoded).unwrap(),
        "revision",
    );
    assert!(
        bytes == serde_json::to_vec(&decoded).unwrap(),
        "revision numeric roundtrip changed: {difference:?}"
    );
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
    let policy = &revision.provenance["assembly_coverage"];
    assert!(!policy["overlaps"].as_array().unwrap().is_empty());
    assert!(policy["overlaps"].as_array().unwrap().iter().any(|o| {
        o["fallback_required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "no-speech")
    }));
    eprintln!(
        "accepted production overlap receipts: {}",
        policy["overlaps"]
    );
    // This real Hindi interval extends beyond its baseline. A missing reading must refuse it.
    let id = "38d0482dac119209983b83022fd4c7d3f68eb8b4bb8a3aa14cbcb456b275c7ff";
    std::fs::remove_file(root.join(format!("{}-{id}.receipt", plan.revision))).unwrap();
    let error = assemble(root, &plan, &pieces, &baseline, 1)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("Speech lacks routed or explicit baseline coverage or marked gap"),
        "{error}"
    );
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_native_native_overlap_is_refused() {
    let (temp, plan, pieces, baseline) = fixture();
    let mut revision = assemble(temp.path(), &plan, &pieces, &baseline, 1).unwrap();
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let native = revision
        .transcript
        .segments
        .iter()
        .find(|s| s.route == "hi")
        .unwrap()
        .clone();
    revision.transcript.segments.push(native);
    assert!(
        commit(&dest, &revision)
            .unwrap_err()
            .to_string()
            .contains("Competing selected readings")
    );
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_fallback_beyond_sentence_bounds_is_refused() {
    let (temp, plan, pieces, baseline) = fixture();
    let mut revision = assemble(temp.path(), &plan, &pieces, &baseline, 1).unwrap();
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let id = revision.provenance["assembly_coverage"]["overlaps"][0]["baseline"]
        .as_str()
        .unwrap()
        .to_owned();
    let fallback = revision
        .transcript
        .segments
        .iter_mut()
        .find(|s| s.sources[0].chunk.id == id)
        .unwrap();
    fallback.end += 1.0;
    fallback.sources[0].chunk.end += 1.0;
    assert!(
        commit(&dest, &revision)
            .unwrap_err()
            .to_string()
            .contains("Competing selected readings")
    );
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_overlap_requires_no_speech_evidence() {
    let (temp, plan, pieces, baseline) = fixture();
    let mut revision = assemble(temp.path(), &plan, &pieces, &baseline, 1).unwrap();
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let marker = revision.provenance["no_speech"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    marker["reason"] = "unverified".into();
    assert!(commit(&dest, &revision).is_err());
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_lid_uncertain_overlap_is_recorded_and_publishes() {
    let (temp, plan, pieces, baseline) = fixture();
    let revision = assemble(temp.path(), &plan, &pieces, &baseline, 1).unwrap();
    let records = revision.provenance["routing_fallbacks"].as_array().unwrap();
    let overlaps = revision.provenance["assembly_coverage"]["overlaps"]
        .as_array()
        .unwrap();
    assert!(overlaps.iter().any(|o| {
        o["fallback_required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "lid-uncertain" && records.contains(&r["record"]))
    }));
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_full_guard_marks_exact_four_missing_baseline_intervals() {
    let (temp, plan, pieces, baseline) = fixture();
    let revision =
        super::super::production::assemble_saved(temp.path(), &plan, &pieces, &baseline, 1)
            .unwrap();
    let gaps = revision.provenance["marked_gaps"].as_array().unwrap();
    let actual: Vec<_> = gaps
        .iter()
        .map(|g| (g["start"].as_f64().unwrap(), g["end"].as_f64().unwrap()))
        .collect();
    assert_eq!(
        actual,
        [
            (665.964, 668.944),
            (782.752, 784.560),
            (784.560, 785.780),
            (793.040, 794.128)
        ]
    );
    for gap in gaps {
        assert_eq!(gap["reasons"][0]["reason"], "guard_rejected");
        assert_eq!(gap["baseline"]["code"], "no-speech");
        assert!(
            revision
                .transcript
                .segments
                .iter()
                .any(|s| s.status == "marked-gap"
                    && s.start == gap["start"].as_f64().unwrap()
                    && s.end == gap["end"].as_f64().unwrap()
                    && s.text_original.starts_with("[Speech unavailable:"))
        );
    }
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let saved = load(&dest, Some(&chosen)).unwrap().unwrap().1;
    assert_eq!(
        saved.provenance["marked_gaps"],
        revision.provenance["marked_gaps"]
    );
    let mut tampered = revision.clone();
    tampered
        .transcript
        .segments
        .retain(|s| s.status != "marked-gap");
    assert!(commit(&dest, &tampered).is_err());
    std::fs::remove_file(temp.path().join("baseline-no-speech.json")).unwrap();
    assert!(
        super::super::production::assemble_saved(temp.path(), &plan, &pieces, &baseline, 1)
            .is_err()
    );
    std::fs::write(
        temp.path().join("baseline-no-speech.json"),
        serde_json::to_vec(
            &gaps::capture(include_bytes!("baseline_no_speech_fixture.ndjson")).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(temp.path().join("uncertainty.json"), b"[]").unwrap();
    assert!(
        super::super::production::assemble_saved(temp.path(), &plan, &pieces, &baseline, 1)
            .is_err()
    );
}

#[test]
#[ignore = "requires the public assembly-repro directory in SIDENOTE_ASSEMBLY_REPRO"]
fn saved_public_fallback_overlap_without_router_reason_is_refused() {
    let (temp, plan, pieces, baseline) = fixture();
    let mut revision = assemble(temp.path(), &plan, &pieces, &baseline, 1).unwrap();
    let dest = temp.path().join("published");
    begin(&dest, 1).unwrap();
    let chosen = commit(&dest, &revision).unwrap();
    let id = revision.provenance["assembly_coverage"]["overlaps"][0]["baseline"]
        .as_str()
        .unwrap();
    let unit = baseline.iter().find(|b| b.id == id).unwrap();
    let required = revision.provenance["assembly_coverage"]["fallback_required"]
        .as_array()
        .unwrap();
    let removed: Vec<Value> = required
        .iter()
        .filter(|r| {
            r["interval"]["track"] == unit.track
                && r["interval"]["start"].as_f64().unwrap() < unit.end
                && unit.start < r["interval"]["end"].as_f64().unwrap()
        })
        .map(|r| r["record"].clone())
        .collect();
    assert!(!removed.is_empty());
    revision.provenance["no_speech"]
        .as_object_mut()
        .unwrap()
        .retain(|_, r| !removed.contains(r));
    revision.provenance["routing_fallbacks"]
        .as_array_mut()
        .unwrap()
        .retain(|r| !removed.contains(r));
    revision.provenance["assembly_coverage"]["fallback_required"]
        .as_array_mut()
        .unwrap()
        .retain(|r| !removed.contains(&r["record"]));
    assert!(
        commit(&dest, &revision)
            .unwrap_err()
            .to_string()
            .contains("Competing selected readings")
    );
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
}
