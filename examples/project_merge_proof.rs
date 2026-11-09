//! Separate-process synthetic proof, never a test suite or an installed-app command.
use serde_json::{Value, json};
use sidenote::core::{self, TaskStore, memory::MemoryStore};
use std::{collections::BTreeMap, fs, path::Path};
const FROM: &str = "Northwind Field Guide";
const INTO: &str = "Northwind Atlas";
#[path = "support/project_merge_races.rs"]
mod races;
fn write(path: &Path, value: &impl serde::Serialize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn protected(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(path: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(path).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                visit(&e.path(), out);
            } else {
                out.insert(
                    e.path().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(&core::meetings_dir(), &mut out);
    visit(&core::peers_dir(), &mut out);
    for name in [
        "task-overrides.json",
        "memory/records.json",
        "projects.json",
    ] {
        let path = root.join(name);
        out.insert(path.to_string_lossy().into_owned(), fs::read(path).unwrap());
    }
    out
}
fn peer_write(root: &Path) {
    fn copy(dir: &Path, into: &Path) {
        fs::create_dir_all(into).unwrap();
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let target = into.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &target);
            } else {
                fs::copy(e.path(), target).unwrap();
            }
        }
    }
    let peer = root.join("peer-writer");
    TaskStore::new(peer.clone())
        .update_override("1800000000002-1", 0, "Review the paper prototype", |o| {
            o.owner = Some("Morgan".into())
        })
        .unwrap();
    copy(&peer.join("state"), &root.join("state"));
}
fn all_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            files.extend(all_files(&entry.path()));
        } else {
            files.insert(
                entry.path().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            );
        }
    }
    files
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert!(args.iter().any(|a| a == "--data-dir"));
    core::configure_paths(&args).unwrap();
    let root = core::state_dir();
    assert!(
        core::is_proof_path(&root, "project-merge-proof")
            && core::is_proof_path(&core::meetings_dir(), "project-merge-proof")
            && core::is_proof_path(&core::peers_dir(), "project-merge-proof")
    );
    let command = args[0].to_string_lossy();
    let store = TaskStore::new(root.clone());
    if command == "prepare" {
        assert!(!root.exists(), "Proof must use a fresh folder");
        for (base, id, title, todo) in [
            (
                core::meetings_dir(),
                "1800000000000-1",
                "Northwind workshop",
                true,
            ),
            (
                core::meetings_dir(),
                "1800000000001-1",
                "Northwind research notes",
                false,
            ),
            (
                core::peers_dir().join("synthetic-peer"),
                "1800000000002-1",
                "Northwind peer review",
                true,
            ),
        ] {
            let folder = base.join(id);
            let tasks = if todo {
                json!([{"task":"Review the paper prototype"}])
            } else {
                json!([])
            };
            write(
                &folder.join("english-summary.json"),
                &json!({"title":title,"todo":tasks}),
            );
            write(
                &folder.join("tasks.json"),
                &json!({"meeting_project":FROM,"tasks":[]}),
            );
            fs::write(
                folder.join("notes.md"),
                "Synthetic note. Preserve these exact bytes.\n",
            )
            .unwrap();
        }
        write(
            &root.join("projects.json"),
            &json!({"projects":[FROM,INTO],"aliases":{"Field handbook":FROM}}),
        );
        write(
            &root.join("config.json"),
            &json!({"version":1,"features":{"tasks":true,"memory":true,"projects":true}}),
        );
        write(
            &root.join("task-overrides.json"),
            &json!({"manual":[{"id":"synthetic-manual","task":"Pack workshop cards","project":FROM,"done":true,"added":"2026-11-09"}]}),
        );
        write(
            &root.join("memory/records.json"),
            &json!({"schema_version":1,"records":[{
            "id":"synthetic-memory","type":"project","text":"Use paper cards for the workshop.","project":FROM,
            "sources":[],"first_seen":null,"last_confirmed":null,"status":"active","basis":"owner_confirmed",
            "supersedes":null,"updated_at":"2026-11-09T00:00:00Z","source_fingerprint":"synthetic"
        }],"proposals":[{"id":"synthetic-proposal","op":"add","target_id":null,"text":"Bring spare cards.","project":FROM,
            "sources":[],"rationale":"Synthetic proposal","status":"pending","created_at":"2026-11-09T00:00:00Z","decided_at":null}],"events":[],"migration":null}),
        );
        store.assign_meeting("1800000000001-1", FROM).unwrap();
        write(
            &root.join("projects-before.json"),
            &store.load_projects().unwrap(),
        );
        write(&root.join("protected-before.json"), &protected(&root));
    } else if command == "merge" {
        let preview = store.preview_merge(FROM, INTO).unwrap();
        assert_eq!(
            (
                preview.meetings.len(),
                preview.tasks.len(),
                preview.memory.len()
            ),
            (3, 3, 2)
        );
        write(&root.join("preview.json"), &preview);
        store.merge_projects(&preview).unwrap();
        let p = store.load_projects().unwrap();
        assert_eq!(
            p.merges[FROM].assignment_baseline.as_ref().unwrap().len(),
            8
        );
        let mut legacy = serde_json::to_value(&p.merges[FROM]).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("assignment_baseline");
        let legacy: core::project_merge::ProjectMerge = serde_json::from_value(legacy).unwrap();
        assert!(legacy.assignment_baseline.is_none());
    } else if command == "merged" || command == "old" {
        let merged = command == "merged";
        let p = store.load_projects().unwrap();
        let target = if merged { INTO } else { FROM };
        assert_eq!(p.resolve(FROM), target);
        assert_eq!(p.resolve(" field   HANDBOOKS! "), target);
        assert_eq!(p.resolve("NORTHWIND field-guides"), target);
        assert!(p.project_matches_query(target, "field handbooks!"));
        assert_eq!(p.meeting_projects.get("1800000000001-1").unwrap(), FROM);
        let memory = MemoryStore::new(root.clone());
        assert_eq!(
            memory.list_records().unwrap()[0].project.as_deref(),
            Some(target)
        );
        assert_eq!(
            memory.list_pending_proposals().unwrap()[0]
                .proposal
                .project
                .as_deref(),
            Some(target)
        );
        let (meetings, errors) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
        assert!(errors.is_empty());
        let tasks = core::build_tasks(
            &meetings,
            &core::load_pipeline(&meetings),
            &p,
            &store.load_overrides().unwrap(),
            &Default::default(),
        );
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().all(|t| t.project == target));
        assert!(tasks.iter().find(|t| t.manual).unwrap().done);
        for scope in ["records", "proposals"] {
            let result = core::agent_tools::call(
                "get_memory",
                json!({"scope":scope,"query":"field handbooks!"}),
            )
            .unwrap();
            assert_eq!(result["data"]["items"].as_array().unwrap().len(), 1);
            assert_eq!(result["data"]["items"][0]["project"], target);
        }
        let result = core::agent_tools::call("list_meetings", json!({"project":target})).unwrap();
        assert_eq!(result["data"]["items"].as_array().unwrap().len(), 3);
        let before: BTreeMap<String, Vec<u8>> =
            serde_json::from_slice(&fs::read(root.join("protected-before.json")).unwrap()).unwrap();
        assert_eq!(protected(&root), before, "Source files changed");
        if !merged {
            let before: Value =
                serde_json::from_slice(&fs::read(root.join("projects-before.json")).unwrap())
                    .unwrap();
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                before,
                "Undo did not restore the exact project view"
            );
        }
    } else if command == "incoming" {
        let p = store.load_projects().unwrap();
        assert_eq!(p.matches("field handbooks!"), vec![INTO]);
        let (meetings, errors) = core::scan_sources(&core::meetings_dir(), &core::peers_dir());
        assert!(errors.is_empty());
        let mut pipeline = core::load_pipeline(&meetings);
        for file in pipeline.values_mut() {
            file.meeting_project = Some("field handbooks!".into());
        }
        let tasks = core::build_tasks(
            &meetings,
            &pipeline,
            &p,
            &store.load_overrides().unwrap(),
            &Default::default(),
        );
        assert!(tasks.iter().all(|t| t.project == INTO));
        assert_eq!(p.projects.len(), 2, "Suggestion silently created a project");
    } else if command == "peer" {
        peer_write(&root);
    } else if command == "stale" {
        let preview = store.preview_merge(FROM, INTO).unwrap();
        peer_write(&root);
        assert!(store.merge_projects(&preview).is_err());
        assert_eq!(store.load_projects().unwrap().resolve(FROM), FROM);
    } else if command == "peer-kept" {
        assert_eq!(
            store
                .load_overrides()
                .unwrap()
                .get("1800000000002-1", 0)
                .unwrap()
                .owner
                .as_deref(),
            Some("Morgan")
        );
    } else if command == "move-to-survivor" {
        assert_eq!(store.load_projects().unwrap().resolve(FROM), INTO);
        store
            .update_override("1800000000000-1", 0, "Review the paper prototype", |o| {
                o.project = Some(INTO.into())
            })
            .unwrap();
        write(
            &root.parent().unwrap().join("before-refused.json"),
            &all_files(&root),
        );
    } else if command == "undo-refused" {
        let before: BTreeMap<String, Vec<u8>> = serde_json::from_slice(
            &fs::read(root.parent().unwrap().join("before-refused.json")).unwrap(),
        )
        .unwrap();
        let p = store.load_projects().unwrap();
        let error = store
            .undo_project_merge(FROM, &p.merges[FROM].id)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Some items were moved after the merge, so Undo would not restore them. Move them back by hand or keep the merge."
        );
        assert_eq!(
            store
                .load_overrides()
                .unwrap()
                .get("1800000000000-1", 0)
                .unwrap()
                .project
                .as_deref(),
            Some(INTO)
        );
        assert_eq!(
            all_files(&root),
            before,
            "Refused Undo changed an app-owned file"
        );
        println!("{error}");
    } else if command == "undo" {
        let p = store.load_projects().unwrap();
        store.undo_project_merge(FROM, &p.merges[FROM].id).unwrap();
    } else if command == "backup-guard" {
        let p = store.load_projects().unwrap();
        let merge = &p.merges[FROM];
        let bytes = fs::read(&merge.backup).unwrap();
        let mut altered: Value = serde_json::from_slice(&bytes).unwrap();
        altered["preview"]["from"] = json!("Changed backup");
        write(&merge.backup, &altered);
        assert!(store.undo_project_merge(FROM, &merge.id).is_err());
        fs::write(&merge.backup, bytes).unwrap();
        assert_eq!(store.load_projects().unwrap().resolve(FROM), INTO);
    } else if command.contains("boundary-") || command == "peer-rename" {
        races::run(&root, &command);
    } else {
        panic!("Unknown proof command");
    }
    println!("{}: passed ({})", command, root.display());
}
