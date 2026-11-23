use serde_json::{Value, json};
use sidenote::core::{
    agent_tools, configure_paths,
    meeting_delete::{Eligibility, Storage},
    mutation_generation,
};
use std::{fs, path::Path};

fn write(folder: &Path, name: &str, data: &[u8]) {
    fs::create_dir_all(folder).unwrap();
    fs::write(folder.join(name), data).unwrap();
}

fn meeting(folder: &Path) {
    write(
        folder,
        "manifest.json",
        br#"{"status":"complete","created_at_ms":1,"ended_at_ms":100}"#,
    );
    write(
        folder,
        "english-summary.json",
        br#"{"title":"Synthetic workshop"}"#,
    );
}

fn count(tool: &str, args: Value) -> usize {
    agent_tools::call(tool, args).unwrap()["data"]["items"]
        .as_array()
        .unwrap_or_else(|| panic!("{tool} did not return a page"))
        .len()
}

#[test]
fn catalog_omits_deleted_aggregate_parts_and_late_peers() {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let source = tmp.path().join("meetings");
    let peers = tmp.path().join("peers");
    let device = peers.join("synthetic-device");
    let id = "200-1";
    let part = "100-1";
    meeting(&source.join(part));
    let retained = source.join(part);
    write(&retained, "notes.md", b"synthetic original notes");
    meeting(&state.join("local/meeting-inputs").join(id));
    write(
        &state.join("local/meeting-parts"),
        "200-1.json",
        br#"{"id":"200-1","parts":[{"id":"100-1","started_ms":1,"duration_ms":1000}],"phase":"Ended"}"#,
    );
    configure_paths(&[
        "--data-dir".into(),
        state.clone().into_os_string(),
        "--meetings-dir".into(),
        source.clone().into_os_string(),
        "--peers-dir".into(),
        peers.into_os_string(),
        "--mcp".into(),
    ])
    .unwrap();
    let recent = || sidenote::core::ask_options::recent_meeting(&source, 100);
    assert!(recent().is_some());
    assert_eq!(count("list_meetings", json!({})), 1);
    assert_eq!(count("search_meetings", json!({"query":"workshop"})), 1);
    assert!(agent_tools::call("get_meeting", json!({"id":id})).is_ok());
    let store = Storage::new(&source, &state).unwrap();
    let recycle = || {
        store
            .delete(
                id,
                "Synthetic".into(),
                10,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap()
    };
    let first = recycle();
    assert!(retained.join("manifest.json").exists());
    assert!(sidenote::core::live::write_notes(&retained, "synthetic late save").is_err());
    assert_eq!(
        fs::read(retained.join("notes.md")).unwrap(),
        b"synthetic original notes"
    );
    meeting(&device.join(id));
    meeting(&device.join(part));
    let omitted = || {
        assert!(recent().is_none());
        assert_eq!(count("list_meetings", json!({})), 0);
        assert_eq!(count("search_meetings", json!({"query":"workshop"})), 0);
        for id in [id, part] {
            assert!(agent_tools::call("get_meeting", json!({"id":id})).is_err());
            assert!(agent_tools::meeting_resource(&format!("meeting://{id}")).is_err());
        }
    };
    omitted();
    let deleted_generation = mutation_generation();
    store.restore(&first).unwrap();
    assert_eq!(mutation_generation(), deleted_generation + 1);
    assert!(recent().is_some());
    assert!(agent_tools::call("get_meeting", json!({"id":id})).is_ok());
    let second = recycle();
    assert!(store.purge(&second, 10 + 30 * DAY).unwrap());
    assert_eq!(
        fs::read(retained.join("notes.md")).unwrap(),
        b"synthetic original notes"
    );
    meeting(&device.join(id));
    meeting(&device.join(part));
    omitted();
}

#[cfg(windows)]
#[test]
fn temporary_suppression_read_failure_schedules_refresh_retry() {
    use sidenote::core::file_reads::Scope;
    use std::os::windows::fs::OpenOptionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("local");
    write(&dir, "deleted-meeting-ids.json", b"{}");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(dir.join("deleted-meeting-ids.json"))
        .unwrap();
    let scope = Scope::begin();
    assert!(sidenote::core::meeting_delete::suppressed_ids(tmp.path()).is_err());
    assert!(scope.stats().temporary);
    drop(held);
    assert!(
        sidenote::core::meeting_delete::suppressed_ids(tmp.path())
            .unwrap()
            .is_empty()
    );
}
