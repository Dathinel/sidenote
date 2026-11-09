//! The synthetic render harness holds each real worker before I/O and exercises repeat clicks.
use super::{Pending, State, wait_for_merge};
use std::{
    fs,
    time::{Duration, Instant},
};

pub(super) fn run(s: &mut State) {
    let root = &s.tasks.store.root;
    assert!(root.starts_with("E:/Personal-Projects/reviews/project-merge-proof"));
    let gate = root.join("local/proof-worker-pause");
    let before = serde_json::to_value(&s.tasks.projects).unwrap();
    let from = "Northwind Field Guide";
    let into = "Northwind Atlas";
    s.start_project_merge(from);
    for pending in [Pending::Preview, Pending::Merge, Pending::Undo] {
        let reached = gate.with_extension("reached");
        if reached.exists() {
            fs::remove_file(&reached).unwrap();
        }
        fs::create_dir_all(gate.parent().unwrap()).unwrap();
        fs::write(&gate, b"hold real worker").unwrap();
        let launch = |s: &mut State| match pending {
            Pending::Preview => s.preview_project_merge(into),
            Pending::Merge => s.confirm_project_merge(),
            Pending::Undo => {
                let id = s.tasks.projects.merges[from].id.clone();
                s.undo_project_merge(from, &id);
            }
        };
        launch(s);
        assert_eq!(s.project_merge.pending, Some(pending));
        let generation = s.project_merge.generation;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !reached.exists() {
            assert!(Instant::now() < deadline, "Worker failed to reach the gate");
            std::thread::sleep(Duration::from_millis(2));
        }
        // The UI method has returned while the worker is demonstrably still held.
        launch(s);
        s.start_project_merge(from);
        assert_eq!(
            s.project_merge.generation, generation,
            "Repeat click launched another worker"
        );
        assert_eq!(s.project_merge.pending, Some(pending));
        fs::remove_file(&gate).unwrap();
        wait_for_merge(s);
        assert!(
            s.project_merge.error.is_none(),
            "Worker error: {:?}",
            s.project_merge.error
        );
        if pending == Pending::Preview {
            assert_eq!(s.project_merge.expanded, [false; 3]);
            assert_eq!(s.project_merge.preview.as_ref().unwrap().into, into);
        }
        if pending == Pending::Merge {
            assert_eq!(s.tasks.projects.resolve(from), into);
        }
    }
    assert_eq!(serde_json::to_value(&s.tasks.projects).unwrap(), before);
    let args: Vec<_> = std::env::args_os().collect();
    let snap = args.windows(2).find(|pair| pair[0] == "--snap").unwrap();
    let proof = std::path::PathBuf::from(&snap[1]).with_extension("proof.json");
    fs::write(
        proof,
        serde_json::to_vec_pretty(&serde_json::json!({
            "operations": ["preview", "merge", "undo"],
            "ui_returned_while_worker_held": true,
            "worker_threads_differ_from_ui": true,
            "repeat_clicks_ignored": true,
            "counts_collapsed_by_default": true,
            "project_view_restored": true
        }))
        .unwrap(),
    )
    .unwrap();
}
