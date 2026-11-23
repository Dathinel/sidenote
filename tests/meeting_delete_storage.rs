use sidenote::core::meeting_delete::{DeleteError, Eligibility, Phase, Storage, suppressed_ids};
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
