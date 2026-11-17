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
