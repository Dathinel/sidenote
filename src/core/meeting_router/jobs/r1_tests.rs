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
            .chunks_exact(1600)
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
