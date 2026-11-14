use super::*;
use crate::core::meeting_router::revision::{self, Revision};
fn setup() -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    config.engine_settings.stages.routing_deadline_seconds = 1;
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    (root, folder, settings)
}
fn dir(root: &Path) -> PathBuf {
    root.join("local/transcripts").join(ID)
}
fn selected(root: &Path, folder: &Path) -> (String, Revision) {
    let (id, _) = transcript::chosen(root, ID, folder, None, 0, 10_000)
        .unwrap()
        .unwrap();
    let (_, mut data) = revision::load(&dir(root), Some(&id)).unwrap().unwrap();
    let s = &mut data.transcript.segments[0];
    s.route = "te".into();
    s.lang = "te".into();
    s.text_original = "తెలుగు selected revision".into();
    s.text_en = None;
    s.status = "checked".into();
    let id = revision::commit(&dir(root), &data).unwrap();
    (id, data)
}
#[test]
fn chosen_revision_reaches_helper_and_busy_retry_and_notes_rerun_do_no_asr() {
    let (root, folder, s) = setup();
    let (first, mut data) = selected(&root.0, &folder);
    fs::write(folder.join("english-transcript.md"), "stale English file").unwrap();
    enqueue(&root.0, &s, None);
    let mut busy = Fake::new(vec![Err(Failure::Busy)]);
    drain(&root.0, &s, &mut busy);
    assert_eq!(
        queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision,
        Some(first.clone())
    );
    data.transcript.segments[0].lang = "hi".into();
    data.transcript.segments[0].route = "hi".into();
    data.transcript.segments[0].text_original = "हिंदी later revision".into();
    let later = revision::commit(&dir(&root.0), &data).unwrap();
    assert_ne!(first, later);
    assert!(
        transcript::chosen(&root.0, ID, &folder, Some(&later), 0, 0)
            .unwrap()
            .unwrap()
            .1
            .contains("(Hindi): हिंदी")
    );
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(
        fake.calls[0]
            .0
            .contains("(Telugu): తెలుగు selected revision")
    );
    assert!(!fake.calls[0].0.contains("later revision"));
    assert!(!fake.calls[0].0.contains("stale English file"));
    fs::remove_file(folder.join("english-summary.json")).unwrap();
    // A summary replay needs only the chosen cache, without any original or audio.
    fs::remove_file(folder.join("transcript.ndjson")).unwrap();
    enqueue(&root.0, &s, Some(Request::TryAgain));
    let mut rerun = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut rerun);
    assert!(rerun.calls[0].0.contains("తెలుగు selected revision"));
    assert!(!rerun.calls[0].0.contains("later revision"));
    // A notes-only replay still works after every original transcript has gone away.
    enqueue(&root.0, &s, Some(Request::RedoTasks));
    let mut redo = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut redo);
    assert!(redo.calls[0].0.contains("తెలుగు selected revision"));
    assert!(!root.0.join("local/router-work").exists());
    assert!(
        transcript::chosen(&root.0, ID, &folder, Some(&first), 0, 0)
            .unwrap()
            .unwrap()
            .1
            .contains("తెలుగు")
    );
    assert!(!folder.join("system.wav").exists());
    assert!(!folder.join("microphone.wav").exists());
}
#[test]
fn slow_or_failed_routing_resolves_baseline_and_late_retry_preserves_edits() {
    for failed in [false, true] {
        let (root, folder, s) = setup();
        let queue = root.0.join("local/router-queue");
        fs::create_dir_all(&queue).unwrap();
        fs::write(
            queue.join(format!("{ID}.json")),
            r#"{"id":"1793946600000-123","generation":7}"#,
        )
        .unwrap();
        revision::begin(&dir(&root.0), 7).unwrap();
        assert_eq!(
            transcript::chosen(&root.0, ID, &folder, None, 1000, 1999)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        if failed {
            let work = root.0.join("local/router-work").join(ID);
            fs::create_dir_all(&work).unwrap();
            fs::write(
                work.join("attempt.json"),
                r#"{"generation":7,"child":{"success":false,"timed_out":false}}"#,
            )
            .unwrap();
        }
        let (id, text) = transcript::chosen(
            &root.0,
            ID,
            &folder,
            None,
            1000,
            if failed { 1999 } else { 2000 },
        )
        .unwrap()
        .unwrap();
        assert!(text.contains(PRIVATE));
        assert!(text.contains("(English)"));
        let (_, mut data) = revision::load(&dir(&root.0), Some(&id)).unwrap().unwrap();
        fs::write(folder.join("english-summary.json"), summary()).unwrap();
        let edited = fs::read(folder.join("english-summary.json")).unwrap();
        data.transcript.segments[0].status = "checked".into();
        revision::commit(&dir(&root.0), &data).unwrap();
        enqueue(&root.0, &s, None);
        let mut fake = Fake::new(vec![]);
        drain(&root.0, &s, &mut fake);
        assert!(fake.calls.is_empty());
        assert_eq!(
            fs::read(folder.join("english-summary.json")).unwrap(),
            edited
        );
    }
}
#[test]
fn flag_off_retains_compatibility_even_when_a_routed_revision_exists() {
    let (root, folder, s) = setup();
    let (chosen, _) = selected(&root.0, &folder);
    enqueue(&root.0, &s, None);
    let mut initial = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut initial);
    let path = root.0.join("config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["labs"]["after_stop_language_routing"] = false.into();
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    fs::remove_file(folder.join("english-summary.json")).unwrap();
    enqueue(&root.0, &s, Some(Request::TryAgain));
    assert!(
        queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .is_none()
    );
    let mut compat = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut compat);
    assert!(compat.calls[0].0.contains(PRIVATE));
    assert!(
        transcript::chosen(&root.0, ID, &folder, None, 0, 10_000)
            .unwrap()
            .is_none()
    );
    assert!(
        transcript::chosen(&root.0, ID, &folder, Some(&chosen), 0, 0)
            .unwrap()
            .unwrap()
            .1
            .contains("తెలుగు")
    );
}
#[test]
fn routing_dependency_does_not_starve_another_meetings_notes() {
    let (root, folder, s) = setup();
    let other = "1793946600001-123";
    let second = root.0.join("meetings").join(other);
    fs::create_dir_all(&second).unwrap();
    for name in ["manifest.json", "transcript.ndjson", "quick-title.json"] {
        fs::copy(folder.join(name), second.join(name)).unwrap();
    }
    let path = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["ended_at_ms"] = (chrono::Utc::now().timestamp_millis() + 60_000).into();
    fs::write(path, m.to_string()).unwrap();
    let pending = root.0.join("local/router-queue");
    fs::create_dir_all(&pending).unwrap();
    fs::write(
        pending.join(format!("{ID}.json")),
        r#"{"id":"1793946600000-123","generation":7}"#,
    )
    .unwrap();
    revision::begin(&dir(&root.0), 7).unwrap();
    enqueue(&root.0, &s, None);
    queue::enqueue_in(&root.0, &root.0.join("meetings"), other, None, &s).unwrap();
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(matches!(
        queue::read(&root.0, other).unwrap().unwrap().state,
        JobState::Done { .. }
    ));
    let waiting = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(waiting.state, JobState::Queued));
    assert!(waiting.transcript_revision.is_none());
    assert_eq!(fake.calls.len(), 2);
}
