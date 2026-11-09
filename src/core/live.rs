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
pub const NOTES_UNAVAILABLE: &str = "Meeting notes are not set up on this PC";
pub const NOTES_FAILED: &str = "Meeting notes could not be created. Your recording is saved in Hex. Check the notes setup before trying again.";

pub fn notes_pipeline() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
        .join(".config/hex/scripts/hex-meeting-watch.ps1")
}

pub fn notes_problem(folder: &Path) -> Option<&'static str> {
    if folder.join("english-summary.json").is_file() {
        None
    } else if folder.join("notes-failed").is_file() {
        Some(NOTES_FAILED)
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

/// The folder of the recording in progress. `hint` is the id Hex reports for it, when it does;
/// otherwise the newest folder made at or after `since_ms` (folder ids start with their creation
/// time, in milliseconds) that has no finished notes yet.
pub fn find_folder(root: &Path, since_ms: i64, hint: Option<&str>) -> Option<PathBuf> {
    if let Some(id) =
        hint.filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_digit() || c == '-'))
    {
        let folder = root.join(id);
        if folder.is_dir() {
            return Some(folder);
        }
    }
    let mut best: Option<(i64, PathBuf)> = None;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let Some(made) = id.split('-').next().and_then(|n| n.parse::<i64>().ok()) else {
            continue;
        };
        let path = entry.path();
        if made < since_ms
            || best.as_ref().is_some_and(|(b, _)| *b >= made)
            || path.join("english-summary.json").exists()
        {
            continue;
        }
        best = Some((made, path));
    }
    best.map(|(_, p)| p)
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

/// Writes the notes with a flushed temporary file and a replacing rename, so the pipeline never
/// reads half a file. The temporary file (`notes.<pid>.tmp`) is on the sync ignore list.
pub fn write_notes(folder: &Path, text: &str) -> io::Result<()> {
    if !folder.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "The meeting folder is not there yet",
        ));
    }
    super::store_io::atomic_write(&folder.join(NOTES_FILE), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipeline_failure_is_visible_until_a_summary_arrives() {
        let folder = std::env::temp_dir().join(format!("notes-status-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("notes-failed"), b"failed").unwrap();
        assert_eq!(notes_problem(&folder), Some(NOTES_FAILED));
        fs::write(folder.join("english-summary.json"), b"{}").unwrap();
        assert_eq!(notes_problem(&folder), None);
        fs::remove_dir_all(folder).unwrap();
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
    fn find_folder_prefers_the_hint_then_the_newest_unfinished() {
        let root = std::env::temp_dir().join(format!("live-notes-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for id in ["1000-1", "2000-1", "3000-1"] {
            fs::create_dir_all(root.join(id)).unwrap();
        }
        fs::write(root.join("3000-1").join("english-summary.json"), "{}").unwrap();
        assert_eq!(find_folder(&root, 1500, None), Some(root.join("2000-1")));
        assert_eq!(
            find_folder(&root, 1500, Some("1000-1")),
            Some(root.join("1000-1"))
        );
        assert_eq!(find_folder(&root, 2500, None), None);
        // A hint that is not an id never leaves the root.
        assert_eq!(find_folder(&root, 9999, Some("..\\x")), None);
        let _ = fs::remove_dir_all(&root);
    }
}
