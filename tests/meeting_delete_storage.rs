use sidenote::core::meeting_delete::{
    DeleteError, Eligibility, MoveEvent, Phase, Storage, WriteStage, suppressed_ids,
};
use std::{fs, path::PathBuf};

struct Fixture {
    temp: tempfile::TempDir,
    meetings: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let meetings = temp.path().join("meetings");
        let state = temp.path().join("state");
        for dir in [&meetings, &state] {
            fs::create_dir(dir).unwrap();
        }
        Self {
            temp,
            meetings,
            state,
        }
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn storage(&self) -> Storage {
        Storage::new(&self.meetings, &self.state).unwrap()
    }
    fn session(&self, id: &str, parts: &[&str]) {
        let parts: Vec<_> = parts
            .iter()
            .map(|id| serde_json::json!({"id":id,"started_ms":0,"duration_ms":1}))
            .collect();
        self.write(
            &format!("state/local/meeting-parts/{id}.json"),
            &serde_json::to_vec(&serde_json::json!({"id":id,"parts":parts,"phase":"Ended"}))
                .unwrap(),
        );
        self.write(
            &format!("state/local/meeting-inputs/{id}/manifest.json"),
            b"{}",
        );
    }
}

#[test]
fn inventory_fingerprints_every_hex_file_including_sidecars() {
    let f = Fixture::new();
    f.write("meetings/10/manifest.json", b"{}");
    f.write("meetings/10/unknown/notes.md", b"original");
    f.write("meetings/10/notes.md", b"sidecar");
    f.write("meetings/10/system.wav", b"synthetic WAV");
    f.write("meetings/10/transcript.ndjson", b"synthetic raw");
    f.write(
        "meetings/10/transcript.rolling.ndjson",
        b"synthetic rolling",
    );
    let inventory = f.storage().inventory("10").unwrap();
    assert!(inventory.members.is_empty());
    let notes = inventory
        .hex_inputs
        .iter()
        .find(|f| f.relative == std::path::Path::new("10/notes.md"))
        .unwrap();
    assert_eq!(notes.bytes, 7);
    assert_eq!(notes.hash, blake3::hash(b"sidecar").to_hex().as_str());
    let raw = inventory
        .hex_inputs
        .iter()
        .find(|f| f.relative == std::path::Path::new("10/unknown/notes.md"))
        .unwrap();
    assert_eq!(raw.bytes, 8);
    assert_eq!(raw.hash, blake3::hash(b"original").to_hex().as_str());
    assert!(
        inventory
            .hex_inputs
            .iter()
            .any(|f| f.directory && f.relative == std::path::Path::new("10"))
    );
}

#[test]
fn invalid_source_paths_and_manifests_are_refused() {
    let f = Fixture::new();
    let storage = f.storage();
    assert!(storage.inventory("../10").is_err());
    assert!(storage.inventory("10").is_err());
    f.write("meetings/10/manifest.json", b"[]");
    assert!(storage.inventory("10").is_err());
    f.write("meetings/10/manifest.json", b"{}");
    let manifest = f.meetings.join("10/manifest.json");
    fs::hard_link(&manifest, manifest.with_extension("alias")).unwrap();
    assert!(storage.inventory("10").is_err());
}

#[cfg(windows)]
#[test]
fn junction_in_owned_tree_refuses_inventory() {
    let f = Fixture::new();
    f.write("meetings/10/manifest.json", b"{}");
    let junction = f.meetings.join("10").join("junction");
    assert!(
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&f.state)
            .status()
            .unwrap()
            .success()
    );
    assert!(f.storage().inventory("10").is_err());
    assert!(junction.is_dir());
}

fn bytes_at(path: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut pending = vec![path.to_owned()];
    let mut bytes = Vec::new();
    while let Some(file) = pending.pop() {
        if file.is_dir() {
            pending.extend(
                fs::read_dir(file)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else {
            bytes.push((
                file.strip_prefix(path).unwrap().to_owned(),
                fs::read(file).unwrap(),
            ));
        }
    }
    bytes.sort();
    bytes
}

#[test]
fn multipart_inventory_attributes_only_exclusive_parts_and_state() {
    let f = Fixture::new();
    f.session("10", &["10", "11"]);
    f.write("meetings/10/manifest.json", b"{}");
    f.write("meetings/11/manifest.json", b"{}");
    f.write("meetings/12/manifest.json", b"{}");
    f.write("state/local/evidence/11/audio.wav", b"owned");
    let store = f.storage();
    let inventory = store.inventory("10").unwrap();
    assert_eq!(inventory.ids.into_iter().collect::<Vec<_>>(), ["10", "11"]);
    assert_eq!(inventory.members.len(), 3);
    assert_eq!(
        bytes_at(&f.state.join("local/evidence/11")),
        [(PathBuf::from("audio.wav"), b"owned".to_vec())]
    );
    assert_eq!(
        inventory
            .hex_inputs
            .iter()
            .filter(|entry| entry.directory)
            .count(),
        2
    );
    f.session("12", &["11"]);
    assert!(store.inventory("10").is_err());
    fs::remove_file(f.state.join("local/meeting-parts/12.json")).unwrap();
    fs::remove_file(f.meetings.join("11/manifest.json")).unwrap();
    assert!(store.inventory("10").is_err());
}

fn recorded_fixture(multipart: bool) -> Fixture {
    let f = Fixture::new();
    f.write("meetings/10/manifest.json", b"{}");
    f.write("meetings/10/unknown/bytes", b"\0\xff");
    f.write("meetings/10/notes.md", b"sidecar");
    f.write("meetings/10/system.wav", b"synthetic WAV");
    f.write("meetings/10/transcript.ndjson", b"synthetic raw");
    f.write(
        "meetings/10/transcript.rolling.ndjson",
        b"synthetic rolling",
    );
    f.write("state/local/evidence/10/audio.wav", b"synthetic");
    if multipart {
        f.session("10", &["10", "11"]);
        f.write("meetings/11/manifest.json", b"{}");
        f.write("state/local/notes-jobs/11.json", b"other");
    }
    f
}

fn rename_hex(f: &Fixture, replaced: bool) {
    fs::rename(f.meetings.join("10"), f.temp.path().join("original-hex")).unwrap();
    if replaced {
        f.write("meetings/10/manifest.json", b"{}");
        f.write("meetings/10/notes.md", b"replacement notes");
    }
}

fn assert_hex_unchanged(f: &Fixture, original: &[(PathBuf, Vec<u8>)], replaced: bool) {
    assert_eq!(bytes_at(&f.temp.path().join("original-hex")), original);
    let current = f.meetings.join("10");
    if replaced {
        assert_eq!(
            bytes_at(&current),
            [
                (PathBuf::from("manifest.json"), b"{}".to_vec()),
                (PathBuf::from("notes.md"), b"replacement notes".to_vec()),
            ]
        );
    } else {
        assert!(!current.exists());
    }
}

#[test]
fn journaled_stop_recovers_each_member() {
    for multipart in [false, true] {
        let f = recorded_fixture(multipart);
        let mut events = Vec::new();
        f.storage()
            .delete(
                "10",
                String::new(),
                10,
                |_| Ok(Eligibility::Ready),
                |event| {
                    events.push(event);
                    Ok(())
                },
            )
            .unwrap();
        for (stop, replaced) in
            (0..events.len()).flat_map(|stop| [false, true].map(|replace| (stop, replace)))
        {
            let f = recorded_fixture(multipart);
            let inventory = f.storage().inventory("10").unwrap();
            let hex_before = bytes_at(&f.meetings.join("10"));
            let mut event_number = 0;
            let result = f.storage().delete(
                "10",
                String::new(),
                10,
                |_| Ok(Eligibility::Ready),
                |event| {
                    assert_eq!(event, events[event_number]);
                    let at = event_number;
                    event_number += 1;
                    if at == stop {
                        rename_hex(&f, replaced);
                        return Err(std::io::Error::other("simulated stop"));
                    }
                    Ok(())
                },
            );
            let trash = f.state.join("local/deleted-meetings");
            let mut recycled = None;
            match result {
                Err(DeleteError::Failed(_)) => {}
                Err(DeleteError::Partial { transaction, .. }) => {
                    let transaction_dir = trash.join(&transaction);
                    if transaction_dir.join("receipt.json").exists() {
                        for _ in 0..2 {
                            assert_eq!(f.storage().recover(&transaction).unwrap(), Phase::Deleted);
                        }
                        recycled = Some(transaction_dir);
                    }
                }
                Ok(_) | Err(DeleteError::Refused(_)) => {
                    panic!("expected a failed or partial transaction")
                }
            }
            for (index, member) in inventory.members.iter().enumerate() {
                assert_eq!(f.state.join(&member.relative).exists(), recycled.is_none());
                if let Some(dir) = &recycled {
                    assert!(dir.join(index.to_string()).exists());
                }
            }
            assert_hex_unchanged(&f, &hex_before, replaced);
            let count = if recycled.is_none() {
                0
            } else if multipart {
                2
            } else {
                1
            };
            assert_eq!(suppressed_ids(&f.state).unwrap().len(), count);
        }
    }
}

#[test]
fn ancestry_is_established_before_receipt_and_move() {
    let steps = [
        MoveEvent::SyncState,
        MoveEvent::CreatedRecycleRoot,
        MoveEvent::SyncLocal,
        MoveEvent::CreatedTransaction,
        MoveEvent::SyncRecycleRoot,
    ];
    for stop in 0..=steps.len() {
        let f = Fixture::new();
        f.write("meetings/10/manifest.json", b"{}");
        f.write("meetings/10/notes.md", b"owned");
        assert!(!f.state.join("local").exists());
        let mut events = Vec::new();
        let result = f.storage().delete(
            "10",
            String::new(),
            10,
            |_| Ok(Eligibility::Ready),
            |event| {
                events.push(event);
                if steps.get(stop) == Some(&event) {
                    return Err(std::io::Error::other("directory operation failed"));
                }
                Ok(())
            },
        );
        if stop < steps.len() {
            assert!(matches!(result, Err(DeleteError::Failed(_))));
            assert_eq!(events, steps[..=stop]);
            assert_eq!(
                fs::read(f.meetings.join("10/manifest.json")).unwrap(),
                b"{}"
            );
            assert!(suppressed_ids(&f.state).unwrap().is_empty());
        } else {
            result.unwrap();
            assert!(events.starts_with(&steps));
            assert_eq!(
                events[steps.len()],
                MoveEvent::Journal(Phase::Deleting, WriteStage::Flush)
            );
        }
    }
}

#[test]
fn oversized_title_and_expanded_inventory_refuse_before_a_move() {
    let f = recorded_fixture(false);
    let s = f.storage();
    let txn = s
        .delete(
            "10",
            String::new(),
            10,
            |_| Ok(Eligibility::Ready),
            |_| Ok(()),
        )
        .unwrap();
    let baseline = fs::metadata(
        f.state
            .join("local/deleted-meetings")
            .join(txn)
            .join("receipt.json"),
    )
    .unwrap()
    .len();
    let cap = sidenote::core::file_reads::SUMMARY_MAX;
    for (multipart, size) in [(false, cap), (true, cap - baseline - 16)] {
        let f = recorded_fixture(multipart);
        let result = f.storage().delete(
            "10",
            "x".repeat(size as usize),
            10,
            |_| Ok(Eligibility::Ready),
            |_| panic!("oversized metadata advanced"),
        );
        assert!(matches!(result, Err(DeleteError::Failed(_))));
        assert_eq!(
            fs::read(f.meetings.join("10/unknown/bytes")).unwrap(),
            b"\0\xff"
        );
        assert_eq!(f.meetings.join("11").exists(), multipart);
        assert!(suppressed_ids(&f.state).unwrap().is_empty());
        assert!(!f.state.join("local/deleted-meetings").exists());
    }
}

#[test]
fn accumulated_suppression_limit_preserves_existing_metadata_and_source() {
    let f = recorded_fixture(false);
    let cap = sidenote::core::file_reads::SUMMARY_MAX as usize;
    let mut map = std::collections::BTreeMap::new();
    let owner = "a".repeat(32);
    for id in 0..((cap - 1) / 138) {
        map.insert(format!("{id:0100}"), owner.clone());
    }
    let slack = cap - serde_json::to_vec(&map).unwrap().len();
    if slack > 38 {
        map.insert("9".repeat(slack - 38), owner);
    }
    let bytes = serde_json::to_vec(&map).unwrap();
    f.write("state/local/deleted-meeting-ids.json", &bytes);
    let result = f.storage().delete(
        "10",
        String::new(),
        10,
        |_| Ok(Eligibility::Ready),
        |_| panic!("oversized suppression advanced"),
    );
    assert!(matches!(result, Err(DeleteError::Failed(_))));
    assert_eq!(
        fs::read(f.state.join("local/deleted-meeting-ids.json")).unwrap(),
        bytes
    );
    assert_eq!(suppressed_ids(&f.state).unwrap().len(), map.len());
    assert_eq!(
        fs::read(f.meetings.join("10/unknown/bytes")).unwrap(),
        b"\0\xff"
    );
}

fn delete(storage: &Storage) -> String {
    storage
        .delete(
            "10",
            "Synthetic".into(),
            100,
            |_| Ok(Eligibility::Ready),
            |_| Ok(()),
        )
        .unwrap()
}

#[test]
fn immediate_undo_and_restart_restore_preserve_every_member_byte() {
    for multipart in [false, true] {
        let f = recorded_fixture(multipart);
        let storage = f.storage();
        let inventory = storage.inventory("10").unwrap();
        let inputs: Vec<_> = inventory
            .hex_inputs
            .iter()
            .filter(|input| !input.directory)
            .map(|input| {
                let path = f.meetings.join(&input.relative);
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        let expected: Vec<_> = inventory
            .members
            .iter()
            .map(|member| {
                let path = f.state.join(&member.relative);
                let bytes = bytes_at(&path);
                (path, bytes)
            })
            .collect();
        let transaction = delete(&storage);
        let dir = f.state.join("local/deleted-meetings").join(&transaction);
        for (index, (path, bytes)) in expected.iter().enumerate() {
            assert!(!path.exists());
            assert_eq!(bytes_at(&dir.join(index.to_string())), *bytes);
        }
        for (path, bytes) in &inputs {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        let restored = if multipart { f.storage() } else { storage };
        assert_eq!(
            restored.transactions().unwrap().as_slice(),
            std::slice::from_ref(&transaction)
        );
        restored.restore(&transaction).unwrap();
        assert_eq!(restored.recover(&transaction).unwrap(), Phase::Restored);
        for (path, bytes) in expected {
            assert_eq!(bytes_at(&path), bytes);
        }
        assert!(suppressed_ids(&f.state).unwrap().is_empty());
        for (path, bytes) in inputs {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn restore_collision_leaves_the_receipt_and_bytes_unchanged() {
    let f = recorded_fixture(false);
    let storage = f.storage();
    let id = delete(&storage);
    let dir = f.state.join("local/deleted-meetings").join(&id);
    let before = bytes_at(&dir);
    f.write("state/local/evidence/10/audio.wav", b"collision");
    assert_eq!(
        storage.restore(&id).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(bytes_at(&dir), before);
    assert_eq!(
        fs::read(f.meetings.join("10/manifest.json")).unwrap(),
        b"{}"
    );
    assert_eq!(
        fs::read(f.state.join("local/evidence/10/audio.wav")).unwrap(),
        b"collision"
    );
}

#[test]
fn eligibility_refuses_without_moving_or_suppressing() {
    let f = recorded_fixture(false);
    let s = f.storage();
    for reason in [
        Eligibility::Recording,
        Eligibility::Paused,
        Eligibility::Processing,
        Eligibility::UnsavedNotes,
        Eligibility::Peer,
    ] {
        assert!(matches!(
            s.delete("10", "Synthetic".into(), 100, |_| Ok(reason), |_| Ok(())),
            Err(DeleteError::Refused(value)) if value == reason
        ));
    }
    assert!(f.meetings.join("10/manifest.json").exists());
    assert!(s.transactions().unwrap().is_empty());
    assert!(suppressed_ids(&f.state).unwrap().is_empty());
}

fn recycle_pair() -> (Fixture, String, PathBuf) {
    let f = recorded_fixture(true);
    f.write(
        "state/local/meeting-inputs/10/unknown",
        b"second member byte",
    );
    f.write("meetings/20/manifest.json", b"{}");
    f.write("meetings/20/unrelated", b"unrelated");
    let s = f.storage();
    let id = delete(&s);
    let other = s
        .delete(
            "20",
            String::new(),
            100,
            |_| Ok(Eligibility::Ready),
            |_| Ok(()),
        )
        .unwrap();
    let other_dir = f.state.join("local/deleted-meetings").join(other);
    (f, id, other_dir)
}

type MemberBytes = (PathBuf, Vec<(PathBuf, Vec<u8>)>);

fn restored_bytes(f: &Fixture) -> Vec<MemberBytes> {
    f.storage()
        .inventory("10")
        .unwrap()
        .members
        .into_iter()
        .map(|member| {
            let bytes = bytes_at(&f.state.join(&member.relative));
            (member.relative, bytes)
        })
        .collect()
}

#[test]
fn undo_after_delete_interrupt_with_hex_folder_changed() {
    for event in [MoveEvent::Moved(0), MoveEvent::Recorded(0)] {
        for replaced in [false, true] {
            let f = recorded_fixture(true);
            let hex_before = bytes_at(&f.meetings.join("10"));
            let originals = restored_bytes(&f);
            let result = f.storage().delete(
                "10",
                "Synthetic".into(),
                100,
                |_| Ok(Eligibility::Ready),
                |at| {
                    if at == event {
                        rename_hex(&f, replaced);
                        return Err(std::io::Error::other(
                            "delete interrupted after a state move",
                        ));
                    }
                    Ok(())
                },
            );
            let Err(DeleteError::Partial { transaction, .. }) = result else {
                panic!("expected an interrupted transaction");
            };
            let reopened = f.storage();
            assert_eq!(reopened.recover(&transaction).unwrap(), Phase::Deleted);
            assert!(suppressed_ids(&f.state).unwrap().contains("10"));
            reopened.restore(&transaction).unwrap();
            assert!(!suppressed_ids(&f.state).unwrap().contains("10"));
            for (relative, bytes) in originals {
                assert_eq!(bytes_at(&f.state.join(relative)), bytes);
            }
            assert_hex_unchanged(&f, &hex_before, replaced);
        }
    }
}

#[test]
fn every_restore_boundary_can_be_retried_after_reopen() {
    let (f, id, _) = recycle_pair();
    let mut events = Vec::new();
    f.storage()
        .restore_checked(&id, |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
    let originals = restored_bytes(&f);
    for (stop, expected_event, replaced) in events
        .into_iter()
        .enumerate()
        .flat_map(|(stop, event)| [false, true].map(|replace| (stop, event, replace)))
    {
        let (f, id, other) = recycle_pair();
        let unrelated = bytes_at(&other);
        let hex_before = bytes_at(&f.meetings.join("10"));
        let mut cursor = 0;
        let err = f.storage().restore_checked(&id, |event| {
            if cursor == stop {
                assert_eq!(event, expected_event);
                rename_hex(&f, replaced);
                return Err(std::io::Error::other("restore interrupted"));
            }
            cursor += 1;
            Ok(())
        });
        assert!(err.is_err());
        let hidden = suppressed_ids(&f.state).unwrap();
        assert_eq!(
            hidden.contains("10"),
            !matches!(
                expected_event,
                MoveEvent::Unhide(WriteStage::Published) | MoveEvent::Unhidden
            )
        );
        let restarted = f.storage();
        restarted.restore(&id).unwrap();
        assert_eq!(restarted.recover(&id).unwrap(), Phase::Restored);
        for (relative, bytes) in &originals {
            assert_eq!(&bytes_at(&f.state.join(relative)), bytes);
        }
        assert_hex_unchanged(&f, &hex_before, replaced);
        assert_eq!(bytes_at(&other), unrelated);
        assert_eq!(
            suppressed_ids(&f.state)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["20"]
        );
    }
}
