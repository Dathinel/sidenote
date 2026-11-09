//! Round-two scenarios use only copies under the explicit proof root.
use super::*;
use sidenote::core::areas::{ProjectChoice, Target};
pub const CASES: &[&str] = &[
    "r2-version-gate",
    "r2-renames",
    "r2-receipt-paths",
    "r2-scoped-undo",
    "r2-quarantine",
    "r2-extra-cap",
    "r2-import-backup",
    "r2-sharing-retry",
    "r2-old-merge",
    "r2-resolver",
    "r2-store-load",
    "r2-backup-retention",
];
fn catalog(store: &TaskStore) -> core::areas::Catalog {
    store.load_projects().unwrap().area_catalog().unwrap()
}
fn peer(store: &TaskStore, name: &str) -> TaskStore {
    let other = TaskStore::new(store.root.join(name));
    for file in [
        "projects.json",
        "task-overrides.json",
        "memory/records.json",
    ] {
        let dest = other.root.join(file);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(store.root.join(file), dest).unwrap();
    }
    copy_tree(&store.root.join("state"), &other.root.join("state"));
    other.load_projects().unwrap();
    other
}
fn references(store: &TaskStore) -> Vec<core::area_ops::Reference> {
    store.area_references().unwrap().references
}
fn migrate(store: &TaskStore) {
    apply(
        store,
        Action::Migrate {
            rows: rows(store, false),
        },
    );
}
fn own(store: &TaskStore) -> String {
    serde_json::from_slice::<Value>(&fs::read(store.root.join("device.json")).unwrap()).unwrap()["id"].as_str().unwrap().into()
}
fn retain_record(store: &TaskStore, target: &Target) {
    let mut record = core::memory::MemoryStore::new(store.root.clone())
        .load()
        .unwrap()
        .records
        .into_iter()
        .find(|r| r.id == "synthetic-memory")
        .unwrap();
    record.assignment = Some(target.clone());
    record.project = Some(CHILD.into());
    let revision = core::areas::new_assignment_revision(&store.root).unwrap();
    let (device, id) = revision.split_once('-').unwrap();
    let changes = json!([{"key":["records",record.id],"at":chrono::Utc::now().timestamp_nanos_opt().unwrap() as u64 | (1u64<<63),"value":record}]);
    let checksum = blake3::hash(
        &serde_json::to_vec(&json!([2, device, id, "memory/records.json", changes, []])).unwrap(),
    )
    .to_hex()
    .to_string();
    write(
        &store
            .root
            .join("state")
            .join(device)
            .join(blake3::hash(b"memory/records.json").to_hex().to_string())
            .join(format!("{id}.json")),
        &json!({"version":2,"device":device,"id":id,"store":"memory/records.json","changes":changes,"sources":[],"checksum":checksum}),
    );
}
pub fn run(mode: &str, store: &TaskStore) {
    let root = &store.root;
    match mode {
        "r2-version-gate" => {
            let other = peer(store, "old-device");
            other.add_project("Peer draft").unwrap();
            let id = own(&other);
            copy_tree(&other.root.join("state"), &root.join("state"));
            let marker = root.join("state").join(&id).join("device.json");
            let mut value: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
            value["app_version"] = json!("99.0.0");
            value["area_format"] = json!(0);
            value["name"] = json!("Synthetic laptop");
            write(&marker, &value);
            store.load_overrides().unwrap();
            let before = files(&root.join("state"));
            let error = store
                .preview_area(Action::Migrate {
                    rows: rows(store, false),
                })
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Update Sidenote on Synthetic laptop first"
            );
            assert!(
                store
                    .update_manual("synthetic-open", |t| {
                        t.assignment = Some(ProjectChoice {
                            project: CHILD.into(),
                            revision: "new-choice".into(),
                        });
                        t.project = Some(CHILD.into());
                    })
                    .is_err()
            );
            assert_eq!(
                before,
                files(&root.join("state")),
                "A refused edit publishes no journal"
            );
            fs::remove_file(&marker).unwrap();
            assert!(
                core::area_version::ensure(root).is_err(),
                "An unmarked recent device is blocked too"
            );
            copy_tree(&other.root.join("state"), &root.join("state"));
            let mut supported: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
            supported["app_version"] = json!("0.0.1");
            supported["area_format"] = json!(1);
            write(&marker, &supported);
            migrate(store);
            let empty_id = "e".repeat(32);
            let folder = root.join("state").join(&empty_id);
            fs::create_dir_all(&folder).unwrap();
            assert!(
                core::area_version::ensure(root).is_err(),
                "A newly seen empty device with no marker is format zero"
            );
            write(
                &folder.join("device.json"),
                &json!({"device":empty_id,"name":"Synthetic fresh device","app_version":"0.0.1","area_format":1,"seen_at":chrono::Utc::now().timestamp()}),
            );
            core::area_version::ensure(root).unwrap();
        }
        "r2-renames" => {
            migrate(store);
            let (id, _) = ids(store);
            let before = catalog(store).projects[&id].clone();
            let a = peer(store, "rename-q");
            let b = peer(store, "rename-s");
            a.rename_project(SOURCE, "Outcome Q").unwrap();
            a.add_project("Seed Q").unwrap();
            b.rename_project(SOURCE, "Outcome S").unwrap();
            b.add_project("Seed S").unwrap();
            copy_tree(&a.root.join("state"), &root.join("state"));
            copy_tree(&b.root.join("state"), &root.join("state"));
            store.add_project("Seed after both renames").unwrap();
            let c = catalog(store);
            assert_eq!(c.projects[&id].area_id, before.area_id);
            assert_eq!(c.projects[&id].converted_to, before.converted_to);
            assert!(
                c.name_conflicts[&id].contains("Outcome Q")
                    && c.name_conflicts[&id].contains("Outcome S")
            );
            apply(
                store,
                Action::ResolveName {
                    project_id: id.clone(),
                    name: "Outcome Q".into(),
                },
            );
            let c = catalog(store);
            assert!(!c.name_conflicts.contains_key(&id));
            assert_eq!(c.projects[&id].name, "Outcome Q");
            assert_eq!(c.projects[&id].area_id, before.area_id);
        }
        "r2-receipt-paths" => {
            let id = apply(
                store,
                Action::Create {
                    name: "Safe domain".into(),
                },
            );
            let receipt = store.area_receipts().unwrap()[&id].clone();
            let own = own(store);
            let original = serde_json::to_value(&receipt).unwrap();
            for name in [
                r"..\..\x",
                r"\\host\share\x",
                r"C:relative",
                r"C:\outside\backup.json",
            ] {
                let mut bad = receipt.clone();
                bad.backup = PathBuf::from(name);
                assert!(core::area_ops::receipt_backup(root, &id, &bad, &own).is_err());
                store
                    .update_projects(|p| {
                        p.other.insert(
                            format!("area-operation:{id}"),
                            serde_json::to_value(&bad).unwrap(),
                        );
                    })
                    .unwrap();
                assert!(store.undo_area(&id).is_err());
                assert_eq!(catalog(store).unreadable_receipts, 1);
            }
            let mut bad = receipt.clone();
            bad.device = "a".repeat(32);
            assert!(core::area_ops::receipt_backup(root, &id, &bad, &own).is_err());
            assert!(!core::area_ops::valid_receipt_id(
                &format!("garbage-{own}-{}", "a".repeat(32)),
                &own
            ));
            store
                .update_projects(|p| {
                    p.other.insert(format!("area-operation:{id}"), original);
                })
                .unwrap();
            let dir = root.join("local/area-operations").join(&id);
            let saved = root.join("synthetic-backup-copy");
            fs::rename(&dir, &saved).unwrap();
            let status=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:SIDENOTE_PROOF_LINK -Target $env:SIDENOTE_PROOF_TARGET | Out-Null"]).env("SIDENOTE_PROOF_LINK",&dir).env("SIDENOTE_PROOF_TARGET",&saved).output().unwrap();
            assert!(
                status.status.success(),
                "Synthetic junction creation failed: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            assert!(
                store.undo_area(&id).is_err(),
                "A backup reached through a junction must be rejected"
            );
            fs::remove_dir(&dir).unwrap();
            fs::rename(saved, &dir).unwrap();
            assert!(core::area_ops::receipt_backup(root, &id, &receipt, &own).is_ok());
            let mut enlarged = receipt.clone();
            enlarged.after.insert(
                "area:unrelated".into(),
                Some(json!({"name":"not this operation"})),
            );
            store
                .update_projects(|p| {
                    p.other.insert(
                        format!("area-operation:{id}"),
                        serde_json::to_value(&enlarged).unwrap(),
                    );
                })
                .unwrap();
            assert!(
                store
                    .undo_area(&id)
                    .unwrap_err()
                    .to_string()
                    .contains("Undo record changed")
            );
            store
                .update_projects(|p| {
                    p.other.insert(
                        format!("area-operation:{id}"),
                        serde_json::to_value(&receipt).unwrap(),
                    );
                })
                .unwrap();
            let forger = peer(store, "receipt-peer");
            let mut fake = receipt.clone();
            fake.checksum = "peer-forgery".into();
            forger
                .update_projects(|p| {
                    p.other.insert(
                        format!("area-operation:{id}"),
                        serde_json::to_value(&fake).unwrap(),
                    );
                })
                .unwrap();
            copy_tree(&forger.root.join("state"), &root.join("state"));
            assert!(
                store.local_area_backup(&id, &fake).is_err(),
                "Peer data cannot claim local receipt ownership"
            );
            store
                .update_projects(|p| {
                    p.other.insert(
                        format!("area-operation:{id}"),
                        serde_json::to_value(&receipt).unwrap(),
                    );
                })
                .unwrap();
            store.undo_area(&id).unwrap();
        }
        "r2-scoped-undo" => {
            migrate(store);
            let (source, child) = ids(store);
            let key = json!(["manual", "manual/synthetic-open"]).to_string();
            let id = apply(
                store,
                Action::Convert {
                    project_id: source,
                    area_name: "Supply".into(),
                    children: Default::default(),
                },
            );
            let other = peer(store, "later-device");
            let area = catalog(&other).area_matches("Supply")[0].id.clone();
            apply(
                &other,
                Action::Assign {
                    key: key.clone(),
                    target: Target {
                        area_id: area,
                        project_id: Some(child.clone()),
                    },
                },
            );
            copy_tree(&other.root.join("state"), &root.join("state"));
            let peer_link =
                store.load_projects().unwrap().other[&format!("area-link:{key}")].clone();
            let all_before = references(store);
            store.undo_area(&id).unwrap();
            let p = store.load_projects().unwrap();
            assert_eq!(p.other[&format!("area-link:{key}")], peer_link);
            assert_eq!(
                references(store)
                    .iter()
                    .find(|r| r.key == key)
                    .unwrap()
                    .project_id,
                Some(child)
            );
            assert!(
                store.area_receipts().unwrap()[&id]
                    .skipped
                    .iter()
                    .any(|name| name.contains("Call the paper mill"))
            );
            assert_ne!(
                references(store),
                all_before,
                "The operation's other links were restored"
            );
        }
        "r2-quarantine" => {
            for i in 0..11 {
                store
                    .add_project(&format!("Example project {i:02}"))
                    .unwrap();
            }
            migrate(store);
            assert_eq!(catalog(store).projects.len(), 14);
            let id = ids(store).0;
            store
                .update_projects(|p| {
                    p.other.insert(
                        format!("area-project:{id}"),
                        json!({"id":id,"name":7,"status":"active"}),
                    );
                    p.other
                        .insert("area-operation:bad".into(), json!({"broken":true}));
                })
                .unwrap();
            let c = catalog(store);
            assert!(!c.read_only);
            assert_eq!(c.quarantined, 1);
            assert_eq!(c.unreadable_receipts, 1);
            assert_eq!(c.projects.len(), 14);
            apply(
                store,
                Action::Create {
                    name: "Still editable".into(),
                },
            );
            assert!(core::memory::MemoryStore::new(root.clone()).load().is_ok());
        }
        "r2-extra-cap" => {
            let long_name = format!("Synthetic outcome {}", "paper ".repeat(20))
                .trim()
                .to_string();
            store.add_project(&long_name).unwrap();
            migrate(store);
            let p = store.load_projects().unwrap();
            assert!(
                p.area_catalog()
                    .unwrap()
                    .project(&p, &long_name)
                    .unwrap()
                    .area_id
                    .is_some(),
                "The area name cap must not quarantine existing long project names"
            );
            let id = catalog(store).area_matches("Supply")[0].id.clone();
            store
                .update_projects(|p| {
                    p.other.get_mut(&format!("area:{id}")).unwrap()["unknown"] =
                        json!({"retained":"yes"});
                })
                .unwrap();
            apply(
                store,
                Action::Rename {
                    area_id: id.clone(),
                    name: "Supplies".into(),
                },
            );
            assert_eq!(
                store.load_projects().unwrap().other[&format!("area:{id}")]["unknown"]["retained"],
                "yes"
            );
            store
                .update_projects(|p| {
                    p.other.get_mut(&format!("area:{id}")).unwrap()["unknown"] =
                        json!("x".repeat(65537));
                })
                .unwrap();
            assert_eq!(catalog(store).quarantined, 1);
            assert!(!catalog(store).read_only);
            apply(
                store,
                Action::Create {
                    name: "Other domain".into(),
                },
            );
            assert_eq!(
                store.load_projects().unwrap().other[&format!("area:{id}")]["unknown"]
                    .as_str()
                    .unwrap()
                    .len(),
                65537,
                "Oversized peer data is retained without copying it into edits"
            );
        }
        "r2-import-backup" => {
            let other = TaskStore::new(root.join("legacy-copy"));
            fs::create_dir_all(&other.root).unwrap();
            write(
                &other.root.join("projects.json"),
                &json!({"projects":["New import"]}),
            );
            let gate = other.root.join("local/area-legacy-backup.pause");
            write(&gate, &true);
            let task = other.clone();
            let handle = std::thread::spawn(move || task.load_projects());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !gate.with_extension("reached").exists() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let backups = files(&other.root.join("backups"));
            assert_eq!(backups.len(), 1);
            let path = backups.keys().next().unwrap();
            fs::write(path, b"corrupt backup").unwrap();
            fs::remove_file(gate).unwrap();
            assert!(
                handle
                    .join()
                    .unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("verification failed")
            );
            let journals = files(&other.root.join("state"));
            assert!(
                journals.keys().all(|p| p.ends_with("device.json")),
                "No import journal published after failed read-back"
            );
        }
        "r2-sharing-retry" => {
            use std::os::windows::fs::OpenOptionsExt;
            let own = own(store);
            let lock = root
                .join("local/locks")
                .join(format!("{}.lock", blake3::hash(b"projects.json").to_hex()));
            let held = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&lock)
                .unwrap();
            let start = std::time::Instant::now();
            let release = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(110));
                drop(held);
            });
            store.add_project("Sharing retry outcome").unwrap();
            release.join().unwrap();
            assert!(start.elapsed() >= std::time::Duration::from_millis(100));
            assert_eq!(own, super::round2::own(store));
            let from = root.join("replace-source.json");
            let to = root.join("replace-target.json");
            fs::write(&from, b"new").unwrap();
            fs::write(&to, b"old").unwrap();
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&to)
                .unwrap();
            let release = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(110));
                drop(held);
            });
            sidenote::platform::replace_file(&from, &to).unwrap();
            release.join().unwrap();
            assert_eq!(fs::read(&to).unwrap(), b"new");
            fs::write(&from, b"next").unwrap();
            let held = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&to)
                .unwrap();
            let start = std::time::Instant::now();
            assert!(sidenote::platform::replace_file(&from, &to).is_err());
            assert!(start.elapsed() >= std::time::Duration::from_millis(180));
            drop(held);
            assert_eq!(fs::read(&to).unwrap(), b"new");
            assert_eq!(fs::read(&from).unwrap(), b"next");
        }
        "r2-old-merge" => {
            store
                .update_projects(|p| p.other.retain(|k, _| !k.starts_with("assignment:")))
                .unwrap();
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/ui-fixtures/hex.json"),
                root.join("hex-fixture.json"),
            )
            .unwrap();
            let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/areas/before.exe");
            assert!(exe.is_file());
            let output = Command::new(exe)
                .arg("--data-dir")
                .arg(root)
                .arg("--meetings-dir")
                .arg(core::meetings_dir())
                .arg("--peers-dir")
                .arg(core::peers_dir())
                .args([
                    "--scene",
                    "merge-done",
                    "--theme",
                    "dark",
                    "--size",
                    "1320x1000",
                    "--at",
                    "1.5",
                ])
                .arg("--snap")
                .arg(root.join("old-merge.png"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "The baseline app must produce the receipt: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let p = store.load_projects().unwrap();
            let (from, merge) = p.merges.iter().find(|(_, m)| m.active).unwrap();
            assert!(merge.baseline_version.is_none());
            assert!(
                merge
                    .assignment_baseline
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|(k, v)| k.starts_with("[\"task\",")
                        && v.as_array().is_some_and(|a| a.len() == 3))
            );
            store.undo_project_merge(from, &merge.id).unwrap();
            assert!(
                !store
                    .load_projects()
                    .unwrap()
                    .merges
                    .get(from)
                    .is_some_and(|m| m.active)
            );
            store
                .update_projects(|current| {
                    current.aliases.insert(from.clone(), merge.into.clone());
                    current.merges.insert(from.clone(), merge.clone());
                })
                .unwrap();
            let key = merge
                .assignment_baseline
                .as_ref()
                .unwrap()
                .keys()
                .find(|k| k.starts_with("[\"task\","))
                .unwrap();
            let parts: Vec<Value> = serde_json::from_str(key).unwrap();
            let revision = core::areas::new_assignment_revision(root).unwrap();
            store
                .update_override(
                    parts[1].as_str().unwrap(),
                    parts[2].as_u64().unwrap() as usize,
                    parts[3].as_str().unwrap(),
                    |edit| {
                        edit.project = Some(CHILD.into());
                        edit.assignment = Some(ProjectChoice {
                            project: CHILD.into(),
                            revision,
                        });
                    },
                )
                .unwrap();
            assert!(
                store.undo_project_merge(from, &merge.id).is_err(),
                "A real later choice must still refuse the old Undo"
            );
        }
        "r2-resolver" => {
            migrate(store);
            let (source, child) = ids(store);
            let key = json!(["memory", "synthetic-memory"]).to_string();
            apply(
                store,
                Action::Convert {
                    project_id: source,
                    area_name: "Supply".into(),
                    children: BTreeMap::from([(key.clone(), child.clone())]),
                },
            );
            let p = store.load_projects().unwrap();
            let area = catalog(store).area_matches("Supply")[0].id.clone();
            let record = core::memory::MemoryStore::new(root.clone())
                .load()
                .unwrap()
                .records
                .into_iter()
                .find(|r| r.id == "synthetic-memory")
                .unwrap();
            assert_eq!(record.project.as_deref(), Some(CHILD));
            // Retained assignments go through the same redirects and later-link precedence.
            let retained = Target {
                area_id: area.clone(),
                project_id: Some(child.clone()),
            };
            retain_record(store, &retained);
            let base = core::areas::assignment_baseline(json!(CHILD), Some(&retained));
            let c = p.area_catalog().unwrap();
            assert_eq!(
                c.resolve_target(
                    &p,
                    &key,
                    &base,
                    Some(core::areas::Assignment::Target(&retained)),
                    CHILD
                )
                .unwrap()
                .project_id,
                Some(child)
            );
            let target_id = c.project(&p, "Archive outcome").unwrap().id.clone();
            apply(
                store,
                Action::Move {
                    project_id: target_id.clone(),
                    area_id: area.clone(),
                },
            );
            let preview = store.preview_merge(SOURCE, "Archive outcome").unwrap();
            store.merge_projects(&preview).unwrap();
            assert_eq!(
                core::memory::MemoryStore::new(root.clone())
                    .load()
                    .unwrap()
                    .records
                    .iter()
                    .find(|r| r.id == "synthetic-memory")
                    .unwrap()
                    .project
                    .as_deref(),
                Some(CHILD)
            );
            apply(
                store,
                Action::Assign {
                    key: key.clone(),
                    target: Target {
                        area_id: area,
                        project_id: Some(target_id.clone()),
                    },
                },
            );
            assert_eq!(
                core::memory::MemoryStore::new(root.clone())
                    .load()
                    .unwrap()
                    .records
                    .iter()
                    .find(|r| r.id == "synthetic-memory")
                    .unwrap()
                    .project
                    .as_deref(),
                Some("Archive outcome")
            );
            assert_eq!(
                references(store)
                    .iter()
                    .find(|r| r.key == key)
                    .unwrap()
                    .project_id,
                Some(target_id.clone())
            );
            let preview = store.preview_merge(CHILD, "Archive outcome").unwrap();
            store.merge_projects(&preview).unwrap();
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            assert_eq!(
                c.resolve_target(
                    &p,
                    "no-later-link",
                    &Value::Null,
                    Some(core::areas::Assignment::Target(&retained)),
                    CHILD
                )
                .unwrap()
                .project_id,
                Some(target_id)
            );
        }
        "r2-store-load" => {
            let peer_id = "b".repeat(32);
            let dir = root
                .join("state")
                .join(&peer_id)
                .join(blake3::hash(b"projects.json").to_hex().to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("bad.json"), b"{").unwrap();
            assert_eq!(store.load_projects().unwrap().projects.len(), 3);
            assert!(store.unreadable_items().unwrap() > 0);
            write(
                &root.join("state").join(&peer_id).join("device.json"),
                &json!({"device":peer_id,"name":"Synthetic peer","app_version":env!("CARGO_PKG_VERSION"),"area_format":1,"seen_at":chrono::Utc::now().timestamp()}),
            );
            apply(
                store,
                Action::Create {
                    name: "Readable remainder".into(),
                },
            );
            assert_eq!(fs::read(dir.join("bad.json")).unwrap(), b"{");
            fs::write(dir.join("huge.json"), vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
            assert!(store.load_projects().is_ok());
            assert!(store.unreadable_items().unwrap() >= 2);
        }
        "r2-backup-retention" => {
            for i in 0..12 {
                apply(
                    store,
                    Action::Create {
                        name: format!("Synthetic domain {i:02}"),
                    },
                );
            }
            let base = root.join("local/area-operations");
            assert_eq!(fs::read_dir(base).unwrap().count(), 10);
            let receipts = store.area_receipts().unwrap();
            let own = own(store);
            assert_eq!(
                receipts
                    .iter()
                    .filter(|(id, r)| core::area_ops::receipt_backup(root, id, r, &own).is_ok())
                    .count(),
                10
            );
            assert!(receipts.values().all(|r| r.backup.is_relative()));
            assert!(
                store
                    .preview_area(Action::Create {
                        name: "x".repeat(81)
                    })
                    .is_err()
            );
        }
        _ => panic!("Unknown round-two scenario"),
    }
}
