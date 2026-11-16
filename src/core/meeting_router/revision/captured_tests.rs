//! Frozen public replay enters through the production receipt reader and assembler.
use super::*;

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
    assert_eq!(load(&dest, None).unwrap().unwrap().0, chosen);
    let policy = &revision.provenance["assembly_coverage"];
    assert!(!policy["overlaps"].as_array().unwrap().is_empty());
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
        error.contains("Owned interval lacks routed or baseline coverage"),
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
