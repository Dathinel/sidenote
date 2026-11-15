//! "New meeting": the live page. Starting a meeting starts a Hex recording and opens a page with
//! a heading ("Meeting at 14:30"), a recording mark, the elapsed time and a large Stop button.
//! After Stop it says "Writing summary" until the normal pipeline has written the meeting, then
//! opens the notes.
//!
//! While recording, the page is a calm notes editor (plain text, saved to the meeting folder's
//! `notes.md` about 2 s after typing stops and again on Stop). A bottom bar holds a recording
//! pill (dot, voice bars, timer, chevron, Stop) next to the Ask pill; the chevron opens a panel
//! above it with the live transcript, which Hex writes to `transcript.rolling.ndjson` while it
//! records (see `core::live`). A background thread reads that file every 2.5 s, only while this
//! page is visible, and skips the parse when the file has not changed.
//!
//! The voice bars are not a level meter. Hex exposes no microphone level, so the bars follow
//! transcript activity: they lift while new words keep arriving and rest in a slow, gentle idle
//! otherwise.
//!
//! After Stop it says "Writing summary" (with the user's notes beside it) until the normal pipeline
//! has written the meeting, then opens it. Nobody types a title: a cheap model names the meeting
//! seconds after Stop (quick-title.json, read here while the notes are made), and the notes' own
//! title replaces it.
use super::{
    State,
    ask::{self, EASE_IN, raised, rise},
    hex,
    list::text_title,
    roles,
    theme::Palette,
    widgets::{dot, hairline, label, link, pressable, quiet_button, scroll, spaced_caps},
};
use chrono::{DateTime, Local, TimeZone};
use rune::{El, ScrollSpec, TextEdit, col, el, row};
use rune_kit::Kit;
use sidenote::core::live::{self as feed_io, LiveLine, Read};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

type Work = Box<dyn FnOnce() + Send>;
pub(super) struct NoteWriter {
    tx: std::sync::mpsc::Sender<Work>,
    thread: thread::JoinHandle<()>,
}
impl NoteWriter {
    fn new() -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<Work>();
        let thread = thread::Builder::new()
            .name("live-notes".into())
            .spawn(move || {
                for work in rx {
                    work();
                }
            })?;
        Ok(Self { tx, thread })
    }
    fn finish(self) {
        drop(self.tx);
        let _ = self.thread.join();
    }
}
type SaveResult = (String, Result<Option<PathBuf>, String>);

/// How long a start may take before the page gives up waiting for Hex to say it is recording.
const START_WAIT: Duration = Duration::from_secs(25);
/// After this long without a new meeting, the page says the notes are slow.
const MAKING_SLOW: Duration = Duration::from_secs(10 * 60);
/// How often the new meeting's folder is checked for its quick title after Stop.
const TITLE_EVERY: Duration = Duration::from_secs(2);
/// How often the live transcript file is read while the page is visible.
const FEED_EVERY: Duration = Duration::from_millis(2500);
/// Notes are saved this long after the last key.
const SAVE_AFTER: Duration = Duration::from_secs(2);
/// The bars count as busy for this long after words last arrived.
const ACTIVE_FOR: Duration = Duration::from_secs(6);
/// With no live file after this long, the panel says so instead of only "Listening".
const NO_FEED_AFTER: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Hex was asked to record and has not said so yet.
    Starting,
    Recording,
    /// Stopped; waiting for the pipeline to write the meeting.
    Making,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transcript {
    Finishing,
    Interrupted,
    Ready,
}

impl Transcript {
    fn heading(self, slow: bool) -> &'static str {
        match self {
            Self::Finishing => "Hex is finishing the transcript",
            Self::Interrupted => "Transcript waiting for Hex",
            Self::Ready if slow => "The summary is taking a while",
            Self::Ready => "Writing the summary",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::Finishing => {
                "A summary can be written once Hex finishes the transcript. You can leave this page."
            }
            Self::Interrupted => {
                "Hex was restarted. It will finish this transcript when it starts again."
            }
            Self::Ready => "The summary opens here when it is ready. You can leave this page.",
        }
    }
}

fn read_transcript_state(folder: &Path) -> Option<Transcript> {
    let raw = sidenote::core::file_reads::read(&folder.join("manifest.json"), 1024 * 1024).ok()?;
    let manifest: sidenote::core::Manifest = serde_json::from_slice(&raw).ok()?;
    match manifest.status.as_deref() {
        Some("recording" | "starting" | "transcribing") => Some(Transcript::Finishing),
        Some("interrupted") if folder.join("final.pending").is_file() => {
            Some(Transcript::Interrupted)
        }
        Some("complete") => Some(Transcript::Ready),
        _ => None,
    }
}

fn feed_summary_state(
    feed: &mut Feed,
    folder: &Path,
    root: &Path,
    off: bool,
    job: Option<&sidenote::core::notes::JobState>,
) -> bool {
    use sidenote::core::{notes, recording_guard};
    let transcript = read_transcript_state(folder);
    let skipped = (transcript == Some(Transcript::Ready))
        .then(|| folder.file_name()?.to_str())
        .flatten()
        .and_then(|id| notes::summary_skip(recording_guard::receipt(root, id), off, job));
    let changed = feed.transcript != transcript || feed.summary_skip != skipped;
    feed.transcript = transcript;
    feed.summary_skip = skipped;
    changed
}

fn making_transcript(s: &State) -> Transcript {
    s.live
        .as_ref()
        .and_then(|live| {
            live.feed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .transcript
        })
        .unwrap_or_else(|| {
            if s.hex
                .status
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|st| st.meeting_state == "transcribing")
            {
                Transcript::Finishing
            } else {
                Transcript::Ready
            }
        })
}

pub(super) fn summary_skip(s: &State) -> Option<sidenote::core::notes::SummarySkip> {
    s.live
        .as_ref()?
        .feed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .summary_skip
}

pub(super) fn scene_making(s: &mut State, status: &str) {
    hex::scene(s, false);
    let mut live = Live::scene(Phase::Making, Some("Weekly planning"));
    live.feed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transcript = Some(match status {
        "transcribing" => Transcript::Finishing,
        "interrupted" => Transcript::Interrupted,
        _ => Transcript::Ready,
    });
    if status == "short" {
        live.feed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .summary_skip = sidenote::core::notes::summary_skip(
            sidenote::core::recording_guard::Eligibility::Short,
            false,
            None,
        );
    }
    if status == "slow" {
        live.stopped = Some(Instant::now() - Duration::from_secs(49 * 60));
    }
    s.ai_notes.settings = Some(sidenote::core::notes::NotesSettings {
        engine: "scripts".into(),
        ..Default::default()
    });
    if let Some(meeting) = s.meetings.iter_mut().find(|m| m.device.is_none()) {
        meeting.summary_raw.clear();
        live.folder = Some(meeting.folder.clone());
    }
    s.live = Some(live);
    s.live_open = true;
    s.live_notes = TextEdit::new("Check the launch date with the team.");
}

/// What the reader thread knows about the live transcript. The page reads it; only the thread
/// (and the page's recording hints) writes it.
#[derive(Default)]
pub struct Feed {
    /// The id Hex reports for the running recording, when it reports one.
    hint: Option<String>,
    /// The start inferred from Hex's status, never the page's local startup time.
    start_ms: Option<i64>,
    /// The meeting folder, once found.
    folder: Option<PathBuf>,
    pub lines: Vec<LiveLine>,
    /// A plain sentence when the file could not be read.
    pub error: Option<String>,
    /// Words seen so far, and when that number last grew. The voice bars follow this.
    words: usize,
    last_words: Option<Instant>,
    stamp: Option<feed_io::Stamp>,
    /// Counts every change, so the page knows to draw again.
    version: u64,
    summary_skip: Option<sidenote::core::notes::SummarySkip>,
    notes_problem: Option<&'static str>,
    transcript: Option<Transcript>,
}

impl Feed {
    fn busy(&self) -> bool {
        self.last_words.is_some_and(|t| t.elapsed() < ACTIVE_FOR)
    }
}

/// Where the notes stand, for the quiet line beside the heading.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Saving {
    /// Nothing typed, or nothing new since the last save.
    Quiet,
    Pending,
    Saved,
    Failed(String),
}

static NEXT_RECORDING_ID: AtomicU64 = AtomicU64::new(1);

pub struct Live {
    // Session-local recording identity; never included in timing records.
    pub(super) recording_id: u64,
    pub(super) notes_response_time: u64,
    pub phase: Phase,
    /// The meetings that existed when this one began, so the new one can be told apart.
    known: HashSet<String>,
    began: Instant,
    stopped: Option<Instant>,
    /// When the meeting began, for the heading. `exact` is false until Hex gives the real start.
    wall_start: DateTime<Local>,
    exact: bool,
    /// The quick title, once its file exists. The title thread fills it in after Stop.
    quick_title: Arc<Mutex<Option<String>>>,
    /// Ends the scanning threads when the page lets go of this meeting.
    cancel: Arc<AtomicBool>,
    /// The live transcript, filled by the reader thread. `watching` tells that thread whether
    /// anyone can see the result: it reads only while the page is shown.
    pub feed: Arc<Mutex<Feed>>,
    watching: Arc<AtomicBool>,
    shown_version: u64,
    /// The transcript panel above the bar.
    pub panel_open: bool,
    /// The meeting folder the notes are saved into, once Hex has made it.
    folder: Option<PathBuf>,
    notes_loaded: bool,
    /// The editor's text at the last look, and the text on disk. They differ while typing.
    seen: String,
    saved: String,
    recovered: Option<String>,
    edited_at: Option<Instant>,
    pub saving: Saving,
    timer_armed: Arc<AtomicBool>,
    /// A render scene: nothing is read, and the bars hold still.
    scene: bool,
    pub close_failed: bool,
    saves: Arc<Mutex<Vec<SaveResult>>>,
    queued: Option<String>,
}

impl Drop for Live {
    fn drop(&mut self) {
        super::response_time::superseded(std::mem::take(&mut self.notes_response_time));
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Live {
    pub(super) fn take_notes_timing(&mut self, recording: u64) -> u64 {
        if self.recording_id == recording {
            std::mem::take(&mut self.notes_response_time)
        } else {
            0
        }
    }

    fn can_close(&self, text: &str) -> bool {
        self.queued.is_none() && (text == self.saved || self.recovered.as_deref() == Some(text))
    }

    fn needs_save(&self, text: &str) -> bool {
        text != self.saved || self.queued.as_ref().is_some_and(|queued| queued != text)
    }

    fn new(phase: Phase, known: HashSet<String>) -> Self {
        Self {
            recording_id: NEXT_RECORDING_ID.fetch_add(1, Ordering::Relaxed),
            notes_response_time: 0,
            phase,
            known,
            began: Instant::now(),
            stopped: (phase == Phase::Making).then(Instant::now),
            wall_start: Local::now(),
            exact: false,
            quick_title: Arc::new(Mutex::new(None)),
            cancel: Arc::new(AtomicBool::new(false)),
            feed: Arc::new(Mutex::new(Feed::default())),
            watching: Arc::new(AtomicBool::new(false)),
            shown_version: 0,
            panel_open: false,
            folder: None,
            notes_loaded: false,
            seen: String::new(),
            saved: String::new(),
            recovered: None,
            edited_at: None,
            saving: Saving::Quiet,
            timer_armed: Arc::new(AtomicBool::new(false)),
            scene: false,
            close_failed: false,
            saves: Arc::new(Mutex::new(Vec::new())),
            queued: None,
        }
    }

    /// A stand-in for a render scene: no thread, and nothing waits on Hex. The start time is
    /// fixed so a render does not change with the clock, and `title` is the quick title if the
    /// scene has one.
    pub fn scene(phase: Phase, title: Option<&str>) -> Self {
        let mut live = Self::new(phase, HashSet::new());
        live.wall_start = Local
            .with_ymd_and_hms(2026, 11, 8, 14, 30, 0)
            .single()
            .unwrap_or(live.wall_start);
        live.exact = true;
        live.scene = true;
        *live.quick_title.lock().unwrap_or_else(|e| e.into_inner()) = title.map(str::to_string);
        live
    }

    fn matches_meeting(&self, meeting: &sidenote::core::Meeting) -> bool {
        if meeting.device.is_some() {
            return false;
        }
        match self
            .folder
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|id| id.to_str())
        {
            Some(id) => meeting.id == id,
            None => meeting
                .started
                .map(|t| t.timestamp_millis())
                .or_else(|| meeting.id.split_once('-')?.0.parse::<i64>().ok())
                .is_some_and(|made| {
                    made >= self.wall_start.timestamp_millis().saturating_sub(120_000)
                }),
        }
    }

    pub fn scene_pending(mut self, id: &str) -> Self {
        self.folder = Some(PathBuf::from(id));
        self
    }

    /// A scene with the transcript panel and lines shown. `lines` are `(seconds, you, text)`.
    pub fn scene_feed(mut self, lines: &[(u64, bool, &str)], panel_open: bool) -> Self {
        {
            let mut feed = self.feed.lock().unwrap_or_else(|e| e.into_inner());
            feed.lines = lines
                .iter()
                .map(|(seconds, you, text)| LiveLine {
                    seconds: *seconds,
                    you: *you,
                    text: (*text).to_string(),
                })
                .collect();
            feed.words = feed.lines.iter().map(|l| l.text.split(' ').count()).sum();
            feed.last_words = (!lines.is_empty()).then(Instant::now);
        }
        self.panel_open = panel_open;
        self
    }

    /// "Meeting at 14:30": the heading until the meeting has a real title.
    fn heading(&self) -> String {
        format!("Meeting at {}", self.wall_start.format("%H:%M"))
    }

    /// "14:30", when the meeting began.
    pub(super) fn clock(&self) -> String {
        self.wall_start.format("%H:%M").to_string()
    }

    pub(super) fn quick_title(&self) -> Option<String> {
        self.quick_title
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl State {
    pub(super) fn live_work(&mut self, work: impl FnOnce() + Send + 'static) -> bool {
        if self.live_writer.is_none() {
            self.live_writer = NoteWriter::new().ok();
        }
        self.live_writer
            .as_ref()
            .is_some_and(|writer| writer.tx.send(Box::new(work)).is_ok())
    }

    fn live_saved(&mut self) -> bool {
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        let results = std::mem::take(&mut *live.saves.lock().unwrap_or_else(|e| e.into_inner()));
        let changed = !results.is_empty();
        for (text, result) in results {
            if live.queued.as_ref() == Some(&text) {
                live.queued = None;
            }
            match result {
                Ok(Some(folder)) => {
                    live.recovered = None;
                    live.folder = Some(folder);
                    live.saved = text;
                    if self.live_notes.text == live.saved {
                        live.edited_at = None;
                        live.saving = Saving::Saved;
                    }
                }
                Ok(None) => {
                    live.recovered = Some(text);
                    live.edited_at = Some(Instant::now());
                    live.saving = Saving::Failed(
                        "Notes are in a recovery copy. Trying the meeting folder again.".into(),
                    );
                }
                Err(error) => {
                    live.edited_at = Some(Instant::now());
                    live.saving = Saving::Failed(error);
                }
            }
        }
        changed
    }

    pub fn live_close(&mut self) -> bool {
        self.live_flush();
        if let Some(writer) = self.live_writer.take() {
            writer.finish();
        }
        self.live_saved();
        let safe = self
            .live
            .as_ref()
            .is_none_or(|live| live.can_close(&self.live_notes.text));
        if !safe {
            if let Some(live) = &mut self.live {
                live.close_failed = true;
            }
            self.go_home();
            self.live_open = true;
        }
        safe
    }

    pub fn on_live(&self) -> bool {
        self.live_open
            && self.live.is_some()
            && self.view == super::tasks::View::Meetings
            && self.selected.is_none()
            && !self.check_open
            && !self.memory_open
            && !self.hex_open
    }

    fn known_ids(&self) -> HashSet<String> {
        self.meetings
            .iter()
            .filter(|m| m.has_summary())
            .map(|m| m.id.clone())
            .collect()
    }

    /// Starts a live page with empty notes and, while a recording runs, the reader thread.
    fn live_begin(&mut self, phase: Phase, known: HashSet<String>) {
        let live = Live::new(phase, known);
        self.live_notes = TextEdit::new("");
        if let Some(waker) = self.waker.clone() {
            let (feed, cancel, watching) = (
                live.feed.clone(),
                live.cancel.clone(),
                live.watching.clone(),
            );
            let _ = thread::Builder::new()
                .name("live-feed".into())
                .spawn(move || feed_loop(&feed, &cancel, &watching, &waker));
        }
        self.live = Some(live);
    }

    /// The New meeting button: start Hex recording and show the live page.
    pub fn new_meeting(&mut self) {
        if hex::start_blocked(self).is_some() {
            return;
        }
        self.go_home();
        self.live_begin(Phase::Starting, self.known_ids());
        self.live_open = true;
        self.hex_record_start();
        self.nav_done();
    }

    /// Back to the live page: from the top-right pill, or the sidebar's status line.
    pub fn open_live(&mut self) {
        if self.live.is_none() {
            if !self.hex_recording() {
                return;
            }
            self.live_begin(Phase::Recording, self.known_ids());
            self.live_sync_start();
        }
        self.go_home();
        self.live_open = true;
        self.nav_done();
    }

    /// Takes the heading's start time from the running recording, until Hex gives the real one.
    fn live_sync_start(&mut self) {
        let start = hex::recording_start(self);
        if let (Some(live), Some((at, exact))) = (self.live.as_mut(), start)
            && !live.exact
            && live.phase != Phase::Making
        {
            live.wall_start = at;
            live.exact = exact;
        }
    }

    /// The sidebar's Hex line: the live page while recording, else the Hex settings.
    pub fn hex_line_press(&mut self) {
        if self.hex_recording() {
            self.open_live();
        } else {
            self.open_hex();
            self.nav_done();
        }
    }

    /// Leaves the live page for the meetings list, keeping a recording going.
    pub fn leave_live(&mut self) {
        if self.live_close() {
            self.go_meetings();
        }
    }

    /// Called after each Hex sample and each scan: moves the page along its three phases.
    pub fn live_sync(&mut self) {
        let recording = self.hex_recording();
        let running = self.hex_running();
        let busy = self.hex_record_busy();
        // A recording begun in Hex itself gets the same page and the same notes afterwards.
        if self.live.is_none() {
            if recording {
                self.live_begin(Phase::Recording, self.known_ids());
                self.live_sync_start();
            }
            return;
        }
        self.live_sync_start();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.close_failed {
            return;
        }
        match live.phase {
            Phase::Starting if recording => live.phase = Phase::Recording,
            Phase::Starting if running && !busy && live.began.elapsed() > START_WAIT => {
                self.live = None;
                self.live_open = false;
            }
            Phase::Recording if running && !recording && !busy => self.live_stopped(),
            Phase::Making if recording => {
                // Another recording began while the notes were still being made.
                let known = self.known_ids();
                self.live_begin(Phase::Recording, known);
                self.live_sync_start();
            }
            _ => {}
        }
    }

    /// Recording ended, by Stop here or in Hex: wait for the notes.
    pub fn live_stopped(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.phase == Phase::Making {
            return;
        }
        self.live_flush();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.phase = Phase::Making;
        live.stopped = Some(Instant::now());
        live.panel_open = false;
        live.watching.store(false, Ordering::Relaxed);
        let title_cancel = live.cancel.clone();
        let quick = live.quick_title.clone();
        // Folder ids start with their creation time; allow for a clock a little behind the start.
        let since_ms = live.wall_start.timestamp_millis().saturating_sub(120_000);
        let known = live.known.clone();
        let waker = self.waker.clone();
        self.request_refresh();
        // The quick title lands a few seconds after Stop, well before the notes, so look often.
        let _ = thread::Builder::new()
            .name("live-title".into())
            .spawn(move || {
                let root = sidenote::core::meetings_dir();
                for _ in 0..(MAKING_SLOW.as_secs() / TITLE_EVERY.as_secs()) {
                    if title_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(title) =
                        sidenote::core::pending_quick_title(&root, since_ms, &known)
                    {
                        *quick.lock().unwrap_or_else(|e| e.into_inner()) = Some(title);
                        if let Some(w) = &waker {
                            w();
                        }
                        break;
                    }
                    thread::sleep(TITLE_EVERY);
                }
            });
        // The normal refresh keeps calling live_scanned, even when notes take hours.
    }

    /// After a scan: the first new local meeting while notes are being made is the one.
    pub fn live_scanned(&mut self) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        if live.phase != Phase::Making || live.close_failed {
            return;
        }
        let found = self
            .meetings
            .iter()
            .find(|m| live.matches_meeting(m) && !live.known.contains(&m.id) && m.has_summary())
            .map(|m| (m.id.clone(), m.folder.clone()));
        if let Some((id, folder)) = found {
            let timing_matched = live.folder.as_ref() == Some(&folder);
            if !timing_matched && let Some(live) = &mut self.live {
                super::response_time::unmatched(std::mem::take(&mut live.notes_response_time));
            }
            let was_open = self.on_live();
            if let Some(live) = &mut self.live {
                live.folder = Some(folder.clone());
            }
            if !self.live_close() {
                return;
            }
            if let Ok(fresh) = sidenote::core::Meeting::load(&folder)
                && let Some(meeting) = self.meetings.iter_mut().find(|m| m.id == id)
            {
                *meeting = fresh;
                self.refresh_list();
            }
            let timing = self
                .live
                .as_mut()
                .map_or(0, |live| std::mem::take(&mut live.notes_response_time));
            self.live = None;
            self.live_open = false;
            // Opening a meeting needs the UI's scroll handle, so `poll` finishes the job.
            if timing_matched {
                super::response_time::ready(timing);
            }
            if was_open {
                self.live_ready = Some(id);
            }
        }
    }
}

// ---- the live transcript and the notes -------------------------------------------------------

/// Reads while the page is shown or a stopped meeting still needs a summary outcome.
fn feed_loop(feed: &Mutex<Feed>, cancel: &AtomicBool, watching: &AtomicBool, waker: &rune::Waker) {
    let root = sidenote::core::meetings_dir();
    let mut wait = Duration::from_millis(600);
    while !cancel.load(Ordering::Relaxed) {
        thread::sleep(wait);
        wait = FEED_EVERY;
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !watching.load(Ordering::Relaxed) {
            continue;
        }
        let (mut folder, hint, start_ms, last) = {
            let f = feed.lock().unwrap_or_else(|e| e.into_inner());
            (f.folder.clone(), f.hint.clone(), f.start_ms, f.stamp)
        };
        if folder.is_none() {
            folder = feed_io::find_folder(&root, hint.as_deref(), start_ms);
        }
        let Some(folder) = folder else {
            continue;
        };
        let read = feed_io::read_rolling(&folder, last);
        let notes_problem = feed_io::notes_problem(&folder);
        let job = folder
            .file_name()
            .and_then(|id| id.to_str())
            .and_then(sidenote::core::notes::status);
        let mut f = feed.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = feed_summary_state(
            &mut f,
            &folder,
            &sidenote::core::state_dir(),
            sidenote::core::notes::scripts_policy() == "off",
            job.as_ref(),
        );
        changed |= f.folder.as_ref() != Some(&folder) || f.notes_problem != notes_problem;
        f.notes_problem = notes_problem;
        f.folder = Some(folder);
        match read {
            Ok(Read::Waiting | Read::Same) => {
                if f.error.take().is_some() {
                    changed = true;
                }
            }
            Ok(Read::Lines(lines, stamp)) => {
                let words = lines.iter().map(|l| l.text.split(' ').count()).sum();
                if words > f.words {
                    f.last_words = Some(Instant::now());
                }
                f.words = words;
                f.lines = lines;
                f.stamp = Some(stamp);
                f.error = None;
                changed = true;
            }
            Err(e) => {
                let message = format!("Could not read the live transcript ({})", e.kind());
                if f.error.as_deref() != Some(message.as_str()) {
                    f.error = Some(message);
                    changed = true;
                }
            }
        }
        if changed {
            f.version += 1;
            drop(f);
            waker();
        }
    }
}

const BARS: usize = 5;

/// The bar heights, in points. Busy: a livelier pattern that shifts with each new word count.
/// Idle: a slow, small swell that moves with the clock. Neither is a level; see the module note.
fn bar_heights(busy: bool, tick: u64, words: usize) -> [f32; BARS] {
    let mut h = [0.0; BARS];
    for (i, h) in h.iter_mut().enumerate() {
        *h = if busy {
            8.0 + ((words * 7 + i * 5) % 11) as f32
        } else {
            5.0 + ((tick as usize + i * 2) % 5) as f32 * 0.9
        };
    }
    h
}

impl State {
    /// Called on every poll. Hands the reader thread its hint and whether to read, adopts the
    /// meeting folder, and saves the notes 2 s after the last key. True when the page changed.
    pub fn live_tick(&mut self) -> bool {
        let saved = self.live_saved();
        let visible = self.on_live() && hex::window_shown();
        let hint = hex::recording_id(self);
        let start_ms = hex::recording_start(self).map(|(at, _)| at.timestamp_millis());
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if live.scene {
            return false;
        }
        let mut changed = saved;
        live.watching
            .store(visible || live.phase == Phase::Making, Ordering::Relaxed);
        {
            let mut feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
            if feed.hint.is_none() {
                feed.hint = hint;
            }
            if feed.start_ms.is_none() {
                feed.start_ms = start_ms;
            }
            if live.folder.is_none() {
                live.folder = feed.folder.clone();
            }
            if feed.version != live.shown_version {
                live.shown_version = feed.version;
                changed = true;
            }
        }
        // A recording that was already running (the app was restarted) has notes on disk.
        if let (Some(folder), false) = (live.folder.clone(), live.notes_loaded) {
            live.notes_loaded = true;
            let existing = feed_io::read_notes(&folder);
            if self.live_notes.text.is_empty() && !existing.is_empty() {
                self.live_notes = TextEdit::new(&existing);
                live.seen = existing.clone();
                live.saved = existing;
                live.saving = Saving::Saved;
                changed = true;
            }
        }
        if self.live_notes.text != live.seen {
            live.seen = self.live_notes.text.clone();
            live.edited_at = Some(Instant::now());
            live.saving = Saving::Pending;
            changed = true;
        }
        if live.needs_save(&live.seen) {
            let due = live.edited_at.is_none_or(|t| t.elapsed() >= SAVE_AFTER);
            if due {
                changed |= self.live_save();
            } else {
                self.live_arm_save();
            }
        }
        changed
    }

    /// Wakes the page once, about 2 s from now, so a save happens with no key pressed.
    fn live_arm_save(&self) {
        let (Some(live), Some(waker)) = (self.live.as_ref(), self.waker.clone()) else {
            return;
        };
        if live.timer_armed.swap(true, Ordering::Relaxed) {
            return;
        }
        let armed = live.timer_armed.clone();
        let _ = thread::Builder::new()
            .name("live-save-timer".into())
            .spawn(move || {
                thread::sleep(SAVE_AFTER + Duration::from_millis(150));
                armed.store(false, Ordering::Relaxed);
                waker();
            });
    }

    /// Writes the notes now. True when the page's state changed.
    fn live_save(&mut self) -> bool {
        let text = self.live_notes.text.clone();
        let hint = hex::recording_id(self);
        let start_ms = hex::recording_start(self).map(|(at, _)| at.timestamp_millis());
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if live.queued.as_ref() == Some(&text) {
            return false;
        }
        let folder = live.folder.clone();
        let recovery = sidenote::core::state_dir()
            .join("unsaved-notes")
            .join(format!("{}.md", live.wall_start.timestamp_millis()));
        let results = live.saves.clone();
        live.queued = Some(text.clone());
        live.saving = Saving::Pending;
        let wake = self.waker.clone();
        let queued = self.live_work(move || {
            let folder = folder.or_else(|| {
                feed_io::find_folder(&sidenote::core::meetings_dir(), hint.as_deref(), start_ms)
            });
            let result = feed_io::preserve_notes(folder, &text, &recovery);
            results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((text, result));
            if let Some(wake) = wake {
                wake();
            }
        });
        if !queued && let Some(live) = self.live.as_mut() {
            live.queued = None;
            live.saving = Saving::Failed(
                "Could not start saving notes. Keep this text before closing Sidenote.".into(),
            );
        }
        true
    }

    /// Saves any unsaved notes at once: before Stop, and when the recording ends in Hex.
    pub fn live_flush(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.scene {
            return;
        }
        live.seen = self.live_notes.text.clone();
        if live.needs_save(&live.seen) {
            self.live_save();
        }
    }

    fn live_toggle_panel(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.panel_open = !live.panel_open;
        }
    }
}

// ---- the page ---------------------------------------------------------------------------------

/// The recording pill: dot, voice bars, timer, chevron, and Stop. The first four are one button
/// that opens the live transcript above the bar.
fn rec_pill(s: &State, k: &Kit, p: Palette) -> El {
    let Some(live) = s.live.as_ref() else {
        return el();
    };
    let starting = live.phase == Phase::Starting;
    let (busy, words) = {
        let f = live.feed.lock().unwrap_or_else(|e| e.into_inner());
        (f.busy() && !starting, f.words)
    };
    let tick = if live.scene {
        0
    } else {
        live.began.elapsed().as_secs()
    };
    let bars = row()
        .key("live-bars")
        .gap(3.0)
        .items_center()
        .h(20.0)
        .a11y_hidden()
        .children(
            bar_heights(busy, tick, words)
                .into_iter()
                .enumerate()
                .map(|(i, h)| {
                    el().key(format!("live-bar-{i}"))
                        .w(3.0)
                        .h(h)
                        .radius(1.5)
                        .fill(if busy { p.ink } else { p.soft })
                        // One ease of about 180 ms with no bounce, as the Ask bar.
                        .motion(EASE_IN)
                }),
        );
    let open = live.panel_open && !starting;
    let chevron = if open {
        k.glyph.chevron_down
    } else {
        k.glyph.chevron_up
    };
    let mut group = row()
        .gap(10.0)
        .items_center()
        .child(bars)
        .child(roles::CONTROL.wrap(
            if open {
                "Hide live transcript"
            } else {
                "Show live transcript"
            },
            p.accent,
        ));
    if !starting {
        group = group.child(rune::icon(chevron, 14.0).color(p.faint));
    }
    let group = pressable(group)
        .key("live-chevron")
        .label(if open {
            "Hide the live transcript"
        } else {
            "Show the live transcript"
        })
        .help(if open {
            "Hide the live transcript"
        } else {
            "Show the live transcript, a few seconds behind"
        })
        .preserve_focus_on_press(true)
        .pl(16.0)
        .pr(12.0)
        .py(8.0)
        .radius(999.0)
        .hover(p.chip);
    if starting {
        group.dim(0.8)
    } else {
        group.on_press(|s: &mut State| s.live_toggle_panel())
    }
}

/// "1:05" or "1:02:05" for a line in the live transcript.
fn stamp(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The live transcript above the bar, newest line at the bottom. A scroll area that follows the
/// end: it keeps following new lines, stops when the reader scrolls up, and follows again once
/// they are back at the bottom.
#[derive(Clone, Copy, Debug)]
pub(super) struct PanelLimits {
    pub space: f32,
    pub thread: f32,
    pub transcript: f32,
}
fn panel_limits(
    pane: f32,
    document: f32,
    thread: f32,
    has_ask: bool,
    has_transcript: bool,
    notice: bool,
) -> PanelLimits {
    let space = (pane - document.min((pane - 108.0).max(0.0))).max(0.0);
    let available = (space - 108.0 - if notice { 46.0 } else { 0.0 }).max(0.0);
    let transcript_min = if has_transcript && available >= 140.0 {
        64.0
    } else {
        0.0
    };
    let thread_limit = if has_ask {
        (available - transcript_min - if has_transcript { 20.0 } else { 10.0 }).clamp(0.0, 300.0)
    } else {
        0.0
    };
    let thread_height = if has_ask {
        thread.max(44.0).min(thread_limit)
    } else {
        0.0
    };
    let transcript = if has_transcript {
        (available - thread_height - if has_ask { 20.0 } else { 10.0 }).clamp(0.0, 384.0)
    } else {
        0.0
    };
    PanelLimits {
        space,
        thread: thread_limit,
        transcript,
    }
}
pub(super) fn limits(s: &State) -> PanelLimits {
    let id = s.ask_key();
    panel_limits(
        s.pane_h.get(),
        s.live_document_h.get(),
        s.live_thread_h.get(),
        !s.saved.messages.is_empty() || s.asks.contains_key(&id) || s.ask_errors.contains_key(&id),
        s.live
            .as_ref()
            .is_some_and(|live| live.panel_open && live.phase == Phase::Recording),
        s.ask_settings.fallback_notice.is_some(),
    )
}

fn transcript_panel(s: &State, p: Palette, cap: f32) -> El {
    let Some(live) = s.live.as_ref() else {
        return el();
    };
    let feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
    let header = row()
        .w_full()
        .justify_between()
        .items_center()
        .pl(16.0)
        .pr(8.0)
        .py(8.0)
        .child(spaced_caps("Live transcript", p))
        .child(
            quiet_button("Hide", "live-panel-hide", p)
                .on_press(|s: &mut State| s.live_toggle_panel()),
        );
    let note = |text: &str| {
        roles::SMALL
            .wrap(text, p.soft)
            .key("live-panel-note")
            .role("status")
            .px(16.0)
            .py(18.0)
    };
    let body = if let Some(error) = &feed.error {
        col().w_full().child(note(&format!(
            "{error}. Your notes are still saved, and the full transcript appears after you stop."
        )))
    } else if feed.lines.is_empty() {
        let mut c = col().w_full().child(note(
            "Listening\u{2026} words appear a few seconds after they are said",
        ));
        if live.began.elapsed() > NO_FEED_AFTER && !live.scene {
            c = c.child(
                roles::META
                    .wrap(
                        "Nothing has come from Hex yet. The full transcript still appears after you stop.",
                        p.faint,
                    )
                    .px(16.0)
                .pb(16.0),
            );
        }
        c
    } else {
        let mut rows = col().w_full().px(16.0).py(6.0);
        let mut last: Option<bool> = None;
        for (i, line) in feed.lines.iter().enumerate() {
            let who = (last != Some(line.you)).then_some(line.you);
            last = Some(line.you);
            rows = rows.child(
                row()
                    .key(format!("live-line-{i}"))
                    .w_full()
                    .gap(10.0)
                    .items_start()
                    .py(3.0)
                    .child(roles::META.wrap(stamp(line.seconds), p.faint).w(46.0))
                    .child(
                        roles::META
                            .bold()
                            .wrap(
                                match who {
                                    Some(true) => "You",
                                    Some(false) => "Others",
                                    None => "",
                                },
                                if who == Some(true) { p.accent } else { p.soft },
                            )
                            .w(48.0),
                    )
                    .child(roles::SMALL.wrap(&line.text, p.ink).grow(1.0).min_w(0.0)),
            );
        }
        let max_h = (cap - 44.0).max(0.0);
        col().w_full().min_h(0.0).shrink().child(
            col()
                .key("live-tx-scroll")
                .w_full()
                .max_h(max_h)
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec { stick_bottom: true })
                .child(rows),
        )
    };
    raised(
        col()
            .key("live-panel")
            .w_full()
            .radius(16.0)
            .max_h(cap)
            .min_h(0.0)
            .shrink()
            .clip_rounded(16.0)
            .transition(rise(10.0))
            .child(header)
            .child(hairline(p))
            .child(body.min_h(0.0).shrink()),
        p,
    )
}

/// The bottom bar: the recording pill next to the Ask pill, with the transcript above.
fn bar(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let cap = limits(s).transcript;
    let panel = s
        .live
        .as_ref()
        .filter(|l| l.panel_open && l.phase == Phase::Recording && cap >= 44.0)
        .map(|_| transcript_panel(s, p, cap));
    ask::overlay_with(s, k, p, narrow, layer, None, panel)
}

/// The notes editor: plain text, several lines, under the "Meeting at HH:MM" heading.
fn notes_view(s: &State, k: &Kit, p: Palette, narrow: bool, starting: bool) -> El {
    let saving = s.live.as_ref().map(|l| l.saving.clone());
    let status = match &saving {
        Some(Saving::Pending) => Some(("Saving\u{2026}", p.faint)),
        Some(Saving::Saved) => Some(("Saved", p.faint)),
        _ => None,
    };
    let mut head = row()
        .w_full()
        .gap(12.0)
        .items_end()
        .justify_between()
        .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p).key("live-title"));
    if let Some((words, ink)) = status {
        head = head.child(
            roles::META
                .wrap(words, ink)
                .key("live-saved")
                .role("status")
                .pb(6.0),
        );
    }
    let hint = if starting {
        "Starting the recording. You can start writing now."
    } else {
        "Your notes save as you type. Sidenote writes the summary after you stop."
    };
    let field = rune::text_input(&s.live_notes, roles::READ.style())
        .key("live-notes-input")
        .w_full()
        .min_h(2.0 * roles::READ.lh)
        .color(p.ink)
        .placeholder("Write notes\u{2026}", p.faint)
        .caret(p.ink, k.selection())
        .label("Your notes")
        .on_edit(|s: &mut State| &mut s.live_notes)
        // Return starts a new line here; it would otherwise submit.
        .on_submit(|s: &mut State, _| s.live_notes.insert("\n"));
    let mut c = col()
        .w_full()
        .items_start()
        .gap(if narrow { 10.0 } else { 12.0 })
        .child(head);
    if let Some(Saving::Failed(_)) = saving {
        let recovered = s.live.as_ref().is_some_and(|live| live.recovered.is_some());
        let warning = save_warning(s.hex_recording(), recovered);
        c = c.child(
            col()
                .key("live-save-warning")
                .w_full()
                .padding(16.0)
                .radius(10.0)
                .border(1.0, p.warn)
                .fill(p.warnbg)
                .gap(12.0)
                .role("alert")
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(super::icons::draw(&super::icons::WARNING, 16.0, p.warn))
                        .child(roles::NAME.wrap("Your notes need saving", p.warn)),
                )
                .child(roles::SMALL.wrap(warning, p.ink))
                .child(
                    row()
                        .flex_wrap()
                        .gaps(8.0, 8.0)
                        .child(
                            super::widgets::button(
                                "Copy notes",
                                "live-copy-notes",
                                super::widgets::ButtonRole::Primary,
                                p,
                            )
                            .on_press_tx(|s: &mut State, tx| {
                                tx.copy(s.live_notes.text.clone());
                            }),
                        )
                        .child(
                            super::area::button("Try saving again", "live-save-retry", p)
                                .key("live-close-retry")
                                .on_press(|s: &mut State| {
                                    s.live_flush();
                                }),
                        ),
                ),
        );
    } else {
        c = c.child(roles::META.wrap(hint, p.faint));
    }
    c = c.child(
        col()
            .key("live-notes")
            .w_full()
            .max_h((s.pane_h.get() * 0.35).clamp(96.0, 280.0))
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .mt(if narrow { 4.0 } else { 10.0 })
            .child(rune::auto_grow(field, 2, 400)),
    );
    c.children((!starting).then(|| rec_pill(s, k, p)))
}

fn save_warning(recording: bool, recovered: bool) -> String {
    let saved = if recovered {
        "Sidenote saved a backup copy, but could not save your notes in the meeting folder."
    } else {
        "Sidenote could not save your notes, even to a backup copy."
    };
    let status = if recording {
        " The meeting is still recording."
    } else {
        ""
    };
    format!(
        "{saved}{status} Copy your notes now, then try saving again. Do not close Sidenote yet."
    )
}

/// The page heading: "Meeting at HH:MM" from the start time.
fn heading(s: &State) -> String {
    s.live
        .as_ref()
        .map_or_else(|| "Meeting".into(), Live::heading)
}

fn making_view(s: &State, p: Palette, narrow: bool) -> El {
    if let Some(skipped) = summary_skip(s) {
        return col()
            .key("live-summary-skipped")
            .w_full()
            .gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(roles::SECTION.wrap(skipped.heading(), p.ink).role("status"))
            .child(roles::SMALL.wrap(skipped.detail(), p.soft))
            .children(
                (!s.live_notes.text.trim().is_empty())
                    .then(|| your_notes(s.live_notes.text.trim(), p)),
            )
            .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
    }
    let transcript = making_transcript(s);
    let finishing = transcript != Transcript::Ready;
    let scripts = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|n| n.engine == "scripts");
    if !finishing
        && !scripts
        && let Some(live) = &s.live
    {
        let id = live
            .folder
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str());
        let meeting = s.meetings.iter().find(|m| live.matches_meeting(m));
        let job = id
            .and_then(|id| {
                s.ai_notes
                    .jobs
                    .get(id)
                    .cloned()
                    .map(|j| (id.to_string(), j))
            })
            .or_else(|| {
                meeting.and_then(|m| super::notes_status::job(s, m).map(|j| (m.id.clone(), j)))
            });
        if let Some((id, job)) = job
            && let Some(banner) = super::notes_status::banner_for(s, &id, &job, p)
        {
            return col()
                .key("live-notes-state")
                .w_full()
                .gap(16.0)
                .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
                .child(banner)
                .children(
                    (!s.live_notes.text.trim().is_empty())
                        .then(|| your_notes(s.live_notes.text.trim(), p)),
                )
                .child(link("Open meeting and transcript", p).on_press_tx(
                    move |s: &mut State, tx| {
                        s.leave_live();
                        if !s.on_live() {
                            s.select(&id, tx);
                            s.toggle_transcript();
                        }
                    },
                ))
                .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
        }
    }
    let slow = s
        .live
        .as_ref()
        .and_then(|l| l.stopped)
        .is_some_and(|t| t.elapsed() > MAKING_SLOW);
    let problem = s.live.as_ref().and_then(|live| {
        if !finishing && scripts {
            live.feed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .notes_problem
        } else {
            None
        }
    });
    if let Some(problem) = problem {
        return col().key("live-notes-problem").w_full().gap(16.0)
            .child(text_title(&heading(s), if narrow { 28.0 } else { 36.0 }, p))
            .child(roles::READ.wrap(problem, p.ink).role("status"))
            .child(roles::SMALL.wrap("Your recording is saved in Hex. Your own notes appear in saved meetings once they are saved.", p.soft))
            .children((!s.live_notes.text.trim().is_empty()).then(|| your_notes(s.live_notes.text.trim(), p)))
            .child(link("Back to meetings", p).on_press(|s: &mut State| s.leave_live()));
    }
    // The quick title as soon as it exists; until then the time heading, with a small note.
    let quick = s.live.as_ref().and_then(Live::quick_title);
    let step = |done: bool, text_: &str| {
        row()
            .gap(10.0)
            .items_center()
            .child(if done {
                label("\u{2713}", 14.0, 700.0, p.ok).w(14.0)
            } else {
                dot(8.0, p.accent).mx(3.0)
            })
            .child(if done {
                roles::READ.wrap(text_, p.soft)
            } else {
                roles::READ.bold().wrap(text_, p.ink)
            })
    };
    let bar = row()
        .w(260.0)
        .h(4.0)
        .radius(2.0)
        .fill(p.chip)
        .child(el().w(96.0).h(4.0).radius(2.0).fill(p.accent));
    let mut c = col()
        .key("live-making")
        .w_full()
        .items_start()
        .gap(14.0)
        .child(
            text_title(
                quick.as_deref().unwrap_or(&heading(s)),
                if narrow { 28.0 } else { 36.0 },
                p,
            )
            .key("live-name"),
        )
        .children((quick.is_none() && !finishing).then(|| {
            roles::META
                .wrap("Naming\u{2026}", p.faint)
                .key("live-naming")
                .role("status")
        }))
        .child(
            roles::SECTION
                .wrap(transcript.heading(slow), p.ink)
                .mt(if narrow { 6.0 } else { 18.0 })
                .role("status"),
        )
        .child(bar)
        .child(
            col()
                .gap(6.0)
                .mt(4.0)
                .child(step(
                    true,
                    if finishing {
                        "Recording stopped"
                    } else {
                        "Transcript ready"
                    },
                ))
                .child(step(
                    false,
                    match transcript {
                        Transcript::Finishing => "Finishing the transcript",
                        Transcript::Interrupted => "Waiting for Hex",
                        Transcript::Ready => "Writing the summary",
                    },
                )),
        )
        .child(
            roles::SMALL
                .wrap(transcript.detail(), p.soft)
                .max_w(460.0)
                .mt(6.0),
        );
    if !s.live_notes.text.trim().is_empty() {
        c = c.child(
            your_notes(s.live_notes.text.trim(), p)
                .key("live-your-notes")
                .mt(10.0),
        );
    }
    c = c.child(
        link("Back to meetings", p)
            .key("live-back")
            .mt(6.0)
            .on_press(|s: &mut State| s.leave_live()),
    );
    c
}

/// The user's own notes as paragraphs in a quiet card, with a small caption. Shared with the
/// meeting page.
pub fn your_notes(notes: &str, p: Palette) -> El {
    let mut body = col().w_full().gap(6.0);
    for line in notes.lines() {
        body = body.child(if line.trim().is_empty() {
            el().h(6.0)
        } else {
            roles::READ.wrap(line, p.ink)
        });
    }
    col()
        .w_full()
        .gap(10.0)
        .px(22.0)
        .py(16.0)
        .radius(16.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(spaced_caps("Your notes", p))
        .child(body)
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let phase = s.live.as_ref().map_or(Phase::Recording, |l| l.phase);
    let body = if s.live.as_ref().is_some_and(|l| l.close_failed) {
        notes_view(s, k, p, narrow, phase == Phase::Starting)
    } else {
        match phase {
            Phase::Making => making_view(s, p, narrow),
            Phase::Starting => notes_view(s, k, p, narrow, true),
            Phase::Recording => notes_view(s, k, p, narrow, false),
        }
    };
    let lost = phase == Phase::Recording && !s.hex_running();
    let mut doc = col()
        .key("live")
        .w_full()
        .max_w(720.0)
        .px(if narrow { 20.0 } else { 40.0 })
        .pt(if narrow { 20.0 } else { 40.0 })
        .pb(if phase == Phase::Making { 56.0 } else { 12.0 })
        .items_start()
        .gap(16.0);
    if lost {
        doc = doc.child(
            roles::SMALL
                .medium()
                .wrap(
                    "Hex is not answering. The recording may still be running in Hex.",
                    p.warn,
                )
                .role("alert"),
        );
    }
    if phase == Phase::Making
        && let Some(live) = &s.live
        && let Saving::Failed(error) = &live.saving
    {
        doc = doc.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    doc = doc.child(body);
    let mut pane = col()
        .key("live-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg)
        .child(
            scroll(col().w_full().items_center().child(doc), "live-scroll").min_h(
                if phase == Phase::Making {
                    0.0
                } else {
                    s.live_document_h
                        .get()
                        .min((s.pane_h.get() - 108.0).max(0.0))
                },
            ),
        );
    if phase != Phase::Making {
        pane = pane.child(bar(s, k, p, narrow, layer));
    }
    pane
}

#[cfg(test)]
mod writer_tests {
    #[test]
    fn feed_skip_requires_ready_receipt_and_clears_on_resume() {
        use sidenote::core::notes::{JobState, SummarySkip};
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let folder = temp.path().join("1800000000000-1");
        fs::create_dir(&folder).unwrap();
        let manifest = |status| {
            let value = serde_json::json!({"status":status,"duration_ms":16700});
            fs::write(folder.join("manifest.json"), value.to_string()).unwrap();
        };
        fs::write(folder.join("transcript.ndjson"), "").unwrap();
        manifest("complete");
        let mut feed = Feed::default();
        feed_summary_state(&mut feed, &folder, &root, false, None);
        assert_eq!(feed.summary_skip, None);
        assert!(!root.exists(), "UI read created guard state or a lock");
        for job in [
            JobState::Queued,
            JobState::Running,
            JobState::BackendError {
                next_try_ms: Some(i64::MAX),
                exit: None,
            },
        ] {
            feed_summary_state(&mut feed, &folder, &root, true, Some(&job));
            assert_eq!(feed.summary_skip, None);
        }
        let receipt = root.join("local/recording-guard/1800000000000-1.json");
        fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        fs::write(&receipt, r#"{"short":true,"kept":false}"#).unwrap();
        fs::write(folder.join("final.pending"), "").unwrap();
        for status in ["transcribing", "interrupted", "recording", "starting"] {
            manifest("complete");
            feed_summary_state(&mut feed, &folder, &root, false, None);
            assert_eq!(feed.summary_skip, Some(SummarySkip::Short));
            manifest(status);
            feed_summary_state(&mut feed, &folder, &root, true, Some(&JobState::Empty));
            assert_eq!(feed.summary_skip, None);
        }
        manifest("complete");
        fs::write(&receipt, r#"{"short":true,"kept":true}"#).unwrap();
        feed_summary_state(&mut feed, &folder, &root, false, None);
        assert_eq!(feed.summary_skip, None);
        assert!(!receipt.with_extension("lock").exists());
    }

    use super::*;

    #[test]
    fn replacing_recording_discards_stop_and_rejects_old_reply() {
        let mut first = Live::scene(Phase::Making, None);
        first.notes_response_time = 17;
        let first_id = first.recording_id;
        let mut current = Some(first);
        drop(current.replace(Live::scene(Phase::Recording, None)));
        let second = current.as_mut().unwrap();
        assert_ne!(second.recording_id, first_id);
        assert_eq!(second.notes_response_time, 0);
        second.notes_response_time = 29;
        assert_eq!(second.take_notes_timing(first_id), 0);
        assert_eq!(second.notes_response_time, 29);
        assert_eq!(second.take_notes_timing(second.recording_id), 29);
    }

    #[test]
    fn panel_budget_preserves_the_document_and_shrinks_transcript_first() {
        for (pane, document) in [(748.0, 340.0), (748.0, 410.0)] {
            let idle = panel_limits(pane, document, 0.0, false, true, false);
            let answer = panel_limits(pane, document, 150.0, true, true, false);
            assert!(answer.transcript < idle.transcript);
            assert!(answer.transcript + answer.thread.min(150.0) + 128.0 <= answer.space + 0.1);
            assert!(answer.space + document <= pane + 0.1);
        }
    }
    #[test]
    fn restart_and_late_summary_preserve_live_notes() {
        use sidenote::{
            core,
            hex_engine::{EngineState, HexError},
        };
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        use std::{fs, process::Command};

        // Give State and all of its workers private paths without changing other tests' env.
        if std::env::var_os("SIDENOTE_LIVE_TIMELINE_CHILD").is_none() {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("artifacts")
                .join(format!("live-timeline-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "ui::live::writer_tests::restart_and_late_summary_preserve_live_notes",
                    "--nocapture",
                ])
                .env("SIDENOTE_LIVE_TIMELINE_CHILD", "1")
                .env("APPDATA", root.join("appdata"))
                .env("LOCALAPPDATA", root.join("localappdata"))
                .env("USERPROFILE", root.join("home"))
                .env("SIDENOTE_DATA_DIR", root.join("state"))
                .env("SIDENOTE_STATE_DIR", root.join("state"))
                .env("SIDENOTE_HEX_DIR", root.join("meetings"))
                .env("SIDENOTE_PEERS_DIR", root.join("peers"));
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let status = command.status().unwrap();
            assert!(status.success(), "isolated live timeline failed");
            return;
        }
        let root = core::meetings_dir();
        let folder = root.join("1794303362394-45464");
        fs::create_dir_all(&folder).unwrap();
        let manifest = |status: &str| {
            fs::write(
                folder.join("manifest.json"),
                serde_json::to_vec(&serde_json::json!({
                    "status": status, "created_at_ms": 1794303362394_i64
                }))
                .unwrap(),
            )
            .unwrap();
        };
        manifest("recording");
        let mut state = State::new();
        let mut live = Live::scene(Phase::Recording, None);
        live.wall_start = DateTime::from_timestamp_millis(1794303362394 + 1000)
            .unwrap()
            .with_timezone(&Local);
        live.folder = Some(folder.clone());
        state.live = Some(live);
        state.live_open = true;
        state.live_notes = TextEdit::new("Typed notes must survive");
        let status = |name: &str| EngineState {
            labs: Default::default(),
            running: true,
            version: "test".into(),
            protocol: 4,
            recording: None,
            meeting_state: name.into(),
            dictating: Some(false),
            capture_state: "idle".into(),
            model: serde_json::from_str("{}").unwrap(),
        };
        manifest("transcribing");
        state.hex.status = Some(Ok(status("transcribing")));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        assert_eq!(making_transcript(&state), Transcript::Finishing);
        assert_eq!(read_transcript_state(&folder), Some(Transcript::Finishing));
        manifest("interrupted");
        fs::write(folder.join("final.pending"), "").unwrap();
        state.hex.status = Some(Err(HexError::HexNotRunning));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        let transcript = read_transcript_state(&folder);
        assert_eq!(transcript, Some(Transcript::Interrupted));
        state.live.as_ref().unwrap().feed.lock().unwrap().transcript = transcript;
        state.hex.status = Some(Ok(status("idle")));
        state.live_sync();
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Making);
        assert_eq!(making_transcript(&state), Transcript::Interrupted);
        assert_eq!(
            making_transcript(&state).detail(),
            "Hex was restarted. It will finish this transcript when it starts again."
        );
        manifest("complete");
        fs::remove_file(folder.join("final.pending")).unwrap();
        let transcript = read_transcript_state(&folder);
        assert_eq!(transcript, Some(Transcript::Ready));
        state.live.as_ref().unwrap().feed.lock().unwrap().transcript = transcript;
        assert_eq!(
            making_transcript(&state).heading(false),
            "Writing the summary"
        );
        let refresh = |state: &mut State| {
            let options = state.refresh_options();
            state.apply_event(super::super::Event::Refreshed(
                super::super::refresh::Result {
                    generation: core::mutation_generation(),
                    options,
                    data: Some(Box::new(super::super::refresh::Data::read(options))),
                },
            ));
        };
        refresh(&mut state);
        assert!(state.live.is_some());
        state.live.as_mut().unwrap().stopped = Some(Instant::now() - Duration::from_secs(49 * 60));
        state.live.as_mut().unwrap().scene = false;
        state.live.as_mut().unwrap().folder = None;
        assert_eq!(
            making_transcript(&state).heading(true),
            "The summary is taking a while"
        );
        fs::write(
            folder.join("english-summary.json"),
            r#"{"title":"Finished meeting"}"#,
        )
        .unwrap();
        refresh(&mut state);
        assert!(state.live.is_none());
        assert_eq!(state.live_ready.as_deref(), Some("1794303362394-45464"));
        assert_eq!(feed_io::read_notes(&folder), "Typed notes must survive");
        assert_eq!(state.meetings[0].notes, "Typed notes must survive");

        state.live = Some(Live::new(Phase::Making, HashSet::new()));
        state.live.as_mut().unwrap().folder = Some(folder.clone());
        state.live_open = true;
        state.live_notes = TextEdit::new("Notes from the last Stop click");
        state.hex_record_stop(None);
        assert!(
            !state.on_live(),
            "Stop after Hex stopped should release the page"
        );
        assert!(state.live_close());
        assert_eq!(
            feed_io::read_notes(&folder),
            "Notes from the last Stop click"
        );

        // Back leaves the page; completion later must not pull the owner away from the list.
        state.live = Some(Live::new(Phase::Making, HashSet::new()));
        state.live.as_mut().unwrap().folder = Some(folder.clone());
        state.live_open = true;
        state.live_notes = TextEdit::new("Notes saved when leaving");
        state.live_ready = None;
        state.leave_live();
        assert!(!state.on_live());
        assert_eq!(feed_io::read_notes(&folder), "Notes saved when leaving");
        state.live_scanned();
        assert!(state.live.is_none());
        assert!(state.live_ready.is_none());
    }

    #[test]
    fn folder_timestamp_matches_without_hex_id_or_manifest_start() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts/live-match-test")
            .join("1794303362394-45464");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("english-summary.json"), "{}").unwrap();
        let mut meeting = sidenote::core::Meeting::load(&root).unwrap();
        let mut live = Live::scene(Phase::Making, None);
        live.wall_start = DateTime::from_timestamp_millis(1794303362394 + 120_000)
            .unwrap()
            .with_timezone(&Local);
        assert!(live.matches_meeting(&meeting));
        live.wall_start += chrono::Duration::milliseconds(1);
        assert!(!live.matches_meeting(&meeting));
        live.wall_start -= chrono::Duration::milliseconds(1);
        meeting.device = Some("peer".into());
        assert!(!live.matches_meeting(&meeting));
    }

    #[test]
    fn save_warning_tells_the_truth_about_recording_and_backups() {
        for recording in [false, true] {
            for recovered in [false, true] {
                let warning = save_warning(recording, recovered);
                assert_eq!(warning.contains("still recording"), recording);
                assert_eq!(warning.contains("saved a backup copy"), recovered);
                assert!(warning.contains("Copy your notes now"));
                assert!(warning.contains("Do not close Sidenote yet"));
            }
        }
    }

    #[test]
    fn close_requires_the_latest_text_to_be_durable_and_the_queue_drained() {
        let mut live = Live::scene(Phase::Starting, None);
        assert!(live.can_close(""));
        live.saving = Saving::Failed("disk full".into());
        assert!(!live.can_close("draft"));
        live.saved = "older notes".into();
        assert!(!live.can_close(""));
        live.recovered = Some("draft".into());
        assert!(live.can_close("draft"));
        assert!(!live.can_close("newer draft"));
        live.saved = "newer draft".into();
        live.queued = Some("old draft".into());
        assert!(!live.can_close("newer draft"));
        live.queued = None;
        assert!(live.can_close("newer draft"));
    }

    #[test]
    fn a_previous_pending_meeting_cannot_replace_the_current_live_meeting() {
        let root = std::env::temp_dir().join(format!(
            "notes-live-match-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("english-summary.json"), "{}").unwrap();
        let mut meeting = sidenote::core::Meeting::load(&root).unwrap();
        let mut live = Live::scene(Phase::Making, None);
        meeting.started = Some(live.wall_start - chrono::Duration::minutes(20));
        assert!(!live.matches_meeting(&meeting));
        meeting.started = Some(live.wall_start);
        assert!(live.matches_meeting(&meeting));
        live.folder = Some(PathBuf::from("1234-1"));
        assert!(!live.matches_meeting(&meeting));
        meeting.id = "1234-1".into();
        meeting.started = None;
        assert!(live.matches_meeting(&meeting));
        meeting.device = Some("peer".into());
        assert!(!live.matches_meeting(&meeting));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reverting_text_still_flushes_after_an_older_queued_save() {
        let mut live = Live::scene(Phase::Recording, None);
        live.saved = "original".into();
        live.queued = Some("edited".into());
        assert!(live.needs_save("original"));
        live.queued = Some("original".into());
        assert!(!live.needs_save("original"));
    }

    #[test]
    fn close_drains_queued_saves_before_stop() {
        let writer = NoteWriter::new().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        for message in ["first save", "failed save", "last save", "stop"] {
            let tx = tx.clone();
            writer
                .tx
                .send(Box::new(move || {
                    tx.send(message).unwrap();
                }))
                .unwrap();
        }
        writer.finish();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            ["first save", "failed save", "last save", "stop"]
        );
    }
}
