//! Raw public/synthetic inputs enter the normal queue, worker and parent commit.
use super::*;
use crate::core::{self, meeting_router::production::regression::Fixture};
use std::os::windows::fs::OpenOptionsExt;

pub(crate) struct Harness {
    pub fixture: Fixture,
    pub root: PathBuf,
    pub folder: PathBuf,
    pub id: String,
    pub hex: PathBuf,
    prior: Vec<(PathBuf, Option<Vec<u8>>)>,
}
impl Harness {
    pub fn new(labs: bool, language: &str) -> Self {
        let mut fixture = Fixture::new();
        if language == "hi" {
            fixture.force_hindi();
        } else {
            let r = fixture
                .config
                .pipeline
                .after_stop_languages
                .as_mut()
                .unwrap();
            r.primary = "en".into();
            r.supporting.clear();
            r.routes.retain(|k, _| k == "en");
        }
        fixture.config.labs.after_stop_language_routing = labs;
        let root = core::state_dir();
        assert!(root.starts_with(crate::test_home()));
        fs::create_dir_all(&root).unwrap();
        let prior = ["config.json", "meeting-languages.json"]
            .map(|n| {
                let path = root.join(n);
                let bytes = fs::read(&path).ok();
                (path, bytes)
            })
            .to_vec();
        let id = format!("{}-1", chrono::Utc::now().timestamp_nanos_opt().unwrap());
        let folder = core::meetings_dir().join(&id);
        fs::create_dir_all(&folder).unwrap();
        let template = PathBuf::from(std::env::var_os("SIDENOTE_ROUTING_R1_FIXTURE").unwrap());
        fs::copy(
            if language == "hi" {
                template.join("speech-hi.wav")
            } else {
                fixture.folder.join("system.wav")
            },
            folder.join("system.wav"),
        )
        .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        store_io::write(&folder.join("manifest.json"), &serde_json::json!({"status":"complete","created_at_ms":now-60000,"ended_at_ms":now,"duration_ms":60000,"system":{"first_arrival_us":0}})).unwrap();
        let pcm = asr::read_wav(&folder.join("system.wav")).unwrap();
        let peak = pcm
            .as_chunks::<1600>()
            .0
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.iter()
                    .map(|v| v * v)
                    .sum::<f32>()
                    .total_cmp(&b.iter().map(|v| v * v).sum::<f32>())
            })
            .unwrap()
            .0
            * 100;
        store_io::atomic_write(&folder.join("transcript.ndjson"), serde_json::json!({"start_ms":peak,"end_ms":peak+100,"source":"system","text":vec!["synthetic";25].join(" ")}).to_string().as_bytes()).unwrap();
        store_io::write(&root.join("config.json"), &fixture.config).unwrap();
        core::model_catalog::save_meeting_languages(&root, &[language.into()]).unwrap();
        core::recording_guard::keep_in(&root, &id).unwrap();
        let hex = fixture.root.path().join("hex.cmd");
        let model = fixture.root.path().join("model.bin");
        fs::write(&model, b"synthetic model content").unwrap();
        fs::write(
            fixture.root.path().join("status.txt"),
            format!("installed\tparakeet_unified_en\ten\t{}\n", model.display()),
        )
        .unwrap();
        fs::write(&hex, "@echo off\nif \"%1 %2\"==\"model status\" (\n if \"%3\"==\"--model\" exit /b 2\n type \"%~dp0status.txt\"\n exit /b 0\n)\nif \"%3\"==\"--help\" (\n echo --model --language\n exit /b 0\n)\nif \"%1\"==\"status\" (\n echo {\"version\":4,\"model\":{\"preparing\":null},\"capture\":{\"state\":\"idle\",\"pending_jobs\":0},\"meeting\":{\"state\":\"idle\"}}\n exit /b 0\n)\necho synthetic recognized public words\n").unwrap();
        Self {
            fixture,
            root,
            folder,
            id,
            hex,
            prior,
        }
    }
    pub fn run(&self, block_clock: bool) {
        assert!(self.hex.starts_with(self.fixture.root.path()));
        enqueue_in(&self.root, &core::meetings_dir(), &self.id).unwrap();
        let mut clock = None;
        run_pending_with(|_, request| {
            if block_clock {
                let target = request.parent().unwrap().join("normalization.clock.json");
                fs::write(&target, b"held clock").unwrap();
                clock = Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(target)
                        .unwrap(),
                );
            }
            let mut c = Command::new(
                std::env::var_os("SIDENOTE_ROUTING_R1_BIN").expect("built production CLI"),
            );
            c.arg("--route-one")
                .arg("--router-request")
                .arg(request)
                .arg("--data-dir")
                .arg(&self.root)
                .arg("--meetings-dir")
                .arg(core::meetings_dir())
                .arg("--hex-cli")
                .arg(&self.hex);
            Ok(c)
        })
        .unwrap();
        drop(clock);
    }
    pub fn transcript(&self) -> Transcript {
        serde_json::from_slice(
            &inputs::Snapshot::read_file(&destination(&self.root, &self.id), 16 << 20).unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        for (path, bytes) in &self.prior {
            match bytes {
                Some(bytes) => store_io::atomic_write(path, bytes).unwrap(),
                None => {
                    let _ = fs::remove_file(path);
                }
            }
        }
        assert!(self.folder.starts_with(crate::test_home()));
        let _ = fs::remove_dir_all(&self.folder);
    }
}

#[test]
#[ignore = "requires public fixture and built SIDENOTE_ROUTING_R1_BIN"]
fn labs_off_locked_normalization_clock_still_publishes_compatibility() {
    let h = Harness::new(false, "hi");
    h.run(true);
    let t = h.transcript();
    assert!(
        t.hi_load_seconds.is_some(),
        "compatibility recognizer must run"
    );
    assert!(
        t.segments
            .iter()
            .any(|s| s.lang == "hi" && !s.text_original.is_empty())
    );
    let target = h
        .root
        .join("local/router-work")
        .join(&h.id)
        .join("normalization.clock.json");
    assert_eq!(fs::read(target).unwrap(), b"held clock");
}

#[test]
#[ignore = "requires public fixture and built SIDENOTE_ROUTING_R1_BIN"]
fn labs_on_locked_normalization_clock_keeps_production_output() {
    let h = Harness::new(true, "en");
    h.run(true);
    let work = h.root.join("local/router-work").join(&h.id);
    assert!(work.join("batches/output.json").is_file());
    let (_, data) = revision::load(destination(&h.root, &h.id).parent().unwrap(), None)
        .unwrap()
        .unwrap();
    assert!(!data.evidence.is_empty());
    assert!(!data.transcript.degraded());
    assert_eq!(
        fs::read(work.join("normalization.clock.json")).unwrap(),
        b"held clock"
    );
}

#[test]
#[ignore = "requires public fixture and built production CLI"]
fn production_fresh_output_commits_receipts_and_empty_track_origin() {
    let h = Harness::new(true, "en");
    asr::write_wav(&h.folder.join("microphone.wav"), &[]).unwrap();
    let path = h.folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["microphone"] = serde_json::json!({"first_arrival_us":null});
    store_io::write(&path, &m).unwrap();
    let work = h.root.join("local/router-work").join(&h.id);
    assert!(!work.exists(), "no prefilled output or spool");
    h.run(false);
    assert!(
        work.join("batches/output.json").is_file(),
        "producer must publish fresh BatchOutput"
    );
    let (_, data) = revision::load(destination(&h.root, &h.id).parent().unwrap(), None)
        .unwrap()
        .unwrap();
    assert!(!data.transcript.degraded());
    assert!(
        !data.evidence.is_empty(),
        "parent must consume actual worker receipts"
    );
    assert_eq!(
        data.provenance["source_revision"],
        data.transcript.input_checksum
    );
    let tracks = data.provenance["pieces"]["tracks"].as_array().unwrap();
    assert!(
        tracks.iter().any(|t| t["track"] == "microphone"
            && t["origin_sample"] == 0
            && t["sample_count"] == 0)
    );
    assert!(handoff::completed(
        &h.root,
        &h.id,
        &data.transcript.input_checksum
    ));
}

#[test]
#[ignore = "requires public fixture and built production CLI with native ASR"]
fn production_native_adapter_emits_owned_sample_bounds() {
    let h = Harness::new(true, "hi");
    h.run(false);
    let (_, data) = revision::load(destination(&h.root, &h.id).parent().unwrap(), None)
        .unwrap()
        .unwrap();
    assert!(!data.evidence.is_empty());
    assert!(
        data.transcript
            .segments
            .iter()
            .any(|s| s.lang == "hi" && !s.text_original.trim().is_empty())
    );
    let output: BatchOutput = serde_json::from_slice(
        &fs::read(
            h.root
                .join("local/router-work")
                .join(&h.id)
                .join("batches/output.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for (id, r) in output.plan.batches["hi"].iter() {
        let reading = &data.evidence[id];
        assert_eq!(reading["start_sample"].as_u64(), Some(r.range.context.0));
        assert_eq!(reading["end_sample"].as_u64(), Some(r.range.context.1));
        assert_eq!(reading["timing"], "range");
    }
}

#[test]
#[ignore = "requires public fixture and built production CLI"]
fn production_worker_obeys_inner_batch_budget() {
    let mut h = Harness::new(true, "en");
    h.fixture.config.engine_settings.stage_deadline_seconds = Some(1);
    store_io::write(&h.root.join("config.json"), &h.fixture.config).unwrap();
    let script = fs::read_to_string(&h.hex).unwrap().replace(
        "echo synthetic recognized",
        "ping -n 3 127.0.0.1 >nul
echo synthetic recognized",
    );
    fs::write(&h.hex, script).unwrap();
    let job = snapshot(&h.root, &h.id).unwrap().unwrap();
    let work = h.root.join("local/router-work").join(&h.id);
    fs::create_dir_all(&work).unwrap();
    let request = work.join("request.json");
    store_io::write(&request, &job).unwrap();
    let mut cmd = Command::new(std::env::var_os("SIDENOTE_ROUTING_R1_BIN").unwrap());
    cmd.arg("--route-one")
        .arg("--router-request")
        .arg(&request)
        .arg("--data-dir")
        .arg(&h.root)
        .arg("--meetings-dir")
        .arg(core::meetings_dir())
        .arg("--hex-cli")
        .arg(&h.hex);
    // Safety cap is independent of the inner one-second production setting under test.
    let outcome = supervise(cmd, 60.0, None);
    assert!(
        !outcome.timed_out,
        "the safety supervisor must not cause the observed failure"
    );
    assert!(
        !outcome.success,
        "production batches must enforce the engine budget"
    );
    assert!(!work.join("batches/output.json").exists());
}

#[test]
#[ignore = "requires public FLEURS source, real VAD/LID models and built production CLI"]
fn production_discarded_fleurs_english_keeps_full_hex_baseline_and_reason() {
    use crate::core::meeting_router::fallback::{self, Reason};
    let mut h = Harness::new(true, "en");
    h.fixture.config = Fixture::new().config;
    h.fixture.config.labs.after_stop_language_routing = true;
    store_io::write(&h.root.join("config.json"), &h.fixture.config).unwrap();
    core::model_catalog::save_meeting_languages(&h.root, &["en".into(), "hi".into(), "te".into()])
        .unwrap();
    let template = PathBuf::from(std::env::var_os("SIDENOTE_ROUTING_R1_FIXTURE").unwrap());
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../production/fleurs_english_baseline.json")).unwrap();
    assert_eq!(source["id"], "en-5904418145588789259");
    let text = source["hex_baseline"].as_str().unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        ::hex::encode(Sha256::digest(
            fs::read(template.join("speech.wav")).unwrap()
        )),
        source["normalized_wav_sha256"].as_str().unwrap()
    );
    let mut pcm = vec![0.0; 280 * 16000];
    pcm.extend(asr::read_wav(&template.join("speech.wav")).unwrap());
    pcm.resize(293 * 16000, 0.0);
    asr::write_wav(&h.folder.join("system.wav"), &pcm).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    store_io::write(&h.folder.join("manifest.json"),&serde_json::json!({"status":"complete","title":"Public FLEURS English", "created_at_ms":now-293000,"ended_at_ms":now,"duration_ms":293000,"system":{"first_arrival_us":0}})).unwrap();
    store_io::atomic_write(
        &h.folder.join("transcript.ndjson"),
        serde_json::json!({"start_ms":280000,"end_ms":293000,"source":"system","text":text})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    h.run(false);
    let (pin, data) = revision::load(destination(&h.root, &h.id).parent().unwrap(), None)
        .unwrap()
        .unwrap();
    assert!(
        data.provenance.get("plan").is_some(),
        "parent must commit actual assembled output, not whole-job failure fallback"
    );
    let reasons = fallback::read(&data).unwrap();
    assert!(reasons.iter().any(|r| {
        r.reason == Reason::GuardRejected
            && r.interval
                .as_ref()
                .is_some_and(|i| i.start >= 280.0 && i.end <= 293.0)
    }));
    assert!(data.transcript.segments.iter().any(|s| s.lang == "en"
        && s.start == 280.0
        && s.end == 293.0
        && s.text_original == text
        && s.status.contains("confidence")));
    assert!(
        data.transcript
            .segments
            .iter()
            .all(|s| !s.status.contains("no speech window"))
    );
    let coverage: Vec<Chunk> = serde_json::from_slice(
        &fs::read(
            h.root
                .join("local/router-work")
                .join(&h.id)
                .join("batches/coverage.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!coverage.is_empty());
    assert!(
        coverage
            .iter()
            .all(|c| super::super::baseline::source_covered(c, &data.transcript.segments))
    );
    let status =
        core::notes::tests::publish_public_routed(&h.root, &core::meetings_dir(), &h.id, text);
    assert_eq!(status.transcript_revision.as_deref(), Some(pin.as_str()));
    assert!(status.routing_message().contains("confidence was too low"));
}
