//! The Hex engine in the Sidenote window: the status the sidebar and the top right show, and a
//! Hex page with the model, the vocabulary and the settings in sections. Starting and stopping a
//! recording is the New meeting flow in `nav` and `live`.
//!
//! No call here runs on the UI thread. Status comes from the engine's `StatusWatcher`, which a
//! forwarder thread owns. Every other call runs on a short thread and comes back as a `Msg`.
use super::{
    Event, State,
    list::text_title,
    roles,
    theme::Palette,
    widgets::{badge, hairline, label, link, pressable, scroll},
};
mod meeting_languages;
use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, Waker, col, row};
use rune_kit::{Kit, Select, SelectAppearance, Switch, TextInput};
use serde_json::Value;
use sidenote::hex_engine::{
    Capabilities, EngineState, Feature, HexEngine, HexError, ModelInfo, ModelState, Recording,
    Setting, Support, VocabularyEntry,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicIsize, Ordering},
        mpsc::{self, RecvTimeoutError, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};

/// How long after a failed capability lookup the next status sample may try again.
const CAPS_RETRY: Duration = Duration::from_secs(30);
/// How often the forwarder looks for a visibility change while the window is visible.
const FORWARD_TICK: Duration = Duration::from_millis(500);

// ---- window visibility ----------------------------------------------------------------------

static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Called once with the window's handle, so the indicator can tell shown from minimized.
pub fn remember_window(hwnd: isize) {
    WINDOW.store(hwnd, Ordering::Relaxed);
}

/// On screen and not minimized. Before the handle is known the window counts as shown.
pub fn window_shown() -> bool {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return true;
    }
    let handle = hwnd as *mut std::ffi::c_void;
    // SAFETY: the handle belongs to this process's window; both calls only read its state.
    unsafe { IsWindowVisible(handle) != 0 && IsIconic(handle) == 0 }
}

// ---- messages from worker threads -----------------------------------------------------------

pub enum Action {
    Start,
    Stop,
    Set(String),
    Add,
    Remove,
}

pub enum Msg {
    Status(Result<EngineState, HexError>),
    Caps(String, Result<Capabilities, HexError>),
    Data {
        settings: Result<Vec<Setting>, HexError>,
        vocabulary: Result<Vec<VocabularyEntry>, HexError>,
        models: Result<Vec<ModelInfo>, HexError>,
    },
    Settings(Result<Vec<Setting>, HexError>),
    AudioSettings(Result<(), HexError>, Result<Vec<Setting>, HexError>),
    Vocabulary(Result<Vec<VocabularyEntry>, HexError>),
    Done(Action, Result<(), HexError>),
}

fn plain(e: &HexError) -> &'static str {
    match e {
        HexError::HexNotRunning => "Hex is not running.",
        HexError::Unsupported => "This Hex build does not support that.",
        HexError::Timeout => "Hex did not answer in time.",
        HexError::Failed { code, .. } => match code.as_deref() {
            Some("unknown-setting") => "Hex does not know that setting.",
            Some("read-only") => "That setting is read only.",
            Some("invalid-value") => "Hex rejected that value.",
            Some("shortcut-in-use") => "That shortcut is in use.",
            Some("feature-off") => "That feature is off in Hex.",
            Some("not-ready") => "Hex is not ready yet.",
            Some("busy") => "Hex is busy.",
            Some("not-recording") => "No meeting is recording.",
            _ => "Hex could not do that.",
        },
    }
}

// ---- state ----------------------------------------------------------------------------------

pub struct HexState {
    engine: HexEngine,
    /// Tells the forwarder thread when the window is shown or hidden.
    visibility: Option<Sender<bool>>,
    visible: bool,
    /// The latest sample. An error replaces the last good state; nothing stale stays on screen.
    pub status: Option<Result<EngineState, HexError>>,
    was_running: bool,
    caps: Option<Capabilities>,
    caps_version: Option<String>,
    caps_loading: bool,
    caps_retry_at: Option<Instant>,
    pub settings: Vec<Setting>,
    pub vocabulary: Vec<VocabularyEntry>,
    pub models: Vec<ModelInfo>,
    settings_note: Option<&'static str>,
    vocabulary_note: Option<&'static str>,
    models_note: Option<&'static str>,
    data_dirty: bool,
    data_loading: bool,
    /// The note under the top bar, on every page: why Record or Stop failed.
    pub error: Option<&'static str>,
    row_errors: HashMap<String, String>,
    vocab_error: Option<&'static str>,
    record_busy: bool,
    vocab_busy: bool,
    busy_keys: HashSet<String>,
    /// Keys whose draft takes the value Hex reports next, because the write just went through.
    fresh: HashSet<String>,
    drafts: HashMap<String, TextEdit>,
    pub filter: TextEdit,
    pub term: TextEdit,
    pub sounds: TextEdit,
    select_open: Option<String>,
    /// The section the page shows, and which Advanced groups the owner opened.
    pub section: Section,
    open_groups: HashSet<&'static str>,
}

impl Default for HexState {
    fn default() -> Self {
        Self {
            engine: HexEngine::new(),
            visibility: None,
            visible: false,
            status: None,
            was_running: false,
            caps: None,
            caps_version: None,
            caps_loading: false,
            caps_retry_at: None,
            settings: Vec::new(),
            vocabulary: Vec::new(),
            models: Vec::new(),
            settings_note: None,
            vocabulary_note: None,
            models_note: None,
            data_dirty: true,
            data_loading: false,
            error: None,
            row_errors: HashMap::new(),
            vocab_error: None,
            record_busy: false,
            vocab_busy: false,
            busy_keys: HashSet::new(),
            fresh: HashSet::new(),
            drafts: HashMap::new(),
            filter: TextEdit::default(),
            term: TextEdit::default(),
            sounds: TextEdit::default(),
            select_open: None,
            section: Section::Model,
            open_groups: HashSet::new(),
        }
    }
}

impl HexState {
    pub fn typing(&self) -> bool {
        self.filter.focused
            || self.term.focused
            || self.sounds.focused
            || self.drafts.values().any(|d| d.focused)
    }

    fn state(&self) -> Option<&EngineState> {
        self.status.as_ref().and_then(|r| r.as_ref().ok())
    }

    fn running(&self) -> bool {
        self.state().is_some()
    }

    fn recording(&self) -> Option<&Recording> {
        self.state().and_then(|s| s.recording.as_ref())
    }

    /// Hex is up and this build has the command.
    fn can(&self, f: Feature) -> bool {
        self.running() && self.caps.as_ref().and_then(|c| c.get(&f)) == Some(&Support::Supported)
    }

    /// Hex is up and the build says it lacks the command.
    fn lacks(&self, f: Feature) -> bool {
        self.running() && self.caps.as_ref().and_then(|c| c.get(&f)) == Some(&Support::Unsupported)
    }

    /// Opens every Advanced group, for a render that shows what is inside.
    pub fn open_all_groups(&mut self) {
        self.open_groups.extend(["labs", "rare"]);
    }

    pub fn draft_mut(&mut self, key: &str) -> &mut TextEdit {
        self.drafts.entry(key.to_string()).or_default()
    }

    fn setting(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.key == key)
    }

    /// Why nothing on the page can be read or changed right now.
    fn page_note(&self) -> Option<&'static str> {
        match &self.status {
            None => Some("Checking Hex."),
            Some(Err(HexError::HexNotRunning)) => {
                Some("Hex is not running. Start it to read or change these.")
            }
            Some(Err(HexError::Timeout)) => Some("Hex did not answer in time."),
            Some(Err(HexError::Unsupported)) => Some("This version of Hex is not supported."),
            Some(Err(HexError::Failed { .. })) => Some("Could not read Hex."),
            Some(Ok(_)) => None,
        }
    }
}

fn setting_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn editable(st: &Setting) -> bool {
    st.writable
        && st.available != Some(false)
        && !st.secret
        && matches!(
            st.kind.as_str(),
            "bool" | "choice" | "number" | "integer" | "text" | "path"
        )
}

fn is_numeric(kind: &str) -> bool {
    matches!(kind, "number" | "integer")
}

/// True when `new` would change the setting. Numbers compare as numbers, so `1` equals `1.0`.
fn changed(st: &Setting, new: &str) -> bool {
    match st.kind.as_str() {
        "bool" => st.value.as_bool().map(|b| b.to_string()).as_deref() != Some(new),
        k if is_numeric(k) => match (st.value.as_f64(), new.parse::<f64>()) {
            (Some(a), Ok(b)) => a != b,
            _ => setting_text(&st.value) != new,
        },
        _ => setting_text(&st.value) != new,
    }
}

fn pretty(key: &str) -> String {
    // Words that read wrong in lower case ("api key") keep their capitals.
    let words = |s: &str| {
        s.split('_')
            .map(|w| match w {
                "api" | "url" | "id" | "ui" | "gpu" | "cpu" => w.to_uppercase(),
                _ => w.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let text = match key.split_once('.') {
        Some((head, rest)) => format!("{}: {}", words(head), words(rest)),
        None => words(key),
    };
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn elapsed(r: &Recording) -> Option<u64> {
    r.since
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
        .or(r.elapsed_seconds)
}

// ---- actions --------------------------------------------------------------------------------

impl State {
    pub fn on_hex(&self) -> bool {
        self.hex_open && self.view == super::tasks::View::Meetings && self.selected.is_none()
    }

    pub fn open_hex(&mut self) {
        self.go_home();
        self.hex_open = true;
        self.hex.data_dirty = true;
        self.hex.select_open = None;
        self.hex_refresh_if_due();
    }

    /// Runs `work` on its own thread and delivers the result to the UI.
    fn hex_spawn(&self, work: impl FnOnce(HexEngine) -> Msg + Send + 'static) {
        let engine = self.hex.engine.clone();
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        let _ = thread::Builder::new()
            .name("hex-call".into())
            .spawn(move || {
                let _ = events.send(Event::Hex(work(engine)));
                if let Some(w) = &waker {
                    w();
                }
            });
    }

    /// Starts the status watcher. Its forwarder thread owns the watcher and reads its updates.
    pub fn hex_start(&mut self, waker: Waker) {
        let watcher = match self.hex.engine.watch() {
            Ok(w) => w,
            Err(e) => {
                self.hex.status = Some(Err(e));
                return;
            }
        };
        let (commands, input) = mpsc::channel::<bool>();
        let events = self.events_tx.clone();
        let started = thread::Builder::new()
            .name("hex-ui-status".into())
            .spawn(move || {
                let mut visible = false;
                loop {
                    // Hidden, the thread waits for a command and does nothing else.
                    let mut want = visible;
                    if visible {
                        match watcher.updates.recv_timeout(FORWARD_TICK) {
                            Ok(update) => {
                                if events.send(Event::Hex(Msg::Status(update))).is_err() {
                                    return;
                                }
                                waker();
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                        loop {
                            match input.try_recv() {
                                Ok(v) => want = v,
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => return,
                            }
                        }
                    } else {
                        match input.recv() {
                            Ok(v) => want = v,
                            Err(_) => return,
                        }
                    }
                    if want != visible {
                        if want {
                            // A sample left over from before the window hid would block the fresh one.
                            while watcher.updates.try_recv().is_ok() {}
                        }
                        watcher.set_visible(want);
                        visible = want;
                    }
                }
            });
        if started.is_ok() {
            self.hex.visibility = Some(commands);
            self.hex_visibility(window_shown());
        }
    }

    /// Tells the watcher the window is shown or hidden. It makes no Hex calls while hidden.
    pub fn hex_visibility(&mut self, visible: bool) {
        if self.hex.visible == visible {
            return;
        }
        self.hex.visible = visible;
        if let Some(tx) = &self.hex.visibility {
            let _ = tx.send(visible);
        }
    }

    pub fn hex_apply(&mut self, msg: Msg) {
        match msg {
            Msg::Status(r) => self.hex_status(r),
            Msg::Caps(version, r) => {
                self.hex.caps_loading = false;
                match r {
                    Ok(caps) => {
                        self.hex.caps = Some(caps);
                        self.hex.caps_version = Some(version);
                    }
                    Err(_) => self.hex.caps_retry_at = Some(Instant::now() + CAPS_RETRY),
                }
            }
            Msg::Data {
                settings,
                vocabulary,
                models,
            } => {
                self.hex.data_loading = false;
                self.hex_set_settings(settings);
                self.hex_set_vocabulary(vocabulary);
                match models {
                    Ok(m) => {
                        self.hex.models = m;
                        self.hex.models_note = None;
                    }
                    Err(e) => self.hex.models_note = Some(plain(&e)),
                }
            }
            Msg::Settings(r) => self.hex_set_settings(r),
            Msg::AudioSettings(result, settings) => {
                self.hex.busy_keys.remove("meetings.audio_retention");
                self.hex_set_settings(settings);
                if let Err(e) = result {
                    self.hex
                        .row_errors
                        .insert("meetings.audio_retention".into(), plain(&e).into());
                }
            }
            Msg::Vocabulary(r) => self.hex_set_vocabulary(r),
            Msg::Done(action, r) => self.hex_done(action, r),
        }
    }

    fn hex_status(&mut self, r: Result<EngineState, HexError>) {
        let running = r.is_ok();
        if running && !self.hex.was_running {
            self.hex.data_dirty = true;
        }
        self.hex.was_running = running;
        if let Ok(st) = &r
            && self.hex.caps_version.as_deref() != Some(st.version.as_str())
        {
            self.hex.caps = None;
            if !self.hex.caps_loading && self.hex.caps_retry_at.is_none_or(|t| Instant::now() >= t)
            {
                self.hex.caps_loading = true;
                let version = st.version.clone();
                self.hex_spawn(move |e| Msg::Caps(version, e.capabilities()));
            }
        }
        self.hex.status = Some(r);
        self.live_sync();
        self.hex_refresh_if_due();
        // A minimize can slip past the focus event; this catches it on the next sample.
        if !window_shown() {
            self.hex_visibility(false);
        }
    }

    fn hex_refresh_if_due(&mut self) {
        if self.on_hex() && self.hex.data_dirty && self.hex.running() && !self.hex.data_loading {
            self.hex.data_dirty = false;
            self.hex.data_loading = true;
            self.hex_spawn(|e| Msg::Data {
                settings: e.settings_list(),
                vocabulary: e.vocabulary_list(),
                models: e.models_list(),
            });
        }
    }

    fn hex_set_settings(&mut self, r: Result<Vec<Setting>, HexError>) {
        let list = match r {
            Ok(list) => list,
            Err(e) => {
                self.hex.settings_note = Some(plain(&e));
                return;
            }
        };
        self.hex.settings_note = None;
        // A draft the owner changed and has not saved survives the refresh.
        let kept: HashSet<String> = self
            .hex
            .drafts
            .iter()
            .filter(|(k, d)| {
                !self.hex.fresh.contains(*k)
                    && self
                        .hex
                        .setting(k)
                        .is_some_and(|old| d.text != setting_text(&old.value))
            })
            .map(|(k, _)| k.clone())
            .collect();
        self.hex
            .drafts
            .retain(|k, _| list.iter().any(|s| &s.key == k));
        for st in &list {
            if editable(st)
                && !matches!(st.kind.as_str(), "bool" | "choice")
                && !kept.contains(&st.key)
            {
                self.hex
                    .drafts
                    .insert(st.key.clone(), TextEdit::new(setting_text(&st.value)));
            }
        }
        self.hex.fresh.clear();
        self.hex.settings = list;
    }

    fn hex_set_vocabulary(&mut self, r: Result<Vec<VocabularyEntry>, HexError>) {
        match r {
            Ok(v) => {
                self.hex.vocabulary = v;
                self.hex.vocabulary_note = None;
            }
            Err(e) => self.hex.vocabulary_note = Some(plain(&e)),
        }
    }

    fn hex_done(&mut self, action: Action, r: Result<(), HexError>) {
        match action {
            Action::Start | Action::Stop => {
                self.hex.record_busy = false;
                match r {
                    Ok(()) => {
                        self.hex.error = None;
                        if matches!(action, Action::Stop) {
                            // The notes follow; the next sample confirms Hex is idle.
                            self.live_stopped();
                        }
                        self.hex_spawn(|e| Msg::Status(e.status()));
                    }
                    Err(e) => {
                        self.hex.error = Some(plain(&e));
                        // A start that failed leaves nothing to show on the live page.
                        if matches!(action, Action::Start) {
                            self.live = None;
                            self.live_open = false;
                        }
                    }
                }
            }
            Action::Set(key) => {
                self.hex.busy_keys.remove(&key);
                match r {
                    Ok(()) => {
                        self.hex.fresh.insert(key);
                        self.hex_spawn(|e| Msg::Settings(e.settings_list()));
                    }
                    Err(e) => {
                        self.hex.row_errors.insert(key, plain(&e).into());
                    }
                }
            }
            Action::Add | Action::Remove => {
                self.hex.vocab_busy = false;
                match r {
                    Ok(()) => {
                        self.hex.vocab_error = None;
                        self.hex_spawn(|e| Msg::Vocabulary(e.vocabulary_list()));
                    }
                    Err(e) => self.hex.vocab_error = Some(plain(&e)),
                }
            }
        }
    }

    pub fn hex_recording(&self) -> bool {
        self.hex.recording().is_some()
    }

    pub fn hex_running(&self) -> bool {
        self.hex.running()
    }

    pub fn hex_record_busy(&self) -> bool {
        self.hex.record_busy
    }

    pub fn hex_record_start(&mut self) {
        if self.hex.record_busy
            || !self.hex.can(Feature::RecordStart)
            || self.hex.recording().is_some()
        {
            return;
        }
        self.hex.record_busy = true;
        self.hex.error = None;
        self.hex_spawn(|e| Msg::Done(Action::Start, e.record_start(None)));
    }

    pub fn hex_record_stop(&mut self) {
        // Notes typed on the live page reach the folder before the pipeline can start.
        self.live_flush();
        if self.hex.record_busy
            || !self.hex.can(Feature::RecordStop)
            || self.hex.recording().is_none()
        {
            return;
        }
        self.hex.record_busy = true;
        self.hex.error = None;
        self.hex_spawn(|e| Msg::Done(Action::Stop, e.record_stop()));
    }

    /// Sends one setting, and only when the value differs from what Hex reports.
    fn hex_set(&mut self, key: &str, value: String) {
        if !self.hex.can(Feature::SettingsSet) || self.hex.busy_keys.contains(key) {
            return;
        }
        let Some(st) = self.hex.setting(key) else {
            return;
        };
        if !editable(st) || !changed(st, &value) {
            return;
        }
        self.hex.busy_keys.insert(key.to_string());
        self.hex.row_errors.remove(key);
        let key = key.to_string();
        self.hex_spawn(move |e| {
            let r = e.settings_set(&key, &value);
            Msg::Done(Action::Set(key), r)
        });
    }

    fn hex_keep_audio(&mut self, choice: &str) {
        self.hex.select_open = None;
        let value = match choice {
            "24 hours" => "day",
            "7 days" => "week",
            "1 month" => "month",
            "Forever" => "forever",
            _ => return,
        };
        if !self.hex.can(Feature::SettingsSet)
            || self.hex.settings_note.is_some()
            || self.hex.busy_keys.contains("meetings.audio_retention")
            || !["meetings.audio_retention", "labs.expire_meeting_audio"]
                .iter()
                .all(|key| self.hex.setting(key).is_some_and(editable))
        {
            return;
        }
        self.hex.busy_keys.insert("meetings.audio_retention".into());
        self.hex.row_errors.remove("meetings.audio_retention");
        self.hex_spawn(move |e| {
            let result = e
                .settings_set("meetings.audio_retention", value)
                .and_then(|()| {
                    e.settings_set(
                        "labs.expire_meeting_audio",
                        if value == "forever" { "false" } else { "true" },
                    )
                });
            // Even a partial write must show the values Hex actually kept.
            let settings = e.settings_list();
            Msg::AudioSettings(result, settings)
        });
    }

    fn hex_toggle(&mut self, key: &str) {
        let Some(on) = self.hex.setting(key).and_then(|s| s.value.as_bool()) else {
            return;
        };
        self.hex_set(key, (!on).to_string());
    }

    fn hex_choose(&mut self, key: &str, value: &str) {
        self.hex.select_open = None;
        self.hex_set(key, value.to_string());
    }

    fn hex_commit(&mut self, key: &str) {
        let Some(st) = self.hex.setting(key) else {
            return;
        };
        let text = self
            .hex
            .drafts
            .get(key)
            .map(|d| d.text.trim().to_string())
            .unwrap_or_default();
        let valid = match st.kind.as_str() {
            "integer" => text.parse::<i64>().is_ok(),
            "number" => text.parse::<f64>().is_ok_and(f64::is_finite),
            _ => true,
        };
        if !valid {
            self.hex
                .row_errors
                .insert(key.to_string(), "Enter a number.".into());
            return;
        }
        self.hex_set(key, text);
    }

    fn hex_revert(&mut self, key: &str) {
        let text = self
            .hex
            .setting(key)
            .map(|s| setting_text(&s.value))
            .unwrap_or_default();
        self.hex.drafts.insert(key.to_string(), TextEdit::new(text));
        self.hex.row_errors.remove(key);
    }

    fn hex_vocab_add(&mut self) {
        let term = self.hex.term.text.trim().to_string();
        if term.is_empty() || self.hex.vocab_busy || !self.hex.can(Feature::VocabularyAdd) {
            return;
        }
        let sounds: Vec<String> = self
            .hex
            .sounds
            .text
            .split(',')
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .collect();
        self.hex.vocab_busy = true;
        self.hex.vocab_error = None;
        self.hex_spawn(move |e| {
            let sounds: Vec<&str> = sounds.iter().map(String::as_str).collect();
            let r = e.vocabulary_add(&term, &sounds);
            Msg::Done(Action::Add, r)
        });
        self.hex.term = TextEdit::default();
        self.hex.sounds = TextEdit::default();
    }

    fn hex_vocab_remove(&mut self, term: &str) {
        if self.hex.vocab_busy || !self.hex.can(Feature::VocabularyRemove) {
            return;
        }
        self.hex.vocab_busy = true;
        self.hex.vocab_error = None;
        let term = term.to_string();
        self.hex_spawn(move |e| {
            let r = e.vocabulary_remove(&term);
            Msg::Done(Action::Remove, r)
        });
    }
}

// ---- the status words -----------------------------------------------------------------------

/// The dot and words for the indicator, from the latest sample.
fn indicator(s: &State, p: Palette) -> (Color, String) {
    match &s.hex.status {
        None => (p.faint, "Checking Hex".into()),
        Some(Err(HexError::HexNotRunning)) => (p.faint, "Hex not running".into()),
        Some(Err(HexError::Timeout)) => (p.warn, "Hex not answering".into()),
        Some(Err(_)) => (p.warn, "Hex unavailable".into()),
        Some(Ok(st)) => match (&st.recording, st.dictating) {
            (Some(r), _) => (
                p.warn,
                match elapsed(r) {
                    Some(sec) => format!("Recording {}", clock(sec)),
                    None => "Recording".into(),
                },
            ),
            (None, Some(true)) => (p.open, "Dictating".into()),
            (None, _) => (p.ok, "Idle".into()),
        },
    }
}

/// The dot and short words for the sidebar's Hex line.
pub fn short_status(s: &State, p: Palette) -> (Color, String) {
    match &s.hex.status {
        None => (p.faint, "Checking".into()),
        Some(Err(HexError::HexNotRunning)) => (p.faint, "Not running".into()),
        Some(Err(HexError::Timeout)) => (p.warn, "Not answering".into()),
        Some(Err(_)) => (p.warn, "Unavailable".into()),
        Some(Ok(_)) => {
            let (ink, words) = indicator(s, p);
            (ink, words)
        }
    }
}

/// The id Hex gives the running recording, which is the name of its meeting folder.
pub fn recording_id(s: &State) -> Option<String> {
    s.hex.recording().and_then(|r| r.id.clone())
}

/// The elapsed time of the running recording, as `12:34`.
pub fn recording_clock(s: &State) -> Option<String> {
    s.hex.recording().and_then(elapsed).map(clock)
}

/// When the running recording began, by the wall clock. The flag is true when Hex gave the start
/// itself, and false when it is worked back from the elapsed seconds (which can drift a little).
pub fn recording_start(s: &State) -> Option<(chrono::DateTime<chrono::Local>, bool)> {
    let r = s.hex.recording()?;
    if let Some(t) = r.since {
        return Some((t.into(), true));
    }
    let secs = i64::try_from(r.elapsed_seconds?).ok()?;
    Some((
        chrono::Local::now() - chrono::Duration::seconds(secs),
        false,
    ))
}

/// Why a new recording cannot start right now, in words fit for a tooltip.
pub fn start_blocked(s: &State) -> Option<&'static str> {
    let h = &s.hex;
    if !h.running() {
        Some("Hex is not running")
    } else if h.recording().is_some() {
        Some("Already recording")
    } else if h.record_busy {
        Some("Working")
    } else if h.lacks(Feature::RecordStart) {
        Some("This Hex build cannot record")
    } else if !h.can(Feature::RecordStart) {
        Some("Checking what Hex supports")
    } else {
        None
    }
}

/// Why the recording cannot be stopped right now.
pub fn stop_blocked(s: &State) -> Option<&'static str> {
    let h = &s.hex;
    if !h.running() {
        Some("Hex is not running")
    } else if h.recording().is_none() {
        Some("No meeting is recording")
    } else if h.record_busy {
        Some("Working")
    } else if !h.can(Feature::RecordStop) {
        Some("This Hex build cannot stop")
    } else {
        None
    }
}

/// Why Record or Stop failed, with a Dismiss link. Not a dialog.
pub fn error_bar(s: &State, p: Palette) -> Option<El> {
    let note = s.hex.error?;
    Some(
        row()
            .key("hex-error")
            .px(24.0)
            .py(8.0)
            .gap(12.0)
            .items_center()
            .fill(p.warnbg)
            .child(label(note, 13.5, 500.0, p.warn).grow(1.0).role("alert"))
            .child(
                link("Dismiss", p)
                    .key("hex-error-dismiss")
                    .on_press(|s: &mut State| s.hex.error = None),
            ),
    )
}

// ---- the page -------------------------------------------------------------------------------

fn note(text: &str, p: Palette) -> El {
    label(text, 13.5, 500.0, p.warn).role("alert").mb(8.0)
}

fn summary_line(s: &State, p: Palette) -> El {
    let (_, words) = indicator(s, p);
    let text = match s.hex.state() {
        Some(st) => format!(
            "Sidenote {}. Hex {}. {}.",
            sidenote::updates::VERSION,
            st.version,
            words
        ),
        None => format!("Sidenote {}. {words}.", sidenote::updates::VERSION),
    };
    label(text, 14.0, 400.0, p.soft)
}

fn model_section(s: &State, p: Palette) -> El {
    let h = &s.hex;
    let mut sec = col()
        .key("hex-models")
        .w_full()
        .child(section_head(Section::Model, None, p));
    // The list below marks the selected model, so this line only stands in when it is empty.
    if let Some(st) = h.state().filter(|_| h.models.is_empty()) {
        let m = &st.model;
        let name = m
            .name
            .clone()
            .or_else(|| m.id.clone())
            .unwrap_or_else(|| "Unknown".into());
        let mut r = row()
            .w_full()
            .gap(8.0)
            .items_center()
            .flex_wrap()
            .py(6.0)
            .child(
                roles::SMALL
                    .medium()
                    .wrap(format!("Selected: {name}"), p.ink),
            );
        r = r.child(match m.installed {
            Some(true) => badge("Installed", p.ok, Some(p.okbg)),
            Some(false) => badge("Not installed", p.warn, Some(p.warnbg)),
            None => badge("Install state unknown", p.soft, Some(p.chip)),
        });
        if m.preparing.is_some() {
            r = r.child(badge("Preparing", p.accent, Some(p.chip)));
        }
        sec = sec.child(r);
    }
    if let Some(n) = h.models_note.filter(|_| h.running()) {
        sec = sec.child(note(n, p));
    }
    for m in &h.models {
        let mut r = row()
            .key(format!("hex-model-{}", m.id))
            .w_full()
            .gap(8.0)
            .py(5.0)
            .items_center()
            .flex_wrap()
            .child(
                label(
                    m.name.clone(),
                    14.0,
                    if m.selected { 600.0 } else { 400.0 },
                    p.ink,
                )
                .w(170.0),
            )
            .child(
                roles::META
                    .wrap(format!("{}  {}", m.languages.join(", "), m.size), p.faint)
                    .w(190.0),
            )
            .child(
                if m.installed {
                    badge("Installed", p.ok, None)
                } else {
                    badge("Not installed", p.faint, None)
                }
                .w(100.0),
            );
        if m.selected {
            r = r.child(badge("Selected", p.accent, Some(p.chip)));
        }
        sec = sec.child(r);
    }
    sec
}

fn vocabulary_section(s: &State, k: &Kit, p: Palette) -> El {
    let h = &s.hex;
    let can_add = h.can(Feature::VocabularyAdd) && !h.vocab_busy;
    let framed = |e: El, focused: bool| {
        e.border(1.0, if focused { p.accent } else { p.line })
            .radius(8.0)
            .fill(p.card)
    };
    let term = framed(
        TextInput::new(&h.term)
            .plain()
            .placeholder("Word or phrase")
            .label("New vocabulary word")
            .height(36.0)
            .disabled(!can_add)
            .on_edit(|s: &mut State| &mut s.hex.term)
            .on_submit(|s: &mut State, _| s.hex_vocab_add())
            .build(k)
            .key("hex-term"),
        h.term.focused,
    )
    .grow(1.0)
    .min_w(140.0);
    let sounds = framed(
        TextInput::new(&h.sounds)
            .plain()
            .placeholder("Sounds like, optional, comma separated")
            .label("Sounds like")
            .height(36.0)
            .disabled(!can_add)
            .on_edit(|s: &mut State| &mut s.hex.sounds)
            .on_submit(|s: &mut State, _| s.hex_vocab_add())
            .build(k)
            .key("hex-sounds"),
        h.sounds.focused,
    )
    .grow(1.0)
    .min_w(140.0);
    let ready = can_add && !h.term.text.trim().is_empty();
    let mut add = pressable(
        roles::CONTROL
            .bold()
            .wrap("Add", if ready { p.card } else { p.faint }),
    )
    .key("hex-vocab-add")
    .label("Add the word")
    .px(14.0)
    .py(8.0)
    .radius(8.0)
    .fill(if ready { p.accent } else { p.chip });
    if ready {
        add = add.on_press(|s: &mut State| s.hex_vocab_add());
    } else {
        add = add.dim(0.6);
    }
    let mut sec = col()
        .key("hex-vocabulary")
        .w_full()
        .child(section_head(
            Section::Vocabulary,
            Some(h.vocabulary.len()),
            p,
        ))
        .child(
            row()
                .w_full()
                .flex_wrap()
                .items_center()
                .gaps(8.0, 8.0)
                .mb(8.0)
                .child(term)
                .child(sounds)
                .child(add),
        );
    if h.lacks(Feature::VocabularyAdd) {
        sec = sec.child(note("This Hex build cannot add words.", p));
    }
    if let Some(n) = h.vocabulary_note.filter(|_| h.running()).or(h.vocab_error) {
        sec = sec.child(note(n, p));
    }
    if h.vocabulary.is_empty() && h.vocabulary_note.is_none() && h.running() {
        sec = sec.child(roles::SMALL.wrap("No words yet.", p.soft));
    }
    let can_remove = h.can(Feature::VocabularyRemove) && !h.vocab_busy;
    for (i, v) in h.vocabulary.iter().enumerate() {
        let mut text = col()
            .grow(1.0)
            .min_w(0.0)
            .child(roles::CONTROL.line(v.term.clone(), p.ink));
        if !v.sounds_like.is_empty() {
            text = text.child(
                roles::META.line(format!("Sounds like {}", v.sounds_like.join(", ")), p.faint),
            );
        }
        let term = v.term.clone();
        let mut remove = pressable(roles::CONTROL.wrap("Remove", p.soft))
            .key(format!("hex-vocab-rm-{i}"))
            .label(format!("Remove {}", v.term))
            .px(8.0)
            .py(2.0)
            .radius(6.0);
        if can_remove {
            remove = remove
                .hover(p.chip)
                .on_press(move |s: &mut State| s.hex_vocab_remove(&term));
        } else {
            remove = remove.dim(0.5);
        }
        sec = sec
            .child(
                row()
                    .key(format!("hex-vocab-{i}"))
                    .w_full()
                    .py(6.0)
                    .gap(12.0)
                    .items_center()
                    .child(text)
                    .child(remove),
            )
            .child(hairline(p));
    }
    sec
}

// ---- sections ---------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Updates,
    MeetingModels,
    Model,
    Vocabulary,
    General,
    Dictation,
    Meetings,
    Shortcuts,
    Advanced,
    Features,
}

/// The tabs, left to right. Features comes first: it is the one that shapes the app itself.
const SECTIONS: [Section; 10] = [
    Section::Features,
    Section::Updates,
    Section::MeetingModels,
    Section::Model,
    Section::Vocabulary,
    Section::General,
    Section::Dictation,
    Section::Meetings,
    Section::Shortcuts,
    Section::Advanced,
];

impl Section {
    /// Every tab, in the order the page shows them.
    pub const ALL: [Section; 10] = SECTIONS;

    pub fn name(self) -> &'static str {
        match self {
            Self::Updates => "Updates",
            Self::MeetingModels => "Languages",
            Self::Features => "Features",
            Self::Model => "Model",
            Self::Vocabulary => "Vocabulary",
            Self::General => "General",
            Self::Dictation => "Dictation",
            Self::Meetings => "Meetings",
            Self::Shortcuts => "Shortcuts",
            Self::Advanced => "Advanced",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Updates => "Keep Sidenote up to date from your update folder.",
            Self::MeetingModels => "Choose spoken languages and download meeting models.",
            Self::Features => "Choose what appears on this device. Changes apply right away.",
            Self::Model => "The speech model Hex uses to write down what it hears.",
            Self::Vocabulary => "Names and terms Hex should spell the way you do.",
            Self::General => "How Hex starts and how it sounds.",
            Self::Dictation => "Speaking into any app.",
            Self::Meetings => "How Hex records and saves meetings.",
            Self::Shortcuts => "Keys that start, stop and paste.",
            Self::Advanced => "Experimental and rarely used. The defaults suit most people.",
        }
    }
}

/// Where a setting is listed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    In(Section),
    /// Under Advanced: experimental features.
    Labs,
    /// Under Advanced: read only, unavailable, hidden, or not one of the everyday kinds.
    Rare,
}

impl Place {
    fn group_name(self) -> &'static str {
        match self {
            Self::In(s) => s.name(),
            Self::Labs => "Experimental (Labs)",
            Self::Rare => "Rarely used",
        }
    }
}

fn place(st: &Setting) -> Place {
    let key = st.key.as_str();
    let head = key.split('.').next().unwrap_or_default();
    if head == "labs" {
        return Place::Labs;
    }
    // Nothing here can be changed, or it is not on offer, so it is not an everyday setting.
    if !st.writable || st.available == Some(false) || st.secret {
        return Place::Rare;
    }
    if st.kind == "shortcut"
        || key.contains("shortcut")
        || key.contains("hotkey")
        || matches!(head, "shortcuts" | "hotkeys")
    {
        return Place::In(Section::Shortcuts);
    }
    match head {
        "dictation" => Place::In(Section::Dictation),
        "meetings" | "meeting" => Place::In(Section::Meetings),
        "general" | "app" | "appearance" | "startup" | "audio" => Place::In(Section::General),
        _ => Place::Rare,
    }
}

/// The name a person would give it: "Launch at login", without the section it sits in.
fn human(st: &Setting, at: Place) -> String {
    if st.key == "meetings.retention" {
        return "Keep whole meetings".into();
    }
    match (at, st.key.split_once('.')) {
        (Place::In(_) | Place::Labs, Some((_, rest))) => pretty(rest),
        _ => pretty(&st.key),
    }
}

/// The first sentence of what Hex says about a setting.
fn one_line(about: &str) -> String {
    match about.split_once(". ") {
        Some((first, _)) => format!("{}.", first.trim_end_matches('.')),
        None => about.to_string(),
    }
}

fn section_head(section: Section, count: Option<usize>, p: Palette) -> El {
    let mut top = row()
        .gap(8.0)
        .items_center()
        .child(roles::SECTION.wrap(section.name(), p.ink));
    if let Some(n) = count {
        top = top.child(roles::count(n, p));
    }
    col()
        .key(format!("hex-head-{}", section.name()))
        .w_full()
        .gap(2.0)
        .mt(22.0)
        .mb(10.0)
        .child(top)
        .child(roles::SMALL.wrap(section.blurb(), p.soft))
}

fn section_tab(section: Section, active: bool, p: Palette) -> El {
    pressable(label(
        section.name(),
        13.5,
        if active { 600.0 } else { 500.0 },
        if active { p.ink } else { p.soft },
    ))
    .key(format!("hex-tab-{}", section.name()))
    .label(format!("{} settings", section.name()))
    .px(14.0)
    .py(6.0)
    .radius(999.0)
    .fill(if active { p.chip } else { p.bg })
    .border(1.0, if active { p.line } else { p.bg })
    .hover(p.chip)
    .on_press(move |s: &mut State| {
        s.hex.section = section;
        s.hex.filter = TextEdit::default();
        s.hex.select_open = None;
    })
}

fn tabs(s: &State, p: Palette) -> El {
    let searching = !s.hex.filter.text.trim().is_empty();
    let mut r = row().key("hex-tabs").w_full().flex_wrap().gaps(4.0, 4.0);
    for section in SECTIONS {
        r = r.child(section_tab(
            section,
            !searching && s.hex.section == section,
            p,
        ));
    }
    r
}

fn search_box(s: &State, k: &Kit, p: Palette) -> El {
    let h = &s.hex;
    TextInput::new(&h.filter)
        .plain()
        .placeholder("Search all settings")
        .label("Search all settings")
        .on_edit(|s: &mut State| &mut s.hex.filter)
        .on_key(|s: &mut State, e: &KeyEvent, tx: &mut Tx| {
            if e.key == Key::Escape {
                s.hex.filter = TextEdit::default();
                tx.blur();
                return true;
            }
            false
        })
        .height(38.0)
        .build(k)
        .key("hex-filter")
        .border(1.0, if h.filter.focused { p.accent } else { p.line })
        .radius(10.0)
        .fill(p.card)
        .w_full()
}

/// What stands in for the rows when there are none to show.
fn empty_note(s: &State, p: Palette, text_: &'static str) -> Option<El> {
    let h = &s.hex;
    (h.running() && h.settings_note.is_none()).then(|| roles::SMALL.wrap(text_, p.soft).py(8.0))
}

fn state_notes(s: &State, p: Palette) -> Vec<El> {
    let h = &s.hex;
    let mut notes = Vec::new();
    if h.lacks(Feature::SettingsSet) {
        notes.push(note("This Hex build cannot change settings.", p));
    }
    if let Some(n) = h.settings_note.filter(|_| h.running()) {
        notes.push(note(n, p));
    }
    notes
}

fn rows(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, items: &[&Setting]) -> El {
    let mut list = col().w_full();
    if items
        .iter()
        .any(|st| st.key == "meetings.audio_retention" || st.key == "labs.expire_meeting_audio")
    {
        list = list
            .child(keep_audio_row(s, k, p, layer))
            .child(hairline(p));
    }
    for st in items {
        if matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        ) {
            continue;
        }
        list = list
            .child(setting_row(s, k, st, place(st), p, layer))
            .child(hairline(p));
    }
    list
}

/// A fold-away group under Advanced. Closed unless the owner opened it.
fn fold(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, at: Place, items: &[&Setting]) -> El {
    let (id, blurb) = match at {
        Place::Labs => ("labs", "Unfinished features. They can change or go away."),
        _ => (
            "rare",
            "Read only, hidden or unavailable here, and a few that need care.",
        ),
    };
    let open = s.hex.open_groups.contains(id);
    let chevron = if open { "\u{25be}" } else { "\u{25b8}" };
    let header = pressable(
        row()
            .w_full()
            .gap(10.0)
            .items_center()
            .px(8.0)
            .py(10.0)
            .child(roles::CONTROL.wrap(chevron, p.soft).w(12.0))
            .child(roles::TITLE.wrap(at.group_name(), p.ink))
            .child(roles::count(items.len(), p))
            .child(roles::META.line(blurb, p.faint).grow(1.0).ml(6.0)),
    )
    .key(format!("hex-fold-{id}"))
    .label(format!(
        "{}, {} settings. {}",
        at.group_name(),
        items.len(),
        if open { "Collapse" } else { "Expand" }
    ))
    .radius(8.0)
    .hover(p.chip)
    .on_press(move |s: &mut State| {
        if !s.hex.open_groups.remove(id) {
            s.hex.open_groups.insert(id);
        }
    });
    let mut g = col()
        .key(format!("hex-group-{id}"))
        .w_full()
        .child(header)
        .child(hairline(p));
    if open {
        g = g.child(rows(s, k, p, layer, items));
    }
    g
}

/// Every setting that matches the search, whichever section it is in.
fn search_results(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, query: &str) -> El {
    let h = &s.hex;
    let matches: Vec<&Setting> = h
        .settings
        .iter()
        .filter(|st| st.key != "labs.expire_meeting_audio")
        .filter(|st| {
            (st.key == "meetings.audio_retention" && "keep audio".contains(query))
                || st.key.to_lowercase().contains(query)
                || st.about.to_lowercase().contains(query)
                || pretty(&st.key).to_lowercase().contains(query)
        })
        .collect();
    let mut c = col().key("hex-results").w_full().gap(2.0).mt(18.0);
    for n in state_notes(s, p) {
        c = c.child(n);
    }
    if matches.is_empty() {
        let text_ = if h.settings.is_empty() {
            "No settings loaded."
        } else {
            "No settings match."
        };
        if let Some(n) = empty_note(s, p, text_) {
            c = c.child(n);
        }
        return c;
    }
    c = c.child(roles::CONTROL.wrap(
        format!(
            "{} setting{} match",
            matches.len(),
            if matches.len() == 1 { "" } else { "s" }
        ),
        p.soft,
    ));
    for at in [
        Place::In(Section::General),
        Place::In(Section::Dictation),
        Place::In(Section::Meetings),
        Place::In(Section::Shortcuts),
        Place::Labs,
        Place::Rare,
    ] {
        let group: Vec<&Setting> = matches
            .iter()
            .copied()
            .filter(|st| place(st) == at)
            .collect();
        if group.is_empty() {
            continue;
        }
        let title = match at {
            Place::In(_) => at.group_name().to_string(),
            _ => format!("Advanced \u{b7} {}", at.group_name()),
        };
        c = c
            .child(roles::CAPTION.wrap(title, p.faint).mt(16.0).mb(2.0))
            .child(hairline(p))
            .child(rows(s, k, p, layer, &group));
    }
    c
}

/// The page for one section.
fn section_body(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, section: Section) -> El {
    match section {
        Section::Updates => super::updates::section(s, p),
        Section::MeetingModels => {
            super::models::view(s, p, meeting_languages::section(s, k, p, layer))
        }
        Section::Features => features_section(s, k, p),
        Section::Model => model_section(s, p),
        Section::Vocabulary => vocabulary_section(s, k, p),
        Section::Advanced => {
            let h = &s.hex;
            let labs: Vec<&Setting> = h
                .settings
                .iter()
                .filter(|st| place(st) == Place::Labs && st.key != "labs.expire_meeting_audio")
                .collect();
            let rare: Vec<&Setting> = h
                .settings
                .iter()
                .filter(|st| place(st) == Place::Rare)
                .collect();
            let mut c =
                col()
                    .key("hex-advanced")
                    .w_full()
                    .child(section_head(Section::Advanced, None, p));
            for n in state_notes(s, p) {
                c = c.child(n);
            }
            if labs.is_empty()
                && rare.is_empty()
                && let Some(n) =
                    empty_note(s, p, "Nothing here. Everything is in the other sections.")
            {
                c = c.child(n);
            }
            if !labs.is_empty() {
                c = c.child(fold(s, k, p, layer, Place::Labs, &labs));
            }
            if !rare.is_empty() {
                c = c.child(fold(s, k, p, layer, Place::Rare, &rare));
            }
            c
        }
        _ => {
            let items: Vec<&Setting> = s
                .hex
                .settings
                .iter()
                .filter(|st| {
                    place(st) == Place::In(section) && st.key != "meetings.audio_retention"
                })
                .collect();
            let mut c = col()
                .key(format!("hex-section-{}", section.name()))
                .w_full()
                .child(section_head(section, None, p));
            for n in state_notes(s, p) {
                c = c.child(n);
            }
            if section == Section::Meetings {
                c = c.child(hairline(p)).child(keep_audio_row(s, k, p, layer));
            }
            if items.is_empty() {
                if let Some(n) = empty_note(s, p, "No settings here on this build of Hex.") {
                    c = c.child(n);
                }
                return c;
            }
            c.child(hairline(p)).child(rows(s, k, p, layer, &items))
        }
    }
}

fn keep_audio_row(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let h = &s.hex;
    let audio = h.setting("meetings.audio_retention");
    let enabled = h
        .setting("labs.expire_meeting_audio")
        .and_then(|st| st.value.as_bool());
    let current = match enabled {
        Some(false) => Some("Forever"),
        Some(true) => match audio.and_then(|st| st.value.as_str()) {
            Some("day") => Some("24 hours"),
            Some("week") => Some("7 days"),
            Some("month") => Some("1 month"),
            Some("forever") => Some("Forever"),
            _ => None,
        },
        None => None,
    };
    let busy = h.busy_keys.contains("meetings.audio_retention");
    let can = h.can(Feature::SettingsSet)
        && h.settings_note.is_none()
        && !busy
        && ["meetings.audio_retention", "labs.expire_meeting_audio"]
            .iter()
            .all(|key| h.setting(key).is_some_and(editable));
    let mut text = col()
        .grow(1.0)
        .min_w(220.0)
        .gap(2.0)
        .child(roles::CONTROL.wrap("Keep audio", p.ink))
        .child(roles::META.line("Transcripts, notes and summaries stay.", p.faint));
    if let Some(reason) = h.page_note().or(h.settings_note) {
        text = text.child(roles::META.wrap(reason, p.faint));
    } else if !can && !busy {
        text = text.child(roles::META.wrap("This Hex build cannot change audio expiry.", p.faint));
    }
    if busy {
        text = text.child(roles::META.wrap("Saving.", p.faint));
    }
    if let Some(error) = h.row_errors.get("meetings.audio_retention") {
        text = text.child(
            roles::META
                .medium()
                .wrap(error.clone(), p.warn)
                .role("alert"),
        );
    }
    if enabled == Some(true) && current.is_none() {
        text = text.child(roles::META.wrap("Audio follows Keep whole meetings in Hex.", p.faint));
    }
    let options = ["24 hours", "7 days", "1 month", "Forever"].map(str::to_string);
    let control = Select::new("keep-audio", &options)
        .label("Keep audio")
        .current(if h.running() && h.settings_note.is_none() {
            current
        } else {
            None
        })
        .placeholder("Not read from Hex")
        .appearance(SelectAppearance::Field)
        .width(220.0)
        .disabled(!can)
        .open(can && h.select_open.as_deref() == Some("keep-audio"))
        .on_toggle(|s: &mut State| {
            s.hex.select_open = if s.hex.select_open.as_deref() == Some("keep-audio") {
                None
            } else {
                Some("keep-audio".into())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(|s: &mut State, value: &str, _: &mut Tx| s.hex_keep_audio(value))
        .build(k, layer);
    row()
        .w_full()
        .items_center()
        .gap(16.0)
        .py(14.0)
        .child(text)
        .child(control)
}

fn setting_row(s: &State, k: &Kit, st: &Setting, at: Place, p: Palette, layer: &mut Vec<El>) -> El {
    let h = &s.hex;
    let key = st.key.clone();
    let name = human(st, at);
    let can = editable(st) && h.can(Feature::SettingsSet) && !h.busy_keys.contains(&key);
    let mut text = col()
        .gap(2.0)
        .grow(1.0)
        .min_w(220.0)
        .child(roles::CONTROL.wrap(name.clone(), p.ink));
    if !st.about.is_empty() {
        text = text.child(
            roles::META
                .line(one_line(&st.about), p.faint)
                .help_when_truncated(st.about.clone()),
        );
    }
    if st.available == Some(false) {
        text = text.child(roles::META.wrap("Not available here.", p.faint));
    } else if !st.writable {
        text = text.child(roles::META.wrap("Read only.", p.faint));
    }
    if let Some(e) = h.row_errors.get(&key) {
        text = text.child(roles::META.medium().wrap(e.clone(), p.warn).role("alert"));
    }
    if h.busy_keys.contains(&key) {
        text = text.child(roles::META.wrap("Saving.", p.faint));
    }
    // A folder or file path needs more room than a short value.
    let width = if st.kind == "path" { 360.0 } else { 220.0 };
    let control = if !editable(st) {
        let shown = if st.secret {
            "Hidden by Hex".into()
        } else {
            setting_text(&st.value)
        };
        roles::META
            .line(
                if shown.is_empty() {
                    "Not set".into()
                } else {
                    shown.clone()
                },
                p.soft,
            )
            .w(width)
            .help_when_truncated(shown)
    } else {
        match st.kind.as_str() {
            "bool" => {
                let key = key.clone();
                Switch::new(st.value.as_bool().unwrap_or(false))
                    .label(name)
                    .hide_label()
                    .disabled(!can)
                    .on_toggle(move |s: &mut State| s.hex_toggle(&key))
                    .build(k)
                    .key(format!("hs-sw-{}", st.key))
            }
            "choice" => {
                let options: Vec<String> = st.choices.iter().map(setting_text).collect();
                let current = st.value.as_str();
                let open = can && h.select_open.as_deref() == Some(key.as_str());
                let (toggle_key, choose_key) = (key.clone(), key.clone());
                Select::new(format!("hs-sel-{key}"), &options)
                    .label(name)
                    .current(current)
                    .placeholder("Not set")
                    .appearance(SelectAppearance::Field)
                    .width(220.0)
                    .disabled(!can)
                    .open(open)
                    .on_toggle(move |s: &mut State| {
                        s.hex.select_open =
                            if s.hex.select_open.as_deref() == Some(toggle_key.as_str()) {
                                None
                            } else {
                                Some(toggle_key.clone())
                            };
                    })
                    .on_close(|s: &mut State| s.hex.select_open = None)
                    .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
                        s.hex_choose(&choose_key, v)
                    })
                    .build(k, layer)
            }
            _ => {
                let empty = TextEdit::default();
                let draft = h.drafts.get(&key).unwrap_or(&empty);
                let dirty = changed(st, draft.text.trim());
                let (submit_key, esc_key, save_key) = (key.clone(), key.clone(), key.clone());
                let input = TextInput::new(draft)
                    .plain()
                    .label(name)
                    .height(32.0)
                    .disabled(!can)
                    .on_edit(move |s: &mut State| s.hex.draft_mut(&key))
                    .on_submit(move |s: &mut State, _| s.hex_commit(&submit_key))
                    .on_key(move |s: &mut State, e: &KeyEvent, tx: &mut Tx| {
                        if e.key == Key::Escape {
                            s.hex_revert(&esc_key);
                            tx.blur();
                            return true;
                        }
                        false
                    })
                    .build(k)
                    .key(format!("hs-in-{}", st.key))
                    .border(1.0, if draft.focused { p.accent } else { p.line })
                    .radius(8.0)
                    .fill(p.card)
                    .grow(1.0)
                    .min_w(120.0);
                let mut r = row().w(width).gap(6.0).items_center().child(input);
                if dirty && can {
                    r = r.child(
                        link("Save", p)
                            .key(format!("hs-save-{}", st.key))
                            .on_press(move |s: &mut State| s.hex_commit(&save_key)),
                    );
                }
                r
            }
        }
    };
    row()
        .key(format!("hs-{}", st.key))
        .w_full()
        .py(10.0)
        .flex_wrap()
        .items_center()
        .gaps(8.0, 16.0)
        .child(text)
        .child(control)
}

fn features_section(s: &State, k: &Kit, p: Palette) -> El {
    let mut body = col()
        .w_full()
        .gap(8.0)
        .child(section_head(Section::Features, None, p))
        .child(roles::GROUP.wrap("Always on", p.soft))
        .child(roles::SMALL.wrap("Meetings and notes, Ask, and Hex settings.", p.ink))
        .child(
            roles::META
                .wrap(
                    "Meeting to-dos stay visible. Turning a feature off keeps its data.",
                    p.soft,
                )
                .mb(12.0),
        );
    if let Some(error) = &s.feature_error {
        body = body.child(
            roles::META
                .medium()
                .wrap(error.clone(), p.warn)
                .role("alert"),
        );
    }
    for feature in sidenote::core::Feature::ALL {
        body = body
            .child(
                row()
                    .w_full()
                    .items_center()
                    .gap(20.0)
                    .py(12.0)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(4.0)
                            .child(roles::TITLE.wrap(feature.name(), p.ink))
                            .child(roles::META.wrap(feature.description(), p.soft)),
                    )
                    .child(
                        Switch::new(s.config.features.enabled(feature))
                            .label(feature.name())
                            .hide_label()
                            .on_toggle(move |s: &mut State| s.toggle_feature(feature))
                            .build(k)
                            .key(format!("feature-{}", feature.name())),
                    ),
            )
            .child(hairline(p));
    }
    body
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let side = if narrow { 16.0 } else { 40.0 };
    let query = s.hex.filter.text.trim().to_lowercase();
    let mut doc = col()
        .key("hex-top")
        .w_full()
        .max_w(920.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0)
        .child(text_title("Settings", if narrow { 26.0 } else { 30.0 }, p).mb(4.0))
        .child(summary_line(s, p).mb(16.0));
    if let Some(n) = s.hex.page_note() {
        doc = doc.child(note(n, p));
    }
    doc = doc
        .child(search_box(s, k, p).mb(12.0))
        .child(tabs(s, p))
        .child(if query.is_empty() {
            section_body(s, k, p, layer, s.hex.section)
        } else {
            search_results(s, k, p, layer, &query)
        });
    col()
        .key("hex-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg)
        .child(scroll(
            col().w_full().items_center().child(doc),
            "hex-scroll",
        ))
}

/// The Hex page's keys. Letters only count while no text box has the keyboard.
pub fn on_key(s: &mut State, e: &KeyEvent, _tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        if s.hex.select_open.is_some() {
            s.hex.select_open = None;
        } else if !s.hex.filter.text.is_empty() {
            s.hex.filter = TextEdit::default();
        } else {
            s.go_home();
        }
        return true;
    }
    false
}

// ---- render scenes --------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Fixture {
    version: String,
    model: ModelState,
    models: Vec<ModelInfo>,
    vocabulary: Vec<VocabularyEntry>,
    settings: Vec<Setting>,
}

/// Fills the Hex state from `hex-fixture.json` in the scene's state folder, which
/// `tools/render_ui.ps1` copies from `tools/ui-fixtures/hex.json`. A headless render starts no
/// watcher, and nothing here calls Hex, so the shot is the same every time.
pub fn scene(s: &mut State, recording: bool) {
    let Some(dir) = sidenote::env_var("SIDENOTE_STATE_DIR") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(std::path::Path::new(&dir).join("hex-fixture.json"))
    else {
        return;
    };
    let Ok(f) = serde_json::from_str::<Fixture>(&text) else {
        return;
    };
    let caps: Capabilities = [
        Feature::RecordStart,
        Feature::RecordStop,
        Feature::SettingsSet,
        Feature::VocabularyAdd,
        Feature::VocabularyRemove,
    ]
    .into_iter()
    .map(|feature| (feature, Support::Supported))
    .collect();
    s.hex.caps = Some(caps);
    s.hex.caps_version = Some(f.version.clone());
    s.hex.status = Some(Ok(EngineState {
        running: true,
        version: f.version,
        protocol: 1,
        recording: recording.then(|| Recording {
            id: Some("fixture".into()),
            title: None,
            since: None,
            elapsed_seconds: Some(754),
        }),
        meeting_state: if recording { "recording" } else { "idle" }.into(),
        dictating: Some(false),
        capture_state: "ready".into(),
        model: f.model,
    }));
    s.hex.was_running = true;
    s.hex.data_dirty = false;
    s.hex.models = f.models;
    s.hex_set_settings(Ok(f.settings));
    s.hex_set_vocabulary(Ok(f.vocabulary));
}
