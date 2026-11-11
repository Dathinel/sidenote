use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    base: PathBuf,
    store: Store,
}
const ID: &str = "1800000000000-1";
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "sidenote-settle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::new(base.join("data"), base.join("meetings"));
        fs::create_dir_all(store.meetings.join(ID)).unwrap();
        fs::write(
            store.meetings.join(ID).join("manifest.json"),
            r#"{"status":"complete"}"#,
        )
        .unwrap();
        super::super::DeviceSettings::load_in(&store.root).unwrap();
        Self { base, store }
    }
    fn summary(&self) -> Value {
        json!({"title":"Workshop","todo":[{"id":"task-1","title":"Send the sample pack","task":"Send the sample pack","owner":"Alex","due":"2026-11-12","due_default":false,"status":"todo","at":["00:05"]}]})
    }
    fn publish(&self, kind: Kind) -> Entry {
        match kind {Kind::TaskProperties=>self.store.publish_summary(ID,&self.summary(),true).unwrap(),Kind::ProjectAssignment=>self.store.publish_projects(ID,"Workshop",&json!({"meeting_project":"Atlas","tasks":[{"i":0,"project":"Atlas","mine":true}]})).unwrap()}
        self.store.history().unwrap()[0].entry.clone()
    }
    fn value(&self, entry: &Entry) -> Value {
        let value = self
            .store
            .read(&self.store.source(ID, &entry.target.file).unwrap())
            .unwrap()
            .unwrap();
        match entry.kind {
            Kind::TaskProperties => props(&value["todo"][0]),
            Kind::ProjectAssignment => value,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
#[test]
fn every_kind_logs_undoes_and_redoes_without_rewriting_history() {
    for kind in [Kind::TaskProperties, Kind::ProjectAssignment] {
        let f = Fixture::new();
        let entry = f.publish(kind);
        let intent = f.store.intents().unwrap().pop().unwrap();
        let bytes = fs::read(f.store.dir().join(format!("{}.intent.json", intent.id))).unwrap();
        assert_eq!(entry.kind, kind);
        assert_eq!(entry.source_meeting_id, ID);
        assert!(!entry.reason.is_empty());
        assert_eq!(f.value(&entry), entry.after);
        let undo = f.store.undo(&entry.id).unwrap();
        assert_eq!(f.value(&entry), entry.before);
        assert_eq!(undo.undo_of.as_deref(), Some(entry.id.as_str()));
        assert!(f.store.history().unwrap()[0].undone);
        let redo = f.store.undo(&undo.id).unwrap();
        assert_eq!(f.value(&entry), entry.after);
        assert_eq!(redo.undo_of.as_deref(), Some(undo.id.as_str()));
        assert!(!f.store.history().unwrap()[0].undone);
        assert_eq!(
            fs::read(f.store.dir().join(format!("{}.intent.json", intent.id))).unwrap(),
            bytes
        );
    }
}
#[test]
fn every_kind_refuses_later_owner_edits_and_deletion() {
    for kind in [Kind::TaskProperties, Kind::ProjectAssignment] {
        let f = Fixture::new();
        let entry = f.publish(kind);
        super::super::sync_state::update(
            &f.store.root,
            "task-overrides.json",
            |value: &mut Value| {
                *value = json!({"tasks":{ID:{"id:task-1":{"owner":"Jamie"}}}});
            },
        )
        .unwrap();
        let before = f.value(&entry);
        assert!(f.store.undo(&entry.id).is_err());
        assert_eq!(f.value(&entry), before);
        fs::remove_file(f.store.meetings.join(ID).join("manifest.json")).unwrap();
        assert!(f.store.undo(&entry.id).is_err());
    }
}
#[test]
fn every_kind_rolls_forward_a_flushed_intent_and_recognizes_an_already_landed_write() {
    for kind in [Kind::TaskProperties, Kind::ProjectAssignment] {
        let f = Fixture::new();
        let entry = f.publish(kind);
        let intent = f.store.intents().unwrap().pop().unwrap();
        fs::remove_file(f.store.dir().join(format!("{}.done.json", intent.id))).unwrap();
        let path = f.store.source(ID, &intent.mutation.file).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(f.store.recover().unwrap(), 1);
        assert_eq!(f.value(&entry), entry.after);
        assert_eq!(f.store.recover().unwrap(), 0);
        fs::remove_file(f.store.dir().join(format!("{}.done.json", intent.id))).unwrap();
        assert_eq!(f.store.recover().unwrap(), 1);
        assert_eq!(f.value(&entry), entry.after);
    }
}
#[test]
fn recovery_keeps_later_edits_and_never_recreates_a_deleted_meeting() {
    let f = Fixture::new();
    let entry = f.publish(Kind::TaskProperties);
    let intent = f.store.intents().unwrap().pop().unwrap();
    fs::remove_file(f.store.dir().join(format!("{}.done.json", intent.id))).unwrap();
    let path = f.store.source(ID, &FileKind::Summary).unwrap();
    let mut changed = f.summary();
    changed["todo"][0]["owner"] = json!("Jamie");
    store_io::write(&path, &changed).unwrap();
    assert_eq!(f.store.recover().unwrap(), 0);
    assert_eq!(f.store.read(&path).unwrap().unwrap(), changed);
    assert!(f.store.undo(&entry.id).is_err());
    let f = Fixture::new();
    f.publish(Kind::ProjectAssignment);
    let intent = f.store.intents().unwrap().pop().unwrap();
    fs::remove_file(f.store.dir().join(format!("{}.done.json", intent.id))).unwrap();
    fs::remove_file(f.store.meetings.join(ID).join("manifest.json")).unwrap();
    assert!(f.store.recover().is_ok());
}
#[test]
fn a_tombstone_and_recreated_identical_edit_still_block_undo() {
    let f = Fixture::new();
    let entry = f.publish(Kind::TaskProperties);
    for value in [
        json!({"tasks":{ID:{"id:task-1":{"owner":"Alex"}}}}),
        json!({"tasks":{}}),
    ] {
        super::super::sync_state::update(
            &f.store.root,
            "task-overrides.json",
            |current: &mut Value| *current = value,
        )
        .unwrap();
    }
    assert!(f.store.undo(&entry.id).is_err());
}

#[test]
fn undo_and_redo_intents_recover_for_every_kind() {
    for kind in [Kind::TaskProperties, Kind::ProjectAssignment] {
        let f = Fixture::new();
        let entry = f.publish(kind);
        let mut target = entry.id.clone();
        for _ in 0..2 {
            let operation = f.store.undo(&target).unwrap();
            let intent = f
                .store
                .intents()
                .unwrap()
                .into_iter()
                .find(|intent| intent.entries[0].id == operation.id)
                .unwrap();
            fs::remove_file(f.store.dir().join(format!("{}.done.json", intent.id))).unwrap();
            store_io::write(
                &f.store.source(ID, &intent.mutation.file).unwrap(),
                intent.mutation.before.as_ref().unwrap(),
            )
            .unwrap();
            assert_eq!(f.store.recover().unwrap(), 1);
            assert_eq!(f.value(&entry), operation.after);
            target = operation.id;
        }
    }
}

#[test]
fn refusing_a_changed_source_item_keeps_the_persons_description() {
    let f = Fixture::new();
    let entry = f.publish(Kind::TaskProperties);
    let path = f.store.source(ID, &FileKind::Summary).unwrap();
    let mut value = f.store.read(&path).unwrap().unwrap();
    value["todo"][0]["description"] = json!("An owner edit");
    store_io::write(&path, &value).unwrap();
    assert!(f.store.undo(&entry.id).is_err());
    assert_eq!(f.store.read(&path).unwrap().unwrap(), value);
}

#[test]
fn project_undo_refuses_a_later_task_assignment_or_source_deletion() {
    for delete in [false, true] {
        let f = Fixture::new();
        store_io::write(
            &f.store.source(ID, &FileKind::Summary).unwrap(),
            &f.summary(),
        )
        .unwrap();
        let entry = f.publish(Kind::ProjectAssignment);
        if delete {
            store_io::write(
                &f.store.source(ID, &FileKind::Summary).unwrap(),
                &json!({"todo":[]}),
            )
            .unwrap();
        } else {
            let assignment = format!(
                "assignment:{}",
                json!(["task", ID, 0, "Send the sample pack"])
            );
            super::super::sync_state::update(
                &f.store.root,
                "projects.json",
                |value: &mut Value| {
                    value[&assignment] = json!({"project":"Another project"});
                },
            )
            .unwrap();
        }
        assert!(f.store.undo(&entry.id).is_err());
        assert_eq!(f.value(&entry), entry.after);
    }
}

#[test]
fn multiple_task_undos_are_independent_and_counts_follow_redo() {
    let f = Fixture::new();
    let mut summary = f.summary();
    let mut second = summary["todo"][0].clone();
    second["id"] = json!("task-2");
    second["task"] = json!("Book the room");
    summary["todo"].as_array_mut().unwrap().push(second);
    f.store.publish_summary(ID, &summary, true).unwrap();
    let rows = f.store.history().unwrap();
    assert_eq!(f.store.meeting_count(ID).unwrap(), 2);
    let undo = f.store.undo(&rows[0].entry.id).unwrap();
    f.store.undo(&rows[1].entry.id).unwrap();
    assert_eq!(f.store.meeting_count(ID).unwrap(), 0);
    f.store.undo(&undo.id).unwrap();
    assert_eq!(f.store.meeting_count(ID).unwrap(), 1);
}
