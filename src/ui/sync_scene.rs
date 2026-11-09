//! Invented state for sync renders. The render script supplies an isolated data root.
use super::State;
use serde_json::json;
use sidenote::core::{self, ManualTask, memory::MemoryStore};
use std::fs;
fn isolated() {
    assert!(super::isolated(), "Sync scenes need isolated render state");
}
pub(super) fn settle(s: &mut State) {
    while s.edit_busy || s.areas.refresh_pending || s.areas.pending.is_some() {
        let event = s
            .events_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        s.apply_event(event);
    }
}
fn seed_memory() {
    let root = core::state_dir().join("memory");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("records.json"),serde_json::to_vec(&json!({"schema_version":1,"records":[{
        "id":"proof-memory","type":"decision","text":"Original claim","project":null,"people":[],"sources":[],
        "first_seen":null,"last_confirmed":null,"status":"active","basis":"owner_confirmed",
        "supersedes":null,"updated_at":"2026-11-09T00:00:00Z","source_fingerprint":"proof"}],
        "proposals":[],"events":[],"migration":{"version":1,"source_path":"proof","fingerprint":"proof","accepted_at":"2026-11-09T00:00:00Z"}})).unwrap()).unwrap();
}
fn task(s: &mut State) -> (String, String) {
    let (o, id) = s
        .tasks
        .store
        .add_manual(ManualTask {
            task: "Original task".into(),
            ..Default::default()
        })
        .unwrap();
    s.tasks.overrides = o;
    s.rebuild_tasks();
    settle(s);
    let row = format!("manual/{id}");
    s.open_task(&row, 0);
    (row, id)
}
pub fn memory_scene(s: &mut State) {
    isolated();
    seed_memory();
    s.config.features.memory = true;
    s.memory.refresh(false);
    s.memory_open = true;
    s.open_record("proof-memory");
    s.memory_edit_start("proof-memory");
    s.memory.edit.text = "My memory draft".into();
    MemoryStore::default()
        .edit_record("proof-memory", "Peer claim")
        .unwrap();
    s.memory_edit_save();
    settle(s);
}
pub fn task_scene(s: &mut State) {
    isolated();
    s.config.features.tasks = true;
    s.config.features.projects = true;
    s.view = super::tasks::View::Tasks;
    let (_, id) = task(s);
    s.tasks.inputs.title.text = "My saved title".into();
    s.tasks
        .store
        .update_manual(&id, |m| m.task = "Peer title".into())
        .unwrap();
    s.tasks.inputs.title.focused = true;
    assert!(!s.panel_enter());
    settle(s);
}
pub fn arrival_scene(s: &mut State) {
    isolated();
    s.config.features.tasks = true;
    s.config.features.projects = true;
    s.view = super::tasks::View::Tasks;
    let (_, id) = task(s);
    s.tasks.inputs.title.text = "My unsaved draft stays here".into();
    s.tasks
        .store
        .update_manual(&id, |m| m.owner = Some("Peer owner".into()))
        .unwrap();
    let data = super::refresh::Data::read(s.refresh_options());
    s.apply_event(super::Event::Refreshed(super::refresh::Result {
        generation: core::mutation_generation(),
        options: s.refresh_options(),
        data: Some(Box::new(data)),
    }));
}

pub fn failed_task_scene(s: &mut State) {
    isolated();
    s.config.features.tasks = true;
    s.config.features.projects = true;
    s.view = super::tasks::View::Tasks;
    task(s);
    s.tasks.inputs.title.text = "My task draft stays here".into();
    s.tasks.inputs.title.focused = true;
    let original = s.tasks.store.root.clone();
    let blocked = core::state_dir().join("blocked-proof-root");
    fs::write(&blocked, b"file").unwrap();
    s.tasks.store.root = blocked;
    s.panel_enter();
    settle(s);
    s.tasks.store.root = original;
    s.close_panel();
}
