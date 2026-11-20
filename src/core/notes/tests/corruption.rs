use super::routing::{dir, selected};
use super::*;
use crate::core::meeting_router::{jobs, revision};

fn expire(root: &Path, folder: &Path) -> i64 {
    let ended = chrono::Utc::now().timestamp_millis() - 400_000;
    let path = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["ended_at_ms"] = ended.into();
    fs::write(path, m.to_string()).unwrap();
    let mut job = queue::read(root, ID).unwrap().unwrap();
    job.next_try_at = None;
    store_io::write(
        &root.join("local/notes-jobs").join(format!("{ID}.json")),
        &job,
    )
    .unwrap();
    ended
}

#[test]
fn labs_on_busy_pin_corruption_waits_then_uses_hex_without_adopting_a_later_route() {
    for kind in ["truncated", "missing", "checksum"] {
        let (root, folder, settings) = routing_setup();
        let (pin, data) = selected(&root.0, &folder);
        enqueue(&root.0, &settings, None);
        drain(&root.0, &settings, &mut Fake::new(vec![Err(Failure::Busy)]));
        assert_eq!(
            queue::read(&root.0, ID)
                .unwrap()
                .unwrap()
                .transcript_revision
                .as_deref(),
            Some(pin.as_str())
        );
        let path = dir(&root.0).join("revisions").join(format!("{pin}.json"));
        let bad = match kind {
            "missing" => {
                fs::remove_file(&path).unwrap();
                None
            }
            "checksum" => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["segments"][0]["text_original"] = "changed cached reading".into();
                let bytes = serde_json::to_vec(&value).unwrap();
                fs::write(&path, &bytes).unwrap();
                Some(bytes)
            }
            _ => {
                let bytes = b"{truncated".to_vec();
                fs::write(&path, &bytes).unwrap();
                Some(bytes)
            }
        };
        let later =
            jobs::handoff::tests::complete(&root.0, ID, &data.transcript.baseline, "hi").unwrap();
        let latest = dir(&root.0).join("transcript.segments.v1.json");
        let selected_bytes = fs::read(&latest).unwrap();
        let mut waiting = Fake::new(vec![]);
        drain(&root.0, &settings, &mut waiting);
        assert!(waiting.calls.is_empty());
        assert!(matches!(state(&root.0), JobState::Queued));
        assert_eq!(
            queue::read(&root.0, ID)
                .unwrap()
                .unwrap()
                .transcript_revision
                .as_deref(),
            Some(pin.as_str())
        );
        let ended = expire(&root.0, &folder);
        // Make the unrelated release eligible too: the damaged pin still must choose Hex.
        let release = root
            .0
            .join("local/router-work")
            .join(ID)
            .join("released.json");
        let mut released: serde_json::Value =
            serde_json::from_slice(&fs::read(&release).unwrap()).unwrap();
        released["completed_ms"] = (ended + 1).into();
        fs::write(release, released.to_string()).unwrap();
        drain(&root.0, &settings, &mut Fake::new(vec![Err(Failure::Busy)]));
        let retained_pin = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .unwrap();
        let mut helper = Fake::new(vec![Ok(summary()), Ok(sorted())]);
        drain(&root.0, &settings, &mut helper);
        assert!(helper.calls[0].0.contains(PRIVATE));
        assert!(helper.calls[0].0.contains("(English):"));
        assert!(!helper.calls[0].0.contains("selected revision"));
        assert!(matches!(state(&root.0), JobState::Done { .. }));
        let fallback = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .unwrap();
        assert_ne!(fallback, pin);
        assert_ne!(fallback, later);
        assert_eq!(
            fallback, retained_pin,
            "retry must retain the fallback selection"
        );
        export_status(&root.0, &folder, kind);
        let status = crate::core::notes::status_record(&root.0, ID).unwrap();
        assert_eq!(
            status.transcript_revision.as_deref(),
            Some(fallback.as_str())
        );
        assert!(matches!(
            status.state,
            JobState::Done {
                fallback: false,
                ..
            }
        ));
        assert!(status.routing_message().contains("missing or damaged"));
        assert_eq!(
            status.routing_fallbacks[0].reason,
            crate::core::meeting_router::fallback::Reason::PinnedRevisionUnavailable
        );
        assert!(!helper.calls[0].0.contains(&status.routing_message()));
        let (_, saved) = revision::load(&dir(&root.0), Some(&fallback))
            .unwrap()
            .unwrap();
        assert_eq!(saved.provenance["unavailable_revision"], pin);
        assert!(saved.transcript.segments.iter().all(|s| s.lang == "en"));
        assert_eq!(fs::read(&latest).unwrap(), selected_bytes);
        match bad {
            Some(bytes) => assert_eq!(fs::read(&path).unwrap(), bytes),
            None => assert!(!path.exists()),
        }
        assert!(
            crate::core::Config::load_in(&root.0)
                .unwrap()
                .labs
                .after_stop_language_routing
        );
    }
}

#[test]
fn labs_on_malformed_pending_waits_then_uses_hex_and_retains_the_record() {
    for bad in [
        "{truncated".to_owned(),
        "null".to_owned(),
        serde_json::json!({"id":ID,"generation":"invalid"}).to_string(),
        serde_json::json!({"id":ID,"generation":0}).to_string(),
    ] {
        let (root, folder, settings) = routing_setup();
        enqueue(&root.0, &settings, None);
        let pending = root.0.join("local/router-queue").join(format!("{ID}.json"));
        fs::create_dir_all(pending.parent().unwrap()).unwrap();
        fs::write(&pending, &bad).unwrap();
        let mut waiting = Fake::new(vec![]);
        drain(&root.0, &settings, &mut waiting);
        assert!(waiting.calls.is_empty());
        assert!(matches!(state(&root.0), JobState::Queued));
        expire(&root.0, &folder);
        drain(&root.0, &settings, &mut Fake::new(vec![Err(Failure::Busy)]));
        let retained_pin = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .unwrap();
        let mut helper = Fake::new(vec![Ok(summary()), Ok(sorted())]);
        drain(&root.0, &settings, &mut helper);
        assert!(helper.calls[0].0.contains(PRIVATE));
        assert!(helper.calls[0].0.contains("(English):"));
        assert!(matches!(state(&root.0), JobState::Done { .. }));
        assert_eq!(fs::read_to_string(&pending).unwrap(), bad);
        let pin = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .unwrap();
        let (_, saved) = revision::load(&dir(&root.0), Some(&pin)).unwrap().unwrap();
        assert_eq!(pin, retained_pin);
        export_status(&root.0, &folder, "pending");
        let status = crate::core::notes::status_record(&root.0, ID).unwrap();
        assert!(matches!(
            status.state,
            JobState::Done {
                fallback: false,
                ..
            }
        ));
        assert!(
            status
                .routing_message()
                .contains("request record was damaged")
        );
        assert_eq!(
            status.routing_fallbacks[0].reason,
            crate::core::meeting_router::fallback::Reason::PendingRecordInvalid
        );
        assert!(!helper.calls[0].0.contains(&status.routing_message()));
        assert_eq!(saved.provenance["pending_unusable"], true);
        assert!(saved.generation.parse::<u128>().unwrap() > 0);
        assert!(saved.transcript.segments.iter().all(|s| s.lang == "en"));
        assert!(
            crate::core::Config::load_in(&root.0)
                .unwrap()
                .labs
                .after_stop_language_routing
        );
    }
}

fn export_status(root: &Path, folder: &Path, case: &str) {
    let Some(target) = std::env::var_os("SIDENOTE_ROUTING_R1_STATUS_OUT") else {
        return;
    };
    let target = PathBuf::from(target);
    assert!(crate::core::is_proof_path(
        &target,
        "sidenote-routing-r1-status-public"
    ));
    let out = target.join(case);
    let job = out.join("local/notes-jobs").join(format!("{ID}.json"));
    let meeting = out.join("meetings").join(ID);
    fs::create_dir_all(job.parent().unwrap()).unwrap();
    fs::create_dir_all(&meeting).unwrap();
    fs::copy(
        root.join("local/notes-jobs").join(format!("{ID}.json")),
        job,
    )
    .unwrap();
    for name in ["manifest.json", "transcript.ndjson", "english-summary.json"] {
        fs::copy(folder.join(name), meeting.join(name)).unwrap();
    }
}
