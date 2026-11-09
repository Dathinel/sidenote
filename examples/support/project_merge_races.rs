use super::{FROM, INTO, write};
use sidenote::core::{self, TaskStore, memory::MemoryStore};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let into = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &into);
        } else {
            fs::copy(entry.path(), into).unwrap();
        }
    }
}
fn peer(root: &Path) -> TaskStore {
    let peer = root.join("peer");
    let store = TaskStore::new(peer.clone());
    write(
        &peer.join("projects.json"),
        &TaskStore::new(root.into()).load_projects().unwrap(),
    );
    for name in ["task-overrides.json", "memory/records.json"] {
        let into = peer.join(name);
        fs::create_dir_all(into.parent().unwrap()).unwrap();
        fs::copy(root.join(name), into).unwrap();
    }
    store
}
fn own_project_journals(root: &Path) -> usize {
    let id = core::DeviceSettings::load().unwrap().id;
    let folder = root
        .join("state")
        .join(id)
        .join(blake3::hash(b"projects.json").to_hex().as_str());
    fs::read_dir(folder).unwrap().count()
}
fn boundary<T>(root: &Path, incoming: &Path, operation: impl FnOnce() -> T) -> T {
    let gate = root.join("local/proof-publication-pause");
    let reached = gate.with_extension("reached");
    if reached.exists() {
        fs::remove_file(&reached).unwrap();
    }
    fs::write(&gate, b"pause").unwrap();
    let target = root.join("state");
    let incoming = incoming.to_owned();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !reached.exists() {
            assert!(
                Instant::now() < deadline,
                "Publication did not reach the rendezvous"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        copy_tree(&incoming, &target);
        fs::remove_file(gate).unwrap();
    });
    let result = operation();
    writer.join().unwrap();
    result
}
pub fn run(root: &Path, command: &str) {
    let store = TaskStore::new(root.into());
    if command == "peer-rename" {
        store
            .merge_projects(&store.preview_merge(FROM, INTO).unwrap())
            .unwrap();
        let id = store.load_projects().unwrap().merges[FROM].id.clone();
        let peer = peer(root);
        peer.rename_project(FROM, "Harbor research").unwrap();
        copy_tree(&peer.root.join("state"), &root.join("state"));
        let p = store.load_projects().unwrap();
        assert_eq!(p.resolve(FROM), "Harbor research");
        assert_eq!(
            MemoryStore::new(root.into()).list_records().unwrap()[0]
                .project
                .as_deref(),
            Some("Harbor research")
        );
        let error = store.undo_project_merge(FROM, &id).unwrap_err().to_string();
        assert!(error.contains("renamed after the merge"));
        assert_eq!(
            store.load_projects().unwrap().resolve(FROM),
            "Harbor research"
        );
        return;
    }
    let undo = command.starts_with("undo-boundary-");
    let id = if undo {
        store
            .merge_projects(&store.preview_merge(FROM, INTO).unwrap())
            .unwrap();
        Some(store.load_projects().unwrap().merges[FROM].id.clone())
    } else {
        None
    };
    let preview = if undo {
        None
    } else {
        Some(store.preview_merge(FROM, INTO).unwrap())
    };
    let peer = peer(root);
    match command {
        "boundary-merge" => {
            peer.add_project("Harbor research").unwrap();
            let preview = peer.preview_merge(FROM, "Harbor research").unwrap();
            peer.merge_projects(&preview).unwrap();
        }
        "boundary-same-value" => {
            peer.update_projects(|p| {
                p.aliases.insert("Field handbook".into(), INTO.into());
            })
            .unwrap();
            peer.update_projects(|p| {
                p.aliases.insert("Field handbook".into(), FROM.into());
            })
            .unwrap();
        }
        "undo-boundary-merge" => {
            peer.add_project("Harbor research").unwrap();
            let preview = peer.preview_merge(INTO, "Harbor research").unwrap();
            peer.merge_projects(&preview).unwrap();
        }
        "boundary-task" | "undo-boundary-task" => {
            peer.update_override("1800000000002-1", 0, "Review the paper prototype", |o| {
                o.owner = Some("Morgan".into())
            })
            .unwrap();
        }
        "boundary-memory" | "undo-boundary-memory" => {
            MemoryStore::new(peer.root.clone())
                .edit_record("synthetic-memory", "Updated at the publication boundary.")
                .unwrap();
        }
        _ => panic!("Unknown race proof"),
    }
    let original = serde_json::to_value(store.load_projects().unwrap()).unwrap();
    let before = own_project_journals(root);
    let result = boundary(root, &peer.root.join("state"), || {
        if let Some(id) = &id {
            store.undo_project_merge(FROM, id)
        } else {
            store.merge_projects(preview.as_ref().unwrap())
        }
    });
    assert!(result.is_err(), "A stale operation published");
    assert_eq!(
        own_project_journals(root),
        before,
        "Stale local project journal was published"
    );
    let p = store.load_projects().unwrap();
    if command == "boundary-same-value" {
        assert_eq!(serde_json::to_value(&p).unwrap(), original);
    }
    if command.ends_with("merge") {
        assert_eq!(p.resolve(FROM), "Harbor research");
        assert_eq!(
            p.merges[if undo { INTO } else { FROM }].id,
            peer.load_projects().unwrap().merges[if undo { INTO } else { FROM }].id
        );
    } else {
        assert_eq!(p.resolve(FROM), if undo { INTO } else { FROM });
    }
    if command.ends_with("task") {
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
    }
    if command.ends_with("memory") {
        assert_eq!(
            MemoryStore::new(root.into()).list_records().unwrap()[0]
                .text
                .as_deref(),
            Some("Updated at the publication boundary.")
        );
    }
    println!("publication guard: no local journal; incoming peer change preserved");
}
