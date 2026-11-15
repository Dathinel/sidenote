//! These tests require only the named public fixture, never installed meeting data.
use super::*;
use std::collections::BTreeMap;

pub(crate) struct Fixture {
    pub root: tempfile::TempDir,
    pub folder: PathBuf,
    pub work: PathBuf,
    pub config: Config,
}
impl Fixture {
    pub fn new() -> Self {
        let template =
            PathBuf::from(std::env::var_os("SIDENOTE_ROUTING_R1_FIXTURE").expect("public fixture"));
        let name = template.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("sidenote-routing-r1-fixture-"));
        assert!(crate::core::is_proof_path(&template, name));
        let config = serde_json::from_slice(
            &inputs::Snapshot::read_file(&template.join("config.json"), 65536).unwrap(),
        )
        .unwrap();
        let root = tempfile::Builder::new()
            .prefix("sidenote-routing-r1-test-")
            .tempdir()
            .unwrap();
        let folder = root.path().join("meeting");
        let work = root.path().join("work");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::copy(template.join("speech.wav"), folder.join("system.wav")).unwrap();
        let samples = hound::WavReader::open(folder.join("system.wav"))
            .unwrap()
            .duration();
        store_io::write(
            &folder.join("manifest.json"),
            &json!({"status":"complete", "duration_ms":samples/16,"system":{"first_arrival_us":0}}),
        )
        .unwrap();
        store_io::atomic_write(&folder.join("transcript.ndjson"), b"").unwrap();
        Self {
            root,
            folder,
            work,
            config,
        }
    }
    pub fn force_hindi(&mut self) {
        self.config.pipeline.language_mode = "forced:hi".into();
        let routes = self.config.pipeline.after_stop_languages.as_mut().unwrap();
        routes.primary = "hi".into();
        routes.supporting.clear();
        routes.routes.retain(|k, _| k == "hi");
    }
    pub fn produce(&self) -> Plan {
        plan::produce(
            &self.folder,
            &self.work,
            &self.config,
            &BTreeMap::from([("system".into(), 0)]),
            "same-captured-input",
            &mut BTreeMap::new(),
        )
        .unwrap()
        .0
    }
}

#[test]
#[ignore = "requires SIDENOTE_ROUTING_R1_FIXTURE public audio/model fixture"]
fn production_retry_repairs_damaged_derived_pcm() {
    let mut f = Fixture::new();
    f.force_hindi();
    let first = f.produce();
    let range = first.batches["hi"]
        .values()
        .next()
        .expect("real VAD speech");
    assert!(range.wav.starts_with(f.root.path()));
    let original = inputs::Snapshot::read_file(&range.wav, 1 << 20).unwrap();
    let source = inputs::Snapshot::read_file(&f.folder.join("system.wav"), 1 << 20).unwrap();
    let n = hound::WavReader::open(&range.wav).unwrap().duration() as usize;
    asr::write_wav(&range.wav, &vec![0.75; n]).unwrap();
    assert_ne!(
        inputs::Snapshot::read_file(&range.wav, 1 << 20).unwrap(),
        original
    );
    let retry = f.produce();
    assert_eq!(
        Sha256::digest(inputs::Snapshot::read_file(&range.wav, 1 << 20).unwrap()),
        Sha256::digest(original),
        "retry must derive PCM from pinned input"
    );
    assert_eq!(
        first.revision, retry.revision,
        "same source retains receipt identity"
    );
    assert_eq!(
        inputs::Snapshot::read_file(&f.folder.join("system.wav"), 1 << 20).unwrap(),
        source
    );
}

#[test]
#[ignore = "requires SIDENOTE_ROUTING_R1_FIXTURE public audio/model fixture"]
fn production_planner_persists_uncertain_interval_reasons() {
    use crate::core::meeting_router::fallback::{Reason, RoutingFallback};
    let mut f = Fixture::new();
    f.config
        .labs
        .language_detection_settings
        .confidence_threshold = 1.0;
    f.config.labs.language_detection_settings.margin_threshold = 1.0;
    let plan = f.produce();
    assert!(plan.batches.is_empty());
    let reasons: Vec<RoutingFallback> = serde_json::from_slice(
        &inputs::Snapshot::read_file(&f.work.join("uncertainty.json"), 1 << 20).unwrap(),
    )
    .unwrap();
    let coverage: Vec<Chunk> = serde_json::from_slice(
        &inputs::Snapshot::read_file(&f.work.join("coverage.json"), 1 << 20).unwrap(),
    )
    .unwrap();
    assert!(
        !reasons.is_empty(),
        "real VAD/LID decisions must retain their reason"
    );
    assert_eq!(reasons.len(), coverage.len());
    for (reason, c) in reasons.iter().zip(coverage) {
        assert_eq!(reason.reason, Reason::GuardRejected);
        let i = reason.interval.as_ref().unwrap();
        assert_eq!((&i.track, i.start, i.end), (&c.track, c.start, c.end));
    }
}

#[test]
#[ignore = "requires public audio and pinned Silero model"]
fn production_planner_owns_real_speech_across_piece_seams() {
    let mut f = Fixture::new();
    f.force_hindi();
    let mut samples = vec![0.0; 297 * 16000];
    samples.extend(asr::read_wav(&f.folder.join("system.wav")).unwrap());
    asr::write_wav(&f.folder.join("system.wav"), &samples).unwrap();
    store_io::write(&f.folder.join("manifest.json"), &json!({"status":"complete","duration_ms":samples.len()/16,"system":{"first_arrival_us":1000000}})).unwrap();
    let (plan, manifest, _) = plan::produce(
        &f.folder,
        &f.work,
        &f.config,
        &BTreeMap::from([("system".into(), 16000)]),
        "public-seam",
        &mut BTreeMap::new(),
    )
    .unwrap();
    let track = &manifest.tracks[0];
    assert_eq!(track.origin_sample, 16000);
    assert_eq!(track.pieces.len(), 2);
    let mut ranges: Vec<_> = plan.batches["hi"].values().collect();
    ranges.sort_by_key(|r| r.range.context.0);
    assert!(ranges.len() >= 2);
    let mut previous = 0;
    for r in ranges {
        let piece = track.pieces.iter().find(|p| p.id == r.piece).unwrap();
        assert!(piece.start_sample <= r.range.context.0 && r.range.context.1 <= piece.end_sample);
        assert!(r.range.context.0 >= previous);
        assert!(r.range.context.1 - r.range.context.0 <= 480000);
        assert_eq!(
            r.range.members,
            vec![(0, r.range.context)],
            "recognized padding must have a single owner"
        );
        previous = r.range.context.1;
    }
}
