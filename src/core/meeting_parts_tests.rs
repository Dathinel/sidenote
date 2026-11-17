use super::*;
fn part(hex: &Path, id: &str, duration: u64, status: &str) {
    store_io::write(&hex.join(id).join("manifest.json"), &serde_json::json!({"status":status,
        "created_at_ms":id.split('-').next().unwrap().parse::<i64>().unwrap(),"duration_ms":duration})).unwrap();
    store_io::atomic_write(&hex.join(id).join("transcript.ndjson"),b"{\"start_ms\":0,\"end_ms\":1000,\"text\":\"Synthetic transcript\",\"source\":\"microphone\"}\n").unwrap();
}
#[test]
fn restart_paused_and_crashed_part_stay_joined() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    part(&hex, "100-1", 12000, "recording");
    let mut m = Meeting::begin(root.path(), "100-1", 100).unwrap();
    m.refresh(&hex).unwrap();
    m.pause(root.path()).unwrap();
    let m = read(root.path(), "100-1")
        .unwrap()
        .unwrap()
        .restore(root.path(), &hex)
        .unwrap();
    assert_eq!(m.phase, Phase::Paused);
    assert_eq!(m.elapsed_ms(), 12000);
    let mut m = m;
    m.resume(root.path(), "200-2", 200).unwrap();
    part(&hex, "200-2", 30000, "complete");
    let m = read(root.path(), "100-1")
        .unwrap()
        .unwrap()
        .restore(root.path(), &hex)
        .unwrap();
    assert_eq!(m.phase, Phase::Paused);
    assert_eq!(m.parts.len(), 2);
    assert_eq!(m.elapsed_ms(), 42000);
}
#[test]
fn restart_joins_resume_completed_before_status_arrived() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    part(&hex, "100-1", 12000, "complete");
    let mut m = Meeting::begin(root.path(), "100-1", 100).unwrap();
    m.pause(root.path()).unwrap();
    m.pending_start_ms = Some(200);
    m.save(root.path()).unwrap();
    part(&hex, "200-2", 30000, "complete");
    let m = read(root.path(), "100-1")
        .unwrap()
        .unwrap()
        .restore(root.path(), &hex)
        .unwrap();
    assert_eq!(m.parts.len(), 2);
    assert_eq!(m.phase, Phase::Paused);
    assert_eq!(m.elapsed_ms(), 42000);
}
#[test]
fn notes_wait_for_all_parts_and_resume_marks_previous_notes_stale() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    part(&hex, "100-1", 12000, "complete");
    part(&hex, "200-2", 30000, "transcribing");
    let mut m = Meeting::begin(root.path(), "100-1", 100).unwrap();
    m.pause(root.path()).unwrap();
    m.resume(root.path(), "200-2", 200).unwrap();
    m.pause(root.path()).unwrap();
    assert!(m.prepare_notes(root.path(), &hex).is_err());
    assert!(!inputs(root.path()).join("100-1/transcript.ndjson").exists());
    part(&hex, "200-2", 30000, "complete");
    m.prepare_notes(root.path(), &hex).unwrap();
    m.notes_parts = m.requested_parts;
    m.resume(root.path(), "300-3", 300).unwrap();
    assert!(m.stale());
    assert_eq!(
        m.live_lines(&hex)
            .iter()
            .filter(|l| l.text == "Recording resumed")
            .count(),
        2
    );
}

#[test]
fn pending_start_holds_completion_before_first_status() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    let pending = Meeting::pending(root.path(), 100).unwrap();
    part(&hex, "105-1", 30000, "complete");
    assert!(held_in(root.path(), &hex, "105-1").unwrap());
    let joined = read(root.path(), &pending.id).unwrap().unwrap();
    assert_eq!(joined.parts.len(), 1);
    assert_eq!(joined.parts[0].id, "105-1");
    assert_eq!(joined.notes_parts, 0);
    assert!(
        !inputs(root.path())
            .join(&pending.id)
            .join("transcript.ndjson")
            .exists()
    );
}

#[test]
fn duration_accepts_hex_numeric_milliseconds_without_wall_clock_gaps() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    part(&hex, "100-1", 12000, "complete");
    store_io::write(
        &hex.join("100-1/manifest.json"),
        &serde_json::json!({"status":"complete","duration_ms":12000.0}),
    )
    .unwrap();
    let mut m = Meeting::begin(root.path(), "100-1", 100).unwrap();
    m.refresh(&hex).unwrap();
    m.pause(root.path()).unwrap();
    assert_eq!(m.elapsed_ms(), 12000);
}

#[test]
fn script_update_retains_the_previous_notes_until_replacement() {
    let root = tempfile::tempdir().unwrap();
    let hex = root.path().join("hex");
    part(&hex, "100-1", 42000, "complete");
    let mut m = Meeting::begin(root.path(), "100-1", 100).unwrap();
    let folder = m.prepare_notes(root.path(), &hex).unwrap();
    store_io::write(
        &folder.join("english-summary.json"),
        &serde_json::json!({"title":"Previous notes"}),
    )
    .unwrap();
    prepare_script_update(root.path(), "100-1").unwrap();
    assert!(!folder.join("english-summary.json").exists());
    assert_eq!(
        super::super::Meeting::load_pending(&folder).unwrap().title,
        "Previous notes"
    );
}
