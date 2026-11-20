use super::*;

pub(super) fn router_child(work: &Path) -> ! {
    router::vad::TEST_PROBABILITY.with(|p| p.set(Some(1.0)));
    router::production::TEST_NATIVE.with(|f| f.set(Some(|| child("recognizer"))));
    let root = work.ancestors().nth(3).unwrap();
    let result = jobs::run_one_in(&work.join("request.json"), root);
    fs::write(work.join("producer-result.txt"), format!("{result:?}")).unwrap();
    std::process::exit(if result.is_ok() { 0 } else { 23 });
}

#[test]
fn after_stop_producer_reaches_translation_and_notes() {
    if jobs::isolated_test() {
        return;
    }
    let root = crate::core::state_dir();
    let meetings = crate::core::meetings_dir();
    let folder = meeting(
        meetings.parent().unwrap(),
        &vec!["synthetic baseline"; 25].join(" "),
    );
    fs::write(
        folder.join("transcript.ndjson"),
        serde_json::json!({
            "start_ms":0,"end_ms":3000,"source":"system",
            "text":vec!["synthetic baseline";25].join(" ")
        })
        .to_string(),
    )
    .unwrap();
    crate::core::asr::write_wav(&folder.join("system.wav"), &vec![0.0; 48000]).unwrap();
    store_io::write(
        &folder.join("manifest.json"),
        &serde_json::json!({
            "status":"complete", "created_at_ms":1793946600000_i64,
            "ended_at_ms":chrono::Utc::now().timestamp_millis(), "duration_ms":3000,
            "system":{"first_arrival_us":0}
        }),
    )
    .unwrap();
    let settings = settings_fixture();
    let mut config = crate::core::Config::load_in(&root).unwrap();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.pipeline.language_mode = "forced:hi".into();
    config.pipeline.after_stop_languages = Some(
        serde_json::from_value(serde_json::json!({
            "primary":"hi", "supporting":[],
            "routes":{"hi":{"provider":"local","model":"indicconformer-hi-ctc"}}
        }))
        .unwrap(),
    );
    config.labs.after_stop_language_routing = true;
    config.engine_settings.steps.translation = true;
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.engine_settings.stages.translation_model = "translation-fixture".into();
    config.save(&root).unwrap();
    crate::core::model_catalog::save_meeting_languages(&root, &["hi".into()]).unwrap();
    crate::core::recording_guard::keep_in(&root, ID).unwrap();
    jobs::enqueue_automatic(ID).unwrap();
    assert!(
        root.join("local/router-queue")
            .join(format!("{ID}.json"))
            .is_file()
    );
    jobs::run_pending_with(|_, request| {
        Ok(child(&format!(
            "router-produced|{}",
            request.parent().unwrap().display()
        )))
    })
    .unwrap();
    let work = root.join("local/router-work").join(ID);
    assert_eq!(
        fs::read_to_string(work.join("producer-result.txt")).unwrap(),
        "Ok(())"
    );
    assert!(
        work.join("batches/output.json").is_file(),
        "production must publish its handoff"
    );
    assert!(
        work.join("released.json").is_file(),
        "parent must accept the producer receipt"
    );
    queue::enqueue_in(&root, &meetings, ID, None, &settings).unwrap();
    process_in(&root, &meetings, &settings);
    let job = queue::read(&root, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    let selection = transcript::chosen(
        &root,
        ID,
        &folder,
        job.transcript_revision.as_deref(),
        0,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(selection.source(), transcript::Source::ReleasedRevision);
    assert!(
        selection
            .transcript
            .as_ref()
            .unwrap()
            .segments
            .iter()
            .any(|s| s.lang == "hi" && s.translate)
    );
    assert!(
        fs::read_to_string(root.join("translation-input"))
            .unwrap()
            .contains("selected hi words")
    );
    let notes = fs::read_to_string(root.join("notes-input")).unwrap();
    assert!(notes.contains(&selection.render()));
    assert!(!notes.contains("valid-looking translation output"));
    assert_eq!(job.translation.unwrap().reason, "translation_reply_invalid");
    assert!(folder.join("english-summary.json").is_file());
    assert!(!folder.join("english-transcript.md").exists());
    assert!(
        !crate::core::notes::status_record(&root, ID)
            .unwrap()
            .translation_message()
            .is_empty()
    );
}
