//! Exercise the actual hidden worker entry points with synthetic pre-existing jobs.
use serde_json::json;
use sidenote::platform::CommandExt;
use std::{fs, path::Path, process::Command};

fn write(path: &Path, value: serde_json::Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value.to_string()).unwrap();
}
#[test]
fn old_queued_short_jobs_are_skipped_by_both_worker_processes() {
    let root = std::env::temp_dir().join(format!(
        "rec-guard-worker-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let state = root.join("state");
    let meetings = root.join("meetings");
    let id = "1793946600000-1";
    let folder = meetings.join(id);
    write(
        &state.join("config.json"),
        json!({"version":1,"features":{},"pipeline":{"profile":"experimental"},"notes":{"engine":"app","helper":null}}),
    );
    write(
        &folder.join("manifest.json"),
        json!({"status":"complete","duration_ms":29999,"created_at_ms":1793946600000_i64}),
    );
    write(
        &folder.join("transcript.ndjson"),
        json!({"text":"synthetic abandoned recording"}),
    );
    let notes = state.join("local/notes-jobs").join(format!("{id}.json"));
    write(
        &notes,
        json!({"state":"queued","attempts":0,"next_try_at":null,"helper":null,"model":"","instructions":"default","updated_at":1,"message":"queued","request":null}),
    );
    let routing = state.join("local/router-queue").join(format!("{id}.json"));
    write(&routing, json!({"id":id,"generation":1}));
    let short_id = "1793946600000-2";
    write(
        &meetings.join(short_id).join("manifest.json"),
        json!({"status":"complete","duration_ms":96000}),
    );
    write(
        &meetings.join(short_id).join("transcript.ndjson"),
        json!({"text":""}),
    );
    let unknown_id = "1793946600000-3";
    write(
        &meetings.join(unknown_id).join("manifest.json"),
        json!({"status":"complete","duration_ms":30000}),
    );
    sidenote::core::configure_paths(&[
        "proof".into(),
        "--data-dir".into(),
        state.clone().into_os_string(),
        "--meetings-dir".into(),
        meetings.clone().into_os_string(),
        "--peers-dir".into(),
        root.join("peers").into_os_string(),
    ])
    .unwrap();
    sidenote::core::Config::load_in(&state).expect("synthetic config must load");
    sidenote::core::notes::queue::catch_up().unwrap();
    for id in [short_id, unknown_id] {
        assert!(
            !state
                .join("local/notes-jobs")
                .join(format!("{id}.json"))
                .exists()
        );
    }
    let notes_before = fs::read(&notes).unwrap();
    let routing_before = fs::read(&routing).unwrap();
    for flag in ["--notes-pending", "--route-pending"] {
        let output = Command::new(env!("CARGO_BIN_EXE_sidenote"))
            .arg(flag)
            .arg("--automatic")
            .arg("--data-dir")
            .arg(&state)
            .arg("--meetings-dir")
            .arg(&meetings)
            .arg("--peers-dir")
            .arg(root.join("peers"))
            .env("APPDATA", root.join("appdata"))
            .env("LOCALAPPDATA", root.join("localappdata"))
            .env("USERPROFILE", root.join("home"))
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(output.status.success(), "worker failed: {flag}");
    }
    assert_eq!(fs::read(&notes).unwrap(), notes_before);
    assert_eq!(fs::read(&routing).unwrap(), routing_before);
    assert!(!folder.join("english-transcript.md").exists());
    assert!(!state.join("local/router-work").exists());
    let decision: serde_json::Value = serde_json::from_slice(
        &fs::read(
            state
                .join("local/recording-guard")
                .join(format!("{id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(decision["short"], true);
    assert!(decision["detected_at"].is_i64());
}
