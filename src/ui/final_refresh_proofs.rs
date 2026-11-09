//! Final-panel proofs for CI or the launch sweep. Do not execute in the implementation lane.
use super::*;
use serde_json::json;
use sidenote::core::{ManualTask, TaskStore, file_reads};
use std::{collections::BTreeMap, fs, io};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sidenote-final-proof-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let path = std::path::absolute(&self.0).unwrap();
        let parent = std::path::absolute(std::env::temp_dir()).unwrap();
        assert!(path.starts_with(&parent) && path != parent);
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("sidenote-final-proof-")
        );
        fs::remove_dir_all(path).unwrap();
    }
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for e in fs::read_dir(root).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            out.extend(files(&e.path()));
        } else {
            out.insert(e.path(), fs::read(e.path()).unwrap());
        }
    }
    out
}
fn task(root: &Path) -> (TaskStore, String, core::Overrides) {
    let store = TaskStore::new(root.into());
    let (overrides, id) = store
        .add_manual(ManualTask {
            task: "Original title".into(),
            owner: Some("Original owner".into()),
            ..Default::default()
        })
        .unwrap();
    (store, id, overrides)
}
fn journal_dir(root: &Path) -> PathBuf {
    let state = root.join("state");
    let owner = fs::read_dir(state).unwrap().next().unwrap().unwrap().path();
    owner.join(blake3::hash(b"task-overrides.json").to_hex().to_string())
}

#[test]
fn three_hundred_meetings_survive_a_truncated_peer_folder() {
    let f = Fixture::new();
    let local = f.0.join("meetings");
    let peers = f.0.join("peers");
    let peer = peers.join("other-device");
    fs::create_dir_all(&peer).unwrap();
    let (store, id, _) = task(&f.0);
    store.add_project("Kept project").unwrap();
    let context = "x".repeat(230_000);
    for i in 0..300 {
        let folder = peer.join(format!("1800000000000-{i}"));
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join("english-summary.json"),
            serde_json::to_vec(&json!({"title":format!("Meeting {i}"),"context":[context]}))
                .unwrap(),
        )
        .unwrap();
    }
    let scope = file_reads::Scope::begin();
    let (old, errors) = core::scan_sources(&local, &peers);
    assert!(errors.is_empty());
    assert_eq!(old.len(), 300);
    assert!(scope.stats().bytes > 64 * 1024 * 1024);
    drop(scope);
    for i in 0..9000 {
        fs::write(peer.join(format!("extra-{i}.txt")), b"entry").unwrap();
    }
    let scope = file_reads::Scope::begin();
    let (mut incoming, errors) = core::scan_sources(&local, &peers);
    assert!(!errors.is_empty());
    retain_catalog(&old, &mut incoming, &scope.failed());
    assert_eq!(incoming.len(), 300);
    assert_eq!(store.load_overrides().unwrap().manual[0].id, id);
    assert_eq!(
        store.load_projects().unwrap().projects,
        vec!["Kept project"]
    );
}

#[test]
fn refresh_with_legacy_inputs_writes_nothing_over_five_cycles() {
    let f = Fixture::new();
    let (store, _, _) = task(&f.0);
    fs::write(
        f.0.join("projects.json"),
        br#"{"projects":["Legacy project"]}"#,
    )
    .unwrap();
    fs::write(f.0.join("task-overrides.json"), br#"{"manual":[]}"#).unwrap();
    let before = files(&f.0);
    for _ in 0..5 {
        let _scope = file_reads::Scope::begin();
        store.load_overrides().unwrap();
        store.load_projects().unwrap();
        store.load_settings().unwrap();
        core::memory::MemoryStore::new(f.0.clone()).load().unwrap();
    }
    assert_eq!(files(&f.0), before);
}

#[test]
fn listing_failure_refuses_save_and_preserves_other_fields() {
    let f = Fixture::new();
    let (store, id, old) = task(&f.0);
    let row = format!("manual/{id}");
    let base = core::live_edit::task_value(&old, &row, 0);
    let mut mine = base.clone();
    mine["task"] = json!("Kept draft");
    fs::rename(f.0.join("state"), f.0.join("kept-state")).unwrap();
    fs::write(f.0.join("state"), b"not a folder").unwrap();
    let before = files(&f.0);
    let error = store.merge_task(&row, 0, &base, &mine, None).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        super::super::edit_worker::SaveError::from(&error).message(),
        "Could not read the saved data. Your changes are kept."
    );
    assert_eq!(mine["task"], "Kept draft");
    assert_eq!(mine["owner"], "Original owner");
    assert_eq!(files(&f.0), before);
}

#[test]
fn store_total_limit_keeps_the_last_good_store() {
    let f = Fixture::new();
    let (store, id, old) = task(&f.0);
    let dir = journal_dir(&f.0);
    for i in 0..65 {
        fs::File::create(dir.join(format!("large-{i}.json")))
            .unwrap()
            .set_len(file_reads::JOURNAL_MAX)
            .unwrap();
    }
    let scope = file_reads::Scope::begin();
    let result = store.load_overrides();
    assert!(result.is_err());
    assert!(scope.stats().bytes < file_reads::JOURNAL_MAX as usize);
    let retained = result.unwrap_or(old);
    assert_eq!(retained.manual[0].id, id);
}

#[test]
fn permanent_bad_file_does_not_force_reads_or_reset_the_footer_clock() {
    let f = Fixture::new();
    let (store, _, _) = task(&f.0);
    fs::write(journal_dir(&f.0).join("bad.json"), b"invalid").unwrap();
    let scope = file_reads::Scope::begin();
    assert!(store.load_overrides().is_err());
    assert!(scope.stats().skipped > 0);
    assert!(!scope.stats().temporary);
    let mut retry = Retry::default();
    retry.complete(scope.stats().temporary);
    assert!(retry.due.is_none());
    assert!(!retry.ready());
    let before = Instant::now() - Duration::from_secs(120);
    let unchanged = super::super::different(&vec!["kept"], &vec!["kept"]);
    assert!(!unchanged);
    let last_updated = updated_at(Some(before), Some(true), unchanged);
    assert_eq!(last_updated, Some(before));
    assert!(last_updated.unwrap().elapsed() >= Duration::from_secs(120));
    assert!(!refresh_needed(false, false, &retry));
}

#[test]
fn a_refresh_panic_keeps_data_and_the_next_read_continues() {
    let count = PANICS.load(Ordering::Relaxed);
    let (failed, panicked) = read_caught_with(|| {
        let _scope = file_reads::Scope::begin();
        panic!("Synthetic refresh panic");
    });
    assert!(panicked);
    assert!(PANICS.load(Ordering::Relaxed) > count);
    assert!(!file_reads::refreshing());
    assert!(failed.overrides.is_none());
    assert_eq!(failed.retained.len(), 2);
    assert_eq!(failed.errors, vec!["Refresh hit a problem and will retry"]);
    let (next, panicked) = read_caught_with(|| failed);
    assert!(!panicked);
    assert!(next.overrides.is_none());
    let mut retry = Retry::default();
    for delay in [10, 30, 60, 300, 300] {
        retry.failed();
        let remaining = retry
            .due
            .unwrap()
            .saturating_duration_since(Instant::now())
            .as_secs();
        assert!(remaining >= delay - 1 && remaining <= delay);
    }
}
