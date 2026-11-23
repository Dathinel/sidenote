//! The live half of a recording: the transcript Hex writes while it records, and the notes the
//! user types during the meeting.
//!
//! Source choice: Hex keeps `transcript.rolling.ndjson` in the meeting folder while it records
//! (one `{"covered_ms": ...}` header line, then one `{source, start_ms, end_ms, text}` line per
//! spoken stretch, per source). Reading that file costs one `stat` and, when it changed, one
//! read of a few hundred kilobytes at most. No process is started and no token is needed.
//! The notes go to `notes.md` in the same folder, the file the notes pipeline already reads as
//! "My notes" (`hex-meeting-english.ps1`, `hex-meeting-rerun-notes.py`).
use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Where the user's own notes live, beside Hex's files. It syncs like other meeting text.
pub const NOTES_FILE: &str = "notes.md";
pub(super) static NOTES_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub const NOTES_UNAVAILABLE: &str = "Meeting notes are not set up on this PC";
pub const NOTES_FAILED: &str = "Meeting notes could not be created. Your recording is saved in Hex. Check the notes setup before trying again.";

pub fn notes_pipeline() -> PathBuf {
    #[cfg(test)]
    let home = crate::platform::home();
    #[cfg(not(test))]
    let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
    home.join(".config/hex/scripts/hex-meeting-watch.ps1")
}

pub fn notes_problem(folder: &Path) -> Option<&'static str> {
    if folder.join("english-summary.json").is_file() {
        None
    } else if let Some(cause) = super::meeting_job::script_failure(folder) {
        Some(if cause == super::meeting_job::Cause::Other {
            NOTES_FAILED
        } else {
            cause.message()
        })
    } else if !notes_pipeline().is_file() {
        Some(NOTES_UNAVAILABLE)
    } else {
        None
    }
}
const ROLLING_FILE: &str = "transcript.rolling.ndjson";
/// A notes file larger than this is not read back into the editor or the meeting page.
const NOTES_MAX_BYTES: u64 = 1024 * 1024;
/// A rolling transcript larger than this is not read: it would be many hours of speech.
pub const ROLLING_MAX_BYTES: u64 = 24 * 1024 * 1024;

/// One stretch of speech. Hex's live file knows which side spoke (the microphone or the other
/// side of the call), not who, so `you` is all the speaker the page can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveLine {
    pub seconds: u64,
    pub you: bool,
    pub text: String,
}

#[derive(Deserialize)]
struct Raw {
    source: Option<String>,
    start_ms: Option<u64>,
    text: Option<String>,
}

/// The lines in a rolling file, in the order they were spoken. A header, a blank line, a line
/// that is not JSON (a write in progress) or a line without text is skipped.
pub fn parse_rolling(raw: &str) -> Vec<LiveLine> {
    let mut lines: Vec<LiveLine> = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Raw>(l.trim()).ok())
        .filter_map(|r| {
            let text = r.text?.split_whitespace().collect::<Vec<_>>().join(" ");
            (!text.is_empty()).then(|| LiveLine {
                seconds: r.start_ms.unwrap_or(0) / 1000,
                you: r.source.as_deref() == Some("microphone"),
                text,
            })
        })
        .collect();
    // Stable, so two stretches at the same second keep the file's order.
    lines.sort_by_key(|l| l.seconds);
    lines
}

/// What a read of the live transcript found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The file is not there yet: Hex has not written a word.
    Waiting,
    /// The file is the same one as the last read (same size and time): nothing to parse.
    Same,
    Lines(Vec<LiveLine>, Stamp),
}

/// Size and time of the file at a read, to skip a parse when nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp(u64, Option<SystemTime>);

pub fn read_rolling(folder: &Path, last: Option<Stamp>) -> io::Result<Read> {
    let path = folder.join(ROLLING_FILE);
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Read::Waiting),
        Err(e) => return Err(e),
    };
    if meta.len() > ROLLING_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The live transcript file is too large to read",
        ));
    }
    let stamp = Stamp(meta.len(), meta.modified().ok());
    if last == Some(stamp) {
        return Ok(Read::Same);
    }
    let raw = super::file_reads::read(&path, ROLLING_MAX_BYTES)?;
    Ok(Read::Lines(
        parse_rolling(&String::from_utf8_lossy(&raw)),
        stamp,
    ))
}

/// Prefer Hex's recording id; without one, require a unique active manifest near its start.
pub fn find_folder(root: &Path, hint: Option<&str>, start_ms: Option<i64>) -> Option<PathBuf> {
    if let Some(id) = hint {
        let id = Some(id)
            .filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_digit() || c == '-'))?;
        let folder = root.join(id);
        return folder.is_dir().then_some(folder);
    }
    let start_ms = start_ms?;
    // Hex supplies both clocks; 90 s allows startup/poll delay and elapsed-second rounding.
    const START_TOLERANCE_MS: u64 = 90_000;
    let mut found = None;
    for entry in fs::read_dir(root).ok()? {
        let Ok(entry) = entry else { continue };
        let folder = entry.path();
        if !super::file_reads::directory(&folder) {
            continue;
        }
        let Ok(raw) = super::file_reads::read(&folder.join("manifest.json"), 1024 * 1024) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<super::meetings::Manifest>(&raw) else {
            continue;
        };
        if !matches!(manifest.status.as_deref(), Some("recording" | "starting"))
            || manifest
                .created_at_ms
                .is_none_or(|made| made.abs_diff(start_ms) > START_TOLERANCE_MS)
        {
            continue;
        }
        if found.replace(folder).is_some() {
            return None;
        }
    }
    found
}

/// The notes in a meeting folder, or empty when there are none or the file cannot be read.
pub fn read_notes(folder: &Path) -> String {
    let path = folder.join(NOTES_FILE);
    match fs::metadata(&path) {
        Ok(m) if m.len() <= NOTES_MAX_BYTES => {}
        _ => return String::new(),
    }
    let Ok(bytes) = super::file_reads::read(&path, NOTES_MAX_BYTES) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes));
    text.trim().to_string()
}

pub(super) fn recycle_claim() -> Option<std::sync::MutexGuard<'static, ()>> {
    NOTES_WRITE_LOCK.try_lock().ok()
}

/// Writes the notes with a flushed temporary file and a replacing rename, so the pipeline never
/// reads half a file. The temporary file (`notes.<pid>.tmp`) is on the sync ignore list.
pub fn write_notes(folder: &Path, text: &str) -> io::Result<()> {
    let _write = NOTES_WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let state = super::state_dir();
    if folder.parent().is_some_and(|parent| {
        parent == super::meetings_dir() || parent == super::meeting_parts::inputs(&state)
    }) && super::meeting_delete::suppressed_ids(&state)?.contains(
        folder
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    ) {
        return Err(io::ErrorKind::NotFound.into());
    }
    if !folder.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "The meeting folder is not there yet",
        ));
    }
    super::store_io::atomic_write(&folder.join(NOTES_FILE), text.as_bytes())
}

/// Keep a recovery copy when Hex has not created the meeting folder or a save fails.
pub fn preserve_notes(
    folder: Option<PathBuf>,
    text: &str,
    recovery: &Path,
) -> Result<Option<PathBuf>, String> {
    if let Some(folder) = folder {
        match write_notes(&folder, text) {
            Ok(()) => {
                let _ = fs::remove_file(recovery);
                return Ok(Some(folder));
            }
            Err(error) => log_save_failure("meeting_folder", error.kind()),
        }
    }
    let backup = recovery
        .parent()
        .ok_or_else(|| io::Error::other("Missing recovery folder"))
        .and_then(fs::create_dir_all)
        .and_then(|()| super::store_io::atomic_write(recovery, text.as_bytes()));
    backup.map(|()| None).map_err(|error| {
        log_save_failure("recovery_copy", error.kind());
        "Could not save your notes or a recovery copy. Keep this text before closing Sidenote."
            .into()
    })
}

fn log_save_failure(route: &str, kind: io::ErrorKind) {
    crate::listener::log(&format!(
        "live notes save failed route={route} kind={kind:?}"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: i64 = 1_800_000_120_000;
    const START_MS: i64 = NOW_MS - 120_000;

    fn folder_fixture(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("live-folder-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn manifest(root: &Path, id: &str, status: &str, created_at_ms: i64) -> PathBuf {
        let folder = root.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("manifest.json"),
            serde_json::json!({"id": id, "status": status, "created_at_ms": created_at_ms})
                .to_string(),
        )
        .unwrap();
        folder
    }

    fn folder_from_no_id_status(root: &Path) -> Option<PathBuf> {
        let status: serde_json::Value = serde_json::from_str(
            r#"{"protocol":4,"meeting":{"state":"recording","title":null,"elapsed_seconds":120}}"#,
        )
        .unwrap();
        let meeting = &status["meeting"];
        let start_ms = NOW_MS - meeting["elapsed_seconds"].as_i64().unwrap() * 1000;
        find_folder(root, meeting["id"].as_str(), Some(start_ms))
    }

    #[test]
    fn no_id_status_finds_current_recording() {
        let root = folder_fixture("current");
        let current = manifest(&root, "1800000000000-45464", "recording", START_MS);
        assert_eq!(folder_from_no_id_status(&root), Some(current));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_id_status_ignores_previous_unfinished_recording() {
        let root = folder_fixture("unfinished");
        let previous = manifest(
            &root,
            "1799996400000-45464",
            "recording",
            START_MS - 3_600_000,
        );
        write_notes(&previous, "previous notes").unwrap();
        assert_eq!(folder_from_no_id_status(&root), None);
        let current = manifest(&root, "1800000000000-45464", "recording", START_MS);
        assert_eq!(folder_from_no_id_status(&root), Some(current));
        assert_eq!(read_notes(&previous), "previous notes");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_id_status_ignores_completed_recording() {
        let root = folder_fixture("completed");
        manifest(&root, "1800000000000-45464", "complete", START_MS);
        assert_eq!(folder_from_no_id_status(&root), None);
        let current = manifest(&root, "1800000000001-45464", "recording", START_MS + 1);
        assert_eq!(folder_from_no_id_status(&root), Some(current));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_id_status_rejects_two_candidates() {
        let root = folder_fixture("ambiguous");
        manifest(&root, "1800000000000-1", "recording", START_MS);
        manifest(&root, "1800000000001-2", "starting", START_MS + 1);
        assert_eq!(folder_from_no_id_status(&root), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recording_id_wins_over_start_time_candidates() {
        let root = folder_fixture("id");
        let current = manifest(&root, "1800000000000-1", "recording", START_MS);
        manifest(&root, "1800000000001-2", "recording", START_MS + 1);
        assert_eq!(
            find_folder(&root, Some("1800000000000-1"), Some(START_MS)),
            Some(current)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_id_status_accepts_starting_within_the_tolerance_only() {
        let root = folder_fixture("tolerance");
        for offset in [-90_001, -90_000, 90_000, 90_001] {
            let current = manifest(&root, "1800000000000-1", "starting", START_MS + offset);
            let expected = (offset.abs() <= 90_000).then(|| current.clone());
            assert_eq!(folder_from_no_id_status(&root), expected, "offset {offset}");
            fs::remove_dir_all(current).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_id_status_requires_a_valid_manifest_and_creation_time() {
        let root = folder_fixture("invalid");
        let folder = root.join("1800000000000-1");
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(folder_from_no_id_status(&root), None);
        for raw in [
            "{",
            r#"{"status":"recording"}"#,
            r#"{"created_at_ms":1800000000000}"#,
            r#"{"status":"recording","created_at_ms":"1800000000000"}"#,
        ] {
            fs::write(folder.join("manifest.json"), raw).unwrap();
            assert_eq!(folder_from_no_id_status(&root), None);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_lookup_requires_start_time_and_never_falls_back_from_an_explicit_id() {
        let root = folder_fixture("no-guess");
        manifest(&root, "1800000000000-1", "recording", START_MS);
        assert_eq!(find_folder(&root, None, None), None);
        for hint in ["", "../1800000000000-1", "1800000000001-1"] {
            assert_eq!(find_folder(&root, Some(hint), Some(START_MS)), None);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_pipeline_failure_is_visible_until_a_summary_arrives() {
        let folder = std::env::temp_dir().join(format!("notes-status-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("notes-failed"), b"failed").unwrap();
        assert_eq!(notes_problem(&folder), Some(NOTES_FAILED));
        for (category, words) in [
            ("HELPER_AUTH", "sign in"),
            ("HELPER_LIMIT", "usage limit"),
            ("timeout", "too long"),
        ] {
            fs::write(folder.join("pipeline-failed.txt"), format!("try 2026-11-14T12:00:00: pipeline-failure stage=notes category={category} kind=claude exit_code=1 deadline_seconds=600 elapsed_seconds=1.000\n")).unwrap();
            let message = notes_problem(&folder).unwrap();
            assert!(message.contains(words));
            assert!(!message.contains(category));
        }
        fs::write(folder.join("english-summary.json"), b"{}").unwrap();
        assert_eq!(notes_problem(&folder), None);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn missing_meeting_keeps_a_recovery_copy_until_the_save_succeeds() {
        let root = std::env::temp_dir().join(format!("notes-recovery-{}", std::process::id()));
        let recovery = root.join("recovery/notes.md");
        assert!(
            preserve_notes(None, "unsaved text", &recovery)
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::read_to_string(&recovery).unwrap(), "unsaved text");
        let folder = root.join("meeting");
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            preserve_notes(Some(folder.clone()), "latest text", &recovery).unwrap(),
            Some(folder.clone())
        );
        assert_eq!(read_notes(&folder), "latest text");
        assert!(!recovery.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_save_failures_keep_the_draft_retryable() {
        let root = std::env::temp_dir().join(format!("notes-close-failure-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let blocked = root.join("not-a-directory");
        fs::write(&blocked, "block recovery").unwrap();
        let recovery = blocked.join("draft.md");
        let meeting = root.join("missing-meeting");
        let draft = "editable draft";
        assert!(preserve_notes(Some(meeting.clone()), draft, &recovery).is_err());
        fs::create_dir_all(&meeting).unwrap();
        assert_eq!(
            preserve_notes(Some(meeting.clone()), draft, &recovery).unwrap(),
            Some(meeting.clone())
        );
        assert_eq!(read_notes(&meeting), draft);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rolling_lines_come_out_in_spoken_order() {
        let raw = concat!(
            "{\"covered_ms\":{\"system\":9000,\"microphone\":9000}}\n",
            "{\"source\":\"system\",\"start_ms\":5200,\"end_ms\":6000,\"text\":\"Second  line\"}\n",
            "{\"source\":\"microphone\",\"start_ms\":1000,\"end_ms\":2000,\"text\":\"First line\"}\n",
            "{\"source\":\"system\",\"start_ms\":7000,\"end_ms\":7500,\"text\":\"   \"}\n",
            "{\"source\":\"system\",\"start_ms\":8000,"
        );
        let lines = parse_rolling(raw);
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].seconds, lines[0].you), (1, true));
        assert_eq!(lines[0].text, "First line");
        assert_eq!((lines[1].seconds, lines[1].you), (5, false));
        assert_eq!(lines[1].text, "Second line");
    }

    #[test]
    fn notes_round_trip_and_replace() {
        let dir = std::env::temp_dir().join(format!("live-notes-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_notes(&dir), "");
        write_notes(&dir, "- first point\n- second point\n").unwrap();
        assert_eq!(read_notes(&dir), "- first point\n- second point");
        write_notes(&dir, "changed").unwrap();
        assert_eq!(read_notes(&dir), "changed");
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_new_folder() {
        let dir = std::env::temp_dir().join("live-notes-test-missing-folder");
        assert!(write_notes(&dir, "x").is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn startup_close_never_overwrites_the_previous_unfinished_meeting() {
        let root = std::env::temp_dir().join(format!("live-startup-close-{}", std::process::id()));
        let previous = root.join("1800000000000-1");
        fs::create_dir_all(&previous).unwrap();
        write_notes(&previous, "meeting A notes").unwrap();
        let recovery = root.join("recovery/1800000060000.md");
        for hint in [None, Some("1800000060000-1"), Some("../1800000000000-1")] {
            let folder = find_folder(&root, hint, None);
            assert_eq!(folder, None);
            assert!(
                preserve_notes(folder, "meeting B draft", &recovery)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(read_notes(&previous), "meeting A notes");
            assert_eq!(fs::read_to_string(&recovery).unwrap(), "meeting B draft");
        }
        let current = root.join("1800000060000-1");
        fs::create_dir_all(&current).unwrap();
        let folder = find_folder(&root, Some("1800000060000-1"), None);
        assert_eq!(
            preserve_notes(folder, "meeting B draft", &recovery).unwrap(),
            Some(current.clone())
        );
        assert_eq!(read_notes(&previous), "meeting A notes");
        assert_eq!(read_notes(&current), "meeting B draft");
        fs::remove_dir_all(root).unwrap();
    }
}
