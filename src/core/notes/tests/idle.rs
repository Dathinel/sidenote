use super::*;
use crate::core::recording_source::{Session, folders::Discovery};

#[test]
fn changed_same_name_input_replaces_production_done_notes_after_restart() {
    let root = Temp::new();
    let settings = settings_fixture();
    let folder = meeting(&root.0, "Synthetic original");
    let meetings = root.0.join("meetings");
    let cursor = root.0.join("local/recording-source.json");
    let discover = || {
        let mut config = crate::core::Config::default();
        config.notes = Some(settings.clone());
        Session::open(Discovery::new(meetings.clone()).unwrap(), cursor.clone())
            .unwrap()
            .drain(|m| {
                crate::listener::completion::handoff_in(
                    &root.0,
                    &meetings,
                    m.id(),
                    Ok(config.clone()),
                    || queue::enqueue_in(&root.0, &meetings, m.id(), None, &settings),
                )?
                .acknowledge()
            })
            .unwrap();
    };
    discover();
    drain(
        &root.0,
        &settings,
        &mut Fake::new(vec![Ok(summary()), Ok(sorted())]),
    );
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    for restore in [false, true] {
        if restore {
            fs::rename(&folder, root.0.join("saved")).unwrap();
        }
        meeting(
            &root.0,
            if restore {
                "Synthetic restored"
            } else {
                "Synthetic rewritten"
            },
        );
        if restore {
            for name in ["english-summary.json", "english-transcript.md"] {
                fs::copy(root.0.join("saved").join(name), folder.join(name)).unwrap();
            }
            fs::remove_file(root.0.join(format!("local/notes-jobs/{ID}.json"))).unwrap();
        }
        discover();
        assert_eq!(
            state(&root.0),
            JobState::Queued,
            "changed Done job was not queued"
        );
        drain(&root.0, &settings, &mut Fake::new(vec![Err(Failure::Busy)]));
        let changed_summary = summary().replace(
            "Plan the delivery",
            if restore {
                "Restored notes"
            } else {
                "Rewritten notes"
            },
        );
        let mut fake = Fake::new(vec![Ok(changed_summary), Ok(sorted())]);
        drain(&root.0, &settings, &mut fake);
        assert!(fake.calls[0].0.contains(if restore {
            "Synthetic restored"
        } else {
            "Synthetic rewritten"
        }));
        assert!(matches!(state(&root.0), JobState::Done { .. }));
        assert!(
            fs::read_to_string(folder.join("english-summary.json"))
                .unwrap()
                .contains(if restore {
                    "Restored notes"
                } else {
                    "Rewritten notes"
                })
        );
        discover();
        assert!(matches!(state(&root.0), JobState::Done { .. }));
    }
}

#[test]
fn worker_skips_production_done_jobs_without_capture_or_launch() {
    let root = Temp::new();
    let s = settings_fixture();
    meeting(&root.0, &"Synthetic ".repeat(20_000));
    enqueue(&root.0, &s, None);
    drain(
        &root.0,
        &s,
        &mut Fake::new(vec![Ok(summary()), Ok(sorted())]),
    );
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    queue::start_worker_in(&root.0, &root.0.join("meetings"), 0, || {
        panic!("terminal warmup launched")
    })
    .unwrap();
    let before = read_bytes();
    let mut launches = 0;
    queue::start_worker_in(
        &root.0,
        &root.0.join("meetings"),
        chrono::Utc::now().timestamp_millis(),
        || {
            launches += 1;
            Ok(())
        },
    )
    .unwrap();
    let bytes = read_bytes() - before;
    println!("retained Done: read_bytes={bytes}, launches={launches}");
    assert_eq!(launches, 0, "terminal notes launched a worker");
    assert!(
        bytes < 64 * 1024,
        "terminal notes captured transcript: {bytes}"
    );
    assert_eq!(bytes, 0, "unchanged terminal job was reread");
    queue::enqueue_in(
        &root.0,
        &root.0.join("meetings"),
        ID,
        Some(Request::SortAgain),
        &s,
    )
    .unwrap();
    queue::start_worker_in(&root.0, &root.0.join("meetings"), 0, || {
        launches += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(launches, 1, "cached terminal job hid newly queued work");
}

#[test]
fn same_name_short_input_can_become_eligible_and_missing_done_output_recovers() {
    let root = Temp::new();
    let s = settings_fixture();
    let folder = meeting(&root.0, "Synthetic initial");
    fs::write(
        folder.join("transcript.ndjson"),
        r#"{"start_ms":0,"text":"Short"}"#,
    )
    .unwrap();
    enqueue(&root.0, &s, None);
    assert!(queue::read(&root.0, ID).unwrap().is_none());
    meeting(&root.0, "Synthetic eligible replacement");
    let receipt = root.0.join(format!("local/recording-guard/{ID}.json"));
    let before = fs::read(&receipt).unwrap();
    {
        let _readonly = crate::core::ReadOnlyScope::begin(true);
        assert_eq!(
            crate::core::recording_guard::eligibility(&root.0, &root.0.join("meetings"), ID)
                .unwrap(),
            crate::core::recording_guard::Eligibility::Eligible
        );
        assert_eq!(fs::read(&receipt).unwrap(), before);
    }
    enqueue(&root.0, &s, None);
    drain(
        &root.0,
        &s,
        &mut Fake::new(vec![Ok(summary()), Ok(sorted())]),
    );
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    fs::remove_file(folder.join("english-summary.json")).unwrap();
    enqueue(&root.0, &s, None);
    assert_eq!(state(&root.0), JobState::Queued);
    drain(
        &root.0,
        &s,
        &mut Fake::new(vec![Ok(summary()), Ok(sorted())]),
    );
    assert!(matches!(state(&root.0), JobState::Done { .. }));
}

#[test]
fn worker_keeps_queued_and_due_failure_retries_but_defers_future_retry() {
    let root = Temp::new();
    let s = settings_fixture();
    meeting(&root.0, "Synthetic retry");
    enqueue(&root.0, &s, None);
    let mut launches = 0;
    queue::start_worker_in(
        &root.0,
        &root.0.join("meetings"),
        chrono::Utc::now().timestamp_millis(),
        || {
            launches += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(launches, 1);
    drain(
        &root.0,
        &s,
        &mut Fake::new(vec![Err(Failure::Backend { exit: None })]),
    );
    queue::start_worker_in(
        &root.0,
        &root.0.join("meetings"),
        chrono::Utc::now().timestamp_millis(),
        || {
            launches += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(launches, 1, "future retry launched early");
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    let due = job.next_try_at.unwrap();
    queue::start_worker_in(&root.0, &root.0.join("meetings"), due, || {
        launches += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(launches, 2, "due retry was lost");
    job.state = JobState::BackendError {
        next_try_ms: Some(0),
        exit: None,
    };
    job.next_try_at = Some(0);
    store_io::write(&root.0.join(format!("local/notes-jobs/{ID}.json")), &job).unwrap();
    drain(
        &root.0,
        &s,
        &mut Fake::new(vec![Ok(summary()), Ok(sorted())]),
    );
    assert!(matches!(state(&root.0), JobState::Done { .. }));
}

fn read_bytes() -> u64 {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::*;
        let mut counters = std::mem::zeroed();
        assert_ne!(GetProcessIoCounters(GetCurrentProcess(), &mut counters), 0);
        counters.ReadTransferCount
    }
    #[cfg(not(windows))]
    {
        0
    }
}
