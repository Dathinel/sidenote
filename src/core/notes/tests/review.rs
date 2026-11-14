use super::routing::{dir, selected, setup};
use super::*;
use crate::core::meeting_router::{jobs, revision};
fn disable(root: &Path) {
    let p = root.join("config.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    v["labs"]["after_stop_language_routing"] = false.into();
    fs::write(p, v.to_string()).unwrap();
}
#[test]
fn persisted_busy_pin_and_corrupt_pin_are_ignored_after_disable_without_enqueue() {
    for corrupt in [false, true] {
        let (root, folder, s) = setup();
        let (id, _) = selected(&root.0, &folder);
        enqueue(&root.0, &s, None);
        let mut busy = Fake::new(vec![Err(Failure::Busy)]);
        drain(&root.0, &s, &mut busy);
        let mut job = queue::read(&root.0, ID).unwrap().unwrap();
        assert_eq!(job.transcript_revision.as_deref(), Some(id.as_str()));
        if corrupt {
            fs::write(
                dir(&root.0).join("revisions").join(format!("{id}.json")),
                b"bad pinned bytes",
            )
            .unwrap();
        }
        job.message = "waiting for transcript revision".into();
        job.next_try_at = Some(i64::MAX);
        store_io::write(
            &root.0.join("local/notes-jobs").join(format!("{ID}.json")),
            &job,
        )
        .unwrap();
        disable(&root.0);
        let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
        drain(&root.0, &s, &mut fake);
        assert!(fake.calls[0].0.contains(PRIVATE));
        assert!(!fake.calls[0].0.contains("తెలుగు selected revision"));
        assert!(
            queue::read(&root.0, ID)
                .unwrap()
                .unwrap()
                .transcript_revision
                .is_none()
        );
    }
}
#[test]
fn corrupt_latest_waits_then_uses_hex_and_preserves_bad_bytes_and_edited_notes() {
    for malformed in [true, false] {
        let (root, folder, s) = setup();
        let (old, data) = selected(&root.0, &folder);
        let latest = dir(&root.0).join("transcript.segments.v1.json");
        let bad = if malformed {
            b"{broken".to_vec()
        } else {
            let mut v: serde_json::Value =
                serde_json::from_slice(&fs::read(&latest).unwrap()).unwrap();
            v["segments"][0]["text_original"] = "tampered".into();
            serde_json::to_vec(&v).unwrap()
        };
        fs::write(&latest, &bad).unwrap();
        enqueue(&root.0, &s, None);
        let mut pending = Fake::new(vec![]);
        drain(&root.0, &s, &mut pending);
        assert!(matches!(state(&root.0), JobState::Queued));
        let manifest = folder.join("manifest.json");
        let mut m: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        m["ended_at_ms"] = (chrono::Utc::now().timestamp_millis() - 400_000).into();
        fs::write(manifest, m.to_string()).unwrap();
        let mut job = queue::read(&root.0, ID).unwrap().unwrap();
        job.next_try_at = None;
        store_io::write(
            &root.0.join("local/notes-jobs").join(format!("{ID}.json")),
            &job,
        )
        .unwrap();
        let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
        drain(&root.0, &s, &mut fake);
        assert!(fake.calls[0].0.contains(PRIVATE));
        assert_eq!(fs::read(&latest).unwrap(), bad);
        let chosen = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision
            .unwrap();
        assert_ne!(chosen, old);
        assert!(
            revision::load(&dir(&root.0), Some(&chosen))
                .unwrap()
                .unwrap()
                .1
                .transcript
                .degraded()
        );
        let edited = summary().replace("Plan the delivery", "Owner edited these notes");
        fs::write(folder.join("english-summary.json"), &edited).unwrap();
        jobs::handoff::tests::complete(&root.0, ID, &data.transcript.baseline, "hi").unwrap();
        enqueue(&root.0, &s, None);
        let mut again = Fake::new(vec![]);
        drain(&root.0, &s, &mut again);
        assert_eq!(
            fs::read_to_string(folder.join("english-summary.json")).unwrap(),
            edited
        );
    }
}
#[test]
fn edit_and_new_batch_revision_during_helper_never_replace_the_owners_notes() {
    struct Edit<'a> {
        root: &'a Path,
        folder: &'a Path,
        baseline: &'a [crate::core::meeting_router::Chunk],
        calls: usize,
    }
    impl Run for Edit<'_> {
        fn installed(&self, _: Helper) -> bool {
            true
        }
        fn run(
            &mut self,
            _: Helper,
            _: &str,
            prompt: String,
            _: Duration,
        ) -> Result<String, Failure> {
            assert!(prompt.contains("తెలుగు selected revision"));
            self.calls += 1;
            jobs::handoff::tests::complete(self.root, ID, self.baseline, "hi").unwrap();
            fs::write(
                self.folder.join("english-summary.json"),
                summary().replace("Plan the delivery", "Owner edit during generation"),
            )
            .unwrap();
            Ok(summary())
        }
    }
    let (root, folder, s) = setup();
    let (_, data) = selected(&root.0, &folder);
    enqueue(&root.0, &s, None);
    let mut helper = Edit {
        root: &root.0,
        folder: &folder,
        baseline: &data.transcript.baseline,
        calls: 0,
    };
    queue::drain(&root.0, &root.0.join("meetings"), &mut helper, || {
        Ok(s.clone())
    })
    .unwrap();
    assert_eq!(helper.calls, 1);
    assert_eq!(
        fs::read_to_string(folder.join("english-summary.json")).unwrap(),
        summary().replace("Plan the delivery", "Owner edit during generation")
    );
}
