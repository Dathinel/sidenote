//! The Sidenote window: a sidebar that folds to a rail (the five places, the projects, the Hex
//! status), a home list, one meeting as a reading column, the live page of a recording, and
//! every task.
mod arrival;
mod ask;
mod benchmark;
mod check;
mod diary;
mod edit_worker;
mod finder;
mod hex;
mod list;
mod live;
mod meeting;
mod memory;
mod models;
mod more;
mod nav;
mod project;
mod project_merge;
mod roles;
mod sync_scene;
mod tasks;
mod theme;
mod today;
mod todo;
mod updates;
mod widgets;

use rune::{AppSpec, Cx, El, Key, KeyEvent, TextEdit, Tx, Waker, col, row};
use sidenote::core::{
    self, AskProgress, DayGroup, Meeting, Message, Runner, Saved, SearchHit, StateStore,
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

/// The meeting the review renders use.
const REVIEW_MEETING: &str = "1800000000000-1";

const DEFAULT_SIZE: (f32, f32) = (1320.0, 800.0);
/// Under this width the window shows one thing at a time, and the sidebar is a drawer.
const NARROW: f32 = 820.0;
/// A page with less room than this beside the sidebar lays out as it does in a narrow window.
const COMPACT: f32 = 760.0;

/// Background check and migration runners do not belong in the Ask picker.
fn ask_runners() -> Result<Vec<Runner>, String> {
    core::list_runners().map(|v| {
        v.into_iter()
            .filter(|r| {
                !r.name.starts_with("Daily check") && !r.name.starts_with("Memory migration")
            })
            .collect()
    })
}

/// The runners whose program is on this machine.
fn found_runners(runners: &[Runner]) -> HashSet<String> {
    runners
        .iter()
        .filter(|r| core::runners::installed(r))
        .map(|r| r.name.clone())
        .collect()
}

/// The remembered runner when it can run, else the first one that can. None when no agent is
/// installed, which is when the Ask bar says so.
fn default_runner(
    runners: &[Runner],
    found: &HashSet<String>,
    remembered: Option<String>,
) -> Option<String> {
    remembered.filter(|n| found.contains(n)).or_else(|| {
        runners
            .iter()
            .find(|r| found.contains(&r.name))
            .map(|r| r.name.clone())
    })
}

pub enum Event {
    ProjectMerge {
        generation: u64,
        worker: std::thread::ThreadId,
        result: Result<project_merge::Completion, String>,
    },
    Edited(edit_worker::Completion),
    Update(sidenote::updates::Status),
    UpdateFinished(Result<sidenote::updates::Status, String>),
    MigrationPrepared(Result<(), String>),
    /// A Hex engine result from a worker thread.
    Hex(hex::Msg),
    Models(models::Msg),
    /// The scan, its errors, and the pipeline's `tasks.json` of each meeting.
    Meetings(Box<arrival::Snapshot>),
    Progress {
        id: String,
        generation: u64,
        event: AskProgress,
    },
}

pub struct AskRun {
    /// False for a scene's stand-in run that has no worker behind it.
    pub live: bool,
    /// When the question went out, for the "Thinking, 12 s" line.
    pub started: Instant,
    /// Stop kills this run's CLI. The worker holds the other end.
    pub cancel: core::ask::Cancel,
    /// Dropped with the run, which ends the one-second redraw ticker that watches it.
    pub _alive: std::sync::Arc<()>,
    pub submitted_message_id: Option<String>,
    pub generation: u64,
    pub runner: String,
    pub rx: Receiver<Result<String, String>>,
    pub steps: Vec<String>,
}

impl AskRun {
    /// A run a scene shows with nothing behind it, `secs` seconds in.
    fn stand_in(runner: &str, steps: &[&str], secs: u64) -> AskRun {
        let (_tx, rx) = mpsc::channel();
        AskRun {
            live: false,
            started: Instant::now()
                .checked_sub(Duration::from_secs(secs))
                .unwrap_or_else(Instant::now),
            cancel: core::ask::Cancel::new(),
            _alive: Default::default(),
            submitted_message_id: None,
            generation: 0,
            runner: runner.into(),
            rx,
            steps: steps.iter().map(|s| s.to_string()).collect(),
        }
    }
}

pub struct PendingScroll {
    key: String,
    /// Centre the element (a transcript line); otherwise put its top near the viewport top.
    center: bool,
    left: u8,
}

pub struct ListItem {
    pub index: usize,
    pub hit: Option<SearchHit>,
}
pub struct ListGroup {
    pub label: String,
    pub items: Vec<ListItem>,
}

pub struct State {
    pub updates: updates::Updates,
    pub models: models::Models,
    pub config: core::Config,
    pub feature_error: Option<String>,
    pub meetings: Vec<Meeting>,
    pub scan_errors: Vec<String>,
    /// The open meeting. None is the home list.
    pub selected: Option<String>,
    pub search: TextEdit,
    pub groups: Vec<ListGroup>,
    pub cursor: usize,
    pub store: StateStore,
    pub saved: Saved,
    pub notice: Option<String>,
    /// The transcript is showing: a side panel in a wide window, the whole page in a narrow one.
    pub transcript_open: bool,
    pub transcript_original: bool,
    pub transcript_filter: TextEdit,
    /// The "Go to 14:22" box in the transcript panel.
    pub transcript_jump: TextEdit,
    /// Which of the meeting page's two main tabs shows: Summary or Your notes.
    pub meeting_tab: meeting::Tab,
    /// The key of the open "..." menu, so only one is ever open.
    pub more_menu: Option<String>,
    /// A confirmation such as "Copied notes", and when it goes away.
    pub toast: Option<(String, Instant)>,
    /// The Ctrl+K palette, and what is typed in its box.
    pub finder: Option<finder::Finder>,
    pub finder_query: TextEdit,
    pub highlight: Option<usize>,
    pub back: Option<String>,
    /// Set when the open meeting came from a citation in the home thread: the answer to go
    /// back to. "Back" then returns home instead of scrolling this meeting.
    pub cite_back: Option<String>,
    pub question: TextEdit,
    pub runners: Vec<Runner>,
    pub runner: Option<String>,
    pub runner_error: Option<String>,
    /// Names of the runners whose program is on this machine. The model menu greys the rest.
    pub runner_found: HashSet<String>,
    /// "Check again" was pressed and still found nothing.
    pub agents_rechecked: bool,
    /// The Ask bar is open: its box has focus. It follows `question.focused` one commit late, from
    /// `poll`, because a click into a field is a settle commit and those land without motion.
    pub ask_open: bool,
    pub runner_open: bool,
    /// The Ask thread folded down to its header, so the meeting gets the room back.
    pub thread_hidden: bool,
    pub asks: HashMap<String, AskRun>,
    pub ask_errors: HashMap<String, String>,
    pub generation: u64,
    /// The sidebar is open (else a rail of icons). The view folds it once for a narrow window,
    /// so both are cells.
    pub sidebar_open: Cell<bool>,
    pub sidebar_sized: Cell<bool>,
    /// A narrow window's sidebar is a drawer over the page, closed unless asked for.
    pub drawer_open: Cell<bool>,
    /// What the home list shows, and the per-meeting lookups the sidebar's counts need.
    pub filter: nav::Filter,
    pub nav: nav::NavCache,
    /// The recording's page. Nobody types a title: the heading is the start time, then the quick title.
    pub live: Option<live::Live>,
    pub live_open: bool,
    /// What the user types during the meeting. Saved to the meeting folder's `notes.md`.
    pub live_notes: TextEdit,
    /// A finished meeting to open the next time the UI can scroll.
    live_ready: Option<String>,
    /// The home list's keyboard cursor is showing (arrow keys were used).
    pub keyboard: bool,
    /// The meeting whose project menu is open, and the "New project..." form.
    pub project_menu: Option<String>,
    pub project_form: Option<String>,
    pub project_input: TextEdit,
    pub project_merge: project_merge::MergeUi,
    /// Meetings, or every task.
    pub view: tasks::View,
    /// The Daily check page, shown in place of the home list.
    pub check_open: bool,
    /// The open meeting came from a citation on the Daily check page; Back returns there.
    pub check_back: bool,
    pub check: check::CheckState,
    /// The Memory page, shown in place of the home list.
    pub memory_open: bool,
    /// The open meeting came from a citation on the Memory page; Back returns there.
    pub memory_back: bool,
    pub memory: memory::MemState,
    /// The Hex page, shown in place of the home list.
    pub hex_open: bool,
    pub hex: hex::HexState,
    pub tasks: tasks::TasksState,
    /// Under 820 points wide. The view records it for handlers.
    pub narrow: Cell<bool>,
    /// Height of the meeting's scroll area, recorded by the view to place jump targets.
    pub viewport_h: Cell<f32>,
    /// The window's width, as last laid out; the Tasks page sizes its panel from it.
    pub app_w: Cell<f32>,
    /// Height of the whole meeting pane; the Ask thread takes a share of it.
    pub pane_h: Cell<f32>,
    /// Height of the floating Ask bar, so the column can scroll its last line above it.
    pub overlay_h: Cell<f32>,
    /// A scroll to repeat for a few frames, until the element it names has been laid out.
    pending_scroll: Option<PendingScroll>,
    /// Puts the latest question at the top of the thread once it is laid out.
    pending_thread: Option<PendingScroll>,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    edit_busy: bool,
    applied_snapshot: u64,
    applied_incoming: String,
    applied_revisions: core::StoreRevisions,
    scan_tx: Option<arrival::Handle>,
    last_incoming: Option<Instant>,
    waker: Option<Waker>,
}

impl State {
    pub fn toggle_feature(&mut self, feature: core::Feature) {
        let result = core::Config::load_in(&self.store.root).and_then(|mut config| {
            config.features.toggle(feature);
            config.save(&self.store.root)?;
            Ok(config)
        });
        match result {
            Ok(config) => {
                self.config = config;
                self.feature_error = None;
                let f = &self.config.features;
                if (!f.tasks && self.view == tasks::View::Tasks)
                    || (!f.memory && (self.memory_open || self.memory_back))
                    || (!f.daily_check && (self.check_open || self.check_back))
                {
                    self.go_meetings();
                }
                if (!self.config.features.projects
                    && matches!(self.filter, nav::Filter::Project(_)))
                    || (!self.config.features.other_devices && self.filter == nav::Filter::Peers)
                {
                    self.filter = nav::Filter::All;
                    self.refresh_list();
                }
                if !self.config.features.projects {
                    self.project_menu = None;
                    self.project_form = None;
                    self.tasks.ui.project_menu = false;
                    self.tasks.ui.new_project = false;
                    self.tasks.inputs.project = TextEdit::default();
                    self.tasks.rename = None;
                    self.tasks.rename_input = TextEdit::default();
                    if self.tasks.grouping == core::TaskGrouping::Project {
                        self.tasks.grouping = core::TaskGrouping::Due;
                    }
                }
                if self.config.features.memory {
                    self.memory.refresh(true);
                }
                if self.config.features.daily_check {
                    self.check.refresh();
                }
            }
            Err(error) => self.feature_error = Some(format!("Could not save features: {error}")),
        }
    }

    pub fn new() -> Self {
        let (meetings, scan_errors) = core::scan();
        let (events_tx, events_rx) = mpsc::channel();
        let store = StateStore::default();
        let (runners, runner_error) = match ask_runners() {
            Ok(r) => (r, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let runner_found = found_runners(&runners);
        let runner = default_runner(
            &runners,
            &runner_found,
            core::last_used_runner().ok().flatten(),
        );
        let (config, feature_error) = match core::Config::load_in(&store.root) {
            Ok(config) => (config, None),
            Err(error) => (
                core::Config::default(),
                Some(format!("Could not read config.json: {error}")),
            ),
        };
        let tasks = tasks::TasksState::load(&meetings, store.root.clone());
        let prefs = core::UiPrefs::load(&store.root);
        let mut s = Self {
            updates: updates::Updates::default(),
            models: models::Models::load(&store.root),
            config,
            feature_error,
            selected: None,
            meetings,
            scan_errors,
            search: TextEdit::new(""),
            groups: Vec::new(),
            cursor: 0,
            store,
            saved: Saved::default(),
            notice: None,
            transcript_open: false,
            transcript_original: false,
            transcript_filter: TextEdit::new(""),
            transcript_jump: TextEdit::new(""),
            meeting_tab: meeting::Tab::Summary,
            more_menu: None,
            toast: None,
            finder: None,
            finder_query: TextEdit::new(""),
            highlight: None,
            back: None,
            cite_back: None,
            question: TextEdit::new(""),
            runners,
            runner,
            runner_error,
            runner_found,
            agents_rechecked: false,
            ask_open: false,
            runner_open: false,
            thread_hidden: false,
            asks: HashMap::new(),
            ask_errors: HashMap::new(),
            generation: 0,
            sidebar_open: Cell::new(!prefs.sidebar_collapsed.unwrap_or(false)),
            sidebar_sized: Cell::new(false),
            drawer_open: Cell::new(false),
            filter: nav::Filter::All,
            nav: nav::NavCache::default(),
            live: None,
            live_open: false,
            live_notes: TextEdit::new(""),
            live_ready: None,
            keyboard: false,
            project_menu: None,
            project_form: None,
            project_input: TextEdit::new(""),
            project_merge: project_merge::MergeUi::default(),
            view: tasks::View::Meetings,
            check_open: false,
            check_back: false,
            check: check::CheckState::default(),
            memory_open: false,
            memory_back: false,
            memory: memory::MemState::default(),
            hex_open: false,
            hex: hex::HexState::default(),
            tasks,
            narrow: Cell::new(false),
            viewport_h: Cell::new(600.0),
            app_w: Cell::new(DEFAULT_SIZE.0),
            pane_h: Cell::new(800.0),
            overlay_h: Cell::new(150.0),
            pending_scroll: None,
            pending_thread: None,
            events_tx,
            events_rx,
            edit_busy: false,
            applied_snapshot: 0,
            applied_incoming: String::new(),
            applied_revisions: Default::default(),
            scan_tx: None,
            last_incoming: None,
            waker: None,
        };
        s.applied_incoming = arrival::remote_revision(&s.meetings);
        s.rebuild_nav();
        s.refresh_list();
        s.load_saved();
        if s.config.features.daily_check {
            s.check.refresh();
        }
        if s.config.features.memory {
            s.memory.refresh(true);
        }
        s
    }

    pub fn current(&self) -> Option<&Meeting> {
        let id = self.selected.as_ref()?;
        self.meetings.iter().find(|m| &m.id == id)
    }

    /// What the Ask thread is stored under: the open meeting, or `core::ALL_KEY` on the home
    /// page, where Ask covers every meeting.
    pub fn ask_key(&self) -> String {
        self.selected
            .clone()
            .unwrap_or_else(|| core::ALL_KEY.to_string())
    }

    /// The element key of message `i` in the Ask thread on this page.
    pub fn msg_key(&self, i: usize) -> String {
        if self.selected.is_some() {
            format!("msg-{i}")
        } else {
            format!("all-msg-{i}")
        }
    }

    fn load_saved(&mut self) {
        self.saved = match self.store.load(&self.ask_key()) {
            Ok(saved) => saved,
            Err(_) => {
                self.notice = Some(
                    "Could not read the saved ticks and answers. Check the app data folder.".into(),
                );
                Saved::default()
            }
        };
        self.show_latest_question();
    }

    /// Scrolls the thread so the latest question sits at its top, with the answer below it.
    fn show_latest_question(&mut self) {
        self.pending_thread = self
            .saved
            .messages
            .iter()
            .rposition(|m| m.role == "user")
            .map(|i| PendingScroll {
                key: self.msg_key(i),
                center: false,
                left: 3,
            });
    }

    /// Rebuilds the grouped list from the search box.
    pub fn refresh_list(&mut self) {
        let query = self.search.text.trim().to_string();
        let hits: HashMap<String, SearchHit> = if query.is_empty() {
            HashMap::new()
        } else {
            core::search(&self.meetings, &query)
                .into_iter()
                .map(|h| (h.meeting_id.clone(), h))
                .collect()
        };
        let groups: Vec<DayGroup> = core::group_by_day(&self.meetings);
        self.groups = groups
            .into_iter()
            .filter_map(|g| {
                let items: Vec<ListItem> = g
                    .meeting_indices
                    .into_iter()
                    .filter(|i| self.matches_filter(&self.meetings[*i]))
                    .filter(|i| query.is_empty() || hits.contains_key(&self.meetings[*i].id))
                    .map(|index| ListItem {
                        hit: hits.get(&self.meetings[index].id).cloned(),
                        index,
                    })
                    .collect();
                (!items.is_empty()).then_some(ListGroup {
                    label: g.label,
                    items,
                })
            })
            .collect();
        self.cursor = self.cursor.min(self.row_count().saturating_sub(1));
    }

    pub fn row_count(&self) -> usize {
        self.groups.iter().map(|g| g.items.len()).sum()
    }

    pub fn row_at(&self, n: usize) -> Option<&ListItem> {
        self.groups.iter().flat_map(|g| &g.items).nth(n)
    }

    pub fn on_search_changed(&mut self) {
        self.cursor = 0;
        self.refresh_list();
    }

    pub fn focus_search(&mut self) {
        // In the Tasks view the search box is the task search.
        if self.view == tasks::View::Tasks {
            self.focus_task_search();
            return;
        }
        if self.memory_open {
            self.memory.search.focused = true;
            return;
        }
        if self.hex_open {
            self.hex.filter.focused = true;
            return;
        }
        if self.check_open {
            self.go_home();
        }
        // Searching is done on the home list.
        if self.selected.is_some() || self.on_live() {
            self.go_home();
        }
        self.search.focused = true;
    }

    /// Back to the home list.
    pub fn go_home(&mut self) {
        self.view = tasks::View::Meetings;
        self.check_open = false;
        self.check_back = false;
        self.memory_open = false;
        self.memory_back = false;
        self.hex_open = false;
        self.live_open = false;
        self.more_menu = None;
        if self.selected.take().is_some() {
            self.transcript_open = false;
            self.meeting_tab = meeting::Tab::Summary;
            self.highlight = None;
            self.back = None;
            self.cite_back = None;
            self.question = TextEdit::new("");
            self.runner_open = false;
            self.load_saved();
        }
    }

    pub fn select(&mut self, id: &str, tx: &mut Tx) {
        self.view = tasks::View::Meetings;
        self.check_open = false;
        self.check_back = false;
        self.memory_open = false;
        self.memory_back = false;
        self.hex_open = false;
        self.live_open = false;
        self.keyboard = false;
        if self.selected.as_deref() == Some(id) {
            return;
        }
        self.selected = Some(id.to_string());
        self.transcript_open = false;
        self.transcript_filter = TextEdit::new("");
        self.transcript_jump = TextEdit::new("");
        self.meeting_tab = meeting::Tab::Summary;
        self.more_menu = None;
        self.highlight = None;
        self.back = None;
        self.cite_back = None;
        self.question = TextEdit::new("");
        self.notice = None;
        self.runner_open = false;
        self.project_menu = None;
        self.project_form = None;
        self.thread_hidden = false;
        self.pending_scroll = None;
        self.load_saved();
        if let Some(n) = self
            .groups
            .iter()
            .flat_map(|g| &g.items)
            .position(|i| self.meetings[i.index].id == id)
        {
            self.cursor = n;
        }
        tx.scroll_to_y("meeting-scroll", 0.0);
    }

    pub fn open_cursor(&mut self, tx: &mut Tx) {
        if let Some(id) = self
            .row_at(self.cursor)
            .map(|i| self.meetings[i.index].id.clone())
        {
            self.select(&id, tx);
        }
    }

    pub fn move_cursor(&mut self, delta: isize, tx: &mut Tx) {
        let last = self.row_count().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
        if let Some(id) = self
            .row_at(self.cursor)
            .map(|i| self.meetings[i.index].id.clone())
        {
            tx.scroll_into_view(format!("meeting-{id}"));
        }
    }

    pub fn toggle_task(&mut self, task: &str) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let on = !self.saved.checked.contains(task);
        match self.store.set_tick(&id, task, on) {
            Ok(saved) => {
                // The Tasks view reads the same ticks.
                self.tasks.checked.insert(id, saved.checked.clone());
                self.saved = saved;
                self.rebuild_tasks();
            }
            Err(_) => {
                self.notice = Some(
                    "Could not save the tick. Check that the app data folder is writable.".into(),
                )
            }
        }
    }

    /// Opens the transcript at the line for `stamp`, lights it and its neighbours, and
    /// remembers where to come back to.
    pub fn jump(&mut self, stamp: &str, origin: String, tx: &mut Tx) {
        let Some(line) = self.current().and_then(|m| m.target(stamp)) else {
            let untimed = self
                .current()
                .is_some_and(|m| !m.lines.is_empty() && !m.timed());
            self.notice = Some(
                if untimed {
                    "This transcript has no times, so there is no line to jump to."
                } else {
                    "The transcript has no line for that time."
                }
                .into(),
            );
            return;
        };
        self.transcript_original = false;
        self.transcript_open = true;
        self.transcript_filter = TextEdit::new("");
        self.highlight = Some(line);
        self.back = Some(origin);
        self.pending_scroll = Some(PendingScroll {
            key: format!("line-{line}"),
            center: true,
            left: 3,
        });
        tx.scroll_key_to(format!("line-{line}"), self.panel_offset());
    }

    /// Where a transcript line sits below the top of the panel's scroll area when a jump lands:
    /// about the middle of what the panel's search row and footer leave visible.
    fn panel_offset(&self) -> f32 {
        ((self.viewport_h.get() - 190.0) / 2.0).max(60.0)
    }

    /// Where a jump target sits below the top of the scroll area: about the middle of what
    /// the floating Ask bar leaves visible.
    fn jump_offset(&self) -> f32 {
        ((self.viewport_h.get() - self.overlay_h.get()) / 2.0 - 24.0).max(60.0)
    }

    /// Opens the meeting a home-thread citation names: `<id>` or `<id>@<stamp>`. With a stamp it
    /// lands on that transcript line. Back returns to the answer the link was in.
    pub fn open_citation(&mut self, target: &str, origin: String, tx: &mut Tx) {
        let (id, stamp) = match target.split_once('@') {
            Some((id, stamp)) => (id, Some(stamp)),
            None => (target, None),
        };
        if !self.meetings.iter().any(|m| m.id == id) {
            return;
        }
        self.select(id, tx);
        if let Some(stamp) = stamp {
            self.jump(stamp, origin.clone(), tx);
        }
        self.back = Some(origin.clone());
        self.cite_back = Some(origin);
    }

    pub fn go_back(&mut self, tx: &mut Tx) {
        if self.memory_back {
            let key = self.back.take();
            self.go_home();
            self.memory_open = true;
            if self.config.features.memory {
                self.memory.refresh(false);
            }
            self.pending_thread = key.map(|key| PendingScroll {
                key,
                center: false,
                left: 3,
            });
            return;
        }
        if self.check_back {
            let key = self.back.take();
            self.go_home();
            self.check_open = true;
            self.check.refresh();
            self.pending_thread = key.map(|key| PendingScroll {
                key,
                center: false,
                left: 3,
            });
            return;
        }
        if let Some(key) = self.cite_back.take() {
            self.go_home();
            self.thread_hidden = false;
            self.pending_thread = Some(PendingScroll {
                key,
                center: false,
                left: 3,
            });
            return;
        }
        let Some(origin) = self.back.take() else {
            return;
        };
        tx.scroll_key_to(origin, self.jump_offset());
    }

    /// Reads the runner list again and looks for each program on PATH.
    pub fn reload_runners(&mut self) {
        match ask_runners() {
            Ok(r) => {
                self.runner_found = found_runners(&r);
                self.runners = r;
                self.runner_error = None;
                if !self
                    .runner
                    .as_ref()
                    .is_some_and(|n| self.runner_found.contains(n))
                {
                    self.runner = default_runner(&self.runners, &self.runner_found, None);
                }
            }
            Err(e) => self.runner_error = Some(e),
        }
    }

    /// True when at least one runner can start on this machine.
    pub fn has_agent(&self) -> bool {
        !self.runner_found.is_empty()
    }

    /// Stop on a running Ask: its CLI is killed and the question goes back in the box.
    pub fn stop_ask(&mut self) {
        if let Some(run) = self.asks.get(&self.ask_key()) {
            run.cancel.stop();
        }
    }

    /// Everything the home page's Ask sends: each meeting's summary, newest first.
    fn all_context(&self) -> core::ask_all::AllContext {
        core::ask_all::all_context(
            &self.meetings,
            &self.tasks.all,
            &self.tasks.pipeline,
            &self.tasks.projects,
            chrono::Local::now().date_naive(),
            core::ask_all::CONTEXT_CAP,
        )
    }

    pub fn send(&mut self, text: &str) {
        let text = text.trim();
        let id = self.ask_key();
        if text.is_empty() || self.asks.contains_key(&id) {
            return;
        }
        let Some(runner) = self.runner.clone() else {
            self.ask_errors
                .insert(id, "Choose a runner before asking.".into());
            return;
        };
        let (saved, submitted_message_id) = match self.store.append_message_identified(
            &id,
            Message {
                role: "user".into(),
                text: text.to_string(),
                ..Default::default()
            },
        ) {
            Ok(saved) => saved,
            Err(_) => {
                self.notice = Some(
                    "Could not save your question. Check that the app data folder is writable."
                        .into(),
                );
                return;
            }
        };
        let Some(submitted_at) = saved
            .messages
            .iter()
            .position(|m| m.id.as_deref() == Some(submitted_message_id.as_str()))
        else {
            self.notice =
                Some("Your question was removed before the Ask started. Try again.".into());
            return;
        };
        let history = saved.messages[..=submitted_at].to_vec();
        self.saved = saved.clone();
        self.question = TextEdit::new("");
        self.ask_errors.remove(&id);
        self.generation += 1;
        let generation = self.generation;
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        let id_for_events = id.clone();
        self.thread_hidden = false;
        self.show_latest_question();
        let notify = move |event| {
            let _ = events.send(Event::Progress {
                id: id_for_events.clone(),
                generation,
                event,
            });
            if let Some(w) = &waker {
                w();
            }
        };
        let cancel = core::ask::Cancel::new();
        let (rx, first_step) = if self.selected.is_some() {
            let rx = core::ask::spawn(id.clone(), history, runner.clone(), cancel.clone(), notify);
            (rx, "Reading the meeting...")
        } else {
            let context = self.all_context();
            let rx = core::ask_all::spawn(context, history, runner.clone(), cancel.clone(), notify);
            (rx, "Reading your meetings...")
        };
        // Wakes the window once a second while this run lives, so "Thinking, 12 s" counts.
        let alive = std::sync::Arc::new(());
        if let Some(wake) = self.waker.clone() {
            let watch = std::sync::Arc::downgrade(&alive);
            std::thread::spawn(move || {
                while watch.upgrade().is_some() {
                    std::thread::sleep(Duration::from_secs(1));
                    wake();
                }
            });
        }
        self.asks.insert(
            id,
            AskRun {
                live: true,
                started: Instant::now(),
                cancel,
                _alive: alive,
                submitted_message_id: Some(submitted_message_id),
                generation,
                runner,
                rx,
                steps: vec![first_step.into()],
            },
        );
    }

    pub fn clear_thread(&mut self) {
        let id = self.ask_key();
        self.asks.remove(&id);
        self.ask_errors.remove(&id);
        match self.store.clear_history(&id) {
            Ok(saved) => self.saved = saved,
            Err(_) => self.notice = Some("Could not clear the saved answers.".into()),
        }
    }

    fn finish_ask(&mut self, id: &str, run: AskRun, failure: Option<String>) {
        let outcome = match failure {
            Some(message) => Err(message),
            None => run
                .rx
                .try_recv()
                .unwrap_or_else(|_| Err("The answer did not arrive.".into())),
        };
        let result = match outcome {
            Ok(text) => self.store.append_message(
                id,
                Message {
                    role: "assistant".into(),
                    text,
                    ..Default::default()
                },
            ),
            Err(message) => {
                // Take the unanswered question back out so a retry does not repeat it.
                let mut taken = None;
                let result = self.store.update(id, |s| {
                    if let Some(at) = s.messages.iter().position(|m| {
                        m.role == "user" && m.id.is_some() && m.id == run.submitted_message_id
                    }) {
                        taken = Some(s.messages.remove(at));
                    }
                });
                // A stop the owner asked for is not a failure: only the question comes back.
                if message != core::ask::STOPPED {
                    self.ask_errors.insert(id.to_string(), message);
                }
                if self.ask_key() == id
                    && self.question.text.is_empty()
                    && let Some(m) = taken
                {
                    self.question = TextEdit::new(&m.text);
                }
                result
            }
        };
        match result {
            Ok(saved) if self.ask_key() == id => {
                self.saved = saved;
                self.show_latest_question();
            }
            Ok(_) => {}
            Err(_) => self.notice = Some("Could not save the answer.".into()),
        }
    }

    fn apply_event(&mut self, event: Event) {
        match event {
            Event::Edited(result) => self.edit_finished(result),
            Event::ProjectMerge {
                generation,
                worker,
                result,
            } => {
                debug_assert_ne!(worker, std::thread::current().id());
                self.project_merge_finished(generation, result);
            }
            Event::Update(status) => self.update_status(status),
            Event::UpdateFinished(result) => self.update_finished(result),
            Event::Hex(msg) => self.hex_apply(msg),
            Event::Models(msg) => self.models.apply(msg),
            Event::MigrationPrepared(result) => {
                self.memory.preview_running = false;
                if self.config.features.memory {
                    self.memory.refresh(false);
                }
                if let Err(error) = result {
                    self.memory.errors.insert("migration".into(), error);
                }
            }
            Event::Meetings(snapshot) => {
                if self.edit_busy
                    || !snapshot.stamp.valid
                    || snapshot.stamp.generation != core::mutation_generation()
                    || snapshot.stamp.sequence <= self.applied_snapshot
                    || snapshot.options != self.refresh_options()
                {
                    self.rescan();
                    return;
                }
                if self.scan_tx.is_some() {
                    debug_assert_ne!(snapshot.worker, std::thread::current().id());
                }
                let stores_changed = self.applied_revisions != snapshot.stamp.revisions;
                let incoming = self.applied_incoming != snapshot.remote_revision;
                self.applied_incoming = snapshot.remote_revision.clone();
                self.applied_snapshot = snapshot.stamp.sequence;
                self.applied_revisions = snapshot.stamp.revisions.clone();
                let arrival::Snapshot {
                    meetings,
                    errors,
                    pipeline,
                    projects,
                    overrides,
                    cutoff,
                    saved,
                    memory,
                    check,
                    options,
                    ..
                } = *snapshot;
                let before = serde_json::to_value((
                    &self.tasks.projects,
                    &self.tasks.overrides,
                    &self.tasks.checked,
                    &self.tasks.cutoff,
                    &self.saved,
                    &self.memory.records,
                    &self.memory.proposals,
                ))
                .ok();
                let catalog_changed = !same_catalog(&self.meetings, &meetings);
                if !same_catalog(&self.meetings, &meetings) {
                    self.meetings = meetings;
                    // An open meeting that has gone away returns to the home list.
                    if self.selected.is_some() && self.current().is_none() {
                        self.selected = None;
                        self.saved = Saved::default();
                    }
                    self.refresh_list();
                }
                self.scan_errors = errors;
                if let Some(projects) = projects {
                    self.memory.projects = projects.clone();
                    self.tasks.projects = projects;
                }
                if let Some(overrides) = overrides {
                    self.tasks.overrides = overrides;
                }
                if let Some(cutoff) = cutoff {
                    self.tasks.cutoff = cutoff;
                }
                for (id, saved) in &saved {
                    self.tasks.checked.insert(id.clone(), saved.checked.clone());
                }
                if let Some(saved) = saved.get(&self.ask_key()) {
                    self.saved = saved.clone();
                }
                if let Some(file) = memory {
                    self.memory.apply_file(file);
                }
                self.tasks.pipeline = pipeline;
                self.rebuild_tasks();
                self.live_scanned();
                if let Some(check) = check {
                    self.check.apply_refresh(check);
                }
                if options.check_date != self.check.date {
                    self.rescan();
                }
                let after = serde_json::to_value((
                    &self.tasks.projects,
                    &self.tasks.overrides,
                    &self.tasks.checked,
                    &self.tasks.cutoff,
                    &self.saved,
                    &self.memory.records,
                    &self.memory.proposals,
                ))
                .ok();
                if incoming
                    && (catalog_changed || stores_changed)
                    && (catalog_changed || before != after)
                {
                    self.last_incoming = Some(Instant::now());
                }
            }
            Event::Progress {
                id,
                generation,
                event,
            } => {
                let Some(run) = self.asks.get_mut(&id) else {
                    return;
                };
                if run.generation != generation {
                    return;
                }
                match event {
                    AskProgress::Started { .. } => {}
                    AskProgress::Tool { name } => run.steps.push(format!("Using {name}")),
                    AskProgress::Finished => {
                        if let Some(run) = self.asks.remove(&id) {
                            self.finish_ask(&id, run, None);
                        }
                    }
                    AskProgress::Failed { message } => {
                        if let Some(run) = self.asks.remove(&id) {
                            self.finish_ask(&id, run, Some(message));
                        }
                    }
                }
            }
        }
    }

    fn refresh_options(&self) -> arrival::Options {
        arrival::Options {
            check_date: self.check.date,
            memory: self.config.features.memory,
            check: self.config.features.daily_check,
        }
    }
    pub fn rescan(&self) {
        if let Some(handle) = &self.scan_tx {
            handle.request(self.refresh_options());
        }
    }
}

fn same_catalog(a: &[Meeting], b: &[Meeting]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.id == y.id
                && x.folder == y.folder
                && x.device == y.device
                && x.started == y.started
                && x.title == y.title
                && x.minutes == y.minutes
                && x.summary_raw == y.summary_raw
                && x.transcript_raw == y.transcript_raw
                && x.routed_revision == y.routed_revision
                && x.notes == y.notes
        })
}

fn setup(s: &mut State, waker: Waker) {
    if rune::tooling::headless_run().is_some() {
        return;
    }
    s.waker = Some(waker.clone());
    s.updates_start();
    s.hex_start(waker.clone());
    s.scan_tx = Some(arrival::start(waker, s.refresh_options()));
}

fn apply_pending(s: &mut State, headless: bool) -> (bool, usize) {
    let mut changed = false;
    let deadline = Instant::now() + Duration::from_millis(4);
    if !s.edit_busy
        && let Some(snapshot) = s.scan_tx.as_ref().and_then(|h| h.take())
    {
        s.apply_event(Event::Meetings(snapshot));
        changed = true;
    }
    let mut events = 0;
    for _ in 0..8 {
        if !headless && Instant::now() >= deadline {
            break;
        }
        match s.events_rx.try_recv() {
            Ok(event) => {
                s.apply_event(event);
                events += 1;
                changed = true;
            }
            // A headless render has no event loop to wake, so it waits for a running Ask.
            Err(_) if headless && s.asks.values().any(|r| r.live) => {
                if let Ok(event) = s.events_rx.recv_timeout(Duration::from_secs(600)) {
                    s.apply_event(event);
                    changed = true;
                } else {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if Instant::now() >= deadline
        && let Some(waker) = &s.waker
    {
        waker();
    }
    (changed, events)
}
fn poll(s: &mut State, _clock: f64, tx: &mut Tx) -> bool {
    let headless = rune::tooling::headless_run().is_some();
    let (mut changed, _) = apply_pending(s, headless);
    // The Ask bar opens and folds in a commit of its own, so it moves instead of jumping.
    if s.ask_open != s.question.focused {
        s.ask_open = s.question.focused;
        changed = true;
    }
    // The elapsed seconds of a running Ask redraw once a second, woken by its ticker.
    if !headless && s.asks.values().any(|r| r.live) {
        changed = true;
    }
    changed |= s.live_tick();
    changed |= s.expire_toast();
    if let Some(id) = s.live_ready.take() {
        s.select(&id, tx);
        changed = true;
    }
    if let Some(mut pending) = s.pending_scroll.take() {
        if pending.center {
            tx.scroll_key_to(pending.key.clone(), s.panel_offset());
        } else {
            tx.scroll_key_to(pending.key.clone(), 12.0);
        }
        if pending.left > 0 {
            pending.left -= 1;
            s.pending_scroll = Some(pending);
        }
        changed = true;
    }
    if let Some(mut pending) = s.pending_thread.take() {
        tx.scroll_key_to(pending.key.clone(), 0.0);
        if pending.left > 0 {
            pending.left -= 1;
            s.pending_thread = Some(pending);
        }
        changed = true;
    }
    changed
}

fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    // In Tasks a focused button (a tick just pressed) must not swallow J, K or X; only a
    // text box counts as typing there.
    let tasks_view = s.view == tasks::View::Tasks;
    let typing = if tasks_view {
        s.tasks.typing() || s.search.focused
    } else if s.on_check() || s.on_memory() || s.on_hex() {
        // A pressed button keeps focus; only a text box counts as typing on these pages.
        s.check.typing() || s.memory.typing() || s.hex.typing() || s.search.focused
    } else {
        tx.focused().is_some()
    };
    // Ctrl+K opens the palette from any page, and again closes it. While it is open it owns
    // Up, Down, Enter, Tab and Escape, even if a click took the focus from its box.
    if e.is_command('k') {
        s.toggle_finder();
        return true;
    }
    if s.finder.is_some() {
        return finder::key(s, e, tx);
    }
    if !typing && matches!(&e.key, Key::Text(t) if t == "/") {
        s.focus_search();
        return true;
    }
    if e.is_command('b') {
        s.toggle_sidebar();
        return true;
    }
    if typing {
        return false;
    }
    if s.on_check() {
        return check::on_key(s, e, tx);
    }
    if s.on_memory() {
        return memory::on_key(s, e, tx);
    }
    if s.on_hex() {
        return hex::on_key(s, e, tx);
    }
    if tasks_view {
        return tasks::on_key(s, e, tx);
    }
    let home = s.selected.is_none();
    let searching = !s.search.text.trim().is_empty();
    match e.key {
        // On the home list the arrows move a cursor and Enter opens; beside a meeting they
        // step through meetings, as before.
        Key::Up | Key::Down => {
            s.move_cursor(if e.key == Key::Up { -1 } else { 1 }, tx);
            if home {
                s.keyboard = true;
            } else if !searching {
                s.open_cursor(tx);
            }
            true
        }
        Key::Enter if s.row_count() > 0 && (home || searching) => {
            s.open_cursor(tx);
            true
        }
        Key::Escape if searching => {
            s.search = TextEdit::new("");
            s.on_search_changed();
            true
        }
        // T shows or hides the transcript beside the summary.
        Key::Text(ref t)
            if !home
                && t.eq_ignore_ascii_case("t")
                && !(e.mods.cmd || e.mods.ctrl || e.mods.alt) =>
        {
            s.toggle_transcript();
            true
        }
        // Escape closes the transcript first, and a second one leaves the meeting.
        Key::Escape if !home && s.transcript_open => {
            s.transcript_open = false;
            true
        }
        // Escape steps back from a meeting to the home list.
        Key::Escape if !home => {
            s.go_home();
            true
        }
        _ => false,
    }
}

fn top_bar(s: &State, p: theme::Palette, narrow: bool) -> El {
    use widgets::pressable;
    let mut left = row().gap(4.0).items_center().min_w(0.0);
    if s.view == tasks::View::Meetings && s.selected.is_some() {
        left = left.child(
            pressable(roles::CONTROL.wrap("\u{2039}  All meetings", p.soft))
                .key("all-meetings")
                .label("All meetings (Escape)")
                .px(10.0)
                .py(4.0)
                .radius(8.0)
                .hover(p.chip)
                .on_press(|s: &mut State| s.go_home()),
        );
    }
    row()
        .key("top-bar")
        .w_full()
        .h(52.0)
        .px(if narrow { 10.0 } else { 14.0 })
        .gap(12.0)
        .items_center()
        .justify_between()
        .fill(p.bg)
        .child(left)
        .child(nav::actions(s, p, narrow))
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = theme::palette(cx.system_dark());
    let k = theme::kit(&p);
    let narrow = cx
        .layout_with("app", |r| u8::from(r[2] < NARROW))
        .unwrap_or(0)
        == 1;
    s.narrow.set(narrow);
    if let Some(w) = cx.layout_with("app", |r| r[2].round() as i32) {
        s.app_w.set(w as f32);
        // A window opened this narrow starts with the sidebar folded, whatever was remembered.
        if !s.sidebar_sized.replace(true) && (w as f32) <= nav::COLLAPSE_AT {
            s.sidebar_open.set(false);
        }
    }
    if let Some(h) = cx.layout_with("meeting-scroll", |r| r[3].round() as i32) {
        s.viewport_h.set(h as f32);
    }
    if let Some(h) = cx.layout_with("meeting-pane", |r| r[3].round() as i32) {
        s.pane_h.set(h as f32);
    }
    if let Some(h) = cx.layout_with("live-pane", |r| r[3].round() as i32) {
        s.pane_h.set(h as f32);
    }
    if let Some(h) = cx.layout_with("ask-overlay", |r| r[3].round() as i32) {
        s.overlay_h.set(h as f32);
    }
    let window_narrow = narrow;
    let narrow = window_narrow || s.app_w.get() - nav::width(s) < COMPACT;
    let mut layer = Vec::new();
    let tasks_view = s.view == tasks::View::Tasks;
    let on_check = s.on_check();
    let on_memory = s.on_memory();
    let on_hex = s.on_hex();
    let on_live = s.on_live();
    let home = !tasks_view && s.selected.is_none();
    let content = if tasks_view {
        tasks::pane(s, &k, p, narrow, &mut layer)
    } else if on_check {
        check::pane(s, &k, p, narrow, &mut layer)
    } else if on_memory {
        memory::pane(s, &k, p, narrow, &mut layer)
    } else if on_hex {
        hex::pane(s, &k, p, narrow, &mut layer)
    } else if on_live {
        live::pane(s, &k, p, narrow, &mut layer)
    } else if home {
        list::home(s, &k, p, narrow, &mut layer)
    } else {
        meeting::pane(s, &k, p, narrow, &mut layer)
    };
    let mut main = col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .fill(p.bg)
        .child(top_bar(s, p, narrow));
    if let Some(bar) = hex::error_bar(s, p) {
        main = main.child(bar);
    }
    let main = main.child(content);
    let mut app = row()
        .key("app")
        .w_full()
        .h_full()
        .fill(p.bg)
        .on_window_focus(|s: &mut State, focused, _| {
            if focused {
                s.rescan();
            }
            // A lost focus is not a hide: a window on another monitor still shows the indicator.
            s.hex_visibility(focused || hex::window_shown());
        });
    // The rail stays in every window. Open, the sidebar pushes the page aside, except in a
    // narrow window, where it is a drawer over it.
    let open = s.sidebar_is_open();
    app = app
        .child(nav::sidebar(s, p, open && !window_narrow))
        .child(rune::el().w(1.0).h_full().fill(p.line))
        .child(main);
    if window_narrow && open {
        app = app
            .child(
                rune::el()
                    .key("nav-scrim")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(p.shadow)
                    .on_press(|s: &mut State| s.drawer_open.set(false)),
            )
            .child(
                nav::sidebar(s, p, true)
                    .pin(Some(0.0), Some(0.0), None, Some(0.0))
                    .shadow(0.0, 32.0, p.shadow),
            );
    }
    if let Some(finder) = finder::overlay(s, &k, p, window_narrow) {
        app = app.child(finder);
    }
    if let Some((text, _)) = &s.toast {
        app = app.child(widgets::toast(text, p));
    }
    rune_kit::root(app, layer)
}

/// Scenes that tick or edit tasks write files, so they run only against the isolated state
/// folder the render script sets.
fn isolated() -> bool {
    sidenote::env_var("SIDENOTE_STATE_DIR").is_some()
}

/// The sidebar open or folded, whatever the window's width would have chosen.
fn scene_sidebar(s: &mut State, open: bool) {
    s.sidebar_open.set(open);
    s.drawer_open.set(open);
    s.sidebar_sized.set(true);
}

/// Names the first project, so the sidebar and a project filter have something to show on a
/// state folder that has no `projects.json`.
fn scene_projects(s: &mut State) {
    if s.tasks.names.is_empty() {
        s.tasks.names = vec!["Northwind".into(), "Sidenote".into(), "Hiring".into()];
    }
}

/// Home with two meetings marked as from another device, so that filter has a row.
fn scene_filters(s: &mut State) {
    scene_projects(s);
    for (n, m) in s.meetings.iter_mut().enumerate() {
        if n == 1 || n == 3 {
            m.device = Some("desktop".into());
        }
    }
    s.rebuild_nav();
    s.refresh_list();
}

/// The project with the most meetings, for the filter scene.
fn busiest_project(s: &State) -> String {
    s.tasks
        .names
        .iter()
        .max_by_key(|n| s.nav.project_counts.get(*n).copied().unwrap_or(0))
        .cloned()
        .unwrap_or_default()
}

fn scene_tasks(s: &mut State) {
    s.set_view(tasks::View::Tasks);
}

/// The sidebar scenes: the open sidebar with every signal it can show. The render script writes
/// a longer synthetic project list and a memory fixture with proposals; two high findings stand
/// in for a check report that has some. `page` then moves to the page under test.
fn scene_side(s: &mut State, page: fn(&mut State)) {
    hex::scene(s, false);
    page(s);
    scene_filters(s);
    scene_sidebar(s, true);
    s.check.latest.high = 2;
}

/// Opens the panel on the first task, or on the first one with a transcript line.
fn scene_panel(s: &mut State, with_source: bool) {
    scene_tasks(s);
    let row = if with_source {
        s.tasks.first_with_source()
    } else {
        s.tasks.first_row()
    };
    if let Some((id, index)) = row {
        s.open_task(&id, index);
    }
}

fn scene_tick(s: &mut State, index: usize) {
    let task = s
        .meetings
        .iter()
        .find(|m| m.id == REVIEW_MEETING)
        .and_then(|m| m.summary.todo.get(index))
        .map(|t| t.task.clone());
    if let (true, Some(task)) = (isolated(), task) {
        s.toggle_task_at(REVIEW_MEETING, &task);
    }
}

fn scene_due(s: &mut State, index: usize, date: Option<chrono::NaiveDate>) {
    if let (true, Some(d)) = (isolated(), date) {
        s.set_task_due(REVIEW_MEETING, index, tasks::DueChoice::Date(d));
    }
}

/// Pretends only these programs are installed, so an Ask render does not depend on the machine
/// it runs on. `scene_agents(s, &[])` is a machine with no agent at all.
fn scene_agents(s: &mut State, programs: &[&str]) {
    s.runner_found = s
        .runners
        .iter()
        .filter(|r| programs.contains(&r.program.as_str()))
        .map(|r| r.name.clone())
        .collect();
    s.runner = default_runner(&s.runners, &s.runner_found, Some("Claude Sonnet".into()));
}

/// Every agent the picker knows.
const ALL_AGENTS: &[&str] = &["claude", "codex", "opencode"];

/// The invented meeting the notes scene opens.
const NOTES_MEETING: &str = "1794100000000-1";

/// Invented notes and transcript lines for the live scenes: no real meeting content.
const SCENE_NOTES: &str = "Onboarding flow review\n- Launch date: second week of December\n- Pricing page copy is still open\n- Ask Maya for the survey results\n\nFollow up: send the draft on Friday";
const SCENE_LINES: &[(u64, bool, &str)] = &[
    (4, false, "Morning everyone, thanks for joining."),
    (
        9,
        true,
        "Thanks. I would like to start with the launch date.",
    ),
    (17, false, "We are aiming for the second week of December."),
    (
        26,
        true,
        "That works if the pricing page copy is done by then.",
    ),
    (34, false, "Maya, can you share the survey results?"),
    (41, false, "I will send them after the call."),
    (52, true, "Great. Let us also look at the onboarding flow."),
    (
        63,
        false,
        "Step three loses the most people, so start there.",
    ),
    (
        75,
        true,
        "Agreed. I will draft a shorter version of that step.",
    ),
    (88, false, "Can we test it with five customers next week?"),
    (97, true, "Yes, I can set that up on Monday."),
];

/// The live page while recording, with the Ask pill ready and the given notes and lines.
fn scene_live(s: &mut State, notes: &str, lines: &[(u64, bool, &str)], panel: bool) {
    hex::scene(s, true);
    scene_agents(s, ALL_AGENTS);
    let mut live = live::Live::scene(live::Phase::Recording, None).scene_feed(lines, panel);
    if !notes.is_empty() {
        live.saving = live::Saving::Saved;
    }
    s.live = Some(live);
    s.live_open = true;
    s.live_notes = TextEdit::new(notes);
    s.live_notes.focused = !notes.is_empty();
}

fn scene_meeting(s: &mut State) {
    if s.meetings.iter().any(|m| m.id == REVIEW_MEETING) {
        s.selected = Some(REVIEW_MEETING.into());
        s.load_saved();
    }
}

/// The full report of the newest check, opened from the Diary. Scenes that act on it run only
/// against the isolated state folder, where `tools/prepare_check_fixture.py` put a copy of a
/// report.
fn scene_check(s: &mut State) {
    s.open_check();
    if let Some(date) = s.check.dates.first().copied() {
        s.open_report(date);
    }
}

/// The Diary over the invented days of `prepare_check_fixture.py --diary`. In the isolated
/// state folder it first ticks the first task of the two newest meetings, so today has a Done
/// list.
fn scene_diary(s: &mut State) {
    if isolated() {
        let mut ticks: Vec<(String, String)> = s
            .meetings
            .iter()
            .filter_map(|m| Some((m.id.clone(), m.summary.todo.first()?.task.clone())))
            .collect();
        ticks.truncate(2);
        for (id, task) in ticks {
            s.toggle_task_at(&id, &task);
        }
    }
    s.open_check();
}

/// Opens these Diary sections on every day the Diary lists.
fn scene_diary_open(s: &mut State, parts: &[diary::Part]) {
    let mut days: Vec<chrono::NaiveDate> = s
        .meetings
        .iter()
        .filter_map(|m| m.started.map(|t| t.date_naive()))
        .collect();
    days.extend(s.check.dates.iter().copied());
    days.push(s.check.today);
    for d in days {
        for &part in parts {
            s.check.open_parts.insert(diary::part_key(d, part));
        }
    }
}

/// The key of the first open finding with a proposed action.
fn first_action(s: &State) -> Option<String> {
    let r = s.check.report.as_ref()?;
    let i = r.content.findings.iter().position(|f| f.action.is_some())?;
    Some(if r.schema_version == 2 {
        r.content.findings[i].id.clone()
    } else {
        format!("v1:{i}")
    })
}

fn scene_check_scroll(s: &mut State, key: &str) {
    scene_check(s);
    scene_scroll(s, key);
}

/// The panel open on the first finding that has an action.
fn scene_check_panel(s: &mut State) {
    scene_check(s);
    if let Some(key) = first_action(s) {
        s.open_finding(&key);
    }
}

/// Applies the first action, dismisses the last finding that has none and snoozes another, so
/// one render shows every kind of mark. Only against the isolated state folder.
fn scene_check_apply(s: &mut State) {
    scene_check_panel(s);
    let Some(key) = first_action(s) else {
        return;
    };
    if !isolated() {
        return;
    }
    s.apply_finding(&key);
    let keys: Vec<String> = s
        .check
        .report
        .as_ref()
        .map(|r| {
            r.content
                .findings
                .iter()
                .enumerate()
                .filter(|(_, f)| f.action.is_none())
                .map(|(i, f)| {
                    if r.schema_version == 2 {
                        f.id.clone()
                    } else {
                        format!("v1:{i}")
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(last) = keys.last() {
        s.dismiss_finding(last);
    }
    if let (Some(first), true) = (
        keys.first(),
        s.check
            .report
            .as_ref()
            .is_some_and(|r| r.schema_version == 2),
    ) {
        let tomorrow = s.check.today.succ_opt().unwrap_or(s.check.today);
        s.snooze_finding(first, tomorrow);
    }
}

fn scene_memory(s: &mut State) {
    s.open_memory();
}

/// The Memory page with the panel on the first row of the current list.
fn scene_memory_panel(s: &mut State) {
    scene_memory(s);
    if let Some(id) = s.memory.shown_ids().first().cloned() {
        s.open_record(&id);
    }
}

fn scene_hex(s: &mut State, section: hex::Section, open_groups: bool) {
    s.open_hex();
    hex::scene(s, false);
    s.hex.section = section;
    if open_groups {
        s.hex.open_all_groups();
    }
}

fn scene_scroll(s: &mut State, key: &str) {
    s.pending_scroll = Some(PendingScroll {
        key: key.into(),
        center: false,
        left: 3,
    });
}

/// The invented meeting the screens scenes open. It lives in `tools/ui-fixtures/screens`, with
/// four smaller ones, and every name in it is made up.
const SCREENS_MEETING: &str = "1793955600000-1";

/// The screens fixture's main meeting, open, with agents ready for the Ask bar.
fn scene_screens(s: &mut State) {
    hex::scene(s, false);
    scene_agents(s, ALL_AGENTS);
    if s.meetings.iter().any(|m| m.id == SCREENS_MEETING) {
        s.selected = Some(SCREENS_MEETING.into());
        s.load_saved();
    }
}

/// Home for the screens fixture, with agents ready.
fn scene_screens_home(s: &mut State) {
    hex::scene(s, false);
    scene_agents(s, ALL_AGENTS);
}

/// One late to-do, two due today and one due tomorrow, set through the real edit path, so the
/// Today strip counts the same on any day. Only against the isolated state folder.
fn scene_screens_due(s: &mut State) {
    if !isolated() {
        return;
    }
    let today = s.tasks.today;
    let days = [-1_i64, 0, 0, 1];
    for (index, offset) in days.into_iter().enumerate() {
        if let Some(day) = today.checked_add_signed(chrono::Duration::days(offset)) {
            s.set_task_due(SCREENS_MEETING, index, tasks::DueChoice::Date(day));
        }
    }
}

/// Home with all three Today lines: a meeting still being made into notes, to-dos due, and the
/// newest daily check (tools/prepare_check_fixture.py writes a synthetic one).
fn scene_screens_today(s: &mut State) {
    scene_screens_home(s);
    scene_screens_due(s);
    s.live = Some(live::Live::scene(
        live::Phase::Making,
        Some("Pricing review with Maya"),
    ));
}

/// Home with nothing to say: no meeting in progress, no to-do due and no check report, so the
/// Today strip is gone.
fn scene_screens_quiet(s: &mut State) {
    scene_screens_home(s);
    s.tasks.items.clear();
}

pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((360.0, 480.0)),
        fonts: theme::fonts(),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: &[
            ("sync-footer-now", |s| {
                scene_sidebar(s, true);
                s.last_incoming = Some(Instant::now());
            }),
            ("sync-footer-age", |s| {
                scene_sidebar(s, true);
                s.last_incoming = Instant::now().checked_sub(Duration::from_secs(300));
            }),
            ("sync-footer-off", |s| {
                scene_sidebar(s, true);
                s.last_incoming = None;
            }),
            ("sync-memory-merged", sync_scene::memory_scene),
            ("sync-task-merged", sync_scene::task_scene),
            ("sync-arrival-draft", sync_scene::arrival_scene),
            ("updates-settings", updates::settings_scene),
            ("updates-manual", |s| {
                updates::scene(s, sidenote::updates::Phase::Manual)
            }),
            ("updates-preparing", |s| {
                updates::scene(s, sidenote::updates::Phase::Preparing)
            }),
            ("updates-stopping", |s| {
                updates::scene(s, sidenote::updates::Phase::StoppingListener)
            }),
            ("updates-none", |s| {
                updates::scene(s, sidenote::updates::Phase::Idle)
            }),
            ("updates-available", |s| {
                updates::scene(s, sidenote::updates::Phase::Available)
            }),
            ("updates-downloading", |s| {
                updates::scene(s, sidenote::updates::Phase::Downloading)
            }),
            ("updates-ready", |s| {
                updates::scene(s, sidenote::updates::Phase::Ready)
            }),
            ("updates-error", |s| {
                updates::scene(s, sidenote::updates::Phase::Error)
            }),
            ("models-empty", |s| models::scene(s, "empty")),
            ("models-downloading", |s| models::scene(s, "downloading")),
            ("models-installed", |s| models::scene(s, "installed")),
            ("models-checksum-error", |s| {
                models::scene(s, "checksum-error")
            }),
            ("features-off-sidebar", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, true);
            }),
            ("features-off-home", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, false);
            }),
            ("features-off-meeting", |s| {
                hex::scene(s, false);
                scene_meeting(s);
                scene_sidebar(s, true);
                scene_scroll(s, "sec-todo");
            }),
            ("features-tasks-only", |s| {
                hex::scene(s, false);
                scene_tasks(s);
                scene_sidebar(s, true);
            }),
            ("features-settings", |s| {
                scene_hex(s, hex::Section::Features, false);
                scene_sidebar(s, true);
            }),
            // The meeting page with each tab, the transcript panel with a search hit, and the
            // "..." menu. All read the invented meetings in tools/ui-fixtures/screens.
            ("screens-summary", scene_screens),
            ("screens-notes", |s| {
                scene_screens(s);
                s.meeting_tab = meeting::Tab::Notes;
            }),
            ("screens-transcript", |s| {
                scene_screens(s);
                s.transcript_open = true;
                s.transcript_filter = TextEdit::new("launch");
            }),
            ("screens-transcript-jump", |s| {
                scene_screens(s);
                s.transcript_open = true;
                if let Some(line) = s.current().and_then(|m| m.target("11:30")) {
                    s.highlight = Some(line);
                    s.pending_scroll = Some(PendingScroll {
                        key: format!("line-{line}"),
                        center: true,
                        left: 3,
                    });
                }
            }),
            ("screens-more-menu", |s| {
                scene_screens(s);
                s.more_menu = Some(format!("more-meeting-{SCREENS_MEETING}"));
            }),
            ("screens-toast", |s| {
                scene_screens(s);
                s.toast = Some((
                    "Copied notes".into(),
                    Instant::now() + Duration::from_secs(600),
                ));
            }),
            // Home with a row's "..." menu open, the Today strip, and the strip hidden.
            ("screens-home-more", |s| {
                scene_screens_home(s);
                s.more_menu = Some(format!("more-row-{SCREENS_MEETING}"));
            }),
            ("screens-today", scene_screens_today),
            ("screens-today-tasks", |s| {
                scene_screens_home(s);
                scene_screens_due(s);
            }),
            ("screens-today-quiet", scene_screens_quiet),
            // The Ctrl+K palette: empty, with a query, and the query handed to Ask with Tab.
            ("screens-finder", |s| {
                scene_screens_home(s);
                s.open_finder();
            }),
            ("screens-finder-query", |s| {
                scene_screens_home(s);
                s.open_finder();
                s.finder_query = TextEdit::new("launch");
                s.finder_query.focused = true;
                s.finder_changed();
            }),
            // The Ask bar focused over a list with text behind its chips.
            ("screens-ask-focused", |s| {
                scene_screens_home(s);
                s.question.focused = true;
                s.ask_open = true;
            }),
            ("screens-finder-keys", scene_screens_home),
            ("screens-finder-none", |s| {
                scene_screens_home(s);
                s.open_finder();
                s.finder_query = TextEdit::new("zebra");
                s.finder_query.focused = true;
                s.finder_changed();
            }),
            ("screens-finder-off", |s| {
                scene_screens_home(s);
                s.config.features.tasks = false;
                s.config.features.memory = false;
                s.config.features.daily_check = false;
                s.open_finder();
            }),
            ("merge-worker-proof", |s| {
                project_merge::scene(s, "worker-proof")
            }),
            ("merge-preparing", |s| project_merge::scene(s, "preparing")),
            ("merge-merging", |s| project_merge::scene(s, "merging")),
            ("merge-undoing", |s| project_merge::scene(s, "undoing")),
            ("merge-preview-expanded", |s| {
                project_merge::scene(s, "preview-expanded")
            }),
            ("merge-page", |s| project_merge::scene(s, "page")),
            ("merge-picker", |s| project_merge::scene(s, "picker")),
            ("merge-preview", |s| project_merge::scene(s, "preview")),
            ("merge-done", |s| project_merge::scene(s, "done")),
            ("project-choices", |s| {
                project::scene(s, "suggest");
                let id = s.meetings[0].id.clone();
                s.start_project_form(&id);
                s.project_input = TextEdit::new("Harbor research");
                s.selected = Some(id);
                s.load_saved();
            }),
            ("audio-project-meeting", |s| project::scene(s, "meeting")),
            ("audio-project-existing", |s| project::scene(s, "existing")),
            ("audio-project-new", |s| project::scene(s, "suggest")),
            ("audio-project-created", |s| project::scene(s, "create")),
            ("audio-project-dismissed", |s| project::scene(s, "dismiss")),
            ("list", |_| {}),
            ("search", |s| {
                s.search = TextEdit::new("landing");
                s.on_search_changed();
            }),
            ("meeting", scene_meeting),
            // The review meeting with the sidebar folded to its rail.
            ("collapsed", |s| {
                scene_meeting(s);
                scene_sidebar(s, false);
            }),
            // The project chip's menu, and the chip after the suggestion is accepted.
            ("project-menu", |s| {
                scene_meeting(s);
                s.project_menu = Some(REVIEW_MEETING.into());
            }),
            ("project-accepted", |s| {
                scene_meeting(s);
                let name = s.meeting_project(REVIEW_MEETING).and_then(|m| m.name);
                if let (true, Some(name)) = (isolated(), name) {
                    s.set_meeting_project(REVIEW_MEETING, &name);
                }
            }),
            ("decisions", |s| {
                scene_meeting(s);
                scene_sidebar(s, false);
                scene_scroll(s, "sec-decisions");
            }),
            ("discussion", |s| {
                scene_meeting(s);
                scene_scroll(s, "sec-discussion");
            }),
            ("todo", |s| {
                scene_meeting(s);
                if let Some(task) = s
                    .current()
                    .and_then(|m| m.summary.todo.get(1).map(|t| t.task.clone()))
                {
                    s.toggle_task(&task);
                }
                scene_scroll(s, "sec-todo");
            }),
            ("jump", |s| {
                scene_meeting(s);
                if let Some(line) = s.current().and_then(|m| m.target("14:22")) {
                    s.transcript_open = true;
                    s.highlight = Some(line);
                    s.back = Some("pt-0-9".into());
                    s.pending_scroll = Some(PendingScroll {
                        key: format!("line-{line}"),
                        center: true,
                        left: 3,
                    });
                }
            }),
            ("ask", |s| {
                scene_meeting(s);
                scene_agents(s, ALL_AGENTS);
            }),
            // The transcript open on the review meeting, or on the first meeting when that one
            // is not here, which is how a folder of imported meetings is rendered.
            ("transcript", |s| {
                scene_meeting(s);
                if s.selected.is_none() {
                    s.selected = s.meetings.first().map(|m| m.id.clone());
                    s.load_saved();
                }
                s.transcript_open = true;
            }),
            // The review meeting with nothing decided, nothing to do and nothing open.
            ("empty", |s| {
                scene_meeting(s);
                let id = s.selected.clone();
                if let Some(m) = s.meetings.iter_mut().find(|m| Some(&m.id) == id.as_ref()) {
                    m.summary.todo.clear();
                    m.summary.to_settle.clear();
                    for t in &mut m.summary.topics {
                        t.points.retain(|p| p.kind != "decided");
                    }
                }
            }),
            ("ask-running", |s| {
                scene_meeting(s);
                scene_agents(s, ALL_AGENTS);
                if let Some(id) = s.selected.clone() {
                    s.saved.messages.push(Message {
                        role: "user".into(),
                        text: "What do I need to do next?".into(),
                        ..Default::default()
                    });
                    s.asks.insert(
                        id,
                        AskRun::stand_in(
                            "Claude Sonnet (answer only)",
                            &["Reading the meeting...", "Using Read"],
                            11,
                        ),
                    );
                }
            }),
            // The home page's Ask: a saved answer from the fixture, and a run in progress.
            ("home-ask", |s| scene_agents(s, ALL_AGENTS)),
            ("home-ask-running", |s| {
                scene_agents(s, ALL_AGENTS);
                s.saved.messages.push(Message {
                    role: "user".into(),
                    text: core::ask_all::SUGGESTIONS[0].into(),
                    ..Default::default()
                });
                s.asks.insert(
                    core::ALL_KEY.into(),
                    AskRun::stand_in(
                        "Claude Sonnet (answer only)",
                        &["Reading your meetings...", "Using Read"],
                        11,
                    ),
                );
            }),
            ("ask-live", |s| {
                scene_meeting(s);
                s.runner = Some("Claude Sonnet (answer only)".into());
            }),
            ("check", scene_check),
            ("diary", scene_diary),
            ("diary-panel", |s| {
                scene_diary(s);
                scene_diary_open(s, &[diary::Part::Attention]);
                if let Some(key) = first_action(s) {
                    s.open_finding(&key);
                }
            }),
            // Every section of the newest days open, lows included.
            ("diary-open", |s| {
                scene_diary(s);
                scene_diary_open(
                    s,
                    &[
                        diary::Part::Tasks,
                        diary::Part::Memories,
                        diary::Part::Done,
                        diary::Part::Attention,
                        diary::Part::Low,
                    ],
                );
            }),
            ("diary-picker", |s| {
                scene_diary(s);
                s.check.picker_open = true;
            }),
            ("diary-older", |s| {
                scene_diary(s);
                scene_scroll(s, "diary-2026-11-05");
            }),
            ("diary-report", |s| {
                scene_diary(s);
                if let Some(date) = s.check.dates.get(1).copied() {
                    s.open_report(date);
                }
            }),
            ("diary-sidebar", |s| scene_side(s, scene_diary)),
            ("check-panel", scene_check_panel),
            ("check-applied", scene_check_apply),
            ("check-story", |s| {
                scene_check(s);
                if let Some(a) = s.check.report.as_ref().and_then(|r| {
                    r.content.story_alignment.iter().find(|a| {
                        matches!(a.status, sidenote::core::check_v2::AlignmentStatus::Tension)
                    })
                }) {
                    let name = a.project.clone();
                    s.check.story_open.insert(name);
                }
                scene_scroll(s, "check-story");
            }),
            ("check-changed", |s| {
                scene_check(s);
                s.check.gone = Some(check::Gone::Unmatched);
                scene_scroll(s, "check-changed");
            }),
            ("check-history", |s| scene_check_scroll(s, "check-history")),
            ("check-details", |s| {
                scene_check(s);
                s.check.details_open = true;
            }),
            ("check-snooze", |s| {
                scene_check_panel(s);
                s.check.snooze_menu = true;
            }),
            ("check-snooze-pick", |s| {
                scene_check_panel(s);
                s.check.snooze_pick = true;
                s.check.snooze_date.focused = true;
            }),
            ("check-picker", |s| {
                scene_check(s);
                s.check.picker_open = true;
            }),
            // The Hex page, and the home list with the engine idle or recording. All three read
            // tools/ui-fixtures/hex.json, so no render calls the live Hex.
            ("hex", |s| {
                s.open_hex();
                hex::scene(s, false);
            }),
            ("hex-list", |s| hex::scene(s, false)),
            ("hex-list-recording", |s| hex::scene(s, true)),
            // The sections of the Hex page: Model is the default; Advanced is closed, then open.
            ("hex-vocabulary", |s| {
                scene_hex(s, hex::Section::Vocabulary, false)
            }),
            ("hex-general", |s| {
                scene_hex(s, hex::Section::General, false)
            }),
            ("hex-dictation", |s| {
                scene_hex(s, hex::Section::Dictation, false)
            }),
            ("hex-meetings", |s| {
                scene_hex(s, hex::Section::Meetings, false)
            }),
            ("routing-settings", |s| {
                models::scene(s, "installed");
                s.config.pipeline.profile = "experimental".into();
                s.config.pipeline.meeting_languages = vec!["en".into(), "hi".into()];
            }),
            ("routing-original", |s| {
                scene_meeting(s);
                s.transcript_open = true;
                s.transcript_original = true;
                if let Some(m) = s.meetings.iter_mut().find(|m| m.id == REVIEW_MEETING) {
                    use sidenote::core::meeting_router::{Chunk, Offset, Segment, Transcript};
                    m.routed = Some(Transcript {
                        schema: "transcript.segments.v1".into(),
                        experimental: true,
                        input_checksum: String::new(),
                        config_checksum: String::new(),
                        baseline: Vec::new(),
                        mixed_calls: 0,
                        mixed_seconds: 0.0,
                        english_reading: m.transcript_raw.clone(),
                        segments: m
                            .lines
                            .iter()
                            .enumerate()
                            .map(|(i, l)| Segment {
                                start: l.seconds.unwrap_or(0) as f64,
                                end: l.seconds.unwrap_or(0) as f64 + 3.0,
                                track: "system".into(),
                                route: if i == 2 { "hi" } else { "en" }.into(),
                                lang: if i == 2 { "hi" } else { "en" }.into(),
                                lang_conf: None,
                                text_original: l.text.clone(),
                                text_en: Some(l.text.clone()),
                                english_reading: l.text.clone(),
                                provider: if i == 2 { "runtime" } else { "hex" }.into(),
                                model: if i == 2 {
                                    "indicconformer-hi-ctc"
                                } else {
                                    "parakeet_unified_en"
                                }
                                .into(),
                                status: if i == 1 {
                                    "language not checked"
                                } else {
                                    "checked"
                                }
                                .into(),
                                translate: false,
                                sources: vec![Offset {
                                    chunk: Chunk {
                                        id: i.to_string(),
                                        start: l.seconds.unwrap_or(0) as f64,
                                        end: l.seconds.unwrap_or(0) as f64 + 3.0,
                                        track: "system".into(),
                                        speaker: Some(l.who.clone()),
                                        turn: None,
                                        english: l.text.clone(),
                                        audio_start: None,
                                    },
                                    joined_start: 0.0,
                                    joined_end: 3.0,
                                    scores: None,
                                }],
                            })
                            .collect(),
                        wall_seconds: 0.0,
                        lid_load_seconds: None,
                        hi_load_seconds: None,
                        warnings: Vec::new(),
                    });
                    m.routed_revision = m.routed.as_ref().map(|r| r.revision()).unwrap_or_default();
                }
                // Exercise an arriving artifact while this meeting is already open.
                let mut published = s.meetings.clone();
                if let Some(meeting) = published.iter_mut().find(|m| m.id == REVIEW_MEETING)
                    && let Some(routed) = meeting.routed.as_mut()
                {
                    routed.segments[2].text_original = "Prepare the updated color samples.".into();
                    meeting.routed_revision = routed.revision();
                }
                let (memory, check) = (s.config.features.memory, s.config.features.daily_check);
                s.config.features.memory = false;
                s.config.features.daily_check = false;
                let mut snapshot = arrival::Snapshot::read();
                snapshot.meetings = published;
                snapshot.pipeline = s.tasks.pipeline.clone();
                s.apply_event(Event::Meetings(Box::new(snapshot)));
                s.config.features.memory = memory;
                s.config.features.daily_check = check;
            }),
            ("hex-shortcuts", |s| {
                scene_hex(s, hex::Section::Shortcuts, false)
            }),
            ("hex-advanced", |s| {
                scene_hex(s, hex::Section::Advanced, false)
            }),
            ("hex-advanced-open", |s| {
                scene_hex(s, hex::Section::Advanced, true)
            }),
            ("hex-search", |s| {
                scene_hex(s, hex::Section::Model, false);
                s.hex.filter = TextEdit::new("shortcut");
            }),
            // The sidebar open and folded on the home list, with projects and a device filter.
            ("home-open", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, true);
            }),
            ("home-collapsed", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, false);
            }),
            ("home-project", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, true);
                let name = busiest_project(s);
                s.set_filter(nav::Filter::Project(name));
            }),
            ("home-peers", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_sidebar(s, true);
                s.set_filter(nav::Filter::Peers);
            }),
            // Other pages with the sidebar open, to check each page beside it.
            ("tasks-open", |s| {
                hex::scene(s, false);
                scene_filters(s);
                scene_tasks(s);
                scene_sidebar(s, true);
            }),
            // The sidebar beside each page it serves, with an update waiting, with a project
            // chosen, with every project shown, and folded to its rail.
            ("sidebar-meetings", |s| scene_side(s, |_| {})),
            ("sidebar-meeting", |s| scene_side(s, scene_meeting)),
            ("sidebar-tasks", |s| scene_side(s, scene_tasks)),
            ("sidebar-memory", |s| scene_side(s, scene_memory)),
            ("sidebar-check", |s| scene_side(s, scene_check)),
            ("sidebar-settings", |s| scene_side(s, |s| s.open_hex())),
            ("sidebar-update", |s| {
                scene_side(s, |s| {
                    updates::scene(s, sidenote::updates::Phase::Ready);
                    s.go_meetings();
                })
            }),
            ("sidebar-project", |s| {
                scene_side(s, |_| {});
                if let Some(name) = s.tasks.names.get(1).cloned() {
                    s.set_filter(nav::Filter::Project(name));
                }
            }),
            ("sidebar-all", |s| scene_side(s, nav::scene_all)),
            ("sidebar-rail", |s| {
                scene_side(s, |_| {});
                scene_sidebar(s, false);
            }),
            // The end of the list, to see the last meetings clear the Ask bar.
            ("home-end", |s| {
                let last = s
                    .row_count()
                    .checked_sub(1)
                    .and_then(|n| s.row_at(n))
                    .map(|i| s.meetings[i.index].id.clone());
                if let Some(id) = last {
                    scene_scroll(s, &format!("meeting-{id}"));
                }
            }),
            // The live page while recording, while starting, and while the notes are made.
            ("live-recording", |s| scene_live(s, "", &[], false)),
            ("live-starting", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
                s.live = Some(live::Live::scene(live::Phase::Starting, None));
                s.live_open = true;
            }),
            // Notes typed while recording, the transcript panel open, and the panel before the
            // first line. The notes and lines are invented.
            ("live-notes-typed", |s| {
                scene_live(s, SCENE_NOTES, &[], false)
            }),
            ("live-panel", |s| {
                scene_live(s, SCENE_NOTES, SCENE_LINES, true)
            }),
            ("live-panel-empty", |s| {
                scene_live(s, SCENE_NOTES, &[], true)
            }),
            ("live-panel-closed", |s| {
                scene_live(s, SCENE_NOTES, SCENE_LINES, false)
            }),
            ("live-making-notes", |s| {
                hex::scene(s, false);
                s.live = Some(live::Live::scene(
                    live::Phase::Making,
                    Some("Onboarding flow and launch date"),
                ));
                s.live_open = true;
                s.live_notes = TextEdit::new(SCENE_NOTES);
            }),
            // A finished meeting with the user's own notes above the AI notes. It reads the
            // invented meeting in tools/ui-fixtures/live-notes/meetings.
            ("meeting-your-notes", |s| {
                if s.meetings.iter().any(|m| m.id == NOTES_MEETING) {
                    s.selected = Some(NOTES_MEETING.into());
                    s.load_saved();
                }
                scene_agents(s, ALL_AGENTS);
            }),
            // Notes being made: with the quick title, and before it has arrived ("Naming...").
            ("live-making", |s| {
                hex::scene(s, false);
                s.live = Some(live::Live::scene(
                    live::Phase::Making,
                    Some("Weekly planning with Priya"),
                ));
                s.live_open = true;
            }),
            ("live-naming", |s| {
                hex::scene(s, false);
                s.live = Some(live::Live::scene(live::Phase::Making, None));
                s.live_open = true;
            }),
            // Recording while a meeting is open: the top right still has the way back and Stop.
            ("recording-away", |s| {
                hex::scene(s, true);
                s.live = Some(live::Live::scene(live::Phase::Recording, None));
                scene_meeting(s);
            }),
            // The Ask bar: at rest, with the box focused, with a question typed, with the
            // Suggestions button pressed, and with the model menu open.
            ("ask-idle", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
            }),
            ("ask-focused", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
                s.question.focused = true;
                s.ask_open = true;
            }),
            ("ask-typed", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
                s.question = TextEdit::new("What is still open with the hiring plan?");
                s.question.focused = true;
                s.ask_open = true;
            }),
            // Focus shows the suggestions now, so this is the same bar the old Suggestions
            // button opened.
            ("ask-suggestions", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
                s.question.focused = true;
                s.ask_open = true;
            }),
            ("ask-model", |s| {
                hex::scene(s, false);
                scene_agents(s, ALL_AGENTS);
                s.runner_open = true;
            }),
            // Only Claude Code installed: the other agents say so and cannot be picked.
            ("ask-model-missing", |s| {
                hex::scene(s, false);
                scene_agents(s, &["claude"]);
                s.runner_open = true;
            }),
            // No agent on this machine, before and after Check again.
            ("ask-no-agent", |s| {
                hex::scene(s, false);
                scene_agents(s, &[]);
            }),
            ("ask-no-agent-again", |s| {
                hex::scene(s, false);
                scene_agents(s, &[]);
                s.agents_rechecked = true;
            }),
            ("ask-focused-meeting", |s| {
                hex::scene(s, false);
                scene_meeting(s);
                scene_agents(s, ALL_AGENTS);
                s.question.focused = true;
                s.ask_open = true;
            }),
            ("memory", scene_memory),
            ("memory-panel", scene_memory_panel),
            ("memory-review", |s| {
                scene_memory(s);
                s.memory.tab = memory::Tab::Review;
                if let Some(id) = s.memory.shown_ids().first().cloned() {
                    s.open_record(&id);
                }
            }),
            ("memory-retired", |s| {
                scene_memory(s);
                s.memory.scope = memory::Scope::Retired;
            }),
            ("memory-skim", |s| {
                scene_memory(s);
                s.memory.before = memory::Before::Candidates;
                if let Some(id) = s.memory.shown_ids().get(2).cloned() {
                    s.open_record(&id);
                }
            }),
            ("tasks", scene_tasks),
            ("tasks-meeting", |s| {
                scene_tasks(s);
                s.tasks.grouping = core::TaskGrouping::Meeting;
                s.tasks.done = core::DoneFilter::All;
                scene_tick(s, 0);
            }),
            ("tasks-due", |s| {
                scene_tasks(s);
                s.tasks.grouping = core::TaskGrouping::Due;
                // One task a day late and one due today, through the real edit path.
                let today = s.tasks.today;
                scene_due(s, 0, today.pred_opt());
                scene_due(s, 1, Some(today));
            }),
            // The panel open on a task: properties, and the transcript lines it came from.
            ("tasks-panel", |s| scene_panel(s, true)),
            ("tasks-expanded", |s| scene_panel(s, true)),
            // The same, grouped by project, so the list drops its project column.
            ("tasks-panel-project", |s| {
                s.tasks.grouping = core::TaskGrouping::Project;
                scene_panel(s, true);
            }),
            ("tasks-group-project", |s| {
                scene_tasks(s);
                s.tasks.grouping = core::TaskGrouping::Project;
            }),
            ("tasks-group-menu", |s| {
                scene_tasks(s);
                s.tasks.group_menu = true;
            }),
            // A cutoff in the scene's tasks-settings.json (tools/render_ui.ps1 writes it): the
            // old tasks are gone and the line at the end says so. The menu opens its choices.
            ("tasks-cleared", |s| {
                scene_tasks(s);
                scene_scroll(s, "tasks-cleared");
            }),
            ("tasks-cleared-menu", |s| {
                scene_tasks(s);
                scene_scroll(s, "tasks-cleared");
                s.tasks.cleared_menu = true;
            }),
            // The panel's project menu, and its due box ready to type into.
            ("tasks-menu", |s| {
                scene_panel(s, false);
                s.tasks.ui.project_menu = true;
            }),
            ("tasks-quick-due", |s| scene_panel(s, false)),
            // Two tasks done, the first group folded, the Done section closed.
            ("tasks-collapsed", |s| {
                scene_tasks(s);
                scene_tick(s, 0);
                scene_tick(s, 2);
                if let Some(g) = s.tasks.arranged().groups.first() {
                    s.tasks.collapsed.insert(g.key.clone());
                }
                scene_scroll(s, "tgroup-0");
            }),
            ("tasks-done-open", |s| {
                scene_tasks(s);
                scene_tick(s, 0);
                scene_tick(s, 2);
                s.tasks.done_open = true;
                s.pending_scroll = Some(PendingScroll {
                    key: "tasks-done".into(),
                    center: false,
                    left: 3,
                });
            }),
            // The project groups, for the rename script.
            ("tasks-groups", |s| {
                scene_tasks(s);
                s.tasks.grouping = core::TaskGrouping::Project;
                scene_scroll(s, "tgroup-0");
            }),
            ("tasks-add", |s| {
                scene_tasks(s);
                s.open_add();
            }),
            ("tasks-edit", |s| {
                scene_panel(s, false);
                s.tasks.inputs.due = rune::TextEdit::new("");
                s.tasks.inputs.due.focused = true;
            }),
            ("tasks-edit-owner", |s| {
                scene_panel(s, false);
                s.tasks.ui.owner_edit = true;
                s.tasks.inputs.owner = rune::TextEdit::new("");
                s.tasks.inputs.owner.focused = true;
            }),
        ],
        ..AppSpec::new("Sidenote", DEFAULT_SIZE, State::new, view)
    }
}

pub fn run() {
    // `tools/measure_launch.py` sets this to a file path to learn when the first frame was
    // presented. The file gets the Unix time in microseconds. Nothing is written otherwise.
    let mut app = benchmark::configure(rune::App::new(spec()));
    if let Some(path) = sidenote::env_var("SIDENOTE_TRACE_FIRST_FRAME") {
        let mut done = false;
        app = app.on_frame_encoded(move |f| {
            if !done && f.outcome() == rune::FrameOutcome::Drawn {
                done = true;
                let micros = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_micros());
                let _ = std::fs::write(&path, micros.to_string());
            }
        });
    }
    // The OS places a new window at a cascade offset, which at 125% scaling on a 1080p screen
    // pushes the bottom (and the Ask bar) off screen. Fit the size to the work area and centre.
    app = app.on_window(|win| {
        let hwnd = win
            .with_raw_handle(|h| match h.as_raw() {
                rune::raw_window_handle::RawWindowHandle::Win32(w) => w.hwnd.get(),
                _ => 0,
            })
            .unwrap_or(0);
        if hwnd != 0 {
            hex::remember_window(hwnd);
            sidenote::platform::set_window_icon(hwnd);
            if benchmark::show_offscreen(win, hwnd) {
                return;
            }
            win.resize_instant(sidenote::platform::fit_to_work_area(hwnd, DEFAULT_SIZE));
        }
        win.center();
    });
    app.run();
}
