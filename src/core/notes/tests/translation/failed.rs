use super::*;
use std::sync::{Arc, atomic::AtomicBool};

pub(super) fn router_child(work: &Path) -> ! {
    let request: serde_json::Value = store_io::read(&work.join("request.json")).unwrap();
    let wav = PathBuf::from(request["folder"].as_str().unwrap()).join("microphone.wav");
    let range: Range = serde_json::from_value(serde_json::json!({
        "track":"microphone", "piece":"fixture", "wav":wav,
        "range":{"context":[16000,32000],"members":[[0,[16000,32000]]],"language":"hi"},
        "model_id":"fixture-failure", "model_sha256":"a".repeat(64)
    }))
    .unwrap();
    let mut worker = batches::worker::Worker::spawn(
        child("recognizer-failed"),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(10),
    )
    .unwrap();
    let result = worker.recognize(&range);
    let cleanup = worker.finish();
    fs::write(
        work.join("recognizer-error.txt"),
        format!("recognizer={result:?}\ncleanup={cleanup:?}\n"),
    )
    .unwrap();
    std::process::exit(23);
}

#[test]
fn recognizer_failure_uses_complete_hex_baseline() {
    if jobs::isolated_test() {
        return;
    }
    let root = crate::core::state_dir();
    let meetings = crate::core::meetings_dir();
    let folder = meeting(meetings.parent().unwrap(), PRIVATE);
    crate::core::asr::write_wav(&folder.join("microphone.wav"), &vec![0.0; 48000]).unwrap();
    let settings = settings_fixture();
    let mut config = crate::core::Config::load_in(&root).unwrap();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.save(&root).unwrap();
    jobs::enqueue(ID).unwrap();
    jobs::run_pending_with(|_, request| {
        Ok(child(&format!(
            "router-failed|{}",
            request.parent().unwrap().display()
        )))
    })
    .unwrap();
    let raw = fs::read_to_string(
        root.join("local/router-work")
            .join(ID)
            .join("recognizer-error.txt"),
    )
    .unwrap();
    println!("{raw}");
    assert!(
        raw.contains("Exited") && raw.contains("cleanup=Ok(())"),
        "{raw}"
    );
    let selection = transcript::chosen(
        &root,
        ID,
        &folder,
        None,
        0,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap()
    .unwrap();
    queue::enqueue_in(&root, &meetings, ID, None, &settings).unwrap();
    process_in(&root, &meetings, &settings);
    let job = queue::read(&root, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    let disposition = job.translation.unwrap();
    assert_eq!(disposition.source, transcript::Source::HexBaseline);
    assert_eq!(disposition.reason, "language_passes_not_available");
    assert!(
        job.routing_fallbacks
            .iter()
            .any(|r| r.reason == router::fallback::Reason::RoutingFailed)
    );
    assert!(
        fs::read_to_string(root.join("notes-input"))
            .unwrap()
            .contains(&selection.render())
    );
    assert!(!root.join("translation-input").exists());
}
