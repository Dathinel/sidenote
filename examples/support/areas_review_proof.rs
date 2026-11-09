use super::*;
use sidenote::core::areas::{ProjectChoice, Status, Target};
pub const CASES: &[&str] = &[
    "review-offline-init",
    "review-late-link",
    "review-old-rename",
    "review-merge",
    "review-area-only",
    "review-explicit-choice",
    "review-batch-child",
    "review-proposal",
    "review-schema",
    "review-missing",
];
fn catalog(store: &TaskStore) -> core::areas::Catalog {
    store.load_projects().unwrap().area_catalog().unwrap()
}
fn area(store: &TaskStore, name: &str) -> String {
    catalog(store).area_matches(name)[0].id.clone()
}
fn all_tasks(store: &TaskStore) -> Vec<core::TaskItem> {
    let (m, e) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
    assert!(e.is_empty());
    core::build_tasks(
        &m,
        &core::load_pipeline(&m),
        &store.load_projects().unwrap(),
        &store.load_overrides().unwrap(),
        &Default::default(),
    )
}
fn peer_before(store: &TaskStore) -> TaskStore {
    let peer = TaskStore::new(store.root.join("offline-peer"));
    fs::create_dir_all(&peer.root).unwrap();
    fs::copy(
        store.root.join("projects.json"),
        peer.root.join("projects.json"),
    )
    .unwrap();
    fs::copy(
        store.root.join("task-overrides.json"),
        peer.root.join("task-overrides.json"),
    )
    .unwrap();
    fs::create_dir_all(peer.root.join("memory")).unwrap();
    fs::copy(
        store.root.join("memory/records.json"),
        peer.root.join("memory/records.json"),
    )
    .unwrap();
    copy_tree(&store.root.join("state"), &peer.root.join("state"));
    peer
}
pub fn run(mode: &str, store: &TaskStore) {
    let root = &store.root;
    match mode {
        "review-offline-init" => {
            let peer = peer_before(store);
            let (source, child_id) = ids(store);
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, true),
                },
            );
            peer.add_project("Independent peer outcome").unwrap();
            peer.rename_project(CHILD, "Supplier outcome").unwrap();
            for path in files(&peer.root.join("state")).keys() {
                let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                let device: Value =
                    serde_json::from_slice(&fs::read(peer.root.join("device.json")).unwrap())
                        .unwrap();
                if value["device"] == device["id"] {
                    assert!(!value["changes"].as_array().unwrap().iter().any(|v| {
                        v["key"][0]
                            .as_str()
                            .is_some_and(|k| k.starts_with("area-project:"))
                    }));
                }
            }
            copy_tree(&peer.root.join("state"), &root.join("state"));
            let c = catalog(store);
            assert!(c.projects[&source].converted_to.is_some());
            assert!(c.projects[&child_id].area_id.is_some());
            let p = store.load_projects().unwrap();
            assert_eq!(c.project(&p, "Supplier outcome").unwrap().id, child_id);
        }
        "review-late-link" => {
            let id = apply(
                store,
                Action::Create {
                    name: "Late domain".into(),
                },
            );
            let area = area(store, "Late domain");
            let peer = peer_before(store);
            apply(
                &peer,
                Action::Assign {
                    key: json!(["manual", "manual/synthetic-open"]).to_string(),
                    target: Target {
                        area_id: area.clone(),
                        project_id: None,
                    },
                },
            );
            store.undo_area(&id).unwrap();
            copy_tree(&peer.root.join("state"), &root.join("state"));
            let c = catalog(store);
            assert_eq!(c.areas[&area].status, Status::Retired);
            assert!(!c.problems.is_empty());
            assert!(
                all_tasks(store)
                    .iter()
                    .any(|t| t.meeting_id == "manual/synthetic-open"
                        && t.area_id.as_ref() == Some(&area))
            );
            assert!(core::memory::MemoryStore::new(root.clone()).load().is_ok());
        }
        "review-old-rename" => {
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let (source, _) = ids(store);
            let parent = catalog(store).projects[&source].area_id.clone();
            let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/areas/before.exe");
            assert!(
                exe.is_file(),
                "The baseline app binary is required for the mixed-version proof"
            );
            let script = root.join("legacy-rename.json");
            write(
                &script,
                &json!({"events":[{"t":0.3,"type":"tap","key":"trename-0"},{"t":0.8,"type":"type","text":"Supply outcome"},{"t":2.4,"type":"key","name":"Enter"},{"t":3.2,"type":"mark","name":"end"}]}),
            );
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/ui-fixtures/hex.json"),
                root.join("hex-fixture.json"),
            )
            .unwrap();
            let output = Command::new(exe)
                .arg("--data-dir")
                .arg(root)
                .arg("--meetings-dir")
                .arg(core::meetings_dir())
                .arg("--peers-dir")
                .arg(core::peers_dir())
                .args([
                    "--scene",
                    "tasks-group-project",
                    "--theme",
                    "dark",
                    "--size",
                    "1320x900",
                    "--at",
                    "mark:end",
                ])
                .arg("--script")
                .arg(script)
                .arg("--snap")
                .arg(root.join("older-edit.png"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Old app rename failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            let renamed = c.project(&p, "Supply outcome").unwrap();
            assert_eq!(renamed.id, source);
            assert_eq!(renamed.area_id, parent);
            apply(
                store,
                Action::Convert {
                    project_id: source.clone(),
                    area_name: "Supply".into(),
                    children: Default::default(),
                },
            );
            store
                .update_projects(|p| {
                    for value in p.aliases.values_mut() {
                        if value == "Supply outcome" {
                            *value = "Supply domain".into();
                        }
                    }
                    p.aliases
                        .insert("Supply outcome".into(), "Supply domain".into());
                    p.projects.push("Supply domain".into());
                })
                .unwrap();
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            assert_eq!(c.project(&p, "Supply domain").unwrap().id, source);
            assert!(p.is_area_alias("Supply domain"));
            assert!(!core::project_names(&p, &[]).contains(&"Supply domain".to_string()));
        }
        "review-merge" => {
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let (source, child_id) = ids(store);
            let supply = area(store, "Supply");
            let key = json!(["manual", "manual/synthetic-open"]).to_string();
            apply(
                store,
                Action::Assign {
                    key,
                    target: Target {
                        area_id: supply.clone(),
                        project_id: Some(source),
                    },
                },
            );
            let preview = store.preview_merge(SOURCE, CHILD).unwrap();
            store.merge_projects(&preview).unwrap();
            let c = catalog(store);
            assert!(c.projects.contains_key(&child_id));
            assert_eq!(c.projects[&child_id].area_id, Some(supply));
            assert!(
                all_tasks(store)
                    .iter()
                    .any(|t| t.meeting_id == "manual/synthetic-open" && t.project == CHILD)
            );
            let id = store.load_projects().unwrap().merges[SOURCE].id.clone();
            store.undo_project_merge(SOURCE, &id).unwrap();
            assert!(all_tasks(store).iter().any(|t| t.project == SOURCE));
            assert!(
                store
                    .preview_merge(SOURCE, "Archive outcome")
                    .and_then(|preview| store.merge_projects(&preview))
                    .is_err()
            );
        }
        "review-area-only" => {
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let (source, _) = ids(store);
            let area = area(store, "Studio");
            for key in [
                core::areas::meeting_key("1800000000001-1"),
                json!(["memory", "synthetic-memory"]).to_string(),
                json!(["proposal", "synthetic-proposal"]).to_string(),
            ] {
                apply(
                    store,
                    Action::Assign {
                        key,
                        target: Target {
                            area_id: area.clone(),
                            project_id: None,
                        },
                    },
                );
            }
            let preview = store
                .preview_area(Action::Convert {
                    project_id: source,
                    area_name: "Supply".into(),
                    children: Default::default(),
                })
                .unwrap();
            assert!(
                !preview
                    .references
                    .iter()
                    .any(|r| r.key == core::areas::meeting_key("1800000000001-1")
                        || r.kind == "memory"
                        || r.kind == "proposal")
            );
            store.apply_area(&preview).unwrap();
            let inventory = store.area_references().unwrap();
            assert!(
                inventory
                    .references
                    .iter()
                    .filter(|r| r.kind == "memory" || r.kind == "proposal")
                    .all(|r| r.area_id.as_ref() == Some(&area) && r.project_id.is_none())
            );
        }
        "review-explicit-choice" => {
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let area = area(store, "Studio");
            apply(
                store,
                Action::Assign {
                    key: json!(["manual", "manual/synthetic-open"]).to_string(),
                    target: Target {
                        area_id: area.clone(),
                        project_id: None,
                    },
                },
            );
            store
                .update_manual("synthetic-open", |m| {
                    m.edit(|o| {
                        o.project = Some(SOURCE.into());
                        o.assignment = Some(ProjectChoice {
                            project: SOURCE.into(),
                            revision: "explicit-manual-choice".into(),
                        });
                    })
                })
                .unwrap();
            assert!(
                all_tasks(store)
                    .iter()
                    .any(|t| t.meeting_id == "manual/synthetic-open" && t.project == SOURCE)
            );
            let key = core::areas::meeting_key("1800000000001-1");
            apply(
                store,
                Action::Assign {
                    key,
                    target: Target {
                        area_id: area,
                        project_id: None,
                    },
                },
            );
            let p = store.assign_meeting("1800000000001-1", SOURCE).unwrap();
            let (m, _) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
            let pipeline = core::load_pipeline(&m);
            let target = core::areas::activity(
                &p,
                &m,
                &pipeline,
                &all_tasks(store),
                &Default::default(),
                chrono::Utc::now(),
            )
            .unwrap();
            assert!(target.meetings["1800000000001-1"].project_id.is_some());
        }
        "review-batch-child" => {
            let mut planned = rows(store, true);
            let (source, child_id) = ids(store);
            let key = json!(["manual", "manual/synthetic-open"]).to_string();
            for row in &mut planned {
                if row.project_id == source {
                    row.children.insert(key.clone(), child_id.clone());
                }
                if row.project_id == child_id {
                    row.convert = true;
                }
            }
            let before = serde_json::to_value(store.load_projects().unwrap()).unwrap();
            for _ in 0..2 {
                assert!(
                    store
                        .preview_area(Action::Migrate {
                            rows: planned.clone()
                        })
                        .unwrap_err()
                        .to_string()
                        .contains("also being")
                );
                planned.reverse();
            }
            assert_eq!(
                before,
                serde_json::to_value(store.load_projects().unwrap()).unwrap()
            );
        }
        "review-proposal" => {
            let path = root.join("memory/records.json");
            let mut memory: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            memory["migration"] = json!({"version":1,"source_path":"synthetic","fingerprint":"synthetic","accepted_at":"2026-11-09T00:00:00Z"});
            write(&path, &memory);
            write(&root.join("protected.json"), &protected(root));
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let (source, child_id) = ids(store);
            apply(
                store,
                Action::Convert {
                    project_id: source,
                    area_name: "Supply".into(),
                    children: BTreeMap::from([(
                        json!(["proposal", "synthetic-proposal"]).to_string(),
                        child_id.clone(),
                    )]),
                },
            );
            core::memory::MemoryStore::new(root.clone())
                .accept_proposal("synthetic-proposal")
                .unwrap();
            let file = core::memory::MemoryStore::new(root.clone()).load().unwrap();
            let record = file
                .records
                .iter()
                .find(|r| r.id == "memory-synthetic-proposal")
                .unwrap();
            assert_eq!(record.project.as_deref(), Some(CHILD));
            assert_eq!(
                record.assignment.as_ref().unwrap().project_id.as_ref(),
                Some(&child_id)
            );
            write(&root.join("accepted-record.json"), record);
            let memory = core::memory::MemoryStore::new(root.clone());
            memory.forget_record("memory-synthetic-proposal").unwrap();
            let file = memory.load().unwrap();
            let forgotten = file
                .records
                .iter()
                .find(|r| r.id == "memory-synthetic-proposal")
                .unwrap();
            assert!(forgotten.assignment.is_none());
            assert!(forgotten.project.is_none());
            assert!(forgotten.text.is_none());
        }
        "review-schema" => {
            apply(
                store,
                Action::Migrate {
                    rows: rows(store, false),
                },
            );
            let area = area(store, "Supply");
            store
                .update_projects(|p| {
                    p.other.get_mut(&format!("area:{area}")).unwrap()["future_field"] =
                        json!({"keep":true});
                })
                .unwrap();
            apply(
                store,
                Action::Rename {
                    area_id: area.clone(),
                    name: "Supply domain".into(),
                },
            );
            assert_eq!(
                store.load_projects().unwrap().other[&format!("area:{area}")]["future_field"],
                json!({"keep":true})
            );
            store
                .update_projects(|p| {
                    p.other.get_mut(&format!("area:{area}")).unwrap()["schema_version"] = json!(2);
                })
                .unwrap();
            assert!(catalog(store).read_only);
            assert!(
                store
                    .preview_area(Action::Rename {
                        area_id: area,
                        name: "Do not write".into()
                    })
                    .is_err()
            );
        }
        "review-missing" => {
            let key = json!(["manual", "manual/synthetic-open"]).to_string();
            store.update_projects(|p|{p.other.insert(format!("area-link:{key}"),json!({"target":{"area_id":"unseen-area","project_id":null},"baseline":SOURCE}));}).unwrap();
            let c = catalog(store);
            assert_eq!(c.areas["unseen-area"].name, "Missing area");
            assert_eq!(all_tasks(store).len(), 4);
            assert!(core::memory::MemoryStore::new(root.clone()).load().is_ok());
        }
        "review-undo-check-old" => {
            assert_eq!(
                catalog(store)
                    .areas
                    .values()
                    .filter(|a| a.status != Status::Retired)
                    .count(),
                2
            );
        }
        "review-undo-check-new" => {
            assert!(
                catalog(store)
                    .areas
                    .values()
                    .all(|a| a.status == Status::Retired)
            );
            assert!(store.area_receipts().unwrap().values().all(|r| !r.active));
            assert_eq!(all_tasks(store).len(), 4);
        }
        _ => panic!("Unknown review proof"),
    }
}
pub fn logical_projects(mut p: core::Projects) -> Value {
    p.other.retain(|key, value| {
        !key.starts_with("area-operation:")
            && !key.starts_with("area-identity:")
            && !(key.starts_with("area:") && value["status"] == "retired")
    });
    serde_json::to_value(p).unwrap()
}
