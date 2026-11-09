//! Synthetic, separate-process rehearsal. This is not a test runner.
use serde_json::{Value, json};
use sidenote::core::{
    self, TaskStore,
    area_ops::{Action, MigrationRow},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const SOURCE: &str = "Supply planning";
const CHILD: &str = "Supplier selection";
#[path = "support/areas_review_proof.rs"]
mod review;
fn write(path: &Path, value: &impl serde::Serialize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn files(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if !path.exists() {
        return out;
    }
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            out.extend(files(&entry.path()));
        } else {
            out.insert(
                entry.path().to_string_lossy().into_owned(),
                blake3::hash(&fs::read(entry.path()).unwrap())
                    .to_hex()
                    .to_string(),
            );
        }
    }
    out
}
fn protected(root: &Path) -> BTreeMap<String, String> {
    let mut out = files(&core::meetings_dir());
    out.extend(files(&core::peers_dir()));
    for name in [
        "projects.json",
        "task-overrides.json",
        "memory/records.json",
        "config.json",
    ] {
        let path = root.join(name);
        out.insert(
            path.to_string_lossy().into_owned(),
            blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string(),
        );
    }
    out
}
fn seed(root: &Path, store: &TaskStore) {
    assert!(!root.exists(), "Use a fresh isolated proof folder");
    fs::create_dir_all(core::peers_dir()).unwrap();
    for (i, todo) in [(0, true), (1, false), (2, true)] {
        let base = if i == 2 {
            core::peers_dir().join("synthetic-peer")
        } else {
            core::meetings_dir()
        };
        let folder = base.join(format!("180000000000{i}-1"));
        write(
            &folder.join("manifest.json"),
            &json!({"created_at_ms":1800000000000_i64+i}),
        );
        write(
            &folder.join("english-summary.json"),
            &json!({"title":format!("Synthetic supplier meeting {i}"), "todo": if todo { json!([{"task":"Compare paper samples"}]) } else { json!([]) }}),
        );
        write(
            &folder.join("tasks.json"),
            &json!({"meeting_project":SOURCE,"tasks":[]}),
        );
        fs::write(
            folder.join("notes.md"),
            b"Keep these synthetic note bytes.\r\n",
        )
        .unwrap();
    }
    write(
        &root.join("projects.json"),
        &json!({"projects":[SOURCE,CHILD,"Archive outcome"],"aliases":{"Supply desk":SOURCE}}),
    );
    write(
        &root.join("task-overrides.json"),
        &json!({"manual":[
        {"id":"synthetic-open","task":"Call the paper mill","project":SOURCE,"done":false,"added":"2026-11-09"},
        {"id":"synthetic-done","task":"File the samples","project":SOURCE,"done":true,"added":"2026-11-09"}]}),
    );
    write(
        &root.join("config.json"),
        &json!({"version":1,"features":{"projects":true,"tasks":true,"memory":true}}),
    );
    write(
        &root.join("memory/records.json"),
        &json!({"schema_version":1,"records":[{
        "id":"synthetic-memory","type":"project","text":"Use recycled paper.","project":SOURCE,"sources":[],"first_seen":null,"last_confirmed":null,"status":"active","basis":"owner_confirmed","supersedes":null,"updated_at":"2026-11-09T00:00:00Z","source_fingerprint":"synthetic"}],
        "proposals":[{"id":"synthetic-proposal","op":"add","target_id":null,"text":"Bring sample cards.","project":SOURCE,"sources":[],"rationale":"Synthetic","status":"pending","created_at":"2026-11-09T00:00:00Z","decided_at":null}],"events":[],"migration":null}),
    );
    store.assign_meeting("1800000000001-1", SOURCE).unwrap();
    for name in [SOURCE, CHILD, "Archive outcome"] {
        store.add_project(name).unwrap();
    }
    write(&root.join("protected.json"), &protected(root));
    write(&root.join("before.json"), &store.load_projects().unwrap());
}
fn apply(store: &TaskStore, action: Action) -> String {
    let preview = store.preview_area(action).unwrap();
    write(&store.root.join("last-preview.json"), &preview);
    let p = store.apply_area(&preview).unwrap();
    let receipts = store.area_receipts().unwrap();
    let id = receipts
        .keys()
        .find(|id| {
            p.other.get(&format!("area-operation:{id}")).is_some()
                && !store.root.join(format!("seen-{id}.json")).exists()
        })
        .unwrap()
        .clone();
    write(&store.root.join(format!("seen-{id}.json")), &true);
    write(&store.root.join("undo-id.json"), &id);
    id
}
fn ids(store: &TaskStore) -> (String, String) {
    let p = store.load_projects().unwrap();
    let c = p.area_catalog().unwrap();
    (
        c.project(&p, SOURCE).unwrap().id.clone(),
        c.project(&p, CHILD).unwrap().id.clone(),
    )
}
fn rows(store: &TaskStore, convert: bool) -> Vec<MigrationRow> {
    let p = store.load_projects().unwrap();
    p.area_catalog()
        .unwrap()
        .projects
        .values()
        .filter(|p| p.converted_to.is_none())
        .map(|p| MigrationRow {
            project_id: p.id.clone(),
            area_name: if p.name == "Archive outcome" {
                "Studio"
            } else {
                "Supply"
            }
            .into(),
            convert: convert && p.name == SOURCE,
            children: Default::default(),
            assignments: Default::default(),
        })
        .collect()
}
fn child(root: &Path, command: &str, crash: Option<&str>) -> i32 {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg(command)
        .arg("--data-dir")
        .arg(root)
        .arg("--meetings-dir")
        .arg(root.join("meetings"))
        .arg("--peers-dir")
        .arg(root.join("peers"));
    if let Some(stage) = crash {
        cmd.env("SIDENOTE_AREA_CRASH", stage);
    }
    cmd.status().unwrap().code().unwrap()
}
fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for e in fs::read_dir(source).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &target.join(e.file_name()));
        } else {
            fs::copy(e.path(), target.join(e.file_name())).unwrap();
        }
    }
}
fn verify_bytes(root: &Path) {
    let before: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("protected.json")).unwrap()).unwrap();
    assert_eq!(before, protected(root), "Source bytes changed");
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert!(args.iter().any(|a| a == "--data-dir"));
    core::configure_paths(&args).unwrap();
    let root = core::state_dir();
    let allowed = Path::new("E:/Personal-Projects/reviews/areas-proof");
    assert!(
        root.starts_with(allowed)
            && core::meetings_dir().starts_with(allowed)
            && core::peers_dir().starts_with(allowed)
    );
    let mode = args[0].to_string_lossy();
    let store = TaskStore::new(root.clone());
    match mode.as_ref() {
        "run" => {
            fs::create_dir_all(&root).unwrap();
            let mut evidence = Vec::new();
            for scenario in [
                "move",
                "convert",
                "batch",
                "stale",
                "later-move",
                "peer",
                "checksum",
                "activity",
                "boundary",
                "undo-boundary",
                "legacy",
                "child-move",
                "raw-move",
            ]
            .into_iter()
            .chain(review::CASES.iter().copied())
            {
                let data = root.join(scenario);
                assert_eq!(child(&data, "prepare", None), 0);
                assert_eq!(child(&data, scenario, None), 0, "{scenario}");
                if matches!(scenario, "move" | "convert" | "batch" | "peer") {
                    assert_eq!(child(&data, "undo", None), 0);
                }
                assert_eq!(child(&data, "bytes", None), 0);
                evidence.push(json!({"scenario":scenario,"passed":true,"data":data}));
            }
            for stage in ["backup", "staged", "published"] {
                let data = root.join(format!("crash-{stage}"));
                assert_eq!(child(&data, "prepare", None), 0);
                assert_eq!(child(&data, "batch", Some(stage)), 86);
                if stage == "published" {
                    assert_eq!(child(&data, "new", None), 0);
                } else {
                    assert_eq!(child(&data, "old", None), 0);
                    assert_eq!(child(&data, "batch", None), 0);
                }
                assert_eq!(child(&data, "undo", None), 0);
                assert_eq!(child(&data, "bytes", None), 0);
                evidence
                    .push(json!({"scenario":format!("crash-{stage}"),"passed":true,"data":data}));
            }
            for stage in ["undo-staged", "undo-publication", "undo-published"] {
                let data = root.join(format!("crash-{stage}"));
                assert_eq!(child(&data, "prepare", None), 0);
                assert_eq!(child(&data, "batch", None), 0);
                assert_eq!(child(&data, "undo", Some(stage)), 86);
                if stage != "undo-published" {
                    assert_eq!(child(&data, "review-undo-check-old", None), 0);
                    assert_eq!(child(&data, "undo", None), 0);
                }
                assert_eq!(child(&data, "review-undo-check-new", None), 0);
                assert_eq!(child(&data, "bytes", None), 0);
                evidence
                    .push(json!({"scenario":format!("crash-{stage}"),"passed":true,"data":data}));
            }
            write(&root.join("results.json"), &evidence);
            println!(
                "{} isolated scenarios passed; evidence: {}",
                evidence.len(),
                root.join("results.json").display()
            );
        }
        "prepare" => seed(&root, &store),
        "move" | "peer" => {
            apply(
                &store,
                Action::Create {
                    name: "Supply".into(),
                },
            );
            let (id, _) = ids(&store);
            let area = store
                .load_projects()
                .unwrap()
                .area_catalog()
                .unwrap()
                .area_matches("Supply")[0]
                .id
                .clone();
            write(&root.join("before.json"), &store.load_projects().unwrap());
            let preview = store
                .preview_area(Action::Move {
                    project_id: id.clone(),
                    area_id: area.clone(),
                })
                .unwrap();
            assert_eq!(
                preview
                    .references
                    .iter()
                    .filter(|r| r.kind == "meeting")
                    .count(),
                3
            );
            assert_eq!(
                preview
                    .references
                    .iter()
                    .filter(|r| r.kind == "task")
                    .count(),
                4
            );
            assert_eq!(
                preview
                    .references
                    .iter()
                    .filter(|r| r.kind == "memory" || r.kind == "proposal")
                    .count(),
                2
            );
            apply(&store, preview.action);
            let p = store.load_projects().unwrap();
            assert_eq!(
                p.area_catalog().unwrap().projects[&id].area_id.as_ref(),
                Some(&area)
            );
            if mode == "peer" {
                let peer = TaskStore::new(root.join("peer-writer"));
                peer.update_override("1800000000002-1", 0, "Compare paper samples", |o| {
                    o.owner = Some("Taylor".into())
                })
                .unwrap();
                copy_tree(&peer.root.join("state"), &root.join("state"));
                let journals = files(&peer.root.join("state"));
                write(&root.join("peer-journals.json"), &journals);
            }
        }
        "convert" => {
            apply(
                &store,
                Action::Migrate {
                    rows: rows(&store, false),
                },
            );
            write(&root.join("before.json"), &store.load_projects().unwrap());
            let (id, child) = ids(&store);
            let base = store
                .preview_area(Action::Convert {
                    project_id: id.clone(),
                    area_name: "Supply".into(),
                    children: Default::default(),
                })
                .unwrap();
            let key = base
                .references
                .iter()
                .find(|r| r.key.starts_with("[\"task\","))
                .unwrap()
                .key
                .clone();
            apply(
                &store,
                Action::Convert {
                    project_id: id.clone(),
                    area_name: "Supply".into(),
                    children: BTreeMap::from([(key, child.clone())]),
                },
            );
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            assert!(c.projects[&id].converted_to.is_some());
            assert!(matches!(
                c.legacy_aliases.get("Supply desk"),
                Some(core::areas::LegacyAlias::Area { .. })
            ));
            let (m, e) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
            assert!(e.is_empty());
            let tasks = core::build_tasks(
                &m,
                &core::load_pipeline(&m),
                &p,
                &store.load_overrides().unwrap(),
                &Default::default(),
            );
            assert_eq!(tasks.len(), 4);
            assert_eq!(tasks.iter().filter(|t| t.project == CHILD).count(), 1);
            assert_eq!(
                tasks
                    .iter()
                    .filter(|t| t.project == core::UNSORTED && t.area_id.is_some())
                    .count(),
                3
            );
            assert_eq!(c.projects[&child].area_id, c.projects[&id].converted_to);
            let activity = core::areas::activity(
                &p,
                &m,
                &core::load_pipeline(&m),
                &tasks,
                &Default::default(),
                chrono::Utc::now(),
            )
            .unwrap();
            assert_eq!(activity.meetings.len(), 3);
            assert!(
                activity
                    .meetings
                    .values()
                    .all(|target| target.project_id.is_none())
            );
        }
        "batch" => {
            let rows = rows(&store, true);
            let before = files(&root.join("local/area-operations")).len();
            apply(&store, Action::Migrate { rows });
            assert_eq!(files(&root.join("local/area-operations")).len(), before + 1);
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            assert_eq!(c.areas.len(), 2);
            assert!(c.projects.values().all(|p| p.area_id.is_some()));
        }
        "old" => {
            let before: Value =
                serde_json::from_slice(&fs::read(root.join("before.json")).unwrap()).unwrap();
            assert_eq!(
                before,
                serde_json::to_value(store.load_projects().unwrap()).unwrap()
            );
            assert!(store.area_receipts().unwrap().is_empty());
        }
        "new" => {
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            assert_eq!(c.areas.len(), 2);
            assert_eq!(c.projects.len(), 3);
            assert!(c.projects.values().all(|p| p.area_id.is_some()));
            let receipts = store.area_receipts().unwrap();
            assert_eq!(receipts.len(), 1);
            write(&root.join("undo-id.json"), receipts.keys().next().unwrap());
        }
        "undo" => {
            let id: String =
                serde_json::from_slice(&fs::read(root.join("undo-id.json")).unwrap()).unwrap();
            store.undo_area(&id).unwrap();
            let p = store.load_projects().unwrap();
            let before: core::Projects =
                serde_json::from_slice(&fs::read(root.join("before.json")).unwrap()).unwrap();
            assert_eq!(
                review::logical_projects(p),
                review::logical_projects(before)
            );
            if root.join("peer-journals.json").exists() {
                assert_eq!(
                    store
                        .load_overrides()
                        .unwrap()
                        .get("1800000000002-1", 0)
                        .unwrap()
                        .owner
                        .as_deref(),
                    Some("Taylor")
                );
                let saved: BTreeMap<String, String> =
                    serde_json::from_slice(&fs::read(root.join("peer-journals.json")).unwrap())
                        .unwrap();
                for (path, hash) in saved {
                    let path = PathBuf::from(path);
                    let rel = path.strip_prefix(root.join("peer-writer")).unwrap();
                    assert_eq!(
                        blake3::hash(&fs::read(root.join(rel)).unwrap())
                            .to_hex()
                            .to_string(),
                        hash
                    );
                }
            }
        }
        "stale" => {
            let preview = store
                .preview_area(Action::Migrate {
                    rows: rows(&store, false),
                })
                .unwrap();
            store
                .update_override("1800000000000-1", 0, "Compare paper samples", |o| {
                    o.owner = Some("Morgan".into())
                })
                .unwrap();
            assert!(
                store
                    .apply_area(&preview)
                    .unwrap_err()
                    .to_string()
                    .contains("changed")
            );
            assert!(
                store
                    .load_projects()
                    .unwrap()
                    .area_catalog()
                    .unwrap()
                    .areas
                    .is_empty()
            );
        }
        "later-move" => {
            let id = apply(
                &store,
                Action::Migrate {
                    rows: rows(&store, false),
                },
            );
            let p = store.load_projects().unwrap();
            let c = p.area_catalog().unwrap();
            let (source, _) = ids(&store);
            let target = c.area_matches("Studio")[0].id.clone();
            apply(
                &store,
                Action::Move {
                    project_id: source,
                    area_id: target,
                },
            );
            let before = serde_json::to_value(store.load_projects().unwrap()).unwrap();
            assert!(
                store
                    .undo_area(&id)
                    .unwrap_err()
                    .to_string()
                    .contains("changed later")
            );
            assert_eq!(
                before,
                serde_json::to_value(store.load_projects().unwrap()).unwrap()
            );
        }
        "child-move" | "raw-move" => {
            apply(
                &store,
                Action::Migrate {
                    rows: rows(&store, false),
                },
            );
            let (source, child) = ids(&store);
            let base = store
                .preview_area(Action::Convert {
                    project_id: source.clone(),
                    area_name: "Supply".into(),
                    children: Default::default(),
                })
                .unwrap();
            let task = base
                .references
                .iter()
                .find(|r| r.kind == "task")
                .unwrap()
                .key
                .clone();
            let memory = base
                .references
                .iter()
                .find(|r| r.kind == "memory")
                .unwrap()
                .key
                .clone();
            let id = apply(
                &store,
                Action::Convert {
                    project_id: source,
                    area_name: "Supply".into(),
                    children: BTreeMap::from([(task, child.clone()), (memory, child.clone())]),
                },
            );
            assert_eq!(
                core::memory::MemoryStore::new(root.clone())
                    .list_records()
                    .unwrap()[0]
                    .project
                    .as_deref(),
                Some(CHILD)
            );
            if mode == "child-move" {
                let p = store.load_projects().unwrap();
                let area = p.area_catalog().unwrap().area_matches("Studio")[0]
                    .id
                    .clone();
                apply(
                    &store,
                    Action::Move {
                        project_id: child,
                        area_id: area.clone(),
                    },
                );
                let p = store.load_projects().unwrap();
                let (m, e) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
                assert!(e.is_empty());
                let tasks = core::build_tasks(
                    &m,
                    &core::load_pipeline(&m),
                    &p,
                    &store.load_overrides().unwrap(),
                    &Default::default(),
                );
                assert!(
                    tasks
                        .iter()
                        .any(|t| t.project == CHILD && t.area_id.as_ref() == Some(&area))
                );
            } else {
                store
                    .update_manual("synthetic-done", |t| t.project = Some(CHILD.into()))
                    .unwrap();
            }
            assert!(store.undo_area(&id).is_err());
        }
        "checksum" => {
            let id = apply(
                &store,
                Action::Migrate {
                    rows: rows(&store, false),
                },
            );
            let receipt = &store.area_receipts().unwrap()[&id];
            let mut value: Value =
                serde_json::from_slice(&fs::read(&receipt.backup).unwrap()).unwrap();
            value["preview"]["fingerprint"] = json!("tampered");
            write(&receipt.backup, &value);
            assert!(
                store
                    .undo_area(&id)
                    .unwrap_err()
                    .to_string()
                    .contains("backup changed")
            );
        }
        "activity" => {
            apply(
                &store,
                Action::Migrate {
                    rows: rows(&store, false),
                },
            );
            let (meetings, e) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
            assert!(e.is_empty());
            let p = store.load_projects().unwrap();
            let pipeline = core::load_pipeline(&meetings);
            let tasks = core::build_tasks(
                &meetings,
                &pipeline,
                &p,
                &store.load_overrides().unwrap(),
                &Default::default(),
            );
            let active = core::areas::activity(
                &p,
                &[],
                &pipeline,
                &tasks,
                &Default::default(),
                chrono::Utc::now(),
            )
            .unwrap();
            let (source, child) = ids(&store);
            assert!(active.projects.contains(&source));
            assert!(!active.projects.contains(&child));
            let now = meetings[0].started.unwrap().with_timezone(&chrono::Utc);
            let recent =
                core::areas::activity(&p, &meetings, &pipeline, &[], &Default::default(), now)
                    .unwrap();
            assert!(recent.projects.contains(&source));
            let old = core::areas::activity(
                &p,
                &meetings,
                &pipeline,
                &[],
                &Default::default(),
                now + chrono::Duration::days(31),
            )
            .unwrap();
            assert!(old.projects.is_empty());
            let running = core::areas::activity(
                &p,
                &meetings,
                &pipeline,
                &[],
                &[meetings[0].id.clone()].into(),
                now + chrono::Duration::days(31),
            )
            .unwrap();
            assert!(running.projects.contains(&source));
        }
        "boundary" | "undo-boundary" => {
            if mode == "undo-boundary" {
                apply(
                    &store,
                    Action::Migrate {
                        rows: rows(&store, false),
                    },
                );
            }
            let stage = if mode == "boundary" {
                "publication"
            } else {
                "undo-publication"
            };
            let gate = root.join(format!("local/area-{stage}.pause"));
            write(&gate, &true);
            let root_copy = root.clone();
            let handle = std::thread::spawn(move || {
                child(
                    &root_copy,
                    if stage == "publication" {
                        "refused-batch"
                    } else {
                        "refused-undo"
                    },
                    None,
                )
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !gate.with_extension("reached").exists() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let peer = TaskStore::new(root.join("boundary-peer"));
            peer.add_project("Peer outcome").unwrap();
            copy_tree(&peer.root.join("state"), &root.join("state"));
            fs::remove_file(gate).unwrap();
            assert_eq!(handle.join().unwrap(), 0);
            assert!(
                store
                    .load_projects()
                    .unwrap()
                    .projects
                    .iter()
                    .any(|p| p == "Peer outcome")
            );
        }
        "refused-batch" => {
            let preview = store
                .preview_area(Action::Migrate {
                    rows: rows(&store, false),
                })
                .unwrap();
            assert!(
                store
                    .apply_area(&preview)
                    .unwrap_err()
                    .to_string()
                    .contains("changed")
            );
        }
        "refused-undo" => {
            let id: String =
                serde_json::from_slice(&fs::read(root.join("undo-id.json")).unwrap()).unwrap();
            assert!(
                store
                    .undo_area(&id)
                    .unwrap_err()
                    .to_string()
                    .contains("changed")
            );
        }
        "legacy" => {
            let (id, _) = ids(&store);
            store.rename_project(SOURCE, "Supply outcome").unwrap();
            let p = store.load_projects().unwrap();
            assert_eq!(
                p.area_catalog()
                    .unwrap()
                    .project(&p, "Supply outcome")
                    .unwrap()
                    .id,
                id
            );
            let old: Value =
                serde_json::from_slice(&fs::read(root.join("projects.json")).unwrap()).unwrap();
            assert_eq!(old["projects"][0], SOURCE);
            let all = store.load_overrides().unwrap();
            assert_eq!(all.manual.len(), 2);
        }
        "bytes" => verify_bytes(&root),
        mode if mode.starts_with("review-") => review::run(mode, &store),
        _ => panic!("Unknown proof mode"),
    }
}
