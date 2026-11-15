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
