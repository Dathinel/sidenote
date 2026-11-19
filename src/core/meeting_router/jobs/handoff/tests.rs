use super::*;
use crate::core::meeting_router::{
    batches::{self, EnglishBridge, NativeLanguage, NativeLoader, Range, Recognizer},
    pieces::{Piece, TrackManifest},
    ranges::AsrRange,
};
pub(crate) struct Fake;
impl Recognizer for Fake {
    fn recognize(&mut self, r: &Range) -> Result<serde_json::Value> {
        Ok(serde_json::json!({"text":r.piece}))
    }
}
impl NativeLoader for Fake {
    fn load(&mut self, _: NativeLanguage, _: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(Fake))
    }
}
impl EnglishBridge for Fake {
    fn transcribe(&mut self, r: &Range) -> Result<serde_json::Value> {
        self.recognize(r)
    }
}

/// Produces real spooled plans/receipts, then invokes the same parent hand-off as run_pending.
pub(crate) fn complete(
    root: &Path,
    id: &str,
    baseline: &[Chunk],
    language: &str,
) -> Result<String> {
    produce(root, id, baseline, language)?;
    commit_batches(root, id, 7, "fixture-input", baseline)
}
fn produce(root: &Path, id: &str, baseline: &[Chunk], language: &str) -> Result<()> {
    let generation = 7;
    store_io::write(
        &queue(root).join(format!("{id}.json")),
        &Pending {
            id: id.into(),
            generation,
        },
    )?;
    revision::begin(destination(root, id).parent().unwrap(), generation)?;
    let dir = work(root, id).join("batches");
    fs::create_dir_all(&dir)?;
    let b = &baseline[0];
    let start = (b.start * 16000.0).round() as u64;
    let end = (b.end * 16000.0).round() as u64;
    let wav = dir.join("clip.wav");
    crate::core::asr::write_wav(&wav, &vec![0.0; (end - start) as usize])?;
    let text = match language {
        "te" => "తెలుగు selected revision",
        "hi" => "हिंदी later revision",
        _ => "English selected revision",
    };
    let plan = batches::spool(
        &dir,
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
    )?;
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
        generation: generation.to_string(),
        source_revision: "fixture-input".into(),
        plan,
        manifest,
        fallbacks: vec![],
    };
    store_io::write(&dir.join("output.json"), &output)?;
    batches::run(&dir, &output.plan, &mut Fake, &mut Fake)
}
#[test]
fn handoff_requires_receipts_and_source_generation_and_preserves_provenance() {
    let root = tempfile::tempdir().unwrap();
    let id = "123-4";
    let b:Chunk=serde_json::from_value(serde_json::json!({"id":"english-0","start":1.0,"end":2.0,"track":"system","english":"Baseline"})).unwrap();
    let baseline = vec![b];
    produce(root.path(), id, &baseline, "te").unwrap();
    let mut config = crate::core::Config::default();
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    store_io::write(&root.path().join("config.json"), &config).unwrap();
    let request = work(root.path(), id).join("request.json");
    store_io::write(
        &request,
        &Job {
            id: id.into(),
            folder: root.path().join("no-original-audio"),
            baseline: baseline.clone(),
            config: Default::default(),
            languages: vec!["en".into(), "te".into()],
            input_revision: "fixture-input".into(),
            generation: 7,
            windows: serde_json::Value::Null,
            input: None,
        },
    )
    .unwrap();
    run_one_in(&request, root.path()).unwrap();
    assert!(
        !work(root.path(), id).join("result.json").exists(),
        "Labs must not emit a legacy router transcript"
    );
    let chosen = commit_batches(root.path(), id, 7, "fixture-input", &baseline).unwrap();
    let dir = destination(root.path(), id);
    let (_, r) = revision::load(dir.parent().unwrap(), Some(&chosen))
        .unwrap()
        .unwrap();
    assert_eq!(r.transcript.segments[0].lang, "te");
    assert_eq!(r.evidence.len(), 1);
    assert_eq!(r.alternatives.len(), 1);
    assert!(r.provenance["plan"]["batches"]["te"].is_object());
    assert!(commit_batches(root.path(), id, 6, "fixture-input", &baseline).is_err());
    assert!(commit_batches(root.path(), id, 7, "other-source", &baseline).is_err());
    let batch = work(root.path(), id).join("batches");
    for entry in fs::read_dir(&batch).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|v| v == "receipt") {
            fs::write(p, b"corrupt receipt").unwrap();
        }
    }
    let fallback = commit_batches(root.path(), id, 7, "fixture-input", &baseline).unwrap();
    let (_, r) = revision::load(dir.parent().unwrap(), Some(&fallback))
        .unwrap()
        .unwrap();
    assert!(r.transcript.degraded());
    assert_eq!(r.transcript.segments[0].text_original, "Baseline");
}

#[test]
fn expired_notes_dependency_still_launches_a_bounded_retry_worker() {
    if isolated_test() {
        return;
    }
    let root = crate::core::state_dir();
    let meetings = crate::core::meetings_dir();
    let id = "1793946600000-999998";
    fs::create_dir_all(&root).unwrap();
    let cfg = root.join("config.json");
    let previous = fs::read(&cfg).ok();
    let langs = crate::core::model_catalog::load_meeting_languages(&root).unwrap();
    let mut config = crate::core::Config::default();
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    config.engine_settings.stage_deadline_seconds = Some(15);
    config.engine_settings.stages.routing_deadline_seconds = 1;
    store_io::write(&cfg, &config).unwrap();
    crate::core::model_catalog::save_meeting_languages(&root, &["en".into(), "te".into()]).unwrap();
    let folder = meetings.join(id);
    fs::create_dir_all(&folder).unwrap();
    let words = vec!["synthetic"; 25].join(" ");
    store_io::write(&folder.join("manifest.json"),&serde_json::json!({"status":"complete","created_at_ms":1793946600000_i64,"ended_at_ms":0,"duration_ms":30000})).unwrap();
    fs::write(
        folder.join("transcript.ndjson"),
        serde_json::json!({"start_ms":1000,"end_ms":2000,"source":"system","text":words})
            .to_string(),
    )
    .unwrap();
    let baseline = snapshot(&root, id).unwrap().unwrap().baseline;
    store_io::write(
        &queue(&root).join(format!("{id}.json")),
        &Pending {
            id: id.into(),
            generation: 7,
        },
    )
    .unwrap();
    revision::begin(destination(&root, id).parent().unwrap(), 7).unwrap();
    let (fallback, reading, _) = notes_input(
        &root,
        id,
        None,
        || Ok(baseline.clone()),
        0,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(reading.segments[0].text_original, words);
    let scripts = tempfile::tempdir().unwrap();
    run_pending_with(|job, _| {
        produce(&root, id, &job.baseline, "te")?;
        let path = work(&root, id).join("batches/output.json");
        let mut out: BatchOutput = serde_json::from_slice(&fs::read(&path)?)?;
        out.source_revision = job.input_revision.clone();
        store_io::write(&path, &out)?;
        let mut command = Command::new("cmd.exe");
        command
            .current_dir(scripts.path())
            .args(["/D", "/C", "echo", "started>started"]);
        Ok(command)
    })
    .unwrap();
    assert!(scripts.path().join("started").is_file());
    assert!(!queue(&root).join(format!("{id}.json")).exists());
    let (_, published) = revision::load(destination(&root, id).parent().unwrap(), None)
        .unwrap()
        .unwrap();
    assert_eq!(published.transcript.segments[0].lang, "te");
    assert_eq!(published.evidence.len(), 1);
    assert!(published.provenance["plan"]["batches"]["te"].is_object());
    let (_, unpinned, _) = notes_input(
        &root,
        id,
        None,
        || Ok(baseline.clone()),
        0,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(unpinned.segments[0].lang, "en");
    let (same, pinned, _) = notes_input(
        &root,
        id,
        Some(&fallback),
        || panic!("healthy pin requested original Hex input"),
        0,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(same, fallback);
    assert_eq!(pinned.segments[0].lang, "en");
    if let Some(bytes) = previous {
        fs::write(cfg, bytes).unwrap();
    } else {
        fs::remove_file(cfg).unwrap();
    }
    crate::core::model_catalog::save_meeting_languages(&root, &langs).unwrap();
    assert!(
        folder
            .canonicalize()
            .unwrap()
            .starts_with(crate::test_home().canonicalize().unwrap())
    );
    fs::remove_dir_all(folder).unwrap();
}
