//! "New meeting": the live page. Starting a meeting starts a Hex recording and opens a page with
//! a heading ("Meeting at 14:30"), a recording mark, the elapsed time and a large Stop button.
//! After Stop it says "Making notes" until the normal pipeline has written the meeting, then
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
//! After Stop it says "Making notes" (with the user's notes beside it) until the normal pipeline
//! has written the meeting, then opens it. Nobody types a title: a cheap model names the meeting
//! seconds after Stop (quick-title.json, read here while the notes are made), and the notes' own
//! title replaces it.
use super::{
    State,
    ask::{self, EASE_IN, raised, rise},
    hex,
    list::text_title,
    nav::icon_stop,
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
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// How long a start may take before the page gives up waiting for Hex to say it is recording.
const START_WAIT: Duration = Duration::from_secs(25);
/// After this long without a new meeting, the page says the notes are slow and lets go.
const MAKING_SLOW: Duration = Duration::from_secs(10 * 60);
/// How often the folder is scanned while notes are being made.
const SCAN_EVERY: Duration = Duration::from_secs(6);
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

/// What the reader thread knows about the live transcript. The page reads it; only the thread
/// (and the page's one-time hint) writes it.
#[derive(Default)]
pub struct Feed {
    /// The id Hex reports for the running recording, when it reports one.
    hint: Option<String>,
    /// The meeting folder, once found.
    folder: Option<PathBuf>,
    pub lines: Vec<LiveLine>,
    /// A plain sentence when the file could not be read.
    pub error: Option<String>,
    /// Words seen so far, and when that number last grew. The voice bars follow this.
    words: usize,
    last_words: Option<Instant>,
    stamp: Option<feed_io::Stamp>,
    /// Folders made before this (Unix ms) are not this recording's.
    since_ms: i64,
    /// Counts every change, so the page knows to draw again.
    version: u64,
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

pub struct Live {
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
    edited_at: Option<Instant>,
    pub saving: Saving,
    timer_armed: Arc<AtomicBool>,
    /// A render scene: nothing is read, and the bars hold still.
    scene: bool,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Live {
    fn new(phase: Phase, known: HashSet<String>) -> Self {
        Self {
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
            edited_at: None,
            saving: Saving::Quiet,
            timer_armed: Arc::new(AtomicBool::new(false)),
            scene: false,
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
        self.meetings.iter().map(|m| m.id.clone()).collect()
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
        self.go_meetings();
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
        let cancel = live.cancel.clone();
        let quick = live.quick_title.clone();
        // Folder ids start with their creation time; allow for a clock a little behind the start.
        let since_ms = live.wall_start.timestamp_millis().saturating_sub(120_000);
        let known = live.known.clone();
        let waker = self.waker.clone();
        self.request_refresh();
        // The quick title lands a few seconds after Stop, well before the notes, so look often.
        let title_cancel = cancel.clone();
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
        if let Some(tx) = self.scan_tx.clone() {
            let _ = thread::Builder::new()
                .name("live-scan".into())
                .spawn(move || {
                    // Ends when the page lets go, when the app closes, or after a long wait.
                    for _ in 0..(MAKING_SLOW.as_secs() / SCAN_EVERY.as_secs() + 10) {
                        thread::sleep(SCAN_EVERY);
                        if cancel.load(Ordering::Relaxed) || tx.send(()).is_err() {
                            break;
                        }
                    }
                });
        }
    }

    /// After a scan: the first new local meeting while notes are being made is the one.
    pub fn live_scanned(&mut self) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        if live.phase != Phase::Making {
            return;
        }
        let found = self
            .meetings
            .iter()
            .find(|m| m.device.is_none() && !live.known.contains(&m.id))
            .map(|m| m.id.clone());
        if let Some(id) = found {
            let was_open = self.on_live();
            self.live = None;
            self.live_open = false;
            // Opening a meeting needs the UI's scroll handle, so `poll` finishes the job.
            if was_open {
                self.live_ready = Some(id);
            }
        }
    }
}

// ---- the live transcript and the notes -------------------------------------------------------

/// Reads the live transcript while the page is shown. One `stat` per round when nothing changed.
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
        let (mut folder, hint, since, last) = {
            let f = feed.lock().unwrap_or_else(|e| e.into_inner());
            (f.folder.clone(), f.hint.clone(), f.since_ms, f.stamp)
        };
        if folder.is_none() {
            folder = feed_io::find_folder(&root, since, hint.as_deref());
        }
        let Some(folder) = folder else {
            continue;
        };
        let read = feed_io::read_rolling(&folder, last);
        let mut f = feed.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = f.folder.as_ref() != Some(&folder);
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
        let visible = self.on_live() && hex::window_shown();
        let hint = hex::recording_id(self);
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if live.scene {
            return false;
        }
        let mut changed = false;
        live.watching
            .store(visible && live.phase != Phase::Making, Ordering::Relaxed);
        {
            let mut feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
            if feed.hint.is_none() {
                feed.hint = hint;
            }
            feed.since_ms = live.wall_start.timestamp_millis().saturating_sub(120_000);
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
        if live.seen != live.saved {
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
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        let folder = live.folder.clone().or_else(|| {
            feed_io::find_folder(
                &sidenote::core::meetings_dir(),
                live.wall_start.timestamp_millis().saturating_sub(120_000),
                hint.as_deref(),
            )
        });
        let before = live.saving.clone();
        match folder.map(|f| feed_io::write_notes(&f, &text).map(|()| f)) {
            Some(Ok(folder)) => {
                live.folder = Some(folder);
                live.saved = text;
                live.edited_at = None;
                live.saving = Saving::Saved;
            }
            other => {
                // Hex has not made the folder yet, or the write failed: try again in 2 s.
                live.edited_at = Some(Instant::now());
                live.saving = Saving::Failed(match other {
                    Some(Err(e)) => {
                        format!("Could not save your notes ({}). Trying again.", e.kind())
                    }
                    _ => "Waiting for Hex to start the meeting before saving. Trying again.".into(),
                });
                self.live_arm_save();
            }
        }
        self.live.as_ref().is_some_and(|l| l.saving != before)
    }

    /// Saves any unsaved notes at once: before Stop, and when the recording ends in Hex.
    pub fn live_flush(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.scene || live.phase == Phase::Making {
            return;
        }
        live.seen = self.live_notes.text.clone();
        if live.seen != live.saved {
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

/// Stop, inside the recording pill. It is a second way to stop: the top right keeps its own.
fn stop_pill(s: &State, p: Palette, starting: bool) -> El {
    let why = if starting {
        Some("Starting the recording")
    } else {
        hex::stop_blocked(s)
    };
    let b = pressable(
        row()
            .gap(8.0)
            .items_center()
            .child(icon_stop(10.0, p.card))
            .child(roles::CONTROL.bold().wrap("Stop", p.card)),
    )
    .key("live-stop")
    .pl(14.0)
    .pr(16.0)
    .py(8.0)
    .radius(999.0)
    .fill(p.ink);
    match why {
        None => b
            .label("Stop recording")
            .help("Stop recording and make notes")
            .preserve_focus_on_press(true)
            .on_press(|s: &mut State| s.hex_record_stop()),
        Some(reason) => b.label(format!("Stop ({reason})")).help(reason).dim(0.45),
    }
}

/// The recording pill: dot, voice bars, timer, chevron, and Stop. The first four are one button
/// that opens the live transcript above the bar.
fn rec_pill(s: &State, k: &Kit, p: Palette) -> El {
    let Some(live) = s.live.as_ref() else {
        return el();
    };
    let starting = live.phase == Phase::Starting;
    let clock = hex::recording_clock(s).unwrap_or_else(|| "0:00".into());
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
        .child(
            row()
                .key("live-indicator")
                .role("status")
                .label(if starting { "Starting" } else { "Recording" })
                .child(dot(9.0, if starting { p.faint } else { p.warn })),
        )
        .child(bars)
        .child(roles::NAME.wrap(clock, p.ink).key("live-timer"));
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
    let group = if starting {
        group.dim(0.8)
    } else {
        group.on_press(|s: &mut State| s.live_toggle_panel())
    };
    row()
        .key("live-pill")
        .gap(2.0)
        .items_center()
        .pr(5.0)
        .pl(2.0)
        .py(4.0)
        .radius(999.0)
        .fill(p.card)
        .border(1.0, p.line)
        .shadow(3.0, 16.0, p.shadow)
        .child(group)
        .child(stop_pill(s, p, starting))
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
fn transcript_panel(s: &State, p: Palette) -> El {
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
        let max_h = (s.pane_h.get() * 0.45).clamp(150.0, 340.0);
        col().w_full().child(
            col()
                .key("live-tx-scroll")
                .w_full()
                .max_h(max_h)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(rows),
        )
    };
    raised(
        col()
            .key("live-panel")
            .w_full()
            .radius(16.0)
            .transition(rise(10.0))
            .child(header)
            .child(hairline(p))
            .child(body),
        p,
    )
}

/// The bottom bar: the recording pill next to the Ask pill, with the transcript above.
fn bar(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let panel = s
        .live
        .as_ref()
        .filter(|l| l.panel_open && l.phase == Phase::Recording)
        .map(|_| transcript_panel(s, p));
    ask::overlay_with(s, k, p, narrow, layer, Some(rec_pill(s, k, p)), panel)
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
        "Notes save as you type. Hex writes the full notes after you stop."
    };
    let field = rune::text_input(&s.live_notes, roles::READ.style())
        .key("live-notes-input")
        .w_full()
        .min_h(if narrow { 8.0 } else { 12.0 } * roles::READ.lh)
        .color(p.ink)
        .placeholder("Write notes\u{2026}", p.faint)
        .caret(p.ink, k.selection())
        .label("Meeting notes")
        .on_edit(|s: &mut State| &mut s.live_notes)
        // Return starts a new line here; it would otherwise submit.
        .on_submit(|s: &mut State, _| s.live_notes.insert("\n"));
    let mut c = col()
        .w_full()
        .items_start()
        .gap(if narrow { 10.0 } else { 12.0 })
        .child(head)
        .child(roles::META.wrap(hint, p.faint));
    if let Some(Saving::Failed(why)) = saving {
        c = c.child(roles::SMALL.medium().wrap(why, p.warn).role("alert"));
    }
    c.child(
        col()
            .key("live-notes")
            .w_full()
            .mt(if narrow { 4.0 } else { 10.0 })
            .child(rune::auto_grow(field, if narrow { 8 } else { 12 }, 400)),
    )
}

/// The page heading: "Meeting at HH:MM" from the start time.
fn heading(s: &State) -> String {
    s.live
        .as_ref()
        .map_or_else(|| "Meeting".into(), Live::heading)
}

fn making_view(s: &State, p: Palette, narrow: bool) -> El {
    let slow = s
        .live
        .as_ref()
        .and_then(|l| l.stopped)
        .is_some_and(|t| t.elapsed() > MAKING_SLOW);
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
        .children(
            quick
                .is_none()
                .then(|| roles::META.wrap("Naming\u{2026}", p.faint).key("live-naming").role("status")),
        )
        .child(
            roles::SECTION
                .wrap(if slow { "Notes are taking a while" } else { "Making notes" }, p.ink)
                .mt(if narrow { 6.0 } else { 18.0 })
                .role("status"),
        )
        .child(bar)
        .child(
            col()
                .gap(6.0)
                .mt(4.0)
                .child(step(true, "Recording saved"))
                .child(step(false, "Writing the notes")),
        )
        .child(
            roles::SMALL
                .wrap(
                    if slow {
                        "They will appear in Sidenote when they are ready. You do not need to wait here."
                    } else {
                        "This usually takes a minute or two. The notes open here when they are ready."
                    },
                    p.soft,
                )
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
    let body = match phase {
        Phase::Making => making_view(s, p, narrow),
        Phase::Starting => notes_view(s, k, p, narrow, true),
        Phase::Recording => notes_view(s, k, p, narrow, false),
    };
    let lost = phase == Phase::Recording && !s.hex_running();
    let mut doc = col()
        .key("live")
        .w_full()
        .max_w(720.0)
        .px(if narrow { 20.0 } else { 40.0 })
        .pt(if narrow { 20.0 } else { 40.0 })
        // Room under the editor for the floating bar.
        .pb(if phase == Phase::Making {
            56.0
        } else {
            s.overlay_h.get() + 32.0
        })
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
    doc = doc.child(body);
    let mut pane = col()
        .key("live-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg)
        .child(scroll(
            col().w_full().items_center().child(doc),
            "live-scroll",
        ));
    if phase != Phase::Making {
        pane = pane.child(bar(s, k, p, narrow, layer));
    }
    pane
}
