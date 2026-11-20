//! The language-passes word in `english-summary.json`, through enqueue and drain. Not Windows-only.
use super::*;
use crate::core::meeting_router::{
    batches::{self, EnglishBridge, NativeLanguage, NativeLoader, Range, Recognizer},
    jobs::handoff::{self, BatchOutput},
    pieces::{Piece, PieceManifest, TrackManifest},
    ranges::AsrRange,
    revision,
};
type Done<T> = crate::core::asr::Result<T>;

struct Reading;
impl Recognizer for Reading {
    fn recognize(&mut self, r: &Range) -> Done<serde_json::Value> {
        Ok(serde_json::json!({"text":r.piece}))
    }
}
impl NativeLoader for Reading {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Done<Box<dyn Recognizer>> {
        Ok(Box::new(Reading))
    }
}
impl EnglishBridge for Reading {
    fn transcribe(&mut self, r: &Range) -> Done<serde_json::Value> {
        self.recognize(r)
    }
}

fn setup() -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    config.engine_settings.stages.routing_deadline_seconds = 360;
    let json = serde_json::to_vec(&config).unwrap();
    fs::write(root.0.join("config.json"), json).unwrap();
    set_ended(&folder, chrono::Utc::now().timestamp_millis());
    (root, folder, settings)
}
fn set_ended(folder: &Path, ms: i64) {
    let path = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["ended_at_ms"] = ms.into();
    fs::write(path, m.to_string()).unwrap();
}
fn labs_off(root: &Path) {
    let path = root.join("config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["labs"]["after_stop_language_routing"] = false.into();
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
}
fn dir(root: &Path) -> PathBuf {
    root.join("local/transcripts").join(ID)
}
/// The pending-language record the router leaves behind, so the notes job can see the generation.
fn pending(root: &Path) {
    let queue = root.join("local/router-queue");
    fs::create_dir_all(&queue).unwrap();
    fs::write(
        queue.join(format!("{ID}.json")),
        format!(r#"{{"id":"{ID}","generation":7}}"#),
    )
    .unwrap();
}
/// Builds a released revision through the real hand-off. `corrupt` damages the range receipt, so
/// assembly fills the range from the captured baseline and records no typed reason.
pub(super) fn release(root: &Path, folder: &Path, language: &str, corrupt: bool) -> String {
    let first = transcript::chosen(root, ID, folder, None, 0, 1 << 60)
        .unwrap()
        .unwrap();
    let (_, data) = revision::load(&dir(root), Some(&first.revision))
        .unwrap()
        .unwrap();
    let baseline = data.transcript.baseline;
    pending(root);
    revision::begin(&dir(root), 7).unwrap();
    let work = root.join("local/router-work").join(ID).join("batches");
    fs::create_dir_all(&work).unwrap();
    let b = &baseline[0];
    let (start, end) = (
        (b.start * 16000.0).round() as u64,
        (b.end * 16000.0).round() as u64,
    );
    let wav = work.join("clip.wav");
    crate::core::asr::write_wav(&wav, &vec![0.0; (end - start) as usize]).unwrap();
    let text = if language == "te" {
        "తెలుగు selected revision"
    } else {
        "हिंदी selected revision"
    };
    let plan = batches::spool(
        &work,
        vec![Range {
            track: b.track.clone(),
            piece: text.into(),
            wav,
            range: AsrRange {
                context: (start, end),
                members: vec![(0, (start, end))],
                language: Some(language.into()),
            },
            model_id: format!("fake-{language}"),
            model_sha256: "a".repeat(64),
        }],
    )
    .unwrap();
    let manifest = PieceManifest {
        source_revision: "capture".into(),
        config_checksum: "config".into(),
        tracks: vec![TrackManifest {
            track: b.track.clone(),
            origin_sample: 0,
            sample_count: end,
            pieces: vec![Piece {
                id: text.into(),
                start_sample: 0,
                end_sample: end,
            }],
        }],
    };
    let output = BatchOutput {
        generation: "7".into(),
        source_revision: "fixture-input".into(),
        plan,
        manifest,
        fallbacks: vec![],
        english_transport: serde_json::Value::Null,
    };
    store_io::write(&work.join("output.json"), &output).unwrap();
    #[cfg(windows)]
    batches::run(&work, &output.plan, &mut Reading, &mut Reading).unwrap();
    #[cfg(not(windows))]
    batches::run_host(
        &work,
        &output.plan,
        &mut Reading,
        Duration::from_secs(60),
        &mut |_, _, _| Ok(()),
    )
    .unwrap();
    if corrupt {
        for entry in fs::read_dir(&work).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "receipt") {
                fs::write(path, b"corrupt receipt").unwrap();
            }
        }
    }
    handoff::commit_batches(root, ID, 7, "fixture-input", &baseline).unwrap()
}
fn write_notes(root: &Path, settings: &NotesSettings, request: Option<Request>) -> Fake {
    enqueue(root, settings, request);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(root, settings, &mut fake);
    fake
}
pub(super) fn made_by(folder: &Path) -> serde_json::Value {
    let raw = fs::read(folder.join("english-summary.json")).unwrap();
    serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["made_by"].clone()
}

#[test]
fn routed_summary_records_routed_and_the_revision_it_was_built_from() {
    for language in ["te", "hi"] {
        let (root, folder, s) = setup();
        let revision = release(&root.0, &folder, language, false);
        write_notes(&root.0, &s, None);
        let by = made_by(&folder);
        assert_eq!(by["language_passes"], "routed");
        assert_eq!(by["transcript_revision"], revision.as_str());
    }
}
#[test]
fn missed_deadline_records_english_only() {
    let (root, folder, s) = setup();
    set_ended(&folder, 1000);
    pending(&root.0);
    revision::begin(&dir(&root.0), 7).unwrap();
    write_notes(&root.0, &s, None);
    let by = made_by(&folder);
    assert_eq!(by["language_passes"], "english_only");
    assert!(by["transcript_revision"].is_string());
}
#[test]
fn labs_off_records_english_only_without_a_revision() {
    let (root, folder, s) = setup();
    labs_off(&root.0);
    write_notes(&root.0, &s, None);
    let by = made_by(&folder);
    assert_eq!(by["language_passes"], "english_only");
    assert!(by.get("transcript_revision").is_none());
}
#[test]
fn task_retry_with_labs_off_leaves_the_routed_word_alone() {
    let (root, folder, s) = setup();
    release(&root.0, &folder, "te", false);
    write_notes(&root.0, &s, None);
    assert_eq!(made_by(&folder)["language_passes"], "routed");
    labs_off(&root.0);
    let redo = write_notes(&root.0, &s, Some(Request::RedoTasks));
    assert_eq!(redo.calls.len(), 2, "the task retry did run");
    assert_eq!(made_by(&folder)["language_passes"], "routed");
}
#[test]
fn range_filled_from_the_baseline_without_a_typed_reason_records_english_only() {
    let (root, folder, s) = setup();
    release(&root.0, &folder, "hi", true);
    write_notes(&root.0, &s, None);
    assert_eq!(made_by(&folder)["language_passes"], "english_only");
}
