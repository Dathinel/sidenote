use super::*;
use crate::core::{
    asr,
    meeting_router::batches::{NativeLanguage, Recognizer},
    speech_sessions::{SessionRequest, digest},
};
use std::{
    fs,
    process::{Command, Stdio},
};
fn request() -> SessionRequest {
    SessionRequest {
        client_session: "fake".into(),
        meeting_id: "public".into(),
        languages: vec!["en".into(), "hi".into()],
        language_mode: "auto".into(),
        config_sha256: "a".repeat(64),
    }
}
struct Fake {
    calls: usize,
    interrupt: bool,
}
impl Recognizer for Fake {
    fn recognize(&mut self, range: &Range) -> Result<Value> {
        self.calls += 1;
        if self.interrupt && self.calls == 2 {
            std::process::exit(71);
        }
        Ok(
            json!({"text":"synthetic","timing":"range","start_sample":range.range.context.0,"end_sample":range.range.context.1}),
        )
    }
}
struct Loader(bool);
impl NativeLoader for Loader {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(Fake {
            calls: 0,
            interrupt: self.0,
        }))
    }
}
#[test]
fn speech_job_fixture() {
    let Some(root) = std::env::var_os("SIDENOTE_L7_FIXTURE") else {
        return;
    };
    let store = Store(root.into());
    let (id, _) = store.create("owner", request()).unwrap();
    let audio = store.0.join("synthetic.wav");
    asr::write_wav(&audio, &[0.0; 24000]).unwrap();
    let bytes = fs::read(&audio).unwrap();
    let metadata = PieceRequest {
        track: "system".into(),
        index: 0,
        origin_sample: -12,
        start_sample: 0,
        end_sample: 24000,
        context_start_sample: 0,
        sha256: digest(&bytes),
    };
    let piece = metadata.id(&request());
    store
        .put("owner", &id, &piece, metadata.clone(), &bytes)
        .unwrap();
    store.finish("owner", &id, 1).unwrap();
    let work = store.0.join(&id).join("jobs").join(&piece);
    fs::create_dir_all(&work).unwrap();
    let ranges = (0..3)
        .map(|i| {
            let wav = work.join(format!("{i}.wav"));
            asr::write_wav(&wav, &[0.0; 8000]).unwrap();
            Range {
                track: "system".into(),
                piece: piece.clone(),
                wav,
                range: crate::core::meeting_router::ranges::AsrRange {
                    context: (i * 8000, (i + 1) * 8000),
                    members: vec![],
                    language: Some(if i == 2 { "en" } else { "hi" }.into()),
                },
                model_id: "synthetic".into(),
                model_sha256: "a".repeat(64),
            }
        })
        .collect();
    let plan = batches::spool(&work, ranges).unwrap();
    let interrupted = std::env::var_os("SIDENOTE_L7_INTERRUPT").is_some();
    store
        .tick(|id, piece, _| {
            execute(
                &store,
                id,
                piece,
                &metadata,
                &work,
                &plan,
                &mut Loader(interrupted),
            )
        })
        .unwrap();
}
#[test]
fn speech_job_restart_keeps_range_keys_and_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let store = Store(root.path().join("speech"));
    let child = |interrupt: bool| {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "--exact",
            "core::speech_jobs::runner::tests::speech_job_fixture",
            "--nocapture",
        ])
        .env("SIDENOTE_L7_FIXTURE", &store.0)
        .env_remove("SIDENOTE_L7_INTERRUPT")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        if interrupt {
            c.env("SIDENOTE_L7_INTERRUPT", "1");
        }
        c.status().unwrap().code().unwrap()
    };
    assert_eq!(child(true), 71);
    let (id, _) = store.create("owner", request()).unwrap();
    let before = store.events("owner", &id, 0).unwrap();
    assert_eq!(
        before.iter().filter(|e| e.event == "range.result").count(),
        1
    );
    assert!(!before.iter().any(|e| e.event == "piece.done"));
    assert_eq!(child(false), 0);
    let events = store.events("owner", &id, 0).unwrap();
    let ranges: Vec<_> = events
        .iter()
        .filter(|e| e.event == "range.result")
        .map(|e| e.data.clone())
        .collect();
    assert_eq!(ranges.len(), 3);
    let english = ranges.iter().find(|r| r["route"] == "en").unwrap();
    assert!(english["text"].is_null());
    assert_ne!(ranges[0]["range"], ranges[1]["range"]);
    assert_eq!(
        before
            .iter()
            .find(|e| e.event == "range.result")
            .unwrap()
            .data,
        ranges[0]
    );
    let snapshot = store
        .snapshot("owner", &id, ranges[0]["piece"].as_str().unwrap())
        .unwrap();
    assert_eq!(snapshot["state"], "done");
    assert_eq!(snapshot["ranges"], json!(ranges));
    assert_eq!(store.events("owner", &id, 2).unwrap(), events[2..]);
    assert!(events.iter().any(|e| e.event == "piece.accepted"));
    assert!(events.iter().any(|e| e.event == "piece.progress"));
    assert!(events.iter().any(|e| e.event == "session.done"));
}
