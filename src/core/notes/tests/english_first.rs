//! Notes start from English past the wait limit, through enqueue and drain. Not Windows-only.
use super::*;
use crate::core::meeting_router::{
    batches::{self, EnglishBridge, NativeLanguage, NativeLoader, Range, Recognizer},
    fallback::{Reason, RoutingFallback},
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

fn setup(minutes: u64, ago_ms: i64) -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let path = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["duration_ms"] = (minutes * 60_000).into();
    m["ended_at_ms"] = (chrono::Utc::now().timestamp_millis() - ago_ms).into();
    fs::write(path, m.to_string()).unwrap();
    (root, folder, settings)
}
fn dir(root: &Path) -> PathBuf {
    root.join("local/transcripts").join(ID)
}
fn pending(root: &Path) {
    let queue = root.join("local/router-queue");
    fs::create_dir_all(&queue).unwrap();
    fs::write(
        queue.join(format!("{ID}.json")),
        format!(r#"{{"id":"{ID}","generation":7}}"#),
    )
    .unwrap();
    revision::begin(&dir(root), 7).unwrap();
}
fn release(root: &Path, folder: &Path, language: &str) -> String {
    let first = transcript::chosen(root, ID, folder, None, 0, 10_000)
        .unwrap()
        .unwrap();
    let (_, data) = revision::load(&dir(root), Some(&first.revision))
        .unwrap()
        .unwrap();
    let baseline = data.transcript.baseline;
    pending(root);
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
    handoff::commit_batches(root, ID, 7, "fixture-input", &baseline).unwrap()
}
fn notes(root: &Path, s: &NotesSettings) -> (Fake, Vec<RoutingFallback>) {
    enqueue(root, s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(root, s, &mut fake);
    let job = queue::read(root, ID).unwrap().unwrap();
    (fake, job.routing_fallbacks)
}
fn reasons(reason: Reason) -> Vec<RoutingFallback> {
    vec![RoutingFallback::new(reason)]
}

#[test]
fn long_meeting_starts_from_english_one_second_after_the_end() {
    let (root, folder, s) = setup(30, 1_000);
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(folder.join("english-summary.json").exists());
    assert!(fake.calls[0].0.contains(PRIVATE) && fake.calls[0].0.contains("(English)"));
    assert_eq!(got, reasons(Reason::EnglishFirst));
}
#[test]
fn short_meeting_uses_a_release_that_lands_inside_the_wait() {
    let (root, folder, s) = setup(4, 20_000);
    let pin = release(&root.0, &folder, "te");
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains("తెలుగు selected revision"));
    assert!(got.is_empty());
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.transcript_revision, Some(pin));
}
#[test]
fn short_meeting_without_a_release_falls_back_after_the_wait() {
    let (root, folder, s) = setup(4, 41_000);
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(folder.join("english-summary.json").exists());
    assert!(fake.calls[0].0.contains(PRIVATE));
    assert_eq!(got, reasons(Reason::DeadlineExpired));
}
#[test]
fn zero_minutes_means_never_wait() {
    let (root, folder, s) = setup(4, 1_000);
    let path = root.0.join("config.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    c["engine_settings"]["stages"]["notes_wait_max_minutes"] = 0.into();
    fs::write(path, c.to_string()).unwrap();
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(folder.join("english-summary.json").exists());
    assert!(fake.calls[0].0.contains(PRIVATE));
    assert_eq!(got, reasons(Reason::EnglishFirst));
}
#[test]
fn long_meeting_still_uses_a_release_that_is_already_there() {
    let (root, folder, s) = setup(30, 400_000);
    release(&root.0, &folder, "hi");
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains("हिंदी selected revision"));
    assert!(got.is_empty());
}
