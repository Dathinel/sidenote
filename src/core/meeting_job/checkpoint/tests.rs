use super::*;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

const INPUTS: [u8; 32] = [7; 32];

fn finish(job: &mut Job) {
    while job.advance(|_, _, _| Ok(Availability::Completed)).unwrap() {}
    assert_eq!(job.completed().len(), STAGES.len());
    assert_eq!(job.failure(), None);
}

#[test]
fn two_triggers_execute_one_job_and_share_the_legacy_claim() {
    let temp = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut job = Job::open(temp.path(), INPUTS).unwrap();
    let other = barrier.clone();
    let other_calls = calls.clone();
    let folder = temp.path().to_owned();
    let trigger = std::thread::spawn(move || {
        other.wait();
        let result = Job::open(&folder, INPUTS);
        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::WouldBlock));
        assert!(claim(&folder).is_err());
        other_calls.load(Ordering::SeqCst)
    });
    barrier.wait();
    trigger.join().unwrap();
    while job
        .advance(|_, _, action| {
            assert_eq!(action, Action::Start);
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Availability::Completed)
        })
        .unwrap()
    {}
    drop(job);
    let mut duplicate = Job::open(temp.path(), INPUTS).unwrap();
    assert!(
        !duplicate
            .advance(|_, _, _| panic!("duplicate stage"))
            .unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), STAGES.len());
}

#[test]
fn resume_after_every_completed_boundary_skips_the_completed_prefix() {
    use crate::platform::CommandExt;
    for boundary in 0..=STAGES.len() {
        let temp = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "core::meeting_job::checkpoint::tests::boundary_exit_worker",
            ])
            .env("SYNTHETIC_JOB_FOLDER", temp.path())
            .env("SYNTHETIC_JOB_BOUNDARY", boundary.to_string())
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77));
        let mut restarted = Job::open(temp.path(), INPUTS).unwrap();
        assert_eq!(restarted.completed().len(), boundary);
        let mut remaining = STAGES[boundary..].iter();
        while restarted
            .advance(|stage, _, action| {
                assert_eq!(action, Action::Start);
                assert_eq!(Some(&stage), remaining.next());
                Ok(Availability::Completed)
            })
            .unwrap()
        {}
        assert_eq!(remaining.next(), None);
    }
}

#[test]
fn boundary_exit_worker() {
    let Some(folder) = std::env::var_os("SYNTHETIC_JOB_FOLDER") else {
        return;
    };
    let boundary: usize = std::env::var("SYNTHETIC_JOB_BOUNDARY")
        .unwrap()
        .parse()
        .unwrap();
    let mut job = Job::open(Path::new(&folder), INPUTS).unwrap();
    for stage in STAGES.iter().take(boundary) {
        assert!(
            job.advance(|actual, _, action| {
                assert_eq!((actual, action), (*stage, Action::Start));
                Ok(Availability::Completed)
            })
            .unwrap()
        );
    }
    // Deliberately bypass Drop: restart must reclaim the OS handle and completed journal.
    std::process::exit(77);
}

#[test]
fn interruption_before_or_after_stage_commit_recovers_the_same_key() {
    for (boundary, expected) in STAGES.iter().copied().enumerate() {
        for committed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut receipt = None;
            let mut first_key = String::new();
            let mut effects = 0;
            let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut job = Job::open(temp.path(), INPUTS).unwrap();
                for _ in 0..boundary {
                    job.advance(|_, _, _| Ok(Availability::Completed)).unwrap();
                }
                job.advance(|stage, key, action| {
                    assert_eq!((stage, action), (expected, Action::Start));
                    first_key = key.to_owned();
                    if committed {
                        receipt = Some(key.to_owned());
                        effects += 1;
                    }
                    panic!("synthetic interruption before checkpoint");
                })
                .unwrap();
            }));
            assert!(stopped.is_err());
            let mut restarted = Job::open(temp.path(), INPUTS).unwrap();
            assert_eq!(restarted.completed().len(), boundary);
            assert!(
                restarted
                    .advance(|stage, key, action| {
                        assert_eq!((stage, action), (expected, Action::Recover));
                        assert_eq!(key, first_key);
                        if receipt.as_deref() != Some(key) {
                            effects += 1;
                        }
                        Ok(Availability::Completed)
                    })
                    .unwrap()
            );
            finish(&mut restarted);
            assert_eq!(effects, 1);
        }
    }
}

#[test]
fn unavailable_stages_are_nonfatal_and_input_history_is_retained() {
    let temp = tempfile::tempdir().unwrap();
    let mut job = Job::open(temp.path(), INPUTS).unwrap();
    while job
        .advance(|stage, _, _| {
            Ok(if stage == Stage::Translate {
                Availability::NotAvailable
            } else {
                Availability::Completed
            })
        })
        .unwrap()
    {}
    assert_eq!(job.completed()[4], Availability::NotAvailable);
    assert_eq!(job.failure(), None);
    drop(job);
    let mut changed = Job::open(temp.path(), [8; 32]).unwrap();
    assert_eq!(changed.next_stage(), Some(Stage::Discover));
    finish(&mut changed);
    drop(changed);
    let mut original = Job::open(temp.path(), INPUTS).unwrap();
    assert!(
        !original
            .advance(|_, _, _| panic!("input A was already processed"))
            .unwrap()
    );
    assert_eq!(original.completed()[4], Availability::NotAvailable);
}

#[test]
fn each_stage_retains_its_existing_typed_cause_and_recovers() {
    for (boundary, expected) in STAGES.iter().copied().enumerate() {
        for cause in [Cause::Limit, Cause::Auth, Cause::TimedOut, Cause::Other] {
            let temp = tempfile::tempdir().unwrap();
            let mut job = Job::open(temp.path(), INPUTS).unwrap();
            for _ in 0..boundary {
                job.advance(|_, _, _| Ok(Availability::Completed)).unwrap();
            }
            assert!(!job.advance(|_, _, _| Err(cause)).unwrap());
            drop(job);
            let mut restarted = Job::open(temp.path(), INPUTS).unwrap();
            assert_eq!(restarted.failure(), Some((expected, cause)));
            assert!(
                restarted
                    .advance(|stage, _, action| {
                        assert_eq!((stage, action), (expected, Action::Recover));
                        Ok(Availability::Completed)
                    })
                    .unwrap()
            );
            assert_eq!(restarted.failure(), None);
        }
    }
}

#[test]
fn invalid_journals_fail_closed_and_io_diagnostics_are_content_free() {
    let temp = tempfile::tempdir().unwrap();
    let mut job = Job::open(temp.path(), INPUTS).unwrap();
    job.advance(|_, _, _| Ok(Availability::Completed)).unwrap();
    let path = job.path.clone();
    let valid = serde_json::to_value(&job.journal).unwrap();
    drop(job);
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("inputs", serde_json::to_value([0; 32]).unwrap()),
        (
            "completed",
            serde_json::json!(vec![Availability::Completed; 10]),
        ),
        ("failure", serde_json::json!(Cause::Auth)),
        ("unknown", serde_json::json!("synthetic private text")),
    ] {
        let mut corrupt = valid.clone();
        corrupt[field] = value;
        store_io::write(&path, &corrupt).unwrap();
        let error = Job::open(temp.path(), INPUTS).err().unwrap();
        assert_eq!(error.to_string(), "MEETING_JOB (InvalidData)");
    }
    store_io::atomic_write(&path, b"synthetic private text").unwrap();
    let error = Job::open(temp.path(), INPUTS).err().unwrap();
    assert!(!error.to_string().contains("private"));
}

#[test]
fn failed_start_and_completion_writes_never_authorize_a_duplicate_start() {
    let temp = tempfile::tempdir().unwrap();
    let mut job = Job::open(temp.path(), INPUTS).unwrap();
    let path = job.path.clone();
    std::fs::create_dir_all(&path).unwrap();
    assert!(
        job.advance(|_, _, _| panic!("start was not saved"))
            .is_err()
    );
    std::fs::remove_dir(&path).unwrap();
    let saved = temp.path().join("synthetic-started.json");
    let mut original_key = String::new();
    assert!(
        job.advance(|_, key, action| {
            assert_eq!(action, Action::Start);
            original_key = key.to_owned();
            std::fs::rename(&path, &saved).unwrap();
            std::fs::create_dir(&path).unwrap();
            Ok(Availability::Completed)
        })
        .is_err()
    );
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&saved, &path).unwrap();
    assert!(
        job.advance(|stage, key, action| {
            assert_eq!((stage, action), (Stage::Discover, Action::Recover));
            assert_eq!(key, original_key);
            Ok(Availability::Completed)
        })
        .unwrap()
    );
    assert_eq!(job.completed().len(), 1);
}
