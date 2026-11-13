//! Device-local recording decisions. Hex recordings are always read-only here.
use super::store_io;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub const EXPLANATION: &str =
    "Short recordings (under 30 seconds or 25 words) are hidden and not processed.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    Short,
    Unknown,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    short: bool,
    detected_at: Option<i64>,
    kept: bool,
    #[serde(default)]
    work_queued: bool,
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn state_path(root: &Path, id: &str) -> PathBuf {
    root.join("local/recording-guard")
        .join(format!("{id}.json"))
}
fn classify(folder: &Path) -> Eligibility {
    classify_readable(folder).unwrap_or(Eligibility::Unknown)
}
fn classify_readable(folder: &Path) -> Option<Eligibility> {
    store_io::reject_reparse(folder).ok()?;
    let raw = store_io::read_bounded(&folder.join("manifest.json"), 1024 * 1024).ok()?;
    let original = folder.join("hex-original/transcript.ndjson");
    let source = if original.try_exists().ok()? {
        original
    } else {
        folder.join("transcript.ndjson")
    };
    store_io::reject_reparse(&source).ok()?;
    let file = fs::File::open(source).ok()?;
    let before = file.metadata().ok()?;
    let mut reader = BufReader::new(file);
    let result = classify_reader(&raw, &mut reader)?;
    let after = reader.get_ref().metadata().ok()?;
    if before.len() != after.len() || before.modified().ok()? != after.modified().ok()? {
        return None;
    }
    Some(result)
}
fn classify_reader(raw: &[u8], reader: &mut impl BufRead) -> Option<Eligibility> {
    let manifest: serde_json::Value =
        serde_json::from_slice(raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw)).ok()?;
    if manifest["status"] != "complete" {
        return None;
    }
    let duration = manifest["duration_ms"]
        .as_f64()
        .filter(|d| d.is_finite() && *d >= 0.0)?;
    let mut count = 0;
    let mut line = String::new();
    #[derive(Deserialize)]
    struct Segment {
        text: String,
    }
    // Unicode \w keeps Indic vowel marks and conjuncts within a word; punctuation separates words.
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+").unwrap());
    loop {
        line.clear();
        let n = reader
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_line(&mut line)
            .ok()?;
        if n == 0 {
            break;
        }
        if n > 1024 * 1024 {
            return None;
        }
        if line.trim().is_empty() {
            continue;
        }
        let segment: Segment = serde_json::from_str(line.trim_start_matches('\u{feff}')).ok()?;
        if count < 25 {
            count += WORD
                .find_iter(&segment.text)
                .filter(|m| m.as_str().chars().any(char::is_alphanumeric))
                .take(25 - count)
                .count();
        }
        // Continue validating after 25 words: a later partial JSON line is still UNKNOWN.
    }
    Some(if duration < 30_000.0 || count < 25 {
        Eligibility::Short
    } else {
        Eligibility::Eligible
    })
}
/// The shared enqueue and worker-claim gate. UNKNOWN is retried without starting work.
pub fn eligibility(root: &Path, meetings: &Path, id: &str) -> io::Result<Eligibility> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    apply_eligibility(root, id, classify(&meetings.join(id)))
}
pub(crate) fn eligibility_captured(
    root: &Path,
    id: &str,
    manifest: &[u8],
    transcript: &[u8],
) -> io::Result<Eligibility> {
    apply_eligibility(
        root,
        id,
        classify_reader(manifest, &mut io::Cursor::new(transcript)).unwrap_or(Eligibility::Unknown),
    )
}
fn apply_eligibility(root: &Path, id: &str, result: Eligibility) -> io::Result<Eligibility> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    let path = state_path(root, id);
    let state: State = store_io::read(&path)?;
    if state.kept {
        return Ok(match result {
            Eligibility::Unknown => Eligibility::Unknown,
            _ => Eligibility::Eligible,
        });
    }
    if state.short {
        return Ok(Eligibility::Short);
    }
    if result == Eligibility::Short && !super::paths::read_only() {
        let _lock = store_io::lock(&path.with_extension("lock"))?;
        let mut state: State = store_io::read(&path)?;
        if state.kept {
            return Ok(Eligibility::Eligible);
        }
        state.short = true;
        state
            .detected_at
            .get_or_insert_with(|| chrono::Utc::now().timestamp_millis());
        store_io::write(&path, &state)?;
    }
    Ok(result)
}
/// Read-only visibility also detects older short recordings before a listener has caught up.
pub fn hidden(root: &Path, folder: &Path, id: &str) -> bool {
    if !valid_id(id) {
        return false;
    }
    match store_io::read::<State>(&state_path(root, id)) {
        Ok(s) if s.kept => false,
        Ok(s) => s.short || classify(folder) == Eligibility::Short,
        Err(_) => true,
    }
}
pub fn sources_visible<'a>(
    root: &Path,
    meetings: &Path,
    sources: impl IntoIterator<Item = &'a str>,
) -> bool {
    static CITATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[\[([0-9]+-[0-9-]+)(?:@|\]\])").unwrap());
    sources.into_iter().all(|source| {
        CITATION
            .captures_iter(source)
            .all(|c| !hidden(root, &meetings.join(&c[1]), &c[1]))
    })
}
#[derive(Clone)]
pub struct ShortRecording {
    pub id: String,
    pub date: String,
    pub length: String,
}
pub fn short_recordings(root: &Path, meetings: &Path) -> io::Result<Vec<ShortRecording>> {
    let mut recordings = Vec::new();
    for entry in fs::read_dir(meetings)? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) || eligibility(root, meetings, &id)? != Eligibility::Short {
            continue;
        }
        let raw = store_io::read_bounded(&entry.path().join("manifest.json"), 1024 * 1024)?;
        let m: serde_json::Value =
            serde_json::from_slice(raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&raw))?;
        let date = m["created_at_ms"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%d %b %Y, %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "Date unknown".into());
        let length = m["duration_ms"]
            .as_f64()
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(|d| format!("{} seconds", (d / 1000.0).floor() as u64))
            .unwrap_or_else(|| "Length unknown".into());
        recordings.push(ShortRecording { id, date, length });
    }
    recordings.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(recordings)
}
/// Persist first, then idempotent queues can safely be retried after a crash.
pub fn keep_in(root: &Path, id: &str) -> io::Result<()> {
    if !valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    let path = state_path(root, id);
    let _lock = store_io::lock(&path.with_extension("lock"))?;
    let mut state: State = store_io::read(&path)?;
    if state.kept {
        return Ok(());
    }
    state.short = false;
    state.kept = true;
    store_io::write(&path, &state)
}
pub fn is_kept(root: &Path, id: &str) -> bool {
    valid_id(id) && store_io::read::<State>(&state_path(root, id)).is_ok_and(|s| s.kept)
}
fn keep_queued(root: &Path, id: &str, enqueue: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    keep_in(root, id)?;
    let path = state_path(root, id);
    let _lock = store_io::lock(&path.with_extension("queue.lock"))?;
    let mut state: State = store_io::read(&path)?;
    if !state.work_queued {
        enqueue()?;
        state.work_queued = true;
        store_io::write(&path, &state)?;
    }
    Ok(())
}
pub fn keep(id: &str) -> io::Result<()> {
    let root = super::state_dir();
    keep_queued(&root, id, || {
        if eligibility(&root, &super::meetings_dir(), id)? != Eligibility::Eligible {
            return Err(io::Error::other("Recording is not ready yet"));
        }
        match super::notes::settings()?.engine() {
            super::notes::Engine::App => super::notes::queue::enqueue(id)?,
            super::notes::Engine::Scripts(_) => crate::listener::pipeline(Some(id))?,
            super::notes::Engine::Off => {}
        }
        if super::meeting_router::jobs::configured() {
            super::meeting_router::jobs::enqueue(id)
                .map_err(|_| io::Error::other("Could not queue routing"))?;
        }
        Ok(())
    })?;
    super::notes::queue::start_worker()?;
    super::meeting_router::jobs::start_worker()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "1793946600000-123";
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rec-guard-{}-{}",
            std::process::id(),
            super::super::device::random_id().unwrap()
        ));
        fs::create_dir_all(root.join("meetings").join(ID)).unwrap();
        root
    }
    fn write(root: &Path, duration: serde_json::Value, text: &str) {
        let folder = root.join("meetings").join(ID);
        fs::write(folder.join("manifest.json"), serde_json::json!({"status":"complete", "duration_ms":duration,"created_at_ms":1793946600000_i64}).to_string()).unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            serde_json::json!({"text":text}).to_string(),
        )
        .unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_id_is_rejected_before_opening_inputs() {
        use std::os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::ffi::OsStrExt,
        };
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(
            outside.join("manifest.json"),
            br#"{"status":"complete","duration_ms":30000}"#,
        )
        .unwrap();
        fs::write(
            outside.join("transcript.ndjson"),
            br#"{"text":"synthetic fixture"}"#,
        )
        .unwrap();
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(raw >= 0);
        let watch = unsafe { OwnedFd::from_raw_fd(raw) };
        let name = std::ffi::CString::new(outside.as_os_str().as_bytes()).unwrap();
        assert!(
            unsafe { libc::inotify_add_watch(watch.as_raw_fd(), name.as_ptr(), libc::IN_OPEN) }
                >= 0
        );
        // An absolute ID would replace the configured root in Path::join.
        assert!(
            eligibility(
                root.path(),
                &root.path().join("meetings"),
                outside.to_str().unwrap()
            )
            .is_err()
        );
        let mut events = [0u8; 4096];
        assert_eq!(
            unsafe { libc::read(watch.as_raw_fd(), events.as_mut_ptr().cast(), events.len()) },
            -1,
            "invalid ID unexpectedly opened filesystem inputs"
        );
        assert_eq!(io::Error::last_os_error().kind(), io::ErrorKind::WouldBlock);
    }
    #[test]
    fn strict_boundaries_and_indic_words() {
        let root = fixture();
        let folder = root.join("meetings").join(ID);
        for word in ["word", "తెలుగు", "हिन्दी"] {
            for (duration, words, expected) in [
                (29999, 25, Eligibility::Short),
                (30000, 24, Eligibility::Short),
                (30000, 25, Eligibility::Eligible),
            ] {
                write(&root, duration.into(), &vec![word; words].join(" "));
                assert_eq!(classify(&folder), expected, "{word} {duration} {words}");
            }
        }
    }
    #[test]
    fn unknown_is_not_zero_and_original_wins() {
        let root = fixture();
        let folder = root.join("meetings").join(ID);
        write(&root, serde_json::Value::Null, "");
        assert_eq!(classify(&folder), Eligibility::Unknown);
        write(&root, 30000.into(), "");
        assert_eq!(classify(&folder), Eligibility::Short);
        fs::write(folder.join("transcript.ndjson"), "").unwrap();
        assert_eq!(classify(&folder), Eligibility::Short);
        fs::write(folder.join("transcript.ndjson"), "{\"text\":").unwrap();
        assert_eq!(classify(&folder), Eligibility::Unknown);
        fs::write(
            folder.join("transcript.ndjson"),
            "{\"speaker\":\"twenty five words do not belong here\"}",
        )
        .unwrap();
        assert_eq!(classify(&folder), Eligibility::Unknown);
        fs::write(
            folder.join("transcript.ndjson"),
            format!(
                "{}\n{{",
                serde_json::json!({"text":vec!["word";25].join(" ")})
            ),
        )
        .unwrap();
        assert_eq!(classify(&folder), Eligibility::Unknown);
        fs::rename(folder.join("transcript.ndjson"), folder.join("unfinished")).unwrap();
        assert_eq!(classify(&folder), Eligibility::Unknown);
        write(&root, 29999.into(), "");
        fs::write(
            folder.join("manifest.json"),
            r#"{"status":"transcribing","duration_ms":29999}"#,
        )
        .unwrap();
        assert_eq!(classify(&folder), Eligibility::Unknown);
        write(&root, 30000.into(), &vec!["word"; 25].join(" "));
        fs::create_dir_all(folder.join("hex-original")).unwrap();
        fs::write(
            folder.join("hex-original/transcript.ndjson"),
            r#"{"text":""}"#,
        )
        .unwrap();
        assert_eq!(classify(&folder), Eligibility::Short);
    }
    #[test]
    fn normal_catalog_excludes_short_from_list_ask_tasks_and_memory_sources() {
        let root = fixture();
        let meetings = root.join("meetings");
        write(&root, 29999.into(), "private synthetic source");
        fs::write(meetings.join(ID).join("english-summary.json"), serde_json::json!({"title":"Hidden synthetic title","todo":[{"task":"Hidden synthetic task"}]}).to_string()).unwrap();
        let (catalog, errors) = super::super::scan_in(&meetings);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(catalog.is_empty());
        let pipeline = super::super::load_pipeline_in(&root, &catalog);
        let projects = super::super::Projects::default();
        let tasks = super::super::build_tasks(
            &catalog,
            &pipeline,
            &projects,
            &Default::default(),
            &Default::default(),
        );
        assert!(tasks.is_empty());
        let context = super::super::ask_all::all_context(
            &catalog,
            &tasks,
            &pipeline,
            &projects,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(),
            10000,
        );
        assert_eq!(context.included, 0);
        assert!(!context.text.contains("Hidden synthetic"));
        let citation = format!("[[{ID}@00:01]]");
        assert!(!sources_visible(&root, &meetings, [citation.as_str()]));
        assert_eq!(short_recordings(&root, &meetings).unwrap().len(), 1);
        assert_eq!(
            eligibility(&root, &meetings, ID).unwrap(),
            Eligibility::Short
        );
        keep_in(&root, ID).unwrap();
        assert!(sources_visible(&root, &meetings, [citation.as_str()]));
    }
    #[test]
    fn keep_receipt_queues_once_and_retries_failed_enqueue() {
        let root = fixture();
        write(&root, 29999.into(), "synthetic");
        let mut calls = 0;
        assert!(
            keep_queued(&root, ID, || {
                calls += 1;
                Err(io::Error::other("synthetic queue failure"))
            })
            .is_err()
        );
        keep_queued(&root, ID, || {
            calls += 1;
            Ok(())
        })
        .unwrap();
        keep_queued(&root, ID, || {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 2);
    }
    #[test]
    fn keep_is_durable_and_idempotent_without_hex_writes() {
        let root = fixture();
        let meetings = root.join("meetings");
        write(&root, 29999.into(), "synthetic");
        let before = fs::read(meetings.join(ID).join("transcript.ndjson")).unwrap();
        assert_eq!(
            eligibility(&root, &meetings, ID).unwrap(),
            Eligibility::Short
        );
        assert!(hidden(&root, &meetings.join(ID), ID));
        keep_in(&root, ID).unwrap();
        let saved = fs::read(state_path(&root, ID)).unwrap();
        keep_in(&root, ID).unwrap();
        assert_eq!(fs::read(state_path(&root, ID)).unwrap(), saved);
        assert_eq!(
            eligibility(&root, &meetings, ID).unwrap(),
            Eligibility::Eligible
        );
        assert!(!hidden(&root, &meetings.join(ID), ID));
        assert!(short_recordings(&root, &meetings).unwrap().is_empty());
        assert_eq!(
            fs::read(meetings.join(ID).join("transcript.ndjson")).unwrap(),
            before
        );
    }
}
