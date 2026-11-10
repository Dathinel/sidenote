use chrono::{DateTime, Local, NaiveDate};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn peers_dir() -> PathBuf {
    if let Some(dir) = super::paths::peers() {
        return dir;
    }
    super::paths::appdata_dir("sidenote-peers", "hex-meetings-peers")
}

pub fn meetings_dir() -> PathBuf {
    if let Some(dir) = super::paths::meetings() {
        return dir;
    }
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("hex/meetings")
}
/// Where reports, ticks and task edits live. `SIDENOTE_STATE_DIR` moves it, so renders and
/// checks that apply actions never touch the real files.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = super::paths::data() {
        return dir;
    }
    let root = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default());
    let current = root.join("sidenote");
    // An empty new folder would change which corpus an existing install opens.
    if current.exists() {
        current
    } else {
        root.join("hex-meetings")
    }
}

/// Both background entry points use the same log without relocating an existing file.
pub fn log_path() -> PathBuf {
    let current = state_dir().join("sidenote.log");
    // An explicit data override must also isolate logs from the owner's installation.
    if super::paths::data().is_some() {
        return current;
    }
    let old = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("hex-meeting-watch.log");
    super::paths::prefer_legacy_path(current, old)
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Summary {
    pub schema: u32,
    pub title: String,
    pub kind: String,
    pub people: Vec<String>,
    pub context: Vec<String>,
    pub topics: Vec<Topic>,
    pub todo: Vec<Todo>,
    pub to_settle: Vec<Unsettled>,
    pub verify: Vec<String>,
}
impl Summary {
    pub(super) fn texts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str())
            .chain(std::iter::once(self.kind.as_str()))
            .chain(self.people.iter().map(String::as_str))
            .chain(self.context.iter().map(String::as_str))
            .chain(self.topics.iter().flat_map(|t| {
                std::iter::once(t.title.as_str()).chain(
                    t.points
                        .iter()
                        .flat_map(|p| std::iter::once(p.text.as_str()).chain(p.who.as_deref())),
                )
            }))
            .chain(self.todo.iter().flat_map(|t| {
                std::iter::once(t.text())
                    .chain(std::iter::once(t.title()))
                    .chain(std::iter::once(t.description()))
                    .chain(t.subtasks.iter().map(|s| s.title.as_str()))
                    .chain(t.waiting_on.iter().filter_map(|w| w.text.as_deref()))
                    .chain(t.team.as_deref())
                    .chain(t.owner.as_deref())
                    .chain(t.due.as_deref())
            }))
            .chain(
                self.to_settle.iter().flat_map(|q| {
                    std::iter::once(q.question.as_str()).chain(q.waits_on.as_deref())
                }),
            )
            .chain(self.verify.iter().map(String::as_str))
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Topic {
    pub title: String,
    pub points: Vec<Point>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Point {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub who: Option<String>,
    pub at: Vec<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Todo {
    pub span: Vec<String>,
    pub task: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub waiting_on: Option<WaitingOn>,
    pub subtasks: Vec<Subtask>,
    pub team: Option<String>,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub due_default: bool,
    pub at: Vec<String>,
}
impl Todo {
    pub fn title(&self) -> &str {
        if self.title.trim().is_empty() {
            super::task_titles::split_title(&self.task).0
        } else {
            &self.title
        }
    }
    pub fn description(&self) -> &str {
        if self.description.trim().is_empty() {
            super::task_titles::split_title(&self.task).1
        } else {
            &self.description
        }
    }
    pub fn text(&self) -> &str {
        if self.task.trim().is_empty() && !self.title.trim().is_empty() {
            &self.title
        } else {
            &self.task
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct WaitingOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Subtask {
    pub id: String,
    pub title: String,
    pub done: bool,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Unsettled {
    pub question: String,
    pub waits_on: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub status: Option<String>,
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub created_at_ms: Option<i64>,
    pub duration_ms: Option<f64>,
    pub export_path: Option<String>,
}
/// quick-title.json, written seconds after Stop by hex-meeting-quick-title.ps1 while the notes are
/// still being made. The notes' own title replaces it once english-summary.json lands.
#[derive(Default, Deserialize)]
#[serde(default)]
struct QuickTitle {
    title: String,
}
/// The file is a few dozen bytes; a larger one is not ours.
const QUICK_TITLE_MAX_BYTES: u64 = 4096;
const QUICK_TITLE_MAX_CHARS: usize = 100;

/// The quick title in a meeting folder, if there is a usable one. A missing, oversized,
/// half-written or malformed file is just no title. Peer folders are only ever read.
pub fn read_quick_title(folder: &Path) -> Option<String> {
    let path = folder.join("quick-title.json");
    if fs::metadata(&path).ok()?.len() > QUICK_TITLE_MAX_BYTES {
        return None;
    }
    let parsed: QuickTitle =
        serde_json::from_str(&super::file_reads::text(&path, QUICK_TITLE_MAX_BYTES).ok()?).ok()?;
    let one_line = parsed
        .title
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title: String = one_line.chars().take(QUICK_TITLE_MAX_CHARS).collect();
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

/// The quick title of the meeting that was just recorded: a folder in `local` made at or after
/// `since_ms` (Unix milliseconds; folder ids start with the creation time) that is not yet a
/// known summarised meeting. When several match, the newest wins.
pub fn pending_quick_title(
    local: &Path,
    since_ms: i64,
    known: &std::collections::HashSet<String>,
) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for entry in super::file_reads::dir_iter(local).ok()?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let Some(made) = id.split('-').next().and_then(|n| n.parse::<i64>().ok()) else {
            continue;
        };
        if made < since_ms || known.contains(&id) || best.as_ref().is_some_and(|(b, _)| *b >= made)
        {
            continue;
        }
        if let Some(title) = read_quick_title(&entry.path()) {
            best = Some((made, title));
        }
    }
    best.map(|(_, title)| title)
}

/// One transcript line. Imported transcripts carry no times: `at` is empty and `seconds` is
/// `None`, and `who` is empty when the source names no speaker.
#[derive(Clone)]
pub struct Line {
    pub at: String,
    pub seconds: Option<u64>,
    pub who: String,
    pub language: String,
    pub text: String,
}
#[derive(Clone)]
pub struct Meeting {
    /// Actual text source. Peer folders are only ever opened for reading.
    pub folder: PathBuf,
    pub device: Option<String>,
    pub id: String,
    pub title: String,
    pub summary: Summary,
    pub summary_raw: String,
    pub transcript_raw: String,
    pub routed: Option<super::meeting_router::Transcript>,
    pub routed_revision: String,
    /// The user's own notes from `notes.md`, typed during the meeting (or imported). Empty when none.
    pub notes: String,
    pub lines: Vec<Line>,
    pub started: Option<DateTime<Local>>,
    pub minutes: Option<u64>,
    pub export_path: Option<String>,
    pub search: String,
}
static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[(\d+):(\d+):(\d+)(?:\.(\d+))?\] (\S+) \(([^)]*)\): ?(.*)$").unwrap()
});
pub fn seconds(stamp: &str) -> Option<u64> {
    let parts: Vec<u64> = stamp
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match parts.as_slice() {
        [m, s] if *s < 60 => m.checked_mul(60)?.checked_add(*s),
        [h, m, s] if *m < 60 && *s < 60 => {
            h.checked_mul(3600)?.checked_add(m * 60)?.checked_add(*s)
        }
        _ => None,
    }
}
pub fn short(stamp: &str) -> &str {
    stamp.strip_prefix("00:").unwrap_or(stamp)
}
/// "Speaker A: text" or "(English): text", the labels an imported transcript can start with.
static UNTIMED_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(Speaker [A-Za-z0-9]+)|\(([^)]{1,30})\)):(?: (.*))?$").unwrap()
});
/// A trailing language tag such as "(Hindi+English)".
static LANGUAGE_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\(([A-Za-z]+(?:\+[A-Za-z]+)*)\)$").unwrap());

pub fn parse_transcript(raw: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for raw in raw.lines() {
        if let Some(c) = LINE.captures(raw.trim()) {
            let at = format!(
                "{:02}:{:02}:{:02}",
                c[1].parse::<u64>().unwrap_or(0),
                c[2].parse::<u64>().unwrap_or(0),
                c[3].parse::<u64>().unwrap_or(0)
            );
            lines.push(Line {
                seconds: Some(seconds(&at).unwrap_or(0)),
                at,
                who: c[5].into(),
                language: c[6].into(),
                text: c[7].trim().into(),
            });
        } else if !raw.trim().is_empty()
            && !raw.starts_with('#')
            && let Some(last) = lines.last_mut()
        {
            last.text.push(' ');
            last.text.push_str(raw.trim());
        }
    }
    if lines.is_empty() {
        return parse_untimed(raw);
    }
    lines
}

/// A transcript with no times. When any line names a speaker, an unlabelled line continues the
/// one before it; when none does, every non-empty line is its own.
fn parse_untimed(raw: &str) -> Vec<Line> {
    let body: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let labelled = body.iter().any(|l| UNTIMED_LABEL.is_match(l));
    let mut lines: Vec<Line> = Vec::new();
    for l in body {
        let (who, mut language, text) = match UNTIMED_LABEL.captures(l) {
            Some(c) => (
                c.get(1).map_or("", |m| m.as_str()).to_string(),
                c.get(2).map_or("", |m| m.as_str()).to_string(),
                c.get(3).map_or("", |m| m.as_str()).trim().to_string(),
            ),
            None if labelled => {
                if let Some(last) = lines.last_mut() {
                    last.text.push(' ');
                    last.text.push_str(l);
                    continue;
                }
                (String::new(), String::new(), l.to_string())
            }
            None => (String::new(), String::new(), l.to_string()),
        };
        let mut text = text;
        if let Some(c) = LANGUAGE_TAIL.captures(&text) {
            language = c[1].to_string();
            text.truncate(c.get(0).map_or(text.len(), |m| m.start()));
        }
        if text.is_empty() {
            continue;
        }
        lines.push(Line {
            at: String::new(),
            seconds: None,
            who,
            language,
            text,
        });
    }
    lines
}
impl Meeting {
    pub fn load(folder: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::load_inner(folder, false)
    }
    pub fn load_pending(folder: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let raw = super::file_reads::text(&folder.join("manifest.json"), 1024 * 1024)?;
        let manifest: Manifest = serde_json::from_str(&raw)?;
        if manifest.status.as_deref() != Some("complete") {
            return Err("Meeting is not complete".into());
        }
        Self::load_inner(folder, true)
    }
    pub fn has_summary(&self) -> bool {
        !self.summary_raw.is_empty()
    }
    fn load_inner(
        folder: &Path,
        pending: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let id = folder
            .file_name()
            .ok_or("Missing meeting id")?
            .to_string_lossy()
            .into_owned();
        let summary_raw = match super::file_reads::text(
            &folder.join("english-summary.json"),
            super::file_reads::SUMMARY_MAX,
        ) {
            Ok(raw) => Some(raw),
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && (pending || folder.join(super::live::NOTES_FILE).is_file()) =>
            {
                None
            }
            Err(e) => return Err(e.into()),
        };
        let summary: Summary = summary_raw
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        let summary_raw = summary_raw.unwrap_or_default();
        let manifest: Manifest =
            match super::file_reads::text(&folder.join("manifest.json"), 1024 * 1024) {
                Ok(raw) => serde_json::from_str(&raw)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => Manifest::default(),
                Err(e) => return Err(e.into()),
            };
        let title = if matches!(
            manifest.title_source.as_deref(),
            Some("call" | "calendar" | "user")
        ) {
            manifest.title.clone().filter(|s| !s.is_empty())
        } else {
            None
        }
        .unwrap_or_else(|| {
            if summary.title.is_empty() {
                read_quick_title(folder)
                    .or_else(|| manifest.title.clone().filter(|t| !t.trim().is_empty()))
                    .unwrap_or_else(|| {
                        manifest
                            .created_at_ms
                            .and_then(DateTime::from_timestamp_millis)
                            .map(|t| {
                                t.with_timezone(&Local)
                                    .format("Meeting on %-d %b at %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_else(|| id.clone())
                    })
            } else {
                summary.title.clone()
            }
        });
        let transcript_raw = match super::file_reads::text(
            &folder.join("english-transcript.md"),
            super::live::ROLLING_MAX_BYTES,
        ) {
            Ok(raw) => raw,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let lines = parse_transcript(&transcript_raw);
        let routed: Option<super::meeting_router::Transcript> = if folder
            .parent()
            .is_some_and(|p| p == meetings_dir())
        {
            let path = state_dir()
                .join("local/transcripts")
                .join(&id)
                .join("transcript.segments.v1.json");
            fs::metadata(&path)
                .ok()
                .filter(|m| m.len() <= 24 * 1024 * 1024)
                .and_then(|_| super::file_reads::read(&path, super::live::ROLLING_MAX_BYTES).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        } else {
            None
        };
        let routed_revision = routed.as_ref().map(|r| r.revision()).unwrap_or_default();
        let mut minutes = manifest
            .duration_ms
            .filter(|n| *n > 0.0)
            .map(|n| (n / 60000.0).round_ties_even() as u64);
        if let Some(secs) = lines.iter().rev().find_map(|l| l.seconds) {
            let spoken = (secs as f64 / 60.0).round_ties_even() as u64 + 1;
            if minutes.is_none_or(|m| m > spoken + 30) {
                minutes = Some(spoken);
            }
        }
        let started = manifest
            .created_at_ms
            .filter(|n| *n != 0)
            .and_then(DateTime::from_timestamp_millis)
            .map(|t| t.with_timezone(&Local));
        let notes = super::live::read_notes(folder);
        let search = format!(
            "{title}\n{}\n{notes}\n{transcript_raw}",
            summary.texts().collect::<Vec<_>>().join("\n")
        )
        .to_lowercase();
        Ok(Self {
            folder: folder.to_owned(),
            device: None,
            id,
            title,
            summary,
            summary_raw,
            transcript_raw,
            routed,
            routed_revision,
            notes,
            lines,
            started,
            minutes,
            export_path: manifest.export_path,
            search,
        })
    }
    pub fn points(&self) -> impl Iterator<Item = &Point> {
        self.summary.topics.iter().flat_map(|t| &t.points)
    }
    pub fn target(&self, stamp: &str) -> Option<usize> {
        let target = seconds(stamp)?.saturating_add(1);
        self.lines
            .iter()
            .rposition(|l| l.seconds.is_some_and(|s| s <= target))
    }
    /// Whether the transcript lines carry times. Imported meetings have text only.
    pub fn timed(&self) -> bool {
        self.lines.iter().any(|l| l.seconds.is_some())
    }
    pub fn day_label(&self) -> String {
        super::day_label(
            self.started.map(|s| s.date_naive()),
            Local::now().date_naive(),
        )
    }
    pub fn snippet(&self, query: &str) -> String {
        self.search_hit(query)
            .map(|hit| hit.snippet.text)
            .unwrap_or_default()
    }
}
pub fn scan() -> (Vec<Meeting>, Vec<String>) {
    let (meetings, mut errors) = scan_sources(&meetings_dir(), &peers_dir());
    if !super::file_reads::refreshing() {
        errors.extend(super::sync_state::reconcile(&state_dir()));
    }
    (meetings, errors)
}

/// Local ids reserve their place even while a summary is arriving or cannot be read.
pub fn scan_sources(local: &Path, peers: &Path) -> (Vec<Meeting>, Vec<String>) {
    let (mut meetings, mut errors) = scan_in(local);
    let mut ids: std::collections::BTreeSet<String> =
        meetings.iter().map(|m| m.id.clone()).collect();
    if let Ok(entries) = super::file_reads::dir_iter(local) {
        for entry in entries.take(super::file_reads::ENTRIES_MAX).flatten() {
            if super::file_reads::directory(&entry.path()) {
                ids.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    match super::file_reads::dir_iter(peers) {
        Ok(entries) => {
            let mut devices = Vec::new();
            for entry in entries.take(super::file_reads::ENTRIES_MAX) {
                match entry {
                    Ok(e) if super::file_reads::directory(&e.path()) => devices.push(e.path()),
                    Ok(_) => {}
                    Err(_) => errors.push("Could not read a peer device folder".into()),
                }
            }
            devices.sort();
            for folder in devices {
                let device = folder
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let (peer_meetings, peer_errors) = scan_in_mode(&folder, false);
                errors.extend(
                    peer_errors
                        .into_iter()
                        .map(|e| format!("from {device}: {e}")),
                );
                for mut meeting in peer_meetings {
                    if ids.insert(meeting.id.clone()) {
                        meeting.device = Some(device.clone());
                        meetings.push(meeting);
                    }
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => errors.push("Could not read peer meetings".into()),
    }
    meetings.sort_by(|a, b| b.started.cmp(&a.started).then(b.id.cmp(&a.id)));
    (meetings, errors)
}
/// Returns saved and complete pending local meetings, newest first, plus read errors.
pub fn scan_in(root: &Path) -> (Vec<Meeting>, Vec<String>) {
    scan_in_mode(root, true)
}
fn scan_in_mode(root: &Path, pending: bool) -> (Vec<Meeting>, Vec<String>) {
    let mut meetings = Vec::new();
    let mut errors = Vec::new();
    match super::file_reads::dir_iter(root) {
        Ok(entries) => {
            for entry in entries.take(super::file_reads::ENTRIES_MAX) {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        errors.push("Could not read a meeting folder entry".into());
                        continue;
                    }
                };
                let folder = entry.path();
                if !super::file_reads::directory(&folder) {
                    continue;
                }
                let saved = folder.join("english-summary.json").exists()
                    || folder.join(super::live::NOTES_FILE).is_file();
                let loaded = if saved {
                    Meeting::load(&folder)
                } else if pending {
                    let manifest =
                        super::file_reads::text(&folder.join("manifest.json"), 1024 * 1024);
                    match manifest {
                        Ok(raw) => match serde_json::from_str::<Manifest>(&raw) {
                            Ok(m) if m.status.as_deref() == Some("complete") => {
                                Meeting::load_pending(&folder)
                            }
                            Ok(_) => continue,
                            Err(e) => Err(e.into()),
                        },
                        Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                        Err(e) => Err(e.into()),
                    }
                } else {
                    continue;
                };
                match loaded {
                    Ok(m) => meetings.push(m),
                    Err(_) => {
                        super::file_reads::not_refreshed(&folder);
                        errors.push(format!(
                            "Could not read meeting {}",
                            entry.file_name().to_string_lossy()
                        ));
                    }
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => errors.push("Could not read the meetings folder".into()),
    }
    meetings.sort_by(|a, b| b.started.cmp(&a.started).then(b.id.cmp(&a.id)));
    (meetings, errors)
}
pub fn nice_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_else(|_| date.into())
}

#[cfg(test)]
mod quick_title_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn old_and_v2_task_fields_remain_readable() {
        let old: Todo = serde_json::from_str(r#"{"task":"Send the plan"}"#).unwrap();
        assert_eq!(old.task, "Send the plan");
        assert!(old.id.is_empty());
        assert!(old.subtasks.is_empty());
        let raw = r#"{"task":"Send the plan","id":"stable-id","title":"Send the plan","description":"Include the agreed date.","status":"waiting","waiting_on":{"task_id":"blocker"},"team":"Design","subtasks":[{"id":"child","title":"Check date","done":true}],"span":["00:00:01","00:00:10"]}"#;
        let task: Todo = serde_json::from_str(raw).unwrap();
        assert_eq!(task.id, "stable-id");
        assert_eq!(
            task.waiting_on.as_ref().unwrap().task_id.as_deref(),
            Some("blocker")
        );
        assert!(task.subtasks[0].done);
        let roundtrip: Todo = serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();
        assert_eq!(roundtrip.description, task.description);
        assert_eq!(roundtrip.team.as_deref(), Some("Design"));
        assert_eq!(roundtrip.span, task.span);
    }

    #[test]
    fn personal_notes_without_a_summary_appear_in_saved_meetings() {
        let root = std::env::temp_dir().join(format!("notes-only-{}", std::process::id()));
        let folder = root.join("123456789-1");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("notes.md"), "Keep this thought").unwrap();
        let (meetings, errors) = scan_in(&root);
        assert!(errors.is_empty());
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].notes, "Keep this thought");
        assert!(meetings[0].summary.title.is_empty());
        fs::write(folder.join("english-summary.json"), "invalid").unwrap();
        let (meetings, errors) = scan_in(&root);
        assert!(meetings.is_empty());
        assert_eq!(errors.len(), 1);
        fs::write(folder.join("english-summary.json"), "").unwrap();
        let (meetings, errors) = scan_in(&root);
        assert!(meetings.is_empty());
        assert_eq!(errors.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quick-title-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn complete_local_meetings_without_notes_are_listed_but_peers_are_not() {
        let root = folder("pending-list");
        let local = root.join("local");
        let peers = root.join("peers/device");
        for (base, id, status) in [
            (&local, "1000-1", "complete"),
            (&local, "2000-2", "recording"),
            (&peers, "3000-3", "complete"),
        ] {
            let d = base.join(id);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("manifest.json"),
                format!(r#"{{"status":"{status}","created_at_ms":1000,"title":"Planning"}}"#),
            )
            .unwrap();
            fs::write(
                d.join("english-transcript.md"),
                "[00:00:01] You (English): Plan the launch.",
            )
            .unwrap();
            fs::write(d.join("quick-title.json"), r#"{"title":"Launch plan"}"#).unwrap();
        }
        let (meetings, errors) = scan_sources(&local, &root.join("peers"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(meetings.len(), 1);
        let m = &meetings[0];
        assert_eq!(m.id, "1000-1");
        assert_eq!(m.title, "Launch plan");
        assert!(!m.has_summary());
        assert_eq!(m.lines.len(), 1);
        let known = meetings
            .iter()
            .filter(|m| m.has_summary())
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(
            pending_quick_title(&local, 0, &known).as_deref(),
            Some("Launch plan")
        );
        assert!(Meeting::load_pending(&local.join("2000-2")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_and_cleans_a_title() {
        let dir = folder("clean");
        let file = dir.join("quick-title.json");
        fs::write(
            &file,
            r#"{"title":"  Website  launch\nand pre-orders ","model":"m","created":"c"}"#,
        )
        .unwrap();
        assert_eq!(
            read_quick_title(&dir).as_deref(),
            Some("Website launch and pre-orders")
        );
        fs::write(&file, r#"{"title":"   "}"#).unwrap();
        assert_eq!(read_quick_title(&dir), None);
        fs::write(&file, "{not json").unwrap();
        assert_eq!(read_quick_title(&dir), None);
        fs::write(&file, format!(r#"{{"title":"{}"}}"#, "x".repeat(9000))).unwrap();
        assert_eq!(read_quick_title(&dir), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_the_newest_new_meeting() {
        let root = folder("pending");
        for (id, title) in [("1000-1", "Old"), ("2000-2", "Newer"), ("3000-3", "Known")] {
            let d = root.join(id);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("quick-title.json"),
                format!(r#"{{"title":"{title}"}}"#),
            )
            .unwrap();
        }
        let known: HashSet<String> = ["3000-3".to_string()].into();
        assert_eq!(
            pending_quick_title(&root, 1500, &known).as_deref(),
            Some("Newer")
        );
        assert_eq!(pending_quick_title(&root, 2500, &known), None);
        let _ = fs::remove_dir_all(root);
    }
}
