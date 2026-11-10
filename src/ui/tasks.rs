//! The Tasks view: every to-do from every meeting (and the ones added by hand) in one grouped
//! list, one dense line each. A row opens a panel on the right, which also holds the add form.
use super::{
    PendingScroll, State,
    list::text_title,
    roles,
    theme::Palette,
    widgets::{hairline, label, pressable, scroll, spaced_caps},
};
use chrono::{Duration, Local, NaiveDate};
use rune::{El, Key, KeyEvent, TextEdit, TextStyle, Tx, col, el, row, text};
use rune_kit::{Kit, Menu, MenuItem, Placement, SegmentedControl, TextInput};
use sidenote::core::{
    self, DoneFilter, DueBucket, GroupKind, MANUAL, ManualTask, Meeting, Overrides, Pipeline,
    Projects, TaskCounts, TaskGroup, TaskGrouping, TaskItem, TaskOverride, TaskStore, TaskView,
    UNSORTED, Urgency, Who,
};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Meetings,
    Tasks,
}

/// A project name being edited in its group header.
pub struct Rename {
    pub from: String,
}
pub enum DueChoice {
    Date(NaiveDate),
    /// Show no date at all.
    Clear,
    /// Go back to the date the summary gave.
    Reset,
}

/// What the right-hand panel shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Panel {
    /// One task: its properties and where it came from.
    Task(RowId),
    /// The form for a task added by hand.
    Add,
}

/// The panel's open menus and typed-in alternatives. Reset whenever the panel opens a task.
#[derive(Default)]
pub struct PanelUi {
    pub waiting_menu: bool,
    pub waiting_text: bool,
    pub project_menu: bool,
    pub owner_menu: bool,
    /// "New project..." swaps the project menu for a text box.
    pub new_project: bool,
    /// "Someone else..." swaps the owner menu for a text box.
    pub owner_edit: bool,
    /// A problem with the title, shown under it.
    pub error: Option<String>,
    /// A date that could not be read, shown under the due date.
    pub due_error: Option<String>,
}
/// The panel's text boxes. They live outside the `Option` so a keystroke that arrives after
/// the panel closes has somewhere harmless to go.
#[derive(Default)]
pub struct PanelInputs {
    pub title: TextEdit,
    pub description: TextEdit,
    pub due: TextEdit,
    pub owner: TextEdit,
    pub project: TextEdit,
}

pub type RowId = (String, usize);

pub struct TasksState {
    pub store: TaskStore,
    pub pipeline: Pipeline,
    pub projects: Projects,
    pub overrides: Overrides,
    pub checked: HashMap<String, BTreeSet<String>>,
    /// Every task, cleared or not. Meeting views and Ask read this one.
    pub all: Vec<TaskItem>,
    /// The tasks the page lists: `all` without the cleared ones, unless `show_cleared`.
    pub items: Vec<TaskItem>,
    /// Project names to offer when moving a task.
    pub names: Vec<String>,
    pub counts: TaskCounts,
    pub today: NaiveDate,
    pub grouping: TaskGrouping,
    pub grouping_loaded: bool,
    pub teams: core::Teams,
    pub team_name: TextEdit,
    pub team_members: TextEdit,
    pub team_aliases: TextEdit,
    pub teams_notice: Option<String>,
    pub clear_confirm: Option<i64>,
    pub subtask_input: TextEdit,
    pub waiting_input: TextEdit,
    pub group_menu: bool,
    pub redo_menu: bool,
    pub redo_confirm: Option<String>,
    pub redo_problem: Option<String>,
    pub done: DoneFilter,
    pub who: Who,
    pub include_unassigned: bool,
    /// Tasks from meetings before this day are cleared (`tasks-settings.json`).
    pub cutoff: Option<NaiveDate>,
    /// Show the cleared tasks again. Kept for this session only.
    pub show_cleared: bool,
    pub cleared_menu: bool,
    pub search: TextEdit,
    pub panel: Option<Panel>,
    pub ui: PanelUi,
    pub inputs: PanelInputs,
    pub edit_base: serde_json::Value,
    pub edit_title: String,
    pub edit_description: String,
    pub show_full_titles: bool,
    pub titles_notice: bool,
    pub edit_item: Option<TaskItem>,
    pub edit_failed: bool,
    pub editor_generation: u64,
    /// The project the add form files the task under. None is Unsorted.
    pub add_project: Option<String>,
    pub rename_input: TextEdit,
    pub rename: Option<Rename>,
    pub notice: Option<String>,
    /// The row under the pointer.
    pub hovered: Option<RowId>,
    /// The selected row: moved with the keyboard, set by opening a task.
    pub cursor: Option<RowId>,
    pub keyboard: bool,
    /// Folded groups, by `TaskGroup::key`. Kept for this session only.
    pub collapsed: BTreeSet<String>,
    /// The Done section at the end of the page is open.
    pub done_open: bool,
}

impl TasksState {
    pub fn load(meetings: &[Meeting], root: PathBuf) -> Self {
        let store = TaskStore::new(root);
        let mut notice = None;
        let projects = store.load_projects().unwrap_or_else(|_| {
            notice = Some("Could not read projects.json. Project names may be missing.".into());
            Projects::default()
        });
        let overrides = store.load_overrides().unwrap_or_else(|_| {
            notice =
                Some("Could not read task-overrides.json. Your task edits are not shown.".into());
            Overrides::default()
        });
        let cutoff = store
            .load_settings()
            .unwrap_or_else(|_| {
                notice =
                    Some("Could not read tasks-settings.json. Old tasks are not cleared.".into());
                core::TasksSettings::default()
            })
            .cutoff();
        Self {
            teams: core::Teams::default(),
            team_name: TextEdit::new(""),
            team_members: TextEdit::new(""),
            team_aliases: TextEdit::new(""),
            teams_notice: None,
            clear_confirm: None,
            subtask_input: TextEdit::new(""),
            waiting_input: TextEdit::new(""),
            store: store.clone(),
            pipeline: core::load_pipeline(meetings),
            projects,
            overrides,
            checked: HashMap::new(),
            all: Vec::new(),
            items: Vec::new(),
            names: Vec::new(),
            counts: TaskCounts::default(),
            today: Local::now().date_naive(),
            grouping: TaskGrouping::Due,
            grouping_loaded: false,
            group_menu: false,
            redo_menu: false,
            redo_confirm: None,
            redo_problem: None,
            done: DoneFilter::Open,
            who: Who::Everyone,
            include_unassigned: false,
            cutoff,
            show_cleared: false,
            cleared_menu: false,
            search: TextEdit::new(""),
            panel: None,
            ui: PanelUi::default(),
            inputs: PanelInputs::default(),
            edit_base: serde_json::Value::Null,
            edit_title: String::new(),
            edit_description: String::new(),
            show_full_titles: core::UiPrefs::load(&store.root).show_full_titles,
            titles_notice: !core::UiPrefs::load(&store.root).task_titles_notice_seen,
            edit_item: None,
            edit_failed: false,
            editor_generation: 0,
            add_project: None,
            rename_input: TextEdit::new(""),
            rename: None,
            notice,
            hovered: None,
            cursor: None,
            keyboard: false,
            collapsed: BTreeSet::new(),
            done_open: false,
        }
    }

    fn item(&self, id: &str, index: usize) -> Option<&TaskItem> {
        self.items
            .iter()
            .find(|t| t.meeting_id == id && t.index == index)
            .or_else(|| {
                self.edit_item
                    .as_ref()
                    .filter(|t| t.meeting_id == id && t.index == index)
            })
    }

    fn close_menus(&mut self) {
        self.group_menu = false;
        self.redo_menu = false;
        self.cleared_menu = false;
        self.ui.project_menu = false;
        self.ui.owner_menu = false;
        self.ui.waiting_menu = false;
    }

    /// The page as laid out now: the groups and the folded Done section.
    pub fn arranged(&self) -> TaskView {
        core::arrange(&self.items, &self.filter(), self.grouping, self.today)
    }

    /// Every row the keyboard can reach, top to bottom.
    fn visible_rows(&self) -> Vec<RowId> {
        self.arranged()
            .rows(self.done_open, &self.collapsed)
            .into_iter()
            .map(|i| (self.items[i].meeting_id.clone(), self.items[i].index))
            .collect()
    }

    /// The first task on screen, which scenes use to anchor the panel.
    pub fn first_row(&self) -> Option<RowId> {
        self.visible_rows().into_iter().next()
    }

    /// The first task on screen that came from a meeting with a timestamp.
    pub fn first_with_source(&self) -> Option<RowId> {
        self.visible_rows().into_iter().find(|(id, index)| {
            self.item(id, *index)
                .is_some_and(|t| !t.manual && t.stamp.is_some())
        })
    }

    fn filter(&self) -> core::TaskFilter {
        core::TaskFilter {
            done: self.done,
            who: self.who,
            include_unassigned: self.include_unassigned,
            query: self.search.text.clone(),
        }
    }

    /// A text box in this view has the keyboard, so letters are typing, not commands.
    pub fn typing(&self) -> bool {
        self.search.focused
            || self.rename_input.focused
            || self.inputs.title.focused
            || self.inputs.description.focused
            || self.inputs.due.focused
            || self.inputs.owner.focused
            || self.inputs.project.focused
    }
}

pub const SAVE_FAILED: &str =
    "Could not save that change. Check that the app data folder is writable.";

fn input_with(text: &str) -> TextEdit {
    let mut e = TextEdit::new(text);
    e.focused = true;
    // The old text is selected, so typing replaces it.
    e.set_selection(0, text.len());
    e
}

fn focused_input() -> TextEdit {
    let mut e = TextEdit::new("");
    e.focused = true;
    e
}

fn due_text(d: NaiveDate) -> String {
    d.format("%a %-d %b").to_string()
}

impl State {
    pub(super) fn edit_task_v2(&mut self, id: &str, index: usize, edit: core::TaskV2Edit) {
        let Some(t) = self.tasks.item(id, index) else {
            return;
        };
        let source = if t.manual {
            self.tasks.store.root.clone()
        } else {
            let Some(m) = self
                .meetings
                .iter()
                .find(|m| m.id == id && m.device.is_none())
            else {
                return;
            };
            m.folder.clone()
        };
        self.publish_edit(super::edit_worker::Job::V2 {
            store: self.tasks.store.clone(),
            task: Box::new(t.clone()),
            source,
            edit,
        });
    }
    pub fn cached_project(&self, name: &str) -> Option<String> {
        if let Some(exact) = self.tasks.names.iter().find(|n| n.as_str() == name) {
            return Some(exact.clone());
        }
        let key = core::project_merge::normalized_name(name);
        let mut matches = self
            .tasks
            .names
            .iter()
            .filter(|n| core::project_merge::normalized_name(n) == key);
        let first = matches.next()?.clone();
        matches.next().is_none().then_some(first)
    }
    pub fn rebuild_tasks(&mut self) {
        self.refresh_areas();
    }

    /// Clears the tasks of meetings older than `days` days, or keeps all. Only the page changes;
    /// the meetings and their to-dos are untouched, so any choice can be undone.
    pub fn set_cutoff_days(&mut self, days: Option<i64>) {
        let day = days.map(|d| self.tasks.today - Duration::days(d));
        self.tasks.cleared_menu = false;
        match self.tasks.store.set_cutoff(day) {
            Ok(_) => {
                self.tasks.cutoff = day;
                self.tasks.show_cleared = false;
                self.tasks.cursor = None;
                self.close_panel();
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(SAVE_FAILED.into()),
        }
    }

    /// Tasks shows every task; Meetings goes to the home list.
    pub fn set_view(&mut self, view: View) {
        if view == View::Tasks && !self.config.features.tasks {
            return;
        }
        self.view = view;
        self.tasks.close_menus();
        if view == View::Tasks {
            self.rebuild_tasks();
        } else {
            self.close_panel();
            self.go_home();
        }
    }

    /// Opens a meeting from the Tasks view.
    pub fn open_meeting(&mut self, id: &str, tx: &mut Tx) {
        if id == MANUAL || id.starts_with("manual/") {
            return;
        }
        self.close_panel();
        self.view = View::Meetings;
        self.tasks.close_menus();
        self.select(id, tx);
    }

    /// Opens a meeting at the transcript line for a task's timestamp.
    pub fn open_meeting_at(&mut self, id: &str, index: usize, stamp: &str, tx: &mut Tx) {
        self.open_meeting(id, tx);
        self.jump(stamp, format!("todo-{index}"), tx);
    }

    /// Ticks a task. The To do panel writes the same tick file, so both stay in step.
    pub fn toggle_task_at(&mut self, id: &str, task: &str) {
        let todo = self
            .meetings
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.summary.todo.iter().find(|t| t.text() == task));
        let on = !todo.map_or_else(
            || self.tasks.checked.get(id).is_some_and(|c| c.contains(task)),
            |t| core::todo_done(t, self.tasks.checked.get(id)),
        );
        let stable_id = todo.map(|t| t.id.clone()).unwrap_or_default();
        self.publish_edit(super::edit_worker::Job::Tick {
            store: self.store.clone(),
            id: id.into(),
            task: task.into(),
            stable_id,
            on,
        });
    }

    /// Ticks any row: a meeting task through its meeting's tick file, a hand-made one in
    /// the overrides file.
    pub fn toggle_row(&mut self, id: &str, index: usize) {
        if id == MANUAL || id.starts_with("manual/") {
            self.publish_edit(super::edit_worker::Job::ManualTick {
                store: self.tasks.store.clone(),
                id: id.strip_prefix("manual/").unwrap_or(id).into(),
            });
        } else if let Some(task) = self.tasks.item(id, index).map(|t| t.task.clone()) {
            self.toggle_task_at(id, &task);
        }
    }

    fn change_manual(&mut self, row: &str, change: impl FnOnce(&mut ManualTask)) {
        match self
            .tasks
            .store
            .update_manual(row.strip_prefix("manual/").unwrap_or(row), change)
        {
            Ok(o) => {
                self.tasks.overrides = o;
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(SAVE_FAILED.into()),
        }
    }

    fn apply_override(
        &mut self,
        id: &str,
        index: usize,
        field: super::edit_worker::Field,
        change: impl FnOnce(&mut TaskOverride),
    ) {
        if self.tasks.panel == Some(Panel::Task((id.into(), index))) {
            self.save_task_edit(id, index, None, field, false, change);
            return;
        }
        if id == MANUAL || id.starts_with("manual/") {
            self.change_manual(id, |m| m.edit(change));
            return;
        }
        let Some(task) = self.tasks.item(id, index).map(|t| t.task.clone()) else {
            return;
        };
        match self.tasks.store.update_override(id, index, &task, change) {
            Ok(o) => {
                self.tasks.overrides = o;
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(SAVE_FAILED.into()),
        }
    }

    fn save_task_edit(
        &mut self,
        id: &str,
        index: usize,
        title: Option<&str>,
        field: super::edit_worker::Field,
        close: bool,
        change: impl FnOnce(&mut TaskOverride),
    ) -> bool {
        if self.edit_busy {
            return false;
        }
        let base = self.tasks.edit_base.clone();
        let mine = if id.starts_with("manual/") {
            let Ok(mut m) = serde_json::from_value::<ManualTask>(base.clone()) else {
                return false;
            };
            m.edit(change);
            if let Some(title) = title {
                m.title = Some(title.into());
            }
            serde_json::to_value(m).unwrap()
        } else {
            let mut o: TaskOverride = serde_json::from_value(base.clone()).unwrap_or_default();
            change(&mut o);
            o.task = self.panel_task().map(|t| t.task.clone());
            if let Some(title) = title {
                o.title = Some(title.into());
            }
            serde_json::to_value(o).unwrap()
        };
        let typed = match field {
            super::edit_worker::Field::Title => &self.tasks.inputs.title,
            super::edit_worker::Field::Owner | super::edit_worker::Field::Mine => {
                &self.tasks.inputs.owner
            }
            super::edit_worker::Field::Due => &self.tasks.inputs.due,
            super::edit_worker::Field::Project => &self.tasks.inputs.project,
        }
        .text
        .clone();
        let source = self
            .meetings
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.folder.clone());
        self.tasks.edit_failed = false;
        self.publish_edit(super::edit_worker::Job::Task {
            store: self.tasks.store.clone(),
            row: id.into(),
            index,
            base,
            mine,
            source,
            field,
            typed,
            close,
            session: self.tasks.editor_generation,
        });
        false
    }
    /// Finds a project by name, ignoring case, and adds a new name to the list.
    fn known_project(&mut self, name: &str) -> Option<String> {
        if !self.config.features.projects {
            return None;
        }
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if name == UNSORTED {
            return Some(name.into());
        }
        if let Some(name) = self.cached_project(name) {
            return Some(name);
        }
        self.tasks.notice = Some("Choose an existing project or click Create first.".into());
        None
    }

    pub fn set_task_project(&mut self, id: &str, index: usize, name: &str) {
        if let Some(name) = self.known_project(name) {
            let revision = match core::areas::new_assignment_revision(&self.tasks.store.root) {
                Ok(revision) => revision,
                Err(e) => {
                    self.tasks.notice = Some(e.to_string());
                    return;
                }
            };
            self.apply_override(id, index, super::edit_worker::Field::Project, |o| {
                o.project = Some(name.clone());
                o.assignment = Some(core::areas::ProjectChoice {
                    project: name,
                    revision,
                });
            });
        }
    }

    pub fn set_task_mine(&mut self, id: &str, index: usize, mine: bool) {
        self.apply_override(id, index, super::edit_worker::Field::Mine, |o| {
            o.mine = Some(mine)
        });
    }

    /// A name makes the task someone else's; empty text goes back to the owner the summary
    /// named.
    pub fn set_task_owner(&mut self, id: &str, index: usize, owner: &str) {
        let owner = owner.trim().to_string();
        self.apply_override(id, index, super::edit_worker::Field::Owner, |o| {
            o.mine = (!owner.is_empty()).then_some(false);
            o.owner = (!owner.is_empty()).then_some(owner);
        });
    }

    pub fn set_task_due(&mut self, id: &str, index: usize, due: DueChoice) {
        self.apply_override(id, index, super::edit_worker::Field::Due, |o| {
            o.due = match due {
                DueChoice::Date(d) => Some(d.format("%Y-%m-%d").to_string()),
                DueChoice::Clear => Some(String::new()),
                DueChoice::Reset => None,
            };
        });
    }

    pub fn reset_task(&mut self, id: &str, index: usize) {
        self.edit_task_v2(id, index, core::TaskV2Edit::Reset);
    }

    pub fn delete_manual(&mut self, row: &str) {
        match self
            .tasks
            .store
            .delete_manual(row.strip_prefix("manual/").unwrap_or(row))
        {
            Ok(o) => {
                self.tasks.overrides = o;
                let gone = Some((row.to_owned(), 0));
                if self.tasks.cursor == gone {
                    self.tasks.cursor = None;
                }
                if self.tasks.panel == Some(Panel::Task((row.to_owned(), 0))) {
                    self.tasks.panel = None;
                    self.tasks.ui = PanelUi::default();
                    self.tasks.inputs = PanelInputs::default();
                }
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(SAVE_FAILED.into()),
        }
    }

    pub fn start_rename(&mut self, name: &str) {
        if !self.config.features.projects {
            return;
        }
        self.tasks.close_menus();
        self.tasks.rename_input = input_with(name);
        self.tasks.rename = Some(Rename {
            from: name.to_string(),
        });
    }

    /// Writes an alias old -> new. Naming another existing project merges the two.
    pub fn commit_rename(&mut self) {
        if !self.config.features.projects {
            return;
        }
        let Some(r) = self.tasks.rename.take() else {
            return;
        };
        let typed = self.tasks.rename_input.text.trim().to_string();
        if typed.is_empty() || typed == r.from {
            return;
        }
        let matches = self.tasks.projects.matches(&typed);
        if matches.len() > 1 {
            self.tasks.notice =
                Some("More than one project matches. Choose a survivor with Merge into.".into());
            return;
        }
        let target = matches.first().cloned().unwrap_or(typed);
        if self.tasks.names.contains(&target) && target != r.from {
            self.start_project_merge(&r.from);
            self.preview_project_merge(&target);
            return;
        }
        match self.tasks.store.rename_project(&r.from, &target) {
            Ok(p) => {
                self.tasks.projects = p;
                if self.tasks.collapsed.remove(&r.from) {
                    self.tasks.collapsed.insert(target);
                }
                self.rebuild_tasks();
            }
            Err(_) => self.tasks.notice = Some(SAVE_FAILED.into()),
        }
    }

    pub fn focus_task_search(&mut self) {
        self.tasks.search.focused = true;
    }

    // ---- the right-hand panel ----

    /// Opens a task in the panel and selects its row.
    pub fn open_task(&mut self, id: &str, index: usize) -> bool {
        if self.keep_task_draft() {
            return false;
        }
        if !self.flush_title(false) {
            return false;
        }
        let Some(title) = self.tasks.item(id, index).map(|t| t.task.clone()) else {
            if self.areas.refresh_pending {
                self.areas.open_task_after_refresh = Some((id.into(), index));
            }
            return false;
        };
        let display = self
            .tasks
            .item(id, index)
            .map(|item| shown_title(self, item).to_string())
            .unwrap_or_else(|| title.clone());
        let t = &mut self.tasks;
        t.rename = None;
        t.close_menus();
        t.edit_item = t
            .items
            .iter()
            .find(|t| t.meeting_id == id && t.index == index)
            .cloned();
        t.subtask_input = TextEdit::new("");
        t.waiting_input = TextEdit::new(
            t.edit_item
                .as_ref()
                .and_then(|t| t.waiting_on.as_ref())
                .and_then(|w| w.text.as_deref())
                .unwrap_or_default(),
        );
        t.editor_generation += 1;
        t.edit_base = if let Some(item) = &t.edit_item
            && !item.manual
        {
            serde_json::to_value(
                t.overrides
                    .match_task(id, &item.stable_id, &item.task, index),
            )
            .unwrap_or_default()
        } else {
            core::live_edit::task_value(&t.overrides, id, index)
        };
        if !id.starts_with("manual/")
            && t.edit_base
                .get("task")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text != title)
        {
            t.edit_base = serde_json::Value::Null;
        }
        let description = t
            .edit_item
            .as_ref()
            .map(|item| item.description.clone())
            .unwrap_or_default();
        t.edit_title = display.clone();
        t.edit_description = description.clone();
        t.edit_failed = false;
        t.cursor = Some((id.to_string(), index));
        t.keyboard = true;
        t.panel = Some(Panel::Task((id.to_string(), index)));
        t.ui = PanelUi::default();
        t.inputs = PanelInputs {
            title: TextEdit::new(&display),
            description: TextEdit::new(&description),
            ..PanelInputs::default()
        };
        true
    }

    fn has_task_draft(&self) -> bool {
        self.edit_busy
            || self.tasks.edit_failed
            || (matches!(self.tasks.panel, Some(Panel::Task(_)))
                && (!self.tasks.inputs.owner.text.is_empty()
                    || !self.tasks.inputs.due.text.is_empty()
                    || !self.tasks.inputs.project.text.is_empty()))
    }
    fn keep_task_draft(&mut self) -> bool {
        let keep = self.has_task_draft();
        if keep && self.tasks.notice.is_none() {
            self.tasks.notice = Some("Your draft is kept. Save it before closing.".into());
        }
        keep
    }
    pub fn close_panel(&mut self) -> bool {
        if self.keep_task_draft() {
            return false;
        }
        if !self.flush_title(true) {
            return false;
        }
        let t = &mut self.tasks;
        t.editor_generation += 1;
        t.panel = None;
        t.edit_item = None;
        t.ui = PanelUi::default();
        t.inputs = PanelInputs::default();
        true
    }

    pub fn discard_task(&mut self) -> bool {
        if self.edit_busy {
            return false;
        }
        self.tasks.editor_generation += 1;
        self.tasks.panel = None;
        self.tasks.edit_item = None;
        self.tasks.edit_base = serde_json::Value::Null;
        self.tasks.edit_failed = false;
        self.tasks.ui = PanelUi::default();
        self.tasks.inputs = PanelInputs::default();
        self.tasks.notice = None;
        true
    }
    /// Opens the panel to add a task. One added while a project group is under the cursor
    /// goes to that project.
    pub fn open_add(&mut self) {
        if self.keep_task_draft() {
            return;
        }
        if !self.flush_title(false) {
            return;
        }
        let project = self
            .tasks
            .cursor
            .as_ref()
            .and_then(|(id, i)| self.tasks.item(id, *i))
            .map(|t| t.project.clone())
            .filter(|p| p != UNSORTED && self.tasks.grouping == TaskGrouping::Project);
        let t = &mut self.tasks;
        t.rename = None;
        t.close_menus();
        t.search.focused = false;
        t.inputs = PanelInputs {
            title: focused_input(),
            ..PanelInputs::default()
        };
        t.ui = PanelUi::default();
        t.add_project = project;
        t.panel = Some(Panel::Add);
    }

    /// The panel's open task, if it still exists.
    fn panel_task(&self) -> Option<&TaskItem> {
        match &self.tasks.panel {
            Some(Panel::Task((id, i))) => self.tasks.item(id, *i),
            _ => None,
        }
    }

    /// Save display fields without changing the original identity text.
    fn flush_title(&mut self, close: bool) -> bool {
        let Some(t) = self.panel_task() else {
            return !self.tasks.edit_failed;
        };
        let (id, index) = (t.meeting_id.clone(), t.index);
        let text = self.tasks.inputs.title.text.trim().to_string();
        let description = self.tasks.inputs.description.text.clone();
        let title_changed = text != self.tasks.edit_title;
        let description_changed = description != self.tasks.edit_description;
        if title_changed || description_changed {
            if text.is_empty() {
                self.tasks.ui.error = Some("A task needs some text.".into());
                return false;
            }
            return self.save_task_edit(
                &id,
                index,
                title_changed.then_some(text.as_str()),
                super::edit_worker::Field::Title,
                close,
                move |o| {
                    if description_changed {
                        o.description = Some(description);
                    }
                },
            );
        }
        !self.tasks.edit_failed
    }

    fn commit_title(&mut self) -> bool {
        if self.tasks.inputs.title.text.trim().is_empty() {
            self.tasks.ui.error = Some("A task needs some text.".into());
            return false;
        }
        self.tasks.ui.error = None;
        self.flush_title(false)
    }

    fn commit_due(&mut self) -> bool {
        let Some((id, index)) = self.panel_task().map(|t| (t.meeting_id.clone(), t.index)) else {
            return true;
        };
        let typed = self.tasks.inputs.due.text.trim().to_string();
        let lowered = typed.to_lowercase();
        if typed.is_empty() {
            return true;
        }
        let choice = if matches!(lowered.as_str(), "none" | "no date" | "clear") {
            DueChoice::Clear
        } else if let Some(d) = core::parse_due(&typed, self.tasks.today) {
            DueChoice::Date(d)
        } else {
            self.tasks.ui.due_error =
                Some("Try a date like 12 Nov, 2026-11-12, tomorrow or friday.".into());
            return false;
        };
        self.tasks.ui.due_error = None;
        self.set_task_due(&id, index, choice);
        if self.tasks.edit_failed || self.edit_busy {
            return false;
        }
        self.tasks.inputs.due = TextEdit::default();
        true
    }

    fn commit_owner(&mut self) -> bool {
        if let Some((id, index)) = self.panel_task().map(|t| (t.meeting_id.clone(), t.index)) {
            let typed = self.tasks.inputs.owner.text.trim().to_string();
            self.set_task_owner(&id, index, &typed);
            if self.tasks.edit_failed || self.edit_busy {
                return false;
            }
            self.tasks.ui.owner_edit = false;
            self.tasks.inputs.owner = TextEdit::default();
        }
        true
    }

    fn commit_new_project(&mut self) -> bool {
        if let Some((id, index)) = self.panel_task().map(|t| (t.meeting_id.clone(), t.index)) {
            let typed = self.tasks.inputs.project.text.trim().to_string();
            if self.cached_project(&typed).is_none() {
                self.tasks.notice =
                    Some("Choose an existing project or click Create first.".into());
                return false;
            }
            if !typed.is_empty() {
                self.set_task_project(&id, index, &typed);
            }
            if self.tasks.edit_failed || self.edit_busy {
                return false;
            }
            self.tasks.ui.new_project = false;
            self.tasks.inputs.project = TextEdit::default();
        }
        true
    }

    /// Enter in a panel text box. True when it is done with the box (nothing to correct).
    pub fn panel_enter(&mut self) -> bool {
        match self.tasks.panel {
            Some(Panel::Add) => {
                self.commit_add();
                self.tasks.panel.is_none()
            }
            Some(Panel::Task(_)) => {
                let i = &self.tasks.inputs;
                if i.owner.focused {
                    self.commit_owner()
                } else if i.due.focused {
                    self.commit_due()
                } else if i.project.focused {
                    self.commit_new_project()
                } else if i.title.focused || i.description.focused {
                    self.commit_title()
                } else {
                    true
                }
            }
            None => true,
        }
    }

    pub fn commit_add(&mut self) {
        if self.tasks.panel != Some(Panel::Add) {
            return;
        }
        let t = &self.tasks;
        let title = t.inputs.title.text.trim().to_string();
        if title.is_empty() {
            self.tasks.ui.error = Some("Write what needs doing first.".into());
            return;
        }
        let typed = t.inputs.due.text.trim().to_lowercase();
        let due = if typed.is_empty() || matches!(typed.as_str(), "none" | "no date") {
            None
        } else if let Some(d) = core::parse_due(&typed, t.today) {
            Some(d)
        } else {
            self.tasks.ui.due_error =
                Some("Try a due date like 12 Nov, tomorrow or friday.".into());
            return;
        };
        let owner = if t.ui.owner_edit {
            t.inputs.owner.text.trim().to_string()
        } else {
            String::new()
        };
        let typed_project = t.inputs.project.text.trim().to_string();
        let project_typed = if t.ui.new_project && !typed_project.is_empty() {
            Some(typed_project)
        } else {
            t.add_project.clone()
        };
        let project = match project_typed {
            Some(p) => match self.known_project(&p) {
                Some(p) => Some(p),
                None => return,
            },
            None => None,
        };
        // An empty owner is the owner of this tracker.
        let task = ManualTask {
            task: title,
            project,
            owner: (!owner.is_empty()).then(|| owner.clone()),
            due: due.map(|d| d.format("%Y-%m-%d").to_string()),
            mine: Some(owner.is_empty()),
            added: self.tasks.today.format("%Y-%m-%d").to_string(),
            ..ManualTask::default()
        };
        match self.tasks.store.add_manual(task) {
            Ok((o, id)) => {
                self.tasks.overrides = o;
                self.tasks.panel = None;
                self.tasks.ui = PanelUi::default();
                self.tasks.inputs = PanelInputs::default();
                self.tasks.cursor = Some((format!("{MANUAL}/{id}"), 0));
                self.tasks.keyboard = true;
                self.rebuild_tasks();
                self.pending_scroll = Some(PendingScroll {
                    key: format!("trow-{MANUAL}/{id}-0"),
                    center: false,
                    left: 3,
                });
            }
            Err(_) => self.tasks.ui.error = Some(SAVE_FAILED.into()),
        }
    }

    // ---- the keyboard ----

    pub fn move_task_cursor(&mut self, delta: isize, tx: &mut Tx) {
        let rows = self.tasks.visible_rows();
        if rows.is_empty() {
            return;
        }
        let at = self
            .tasks
            .cursor
            .as_ref()
            .and_then(|c| rows.iter().position(|r| r == c));
        let next = match at {
            // The first press shows the cursor where it already is.
            Some(i) if self.tasks.keyboard => {
                (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize
            }
            Some(i) => i,
            None if delta < 0 => rows.len() - 1,
            None => 0,
        };
        let (id, index) = rows[next].clone();
        // With the panel open on a task, it follows only after accepting the transition.
        if matches!(self.tasks.panel, Some(Panel::Task(_))) && !self.open_task(&id, index) {
            return;
        }
        self.tasks.keyboard = true;
        tx.scroll_into_view(format!("trow-{id}-{index}"));
        self.tasks.cursor = Some((id, index));
    }

    fn cursor_row(&self) -> Option<RowId> {
        let c = self.tasks.cursor.clone()?;
        self.tasks.visible_rows().contains(&c).then_some(c)
    }
}

/// The Tasks view's own keys. Letters only count while no text box has the keyboard.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let Key::Text(t) = &e.key else {
        return match e.key {
            Key::Down => {
                s.move_task_cursor(1, tx);
                true
            }
            Key::Up => {
                s.move_task_cursor(-1, tx);
                true
            }
            Key::Enter => match s.cursor_row() {
                Some((id, index)) if s.tasks.keyboard => {
                    s.open_task(&id, index);
                    true
                }
                _ => false,
            },
            Key::Escape => {
                if s.tasks.panel.is_some() {
                    s.close_panel();
                } else if s.tasks.group_menu {
                    s.tasks.group_menu = false;
                } else if s.tasks.cleared_menu {
                    s.tasks.cleared_menu = false;
                } else if !s.tasks.search.text.is_empty() {
                    s.tasks.search = TextEdit::new("");
                } else if s.tasks.keyboard || s.tasks.cursor.is_some() {
                    s.tasks.keyboard = false;
                    s.tasks.cursor = None;
                } else {
                    return false;
                }
                true
            }
            _ => false,
        };
    };
    match t.as_str() {
        "j" => s.move_task_cursor(1, tx),
        "k" => s.move_task_cursor(-1, tx),
        "n" => s.open_add(),
        "x" | " " => match s.cursor_row() {
            Some((id, index)) if s.tasks.keyboard => s.toggle_row(&id, index),
            _ => return false,
        },
        // E edits: the panel opens on the task, with its title ready to type if it is yours.
        "e" | "d" => match s.cursor_row() {
            Some((id, index)) if s.tasks.keyboard => {
                if !s.open_task(&id, index) {
                    return true;
                }
                if t == "d" {
                    s.tasks.inputs.due = focused_input();
                } else if let Some(task) = s.panel_task().filter(|x| x.manual) {
                    s.tasks.inputs.title = input_with(&task.task.clone());
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

// ---- small pieces ------------------------------------------------------------------------

fn search_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.tasks.search = TextEdit::new("");
        tx.blur();
        return true;
    }
    false
}

fn form_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.tasks.rename = None;
        tx.blur();
        return true;
    }
    false
}

/// Escape in a panel text box closes the panel. Enter goes through `on_submit`.
fn panel_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        if s.close_panel() {
            tx.blur();
        }
        return true;
    }
    false
}

fn segmented(
    k: &Kit,
    key: &str,
    name: &str,
    labels: &[&str],
    selected: usize,
    width: f32,
    pick: impl Fn(&mut State, usize) + Clone + 'static,
) -> El {
    col().key(key.to_string()).child(
        SegmentedControl::new(labels, Some(selected))
            .label(name)
            .width(Some(width))
            .on_pick::<State>(move |s, i, _| pick(s, i))
            .build(k),
    )
}

fn check_box(on: bool, p: Palette) -> El {
    col()
        .size(18.0, 18.0)
        .radius(5.0)
        .center()
        .border(1.5, if on { p.ok } else { p.faint })
        .fill(if on { p.okbg } else { p.card })
        .child(if on {
            label("\u{2713}", 12.0, 800.0, p.ok).font(TextStyle::new(12.0, 800.0).lh(14.0))
        } else {
            el()
        })
}

/// The round status mark of a row: empty while open, filled with a check when done.
fn status_dot(on: bool, size: f32, p: Palette) -> El {
    super::icons::draw(
        if on {
            &super::icons::DONE
        } else {
            &super::icons::CIRCLE
        },
        size.max(16.0),
        if on { p.ok } else { p.faint },
    )
}

pub(super) fn button(name: impl AsRef<str>, key: impl AsRef<str>, primary: bool, p: Palette) -> El {
    let (name, key) = (name.as_ref(), key.as_ref());
    if primary {
        pressable(roles::CONTROL.wrap(name, p.card))
            .key(key.to_string())
            .px(14.0)
            .py(6.0)
            .radius(8.0)
            .fill(p.accent)
    } else {
        pressable(roles::CONTROL.wrap(name, p.soft))
            .key(key.to_string())
            .px(10.0)
            .py(6.0)
            .radius(8.0)
            .hover(p.chip)
    }
}

/// A pill that sets a value: the due shortcuts.
pub(super) fn pill(name: &str, key: &str, on: bool, p: Palette) -> El {
    pressable(if on {
        roles::CONTROL.bold().icon_label(name, p.ink)
    } else {
        roles::CONTROL.regular().icon_label(name, p.ink)
    })
    .key(key.to_string())
    .px(11.0)
    .py(3.0)
    .radius(999.0)
    .fill(if on { p.card } else { p.chip })
    .border(1.0, if on { p.accent } else { p.line })
    .hover(p.line)
}

/// A key cap for the shortcut hints.
pub(super) fn cap(k: &str, p: Palette) -> El {
    text(k.to_string())
        .font(roles::CAPTION.style())
        .color(p.soft)
        .px(5.0)
        .radius(4.0)
        .border(1.0, p.line)
        .fill(p.card)
}

fn input(
    k: &Kit,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    reach: fn(&mut State) -> &mut TextEdit,
    p: Palette,
) -> El {
    input_h(k, edit, key, placeholder, reach, 34.0, p)
}

fn input_h(
    k: &Kit,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    reach: fn(&mut State) -> &mut TextEdit,
    height: f32,
    p: Palette,
) -> El {
    TextInput::new(edit)
        .plain()
        .placeholder(placeholder)
        .on_edit(reach)
        .on_submit(|s: &mut State, _| {
            s.panel_enter();
        })
        .on_key(panel_key)
        .height(height)
        .build(k)
        .key(key.to_string())
        .border(1.0, if edit.focused { p.accent } else { p.line })
        .radius(8.0)
        .fill(p.card)
        .min_w(0.0)
}

// ---- the top of the page -----------------------------------------------------------------

/// "Friday 6 November. 1 overdue, 2 due today. 15 open." The day first, then what is
/// pressing.
fn summary_line(s: &State, p: Palette) -> El {
    let t = &s.tasks;
    let day = t.today.format("%A %-d %B").to_string();
    let count = |b: DueBucket| {
        t.items
            .iter()
            .filter(|x| {
                !x.done && !core::is_cleared(x, t.cutoff) && core::due_bucket(x, t.today) == b
            })
            .count()
    };
    let (late, today) = (count(DueBucket::Overdue), count(DueBucket::Today));
    let mut line = row()
        .flex_wrap()
        .items_center()
        .gaps(2.0, 6.0)
        .child(roles::SMALL.bold().wrap(format!("{day}."), p.ink));
    if late > 0 {
        line = line.child(roles::SMALL.bold().wrap(format!("{late} overdue,"), p.warn));
    }
    if today > 0 {
        line = line.child(roles::SMALL.wrap(format!("{today} due today."), p.soft));
    } else if late == 0 {
        line = line.child(roles::SMALL.wrap("Nothing due today.", p.soft));
    }
    line.child(roles::SMALL.wrap(format!("{} open.", t.counts.open), p.faint))
}

fn title_row(s: &State, k: &Kit, narrow: bool, p: Palette, layer: &mut Vec<El>) -> El {
    let add = pressable(
        row()
            .gap(8.0)
            .items_center()
            .child(super::icons::draw(&super::icons::PLUS, 16.0, p.accent))
            .child(roles::CONTROL.wrap("Add task", p.accent))
            .child(cap("N", p)),
    )
    .key("task-add")
    .label("Add a task by hand (N)")
    .px(10.0)
    .py(5.0)
    .radius(8.0)
    .hover(p.chip)
    .on_press(|s: &mut State| s.open_add());
    row()
        .w_full()
        .items_end()
        .justify_between()
        .mb(4.0)
        .child(text_title("Tasks", if narrow { 26.0 } else { 30.0 }, p))
        .child(
            row()
                .gap(8.0)
                .items_center()
                .child(add.mb(2.0))
                .child(redo_older_menu(s, k, p, layer)),
        )
}

fn group_label(by: TaskGrouping) -> &'static str {
    match by {
        TaskGrouping::Due => "Due date",
        TaskGrouping::Project => "Project",
        TaskGrouping::Meeting => "Meeting",
        TaskGrouping::Team => "Team",
        TaskGrouping::Status => "Status",
        TaskGrouping::Owner => "Owner",
    }
}

/// "Group: Due date" with a menu of the three ways to group.
fn group_menu(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let t = &s.tasks;
    let trigger = pressable(
        row()
            .gap(6.0)
            .items_center()
            .child(roles::META.wrap("Group", p.faint))
            .child(roles::GROUP.wrap(group_label(t.grouping), p.ink))
            .child(label("\u{25be}", 10.0, 400.0, p.faint)),
    )
    .key("group-by")
    .label(format!("Group by: {}", group_label(t.grouping)))
    .px(10.0)
    .py(7.0)
    .radius(8.0)
    .border(1.0, p.line)
    .fill(p.card)
    .hover(p.chip)
    .on_press(|s: &mut State| s.tasks.group_menu = !s.tasks.group_menu);
    let mut menu = Menu::new("group-menu")
        .trigger(trigger)
        .open(t.group_menu)
        .placement(Placement::BottomEnd)
        .min_width(180.0);
    if t.group_menu {
        menu = menu.on_close(|s: &mut State| s.tasks.group_menu = false);
        for by in [
            TaskGrouping::Due,
            TaskGrouping::Project,
            TaskGrouping::Team,
            TaskGrouping::Status,
            TaskGrouping::Owner,
        ] {
            if by == TaskGrouping::Project && !s.config.features.projects {
                continue;
            }
            menu = menu.item(
                MenuItem::new(group_label(by))
                    .checked(t.grouping == by)
                    .on_select(move |s: &mut State| {
                        s.tasks.grouping = by;
                        s.tasks.grouping_loaded = true;
                        s.tasks.group_menu = false;
                        s.publish_edit(super::edit_worker::Job::Grouping {
                            store: s.tasks.store.clone(),
                            by,
                        });
                    }),
            );
        }
    }
    menu.build(k, layer)
}

fn toolbar(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, available: f32) -> El {
    let t = &s.tasks;
    let search = TextInput::new(&t.search)
        .plain()
        .placeholder("Search tasks")
        .on_edit(|s: &mut State| &mut s.tasks.search)
        .on_key(search_key)
        .height(36.0)
        .build(k)
        .key("task-search")
        .border(1.0, if t.search.focused { p.accent } else { p.line })
        .radius(8.0)
        .fill(p.card)
        .w(200.0)
        .grow(1.0)
        .min_w(200.0);
    let who = segmented(
        k,
        "seg-who",
        "For",
        &["Everyone", "Mine"],
        usize::from(t.who == Who::Mine),
        168.0,
        |s, i| {
            s.tasks.who = [Who::Everyone, Who::Mine][i];
            s.tasks.close_menus();
        },
    );
    let done = segmented(
        k,
        "seg-done",
        "Show",
        &["Open", "Done", "All"],
        match t.done {
            DoneFilter::Open => 0,
            DoneFilter::Done => 1,
            DoneFilter::All => 2,
        },
        174.0,
        |s, i| {
            s.tasks.done = [DoneFilter::Open, DoneFilter::Done, DoneFilter::All][i];
            s.tasks.close_menus();
        },
    );
    let controls = row()
        .key("task-filters")
        .flex_wrap()
        .items_center()
        .gaps(8.0, 8.0)
        .child(who)
        .child(done)
        .child(group_menu(s, k, p, layer));
    let fields = if available >= 730.0 {
        row()
            .key("task-toolbar")
            .w_full()
            .items_center()
            .gap(8.0)
            .child(search)
            .child(controls)
    } else {
        col()
            .key("task-toolbar")
            .w_full()
            .gap(8.0)
            .child(search.w_full())
            .child(controls.w_full())
    };
    let mut bar = col().w_full().gap(8.0).child(fields);
    if t.who == Who::Mine {
        bar = bar.child(
            pressable(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(check_box(t.include_unassigned, p))
                    .child(roles::SMALL.wrap("Include unassigned", p.ink)),
            )
            .key("include-unassigned")
            .role("checkbox")
            .checked(t.include_unassigned)
            .label("Include unassigned tasks")
            .py(3.0)
            .on_press(|s: &mut State| s.tasks.include_unassigned = !s.tasks.include_unassigned),
        );
    }
    bar
}

// ---- a task row ------------------------------------------------------------------------

/// A small coloured dot that gives each project the same mark on every row.
fn project_dot(name: &str, p: Palette) -> El {
    let ink = if name == UNSORTED {
        p.faint
    } else {
        let h = name
            .bytes()
            .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
        [p.accent, p.ok, p.open, p.soft][(h % 4) as usize]
    };
    el().size(7.0, 7.0).radius(4.0).fill(ink)
}

/// The owner as a small initial in a circle: "Me" is drawn in the accent. None when nobody
/// owns the task.
fn owner_name(t: &TaskItem) -> Option<(String, bool)> {
    match (t.mine, t.owner.as_deref()) {
        (Some(true), _) => Some(("Me".to_string(), true)),
        (_, Some(o)) => Some((o.to_string(), false)),
        _ => None,
    }
}

fn avatar(name: &str, me: bool, size: f32, p: Palette) -> El {
    let initial: String = if me {
        "Me".into()
    } else {
        name.chars().take(1).flat_map(char::to_uppercase).collect()
    };
    let (fill, ink) = if me {
        (p.accent, p.card)
    } else {
        (p.card, p.soft)
    };
    col()
        .w(if me { size + 12.0 } else { size })
        .h(size)
        .radius(size / 2.0)
        .center()
        .fill(fill)
        .border(1.0, if me { p.accent } else { p.faint })
        .label(name)
        .help(name)
        .child(
            text(initial)
                .font(TextStyle::new(12.0, 600.0).lh(size))
                .color(ink),
        )
}

/// The due date as a short pill: late in amber, today in the accent. A date the summary
/// guessed is drawn softer than one somebody chose.
fn due_chip(t: &TaskItem, today: NaiveDate, p: Palette) -> Option<El> {
    let (phrase, urgency) = core::due_phrase(t, today)?;
    let plain = phrase.strip_prefix("Due ").unwrap_or(&phrase);
    let mut shown = plain.to_string();
    if let Some(first) = shown.get(..1) {
        shown = format!("{}{}", first.to_uppercase(), &shown[1..]);
    }
    let guessed = t.due_default;
    let (ink, weight, fill) = match urgency {
        Urgency::Late => (p.warn, 600.0, Some(p.warnbg)),
        Urgency::Today if guessed => (p.accent, 500.0, None),
        Urgency::Today => (p.accent, 600.0, Some(p.chip)),
        Urgency::Soon if guessed => (p.faint, 500.0, None),
        Urgency::Soon => (p.soft, 500.0, None),
        Urgency::Calm => (p.faint, 400.0, None),
    };
    let chip = text(shown)
        .font(
            roles::Role {
                weight,
                ..roles::META
            }
            .style(),
        )
        .color(ink)
        .px(8.0)
        .py(1.0)
        .radius(999.0);
    Some(match fill {
        Some(f) => chip.fill(f),
        None => chip,
    })
}

/// How much of the row the properties take, which depends on the room the list has.
#[derive(Clone, Copy)]
struct RowLayout {
    /// The project column's width; zero hides it.
    project_w: f32,
}

fn task_row(s: &State, t: &TaskItem, lay: RowLayout, p: Palette) -> El {
    let id = t.meeting_id.clone();
    let index = t.index;
    let me = Some((id.clone(), index));
    let tasks = &s.tasks;
    let on = t.done;
    let selected = matches!(&tasks.panel, Some(Panel::Task(r)) if Some(r) == me.as_ref());
    let at_cursor = tasks.keyboard && tasks.cursor == me && !selected;
    let hovered = tasks.hovered == me;
    let tick = {
        let id = id.clone();
        pressable(
            col()
                .size(26.0, 26.0)
                .center()
                .child(status_dot(on, 16.0, p)),
        )
        .role("checkbox")
        .checked(on)
        .label(t.task.clone())
        .key(format!("ttick-{id}-{index}"))
        .radius(13.0)
        .on_press(move |s: &mut State| s.toggle_row(&id, index))
    };
    let title = if on {
        roles::SMALL.wrap(shown_title(s, t), p.faint).max_lines(2)
    } else {
        roles::SMALL
            .bold()
            .wrap(shown_title(s, t), p.ink)
            .max_lines(2)
    };
    let title = if on {
        title.strike(0, shown_title(s, t).len(), None)
    } else {
        title
    };
    let open = {
        let id = id.clone();
        move |s: &mut State| {
            s.open_task(&id, index);
        }
    };
    let open_title =
        pressable(
            col()
                .min_w(0.0)
                .gap(3.0)
                .child(title)
                .children(
                    (!t.description.is_empty())
                        .then(|| roles::META.wrap(&t.description, p.soft).max_lines(2)),
                )
                .children(
                    (t.status != core::TaskStatus::Todo && !t.done && t.waiting_label.is_none())
                        .then(|| {
                            row()
                                .gap(4.0)
                                .items_center()
                                .children((t.status == core::TaskStatus::Waiting).then(|| {
                                    super::icons::draw(&super::icons::WAITING, 16.0, p.soft)
                                }))
                                .child(roles::META.wrap(t.status_label(), p.soft))
                                .px(6.0)
                                .radius(4.0)
                                .fill(p.chip)
                                .self_start()
                        }),
                )
                .children(
                    t.waiting_label
                        .as_ref()
                        .map(|label| roles::META.wrap(format!("Waiting on: {label}"), p.soft)),
                )
                .children((!t.subtasks.is_empty()).then(|| roles::META.wrap(t.progress(), p.soft))),
        )
        .key(format!("ttask-{id}-{index}"))
        .label(format!("Open the task: {}", t.task))
        .grow(1.0)
        .min_w(0.0)
        .on_press(open.clone());
    let mut props = row().h_full().items_center().gap(10.0);
    if lay.project_w > 0.0 {
        props = props.child(
            row()
                .w(lay.project_w)
                .items_center()
                .gap(6.0)
                .help(&t.project)
                .label(format!("Project: {}", t.project))
                .child(project_dot(&t.project, p))
                .children((lay.project_w > 30.0).then(|| roles::META.line(&t.project, p.faint))),
        );
    }
    props = props
        .child(
            row().w(88.0).justify_end().items_center().child(
                roles::META.line(
                    owner_name(t)
                        .map(|(name, _)| name)
                        .unwrap_or_else(|| "No owner".into()),
                    p.soft,
                ),
            ),
        )
        .child(
            row()
                .w(88.0)
                .justify_end()
                .items_center()
                .child(due_chip(t, tasks.today, p).unwrap_or_else(el)),
        );
    let props = pressable(props)
        .key(format!("tmeta-{id}-{index}"))
        .focusable(false)
        .on_press(open);
    let mut line = row()
        .key(format!("trow-{id}-{index}"))
        .w_full()
        .min_h(42.0)
        .py(7.0)
        .px(8.0)
        .gap(8.0)
        .items_center()
        .radius(6.0)
        .child(tick)
        .child(open_title)
        .child(props)
        .on_hover_change({
            let id = id.clone();
            move |s: &mut State, inside, _| {
                let me = Some((id.clone(), index));
                if inside {
                    s.tasks.hovered = me;
                } else if s.tasks.hovered == me {
                    s.tasks.hovered = None;
                }
            }
        });
    if selected {
        line = line
            .fill(p.sel)
            .child(el().w(2.0).h(18.0).radius(1.0).fill(p.accent).pin(
                Some(0.0),
                Some(10.0),
                None,
                None,
            ));
    } else if at_cursor {
        line = line.fill(p.chip).outline(1.0, 0.0, p.line);
    } else if hovered {
        line = line.fill(p.chip);
    }
    let mut out = col().w_full().child(line);
    for sub in &t.subtasks {
        let (id, sub_id, done) = (t.meeting_id.clone(), sub.id.clone(), sub.done);
        out = out.child(
            pressable(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(status_dot(done, 14.0, p))
                    .child(
                        roles::SMALL
                            .wrap(&sub.title, if done { p.faint } else { p.soft })
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            )
            .key(format!("subtick-{}-{}-{}", t.meeting_id, index, sub.id))
            .label(format!("Sub-task: {}", sub.title))
            .role("checkbox")
            .checked(done)
            .ml(42.0)
            .mr(12.0)
            .py(5.0)
            .radius(5.0)
            .hover(p.chip)
            .on_press(move |s: &mut State| {
                s.edit_task_v2(
                    &id,
                    index,
                    core::TaskV2Edit::SubtaskTick {
                        key: core::subtask_tick_key(
                            &s.tasks
                                .item(&id, index)
                                .map(|t| t.stable_id.clone())
                                .unwrap_or_default(),
                            &sub_id,
                        ),
                        done: !done,
                    },
                )
            }),
        );
    }
    out
}

// ---- the groups --------------------------------------------------------------------------

pub(super) fn chevron(open: bool, p: Palette) -> El {
    label(
        if open { "\u{25bc}" } else { "\u{25b6}" },
        9.0,
        400.0,
        p.faint,
    )
    .w(14.0)
}

fn rename_form(k: &Kit, s: &State, p: Palette) -> El {
    let input = TextInput::new(&s.tasks.rename_input)
        .plain()
        .placeholder("Project name")
        .on_edit(|s: &mut State| &mut s.tasks.rename_input)
        .on_submit(|s: &mut State, _| s.commit_rename())
        .on_key(form_key)
        .height(34.0)
        .build(k)
        .key("rename-input")
        .border(1.0, p.accent)
        .radius(8.0)
        .fill(p.card)
        .grow(1.0)
        .min_w(0.0);
    row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(input)
        .child(button("Save", "rename-save", true, p).on_press(|s: &mut State| s.commit_rename()))
        .child(
            button("Cancel", "rename-cancel", false, p)
                .on_press(|s: &mut State| s.tasks.rename = None),
        )
}

/// A group's heading bar: fold arrow, name, count, and the group's own action.
fn group_header(s: &State, k: &Kit, g: &TaskGroup, n: usize, p: Palette) -> El {
    if let GroupKind::Project(name) = &g.kind
        && s.tasks.rename.as_ref().is_some_and(|r| r.from == *name)
    {
        return col().w_full().pt(10.0).pb(6.0).child(rename_form(k, s, p));
    }
    let open = !s.tasks.collapsed.contains(&g.key);
    let ink = match g.kind {
        GroupKind::Due(DueBucket::Overdue) => p.warn,
        _ => p.ink,
    };
    let key = g.key.clone();
    let mut fold = row()
        .gap(8.0)
        .items_center()
        .min_w(0.0)
        .child(chevron(open, p))
        .child(roles::GROUP.line(&g.label, ink))
        .child(roles::META.medium().wrap(g.rows.len().to_string(), p.faint));
    if let Some(d) = &g.detail {
        fold = fold.child(roles::META.wrap(d, p.faint).ml(4.0));
    }
    let toggle = pressable(fold)
        .key(format!("tgroup-fold-{n}"))
        .label(format!(
            "{} {}, {} open",
            if open { "Fold" } else { "Unfold" },
            g.label,
            g.open
        ))
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(8.0)
        .radius(6.0)
        .on_press(move |s: &mut State| {
            if !s.tasks.collapsed.remove(&key) {
                s.tasks.collapsed.insert(key.clone());
            }
        });
    let mut head = row().w_full().h(32.0).gap(6.0).items_center().child(toggle);
    match &g.kind {
        GroupKind::Project(name) if name != UNSORTED => {
            let merge_name = name.clone();
            head = head.child(
                pressable(roles::META.medium().wrap("Merge into...", p.faint))
                    .key(format!("tmerge-{n}"))
                    .px(6.0)
                    .py(2.0)
                    .radius(6.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.start_project_merge(&merge_name)),
            );
            let name = name.clone();
            head = head.child(
                pressable(roles::META.medium().wrap("Rename", p.faint))
                    .key(format!("trename-{n}"))
                    .label(format!("Rename the project {name}"))
                    .px(6.0)
                    .py(2.0)
                    .mr(6.0)
                    .radius(6.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.start_rename(&name)),
            );
        }
        GroupKind::Meeting(id) if id != MANUAL => {
            let id = id.clone();
            head = head.child(
                pressable(roles::META.medium().wrap("Open", p.accent))
                    .key(format!("tgroup-open-{n}"))
                    .label(format!("Open the meeting {}", g.label))
                    .px(6.0)
                    .py(2.0)
                    .mr(6.0)
                    .radius(6.0)
                    .hover(p.chip)
                    .on_press_tx(move |s: &mut State, tx| s.open_meeting(&id, tx)),
            );
        }
        _ => {}
    }
    head
}

fn group_card(s: &State, k: &Kit, g: &TaskGroup, n: usize, lay: RowLayout, p: Palette) -> El {
    let mut card = col()
        .key(format!("tgroup-{n}"))
        .w_full()
        .mt(roles::GROUP_TOP)
        .gap(1.0)
        .child(group_header(s, k, g, n, p));
    if s.tasks.collapsed.contains(&g.key) {
        return card;
    }
    for &row_index in &g.rows {
        card = card.child(task_row(s, &s.tasks.items[row_index], lay, p));
    }
    card
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn done_section(s: &State, v: &TaskView, lay: RowLayout, p: Palette) -> El {
    let open = s.tasks.done_open;
    let n = v.done.len();
    let toggle = pressable(
        row()
            .gap(8.0)
            .items_center()
            .child(chevron(open, p))
            .child(roles::GROUP.wrap("Done", p.soft))
            .child(roles::META.medium().wrap(n.to_string(), p.faint)),
    )
    .key("tasks-done-toggle")
    .label(format!(
        "{} the {} done",
        if open { "Hide" } else { "Show" },
        plural(n, "task", "tasks")
    ))
    .w_full()
    .h(32.0)
    .px(8.0)
    .radius(6.0)
    .fill(p.side)
    .on_press(|s: &mut State| s.tasks.done_open = !s.tasks.done_open);
    let mut out = col()
        .key("tasks-done")
        .w_full()
        .mt(18.0)
        .gap(1.0)
        .child(toggle);
    if open {
        for &i in &v.done {
            out = out.child(task_row(s, &s.tasks.items[i], lay, p));
        }
    }
    out
}

/// "5 Nov", with the year when it is not this one.
fn cutoff_text(day: NaiveDate, today: NaiveDate) -> String {
    if chrono::Datelike::year(&day) == chrono::Datelike::year(&today) {
        day.format("%-d %b").to_string()
    } else {
        day.format("%-d %b %Y").to_string()
    }
}

/// The quiet line at the end of the list: what is cleared, a way to look at it, and a way to
/// change the cutoff.
fn cleared_line(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let t = &s.tasks;
    let link = |text: &str, key: &str, help: String| {
        pressable(roles::CAPTION.wrap(text, p.accent))
            .key(key.to_string())
            .label(help)
            .px(6.0)
            .py(3.0)
            .radius(6.0)
            .hover(p.chip)
    };
    let mut line = row()
        .key("tasks-cleared")
        .flex_wrap()
        .items_center()
        .gaps(2.0, 4.0)
        .mt(if t.panel.is_some() { 16.0 } else { 20.0 });
    let change_text = if let Some(day) = t.cutoff {
        let day = cutoff_text(day, t.today);
        let (said, action) = if t.show_cleared {
            (
                format!("Showing tasks from meetings before {day}."),
                "Hide them",
            )
        } else {
            (
                format!("Tasks from meetings before {day} are cleared."),
                "Show them",
            )
        };
        line = line.child(roles::META.wrap(said, p.faint)).child(
            link(
                action,
                "cleared-toggle",
                format!("{action}: tasks from meetings before {day}"),
            )
            .on_press(|s: &mut State| {
                s.tasks.show_cleared = !s.tasks.show_cleared;
                s.tasks.cursor = None;
                s.close_panel();
                s.rebuild_tasks();
            }),
        );
        "Change"
    } else {
        "Hide unedited tasks from meetings older than..."
    };
    let trigger = link(
        change_text,
        "cleared-change",
        "Choose the meeting age for hiding unedited tasks".into(),
    )
    .on_press(|s: &mut State| s.tasks.cleared_menu = !s.tasks.cleared_menu);
    let mut menu = Menu::new("cleared-menu")
        .trigger(trigger)
        .open(t.cleared_menu)
        .placement(Placement::TopStart)
        .min_width(180.0);
    if t.cleared_menu {
        menu = menu.on_close(|s: &mut State| s.tasks.cleared_menu = false);
        for (name, days) in [
            ("Older than 7 days", Some(7)),
            ("Older than 14 days", Some(14)),
            ("Older than 30 days", Some(30)),
            ("Keep all", None),
        ] {
            menu = menu.item(
                MenuItem::new(name)
                    .checked(days.is_none() && t.cutoff.is_none())
                    .on_select(move |s: &mut State| {
                        s.tasks.cleared_menu = false;
                        if let Some(days) = days {
                            s.tasks.clear_confirm = Some(days);
                        } else {
                            s.set_cutoff_days(None);
                        }
                    }),
            );
        }
    }
    let mut out = col().w_full().child(line.child(menu.build(k, layer)));
    if let Some(days) = t.clear_confirm {
        let cutoff = t.today - Duration::days(days);
        let count = t
            .all
            .iter()
            .filter(|task| core::is_cleared(task, Some(cutoff)))
            .count();
        out = out.child(super::widgets::banner("Hide old tasks?", format!("Hide {count} unedited tasks from meetings older than {days} days? This includes open and finished tasks. Tasks added by hand or edited stay visible. You can show hidden tasks again here."), super::widgets::BannerKind::Info, p)
            .child(row().gap(8.0).flex_wrap()
                .child(button("Keep tasks", "clear-cancel", true, p).on_press(|s: &mut State| s.tasks.clear_confirm = None))
                .child(button(format!("Hide {count} tasks"), "clear-confirm", false, p).on_press(move |s: &mut State| { s.tasks.clear_confirm = None; s.set_cutoff_days(Some(days)); }))));
    }
    out
}

fn empty_note(s: &State, p: Palette) -> El {
    let (head, body) = if s.tasks.items.is_empty() {
        (
            "No tasks yet",
            "Tasks from your meeting summaries appear here. Press N to add one by hand.",
        )
    } else {
        (
            "Nothing matches",
            "Try All instead of Open, Everyone instead of Mine, or a different search.",
        )
    };
    col()
        .w_full()
        .gap(6.0)
        .pt(40.0)
        .items_start()
        .child(roles::SECTION.wrap(head, p.ink))
        .child(roles::SMALL.wrap(body, p.soft))
}

fn key_hints(p: Palette) -> El {
    let hint = |keys: &[&str], what: &str| {
        let mut r = row().gap(4.0).items_center();
        for k in keys {
            r = r.child(cap(k, p));
        }
        r.child(roles::META.wrap(what, p.faint).ml(2.0))
    };
    row()
        .key("tasks-keys")
        .flex_wrap()
        .gaps(6.0, 16.0)
        .mt(24.0)
        .child(hint(&["J", "K"], "move"))
        .child(hint(&["Enter"], "open"))
        .child(hint(&["X"], "done"))
        .child(hint(&["N"], "add"))
        .child(hint(&["/"], "search"))
        .child(hint(&["Esc"], "close"))
}

// ---- the panel ---------------------------------------------------------------------------

/// A property of the task: a fixed label, then its control.
pub(super) fn prop_row(name: &str, control: El, p: Palette) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .min_h(36.0)
        .child(roles::META.medium().wrap(name, p.faint).w(68.0).mt(8.0))
        .child(control.grow(1.0).min_w(0.0))
}

/// The look of a property value that opens a menu or toggles.
pub(super) fn value_button(content: El, key: &str, help: String, p: Palette) -> El {
    pressable(content)
        .key(key.to_string())
        .label(help)
        .self_start()
        .px(10.0)
        .py(5.0)
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.card)
        .hover(p.chip)
}

fn caret(p: Palette) -> El {
    label("\u{25be}", 10.0, 400.0, p.faint)
}

fn due_control(s: &State, k: &Kit, task: Option<&TaskItem>, p: Palette) -> El {
    let t = &s.tasks;
    let today = t.today;
    let target = task.map(|x| (x.meeting_id.clone(), x.index));
    let (pre, input_key) = if task.is_some() {
        ("pdue", "pdue")
    } else {
        ("add-due", "add-due")
    };
    let typed = core::parse_due(&t.inputs.due.text, today);
    let chip = |name: &str, key: &str, date: Option<NaiveDate>| {
        let on = match (&task, date) {
            (Some(x), Some(d)) => x.due_date == Some(d),
            (Some(x), None) => x.due.is_none(),
            (None, Some(d)) => typed == Some(d),
            (None, None) => t.inputs.due.text.trim().is_empty(),
        };
        let target = target.clone();
        pill(name, &format!("{pre}-{key}"), on, p).on_press(move |s: &mut State| match &target {
            Some((id, index)) => {
                s.tasks.ui.due_error = None;
                let choice = date.map_or(DueChoice::Clear, DueChoice::Date);
                s.set_task_due(id, *index, choice);
                if !s.tasks.edit_failed && !s.edit_busy {
                    s.tasks.inputs.due = TextEdit::default();
                }
            }
            None => {
                s.tasks.ui.due_error = None;
                s.tasks.inputs.due =
                    date.map_or_else(TextEdit::default, |d| TextEdit::new(due_text(d)));
            }
        })
    };
    let mut c = col().w_full().gap(8.0).child(
        row()
            .flex_wrap()
            .gaps(6.0, 6.0)
            .child(chip("Today", "today", Some(today)))
            .child(chip("Tomorrow", "tomorrow", today.succ_opt()))
            .child(chip("Next week", "week", Some(core::next_week(today)))),
    );
    c = c.child(
        row()
            .w_full()
            .gap(6.0)
            .items_center()
            .child(
                input(
                    k,
                    &t.inputs.due,
                    input_key,
                    "Or type: friday, 12 Nov",
                    |s| &mut s.tasks.inputs.due,
                    p,
                )
                .grow(1.0),
            )
            .child(chip("No date", "none", None)),
    );
    if let Some(x) = task {
        let (id, index) = (x.meeting_id.clone(), x.index);
        if let Some((phrase, urgency)) = core::due_phrase(x, today) {
            let (ink, weight) = match urgency {
                Urgency::Late => (p.warn, 600.0),
                _ => (p.soft, 400.0),
            };
            let note = if x.due_default {
                format!(
                    "{} (guessed)",
                    x.due_date
                        .map(|date| date.format("%a %-d %b").to_string())
                        .unwrap_or(phrase)
                )
            } else {
                format!("{phrase}.")
            };
            c = c.child(
                roles::Role {
                    weight,
                    ..roles::META
                }
                .wrap(note, ink),
            );
        }
        if !x.manual
            && t.overrides
                .match_task(&id, &x.stable_id, &x.task, index)
                .is_some_and(|o| o.due.is_some())
        {
            c = c.child(
                pressable(roles::CAPTION.wrap("Use the meeting's date", p.accent))
                    .key("pdue-reset")
                    .self_start()
                    .px(4.0)
                    .ml(-4.0)
                    .radius(5.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.set_task_due(&id, index, DueChoice::Reset)),
            );
        }
    }
    if let Some(msg) = &t.ui.due_error {
        c = c.child(roles::SMALL.medium().wrap(msg, p.warn).role("alert"));
    }
    if task.is_some_and(|x| x.due_default) {
        c = c
            .child(roles::META.wrap(
                "The meeting did not set a date. Sidenote guessed one.",
                p.soft,
            ))
            .child(
                button("Set a date", "task-guessed-date", false, p).on_press(|s: &mut State| {
                    s.tasks.inputs.due.focused = true;
                }),
            );
    }
    c
}

fn owner_control(
    s: &State,
    k: &Kit,
    task: Option<&TaskItem>,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let t = &s.tasks;
    let (pre, input_key) = if task.is_some() {
        ("powner", "powner-input")
    } else {
        ("add-owner", "add-owner")
    };
    if t.ui.owner_edit {
        return col()
            .w_full()
            .gap(4.0)
            .child(
                input(
                    k,
                    &t.inputs.owner,
                    input_key,
                    "Who owns this?",
                    |s| &mut s.tasks.inputs.owner,
                    p,
                )
                .w_full(),
            )
            .child(roles::META.wrap(
                if task.is_some() {
                    "Enter saves. Leave it empty to use the owner from the meeting."
                } else {
                    "Leave it empty to keep the task yours."
                },
                p.faint,
            ));
    }
    let (name, mine) = match task {
        Some(x) => owner_name(x).unwrap_or_else(|| ("No owner".to_string(), false)),
        None => ("Me".to_string(), true),
    };
    let has_owner = task.is_none_or(|x| owner_name(x).is_some());
    let face = if has_owner {
        avatar(&name, mine, 18.0, p)
    } else {
        col().size(18.0, 18.0).radius(9.0).border(1.3, p.faint)
    };
    let trigger = value_button(
        row()
            .gap(8.0)
            .items_center()
            .child(face)
            .child(roles::CONTROL.wrap(&name, if has_owner { p.ink } else { p.soft }))
            .child(caret(p)),
        pre,
        format!("Owner: {name}"),
        p,
    )
    .on_press(|s: &mut State| {
        s.tasks.ui.project_menu = false;
        s.tasks.ui.owner_menu = !s.tasks.ui.owner_menu;
    });
    let mut menu = Menu::new(format!("{pre}-menu"))
        .trigger(trigger)
        .open(t.ui.owner_menu)
        .placement(Placement::BottomStart)
        .min_width(210.0);
    if t.ui.owner_menu {
        menu = menu.on_close(|s: &mut State| s.tasks.ui.owner_menu = false);
        let target = task.map(|x| (x.meeting_id.clone(), x.index));
        let t1 = target.clone();
        menu = menu.item(
            MenuItem::new("Me")
                .checked(task.is_none_or(|x| x.mine == Some(true)))
                .on_select(move |s: &mut State| {
                    s.tasks.ui.owner_menu = false;
                    if let Some((id, index)) = &t1 {
                        s.set_task_mine(id, *index, true);
                    }
                    if !s.tasks.edit_failed && !s.edit_busy {
                        s.tasks.ui.owner_edit = false;
                        s.tasks.inputs.owner = TextEdit::default();
                    }
                }),
        );
        if let Some((id, index)) = target {
            menu = menu.item(
                MenuItem::new("Not me")
                    .checked(task.is_some_and(|x| x.mine == Some(false)))
                    .on_select(move |s: &mut State| {
                        s.tasks.ui.owner_menu = false;
                        s.set_task_mine(&id, index, false);
                    }),
            );
        }
        menu = menu
            .separator()
            .item(
                MenuItem::new("Someone else...").on_select(move |s: &mut State| {
                    s.tasks.ui.owner_menu = false;
                    s.tasks.ui.owner_edit = true;
                    s.tasks.inputs.owner = focused_input();
                }),
            );
    }
    menu.build(k, layer)
}

fn project_control(
    s: &State,
    k: &Kit,
    task: Option<&TaskItem>,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    let t = &s.tasks;
    let (pre, input_key) = if task.is_some() {
        ("pproject", "pproject-input")
    } else {
        ("add-project", "add-project-new")
    };
    if t.ui.new_project {
        let typed = t.inputs.project.text.trim().to_string();
        let matches = t.projects.matches(&typed);
        let mut choices = col()
            .w_full()
            .gap(4.0)
            .child(roles::GROUP.wrap("Use an existing project", p.soft));
        for name in if matches.is_empty() {
            &t.names
        } else {
            &matches
        } {
            let target = task.map(|x| (x.meeting_id.clone(), x.index));
            let name = name.clone();
            choices = choices.child(
                pressable(roles::CONTROL.wrap(&name, p.accent))
                    .py(4.0)
                    .on_press(move |s: &mut State| {
                        if let Some((id, i)) = &target {
                            s.set_task_project(id, *i, &name);
                        } else {
                            s.tasks.add_project = Some(name.clone());
                        }
                        if !s.tasks.edit_failed && !s.edit_busy {
                            s.tasks.ui.new_project = false;
                            s.tasks.inputs.project = TextEdit::default();
                        }
                    }),
            );
        }
        if !typed.is_empty() && matches.is_empty() {
            let target = task.map(|x| (x.meeting_id.clone(), x.index));
            choices = choices.child(
                pressable(
                    roles::CONTROL
                        .bold()
                        .wrap(format!("Create {typed}"), p.accent),
                )
                .key("task-project-create")
                .py(6.0)
                .on_press(move |s: &mut State| {
                    match s.tasks.store.add_project(&typed) {
                        Ok(p) => {
                            s.tasks.projects = p;
                            if let Some((id, i)) = &target {
                                s.set_task_project(id, *i, &typed);
                            } else {
                                s.tasks.add_project = Some(s.tasks.projects.resolve(&typed));
                            }
                            if !s.tasks.edit_failed && !s.edit_busy {
                                s.tasks.ui.new_project = false;
                                s.tasks.inputs.project = TextEdit::default();
                            }
                        }
                        Err(e) => {
                            s.tasks.notice =
                                Some(super::edit_worker::SaveError::from(&e).message().into())
                        }
                    }
                }),
            );
        }
        return col()
            .w_full()
            .gap(4.0)
            .child(
                input(
                    k,
                    &t.inputs.project,
                    input_key,
                    "New project name",
                    |s| &mut s.tasks.inputs.project,
                    p,
                )
                .w_full(),
            )
            .child(choices);
    }
    let current = match task {
        Some(x) => x.project.clone(),
        None => t
            .add_project
            .clone()
            .unwrap_or_else(|| UNSORTED.to_string()),
    };
    let current_label = task
        .and_then(|task| task.area_id.as_ref())
        .and_then(|id| s.areas.catalog.areas.get(id))
        .filter(|_| current == UNSORTED)
        .map(|area| area.name.clone())
        .unwrap_or_else(|| {
            s.areas
                .project_labels
                .get(&current)
                .cloned()
                .unwrap_or_else(|| current.clone())
        });
    let trigger = value_button(
        row()
            .gap(8.0)
            .items_center()
            .child(project_dot(&current, p))
            .child(roles::CONTROL.wrap(&current_label, p.ink))
            .child(caret(p)),
        pre,
        format!("Project: {current}"),
        p,
    )
    .on_press(|s: &mut State| {
        s.tasks.ui.owner_menu = false;
        s.tasks.ui.project_menu = !s.tasks.ui.project_menu;
    });
    let mut menu = Menu::new(format!("{pre}-menu"))
        .trigger(trigger)
        .open(t.ui.project_menu)
        .placement(Placement::BottomStart)
        .min_width(250.0);
    if t.ui.project_menu {
        menu = menu.on_close(|s: &mut State| s.tasks.ui.project_menu = false);
        let target = task.map(|x| (x.meeting_id.clone(), x.index));
        let pick = move |name: String| {
            let target = target.clone();
            move |s: &mut State| {
                s.tasks.ui.project_menu = false;
                match &target {
                    Some((id, index)) => s.set_task_project(id, *index, &name),
                    None => s.tasks.add_project = (name != UNSORTED).then(|| name.clone()),
                }
            }
        };
        for name in t.names.iter().chain(std::iter::once(&UNSORTED.to_string())) {
            menu = menu.item(
                MenuItem::new(name.clone())
                    .checked(*name == current)
                    .on_select(pick(name.clone())),
            );
        }
        if let Some(task) = task {
            menu = menu.separator().group_label("Area-only work");
            for area in s.areas.catalog.areas.values() {
                let (id, key) = (area.id.clone(), core::areas::task_key(task));
                menu = menu.item(MenuItem::new(&area.name).on_select(move |s: &mut State| {
                    s.tasks.ui.project_menu = false;
                    s.assign_area(key.clone(), id.clone());
                }));
            }
        }
        menu = menu
            .separator()
            .item(MenuItem::new("New project...").on_select(|s: &mut State| {
                s.tasks.ui.project_menu = false;
                s.tasks.ui.new_project = true;
                s.tasks.inputs.project = focused_input();
            }));
    }
    menu.build(k, layer)
}

/// The panel's top strip: what it is, and the way out.
fn panel_top(title: &str, overlay: bool, p: Palette) -> El {
    let close = pressable(label("\u{d7}", 22.0, 400.0, p.soft))
        .key("panel-close")
        .label("Close the panel (Escape)")
        .px(10.0)
        .radius(8.0)
        .hover(p.chip)
        .on_press_tx(|s: &mut State, tx| {
            if s.close_panel() {
                tx.blur();
            }
        });
    let mut top = row().w_full().h(48.0).px(12.0).gap(8.0).items_center();
    if overlay {
        top = top.child(
            pressable(roles::CONTROL.icon_label("\u{2039}  Tasks", p.soft))
                .key("panel-back")
                .label("Back to the task list (Escape)")
                .px(10.0)
                .py(4.0)
                .radius(8.0)
                .hover(p.chip)
                .on_press_tx(|s: &mut State, tx| {
                    if s.close_panel() {
                        tx.blur();
                    }
                }),
        );
    }
    // The back button already says where this sits, so the caption is for the wide panel.
    let caption = if overlay && title == "Task" {
        ""
    } else {
        title
    };
    let top = top.child(
        spaced_caps(caption, p)
            .ml(if overlay { 0.0 } else { 8.0 })
            .grow(1.0),
    );
    if overlay { top } else { top.child(close) }
}

/// Where the task came from: the meeting, the transcript lines at its timestamp and the
/// decision behind it. A task added by hand says so.
fn source_section(s: &State, t: &TaskItem, p: Palette) -> El {
    let mut panel = col().w_full().gap(8.0);
    if t.manual {
        let when = t
            .meeting_date
            .map(|d| format!(" on {}", d.format("%-d %b")))
            .unwrap_or_default();
        return panel
            .child(spaced_caps("Source", p))
            .child(roles::META.wrap(
                format!("Added by hand{when}. It did not come from a meeting."),
                p.soft,
            ));
    }
    let Some(m) = s.meetings.iter().find(|m| m.id == t.meeting_id) else {
        return panel
            .child(roles::META.wrap("The meeting is no longer on this computer.", p.faint));
    };
    let source = core::task_source(m, t.index);
    let date = m
        .started
        .map(|d| format!(", {}", d.format("%-d %b %Y")))
        .unwrap_or_default();
    let id = m.id.clone();
    panel = panel.child(spaced_caps("From the meeting", p)).child(
        pressable(roles::CONTROL.wrap(format!("{}{date}", m.title), p.accent))
            .key(format!("tsrc-meet-{}-{}", t.meeting_id, t.index))
            .label(format!("Open the meeting {}", m.title))
            .self_start()
            .px(4.0)
            .ml(-4.0)
            .radius(5.0)
            .hover(p.chip)
            .on_press_tx(move |s: &mut State, tx| s.open_meeting(&id, tx)),
    );
    for &i in &source.lines {
        let Some(l) = m.lines.get(i) else { continue };
        let main = source.at == Some(i);
        let mut line = col()
            .w_full()
            .gap(0.0)
            .px(8.0)
            .py(4.0)
            .radius(6.0)
            .child(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(
                        text(l.who.clone())
                            .font(roles::CAPTION.style())
                            .color(p.soft)
                            .truncate()
                            .min_w(0.0)
                            .shrink(),
                    )
                    .child(
                        text(core::short(&l.at).to_string())
                            .font(roles::META.medium().style().mono())
                            .color(p.faint),
                    ),
            )
            .child(
                text(l.text.clone())
                    .font(roles::SMALL.style())
                    .color(if main { p.ink } else { p.soft })
                    .wrap(),
            );
        if main {
            line = line.fill(p.hit);
        }
        panel = panel.child(line);
    }
    if let (Some(stamp), false) = (t.stamp.clone(), source.lines.is_empty()) {
        let id = t.meeting_id.clone();
        let index = t.index;
        panel = panel.child(
            pressable(roles::CONTROL.wrap("Read on in the transcript", p.accent))
                .key(format!("tsrc-jump-{}-{}", t.meeting_id, t.index))
                .self_start()
                .px(4.0)
                .ml(-4.0)
                .radius(5.0)
                .hover(p.chip)
                .on_press_tx(move |s: &mut State, tx| s.open_meeting_at(&id, index, &stamp, tx)),
        );
    }
    if let Some(d) = &source.decision {
        let mut said = d.text.clone();
        if let Some(who) = &d.who {
            said.push_str(&format!("  \u{b7}  {who}"));
        }
        panel = panel
            .child(spaced_caps("The decision behind it", p).mt(8.0))
            .child(
                row()
                    .gap(8.0)
                    .child(super::icons::draw(&super::icons::FLAG, 16.0, p.ok))
                    .child(roles::SMALL.wrap(said, p.ink).grow(1.0).min_w(0.0)),
            );
    }
    panel
}

fn task_panel(s: &State, k: &Kit, t: &TaskItem, p: Palette, layer: &mut Vec<El>) -> (El, El) {
    let tasks = &s.tasks;
    let (id, index) = (t.meeting_id.clone(), t.index);
    let title = if tasks.inputs.title.focused {
        let field = rune::text_input(&tasks.inputs.title, roles::SECTION.style())
            .key("ptitle")
            .label("Task title")
            .placeholder("At most 6 words. Start with what to do.", p.faint)
            .on_edit(|s: &mut State| &mut s.tasks.inputs.title)
            .on_submit(|s: &mut State, _| {
                s.commit_title();
            })
            .color(p.ink)
            .caret(p.ink, p.hit)
            .wrap_anywhere()
            .w_full();
        rune::auto_grow(field, 2, 2).key("task-title-editor")
    } else {
        pressable(
            roles::SECTION
                .wrap(&tasks.inputs.title.text, p.ink)
                .max_lines(2)
                .w_full()
                .min_h(56.0),
        )
        .key("ptitle")
        .label("Edit task title")
        .on_press(|s: &mut State| s.tasks.inputs.title.focused = true)
    };
    let description = rune::text_input(&tasks.inputs.description, roles::SMALL.style())
        .on_edit(|s: &mut State| &mut s.tasks.inputs.description)
        .placeholder("No description. Add one", p.faint)
        .color(p.ink)
        .caret(p.ink, p.hit)
        .on_submit(|s: &mut State, _| s.tasks.inputs.description.insert("\n"))
        .wrap_anywhere()
        .min_h(80.0)
        .key("pdescription")
        .label("Task description")
        .w_full();
    let mut body = col()
        .w_full()
        .gap(12.0)
        .px(20.0)
        .pt(4.0)
        .pb(28.0)
        .child(title)
        .children(
            tasks
                .inputs
                .title
                .focused
                .then(|| roles::META.wrap("At most 6 words. Start with what to do.", p.faint)),
        )
        .child(rune::auto_grow(description, 3, 8))
        .child(
            button("Save title and description", "task-text-save", false, p).on_press(
                |s: &mut State| {
                    s.commit_title();
                },
            ),
        );
    body = body.child(v2_controls(s, k, t, p, layer));
    if let Some(msg) = &tasks.ui.error {
        body = body.child(roles::SMALL.medium().wrap(msg, p.warn).role("alert"));
    }
    if s.has_task_draft() {
        body = body.child(
            button("Discard my changes", "task-discard", false, p).on_press(|s: &mut State| {
                s.discard_task();
            }),
        );
    }
    body = body
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(prop_row("Owner", owner_control(s, k, Some(t), p, layer), p))
                .child(prop_row("Due", due_control(s, k, Some(t), p), p))
                .children(
                    s.config
                        .features
                        .projects
                        .then(|| prop_row("Project", project_control(s, k, Some(t), p, layer), p)),
                ),
        )
        .child(hairline(p))
        .child(source_section(s, t, p));
    let mut foot = row()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .items_center()
        .child(
            button(
                if t.done { "Mark not done" } else { "Mark done" },
                "pdone",
                !t.done,
                p,
            )
            .on_press(move |s: &mut State| s.toggle_row(&id, index)),
        );
    if t.manual {
        let id = t.meeting_id.clone();
        foot = foot.child(el().grow(1.0)).child(
            pressable(roles::SMALL.medium().wrap("Delete task", p.warn))
                .key("pdelete")
                .px(10.0)
                .py(6.0)
                .radius(8.0)
                .hover(p.warnbg)
                .on_press(move |s: &mut State| s.delete_manual(&id)),
        );
    } else if t.edited {
        let id = t.meeting_id.clone();
        foot = foot.child(el().grow(1.0)).child(
            button("Undo my changes", "preset", false, p)
                .on_press(move |s: &mut State| s.reset_task(&id, index)),
        );
    }
    (body, foot)
}

fn add_panel(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> (El, El) {
    let t = &s.tasks;
    let title = input_h(
        k,
        &t.inputs.title,
        "add-title",
        "What needs doing?",
        |s| &mut s.tasks.inputs.title,
        40.0,
        p,
    )
    .w_full();
    let mut body = col()
        .w_full()
        .gap(18.0)
        .px(20.0)
        .pt(4.0)
        .pb(28.0)
        .child(title);
    if let Some(msg) = &t.ui.error {
        body = body.child(roles::SMALL.medium().wrap(msg, p.warn).role("alert"));
    }
    body = body
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(prop_row("Due", due_control(s, k, None, p), p))
                .children(
                    s.config
                        .features
                        .projects
                        .then(|| prop_row("Project", project_control(s, k, None, p, layer), p)),
                )
                .child(prop_row("Owner", owner_control(s, k, None, p, layer), p)),
        )
        .child(roles::META.wrap("Added by hand, kept on this computer.", p.faint));
    let foot = row()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .items_center()
        .child(button("Add task", "add-save", true, p).on_press(|s: &mut State| s.commit_add()))
        .child(
            button("Cancel", "add-cancel", false, p).on_press_tx(|s: &mut State, tx| {
                s.close_panel();
                tx.blur();
            }),
        )
        .child(
            roles::META
                .wrap("Enter adds, Escape cancels", p.faint)
                .grow(1.0)
                .min_w(0.0),
        );
    (body, foot)
}

/// The right-hand panel for the task or the add form that is open. None when it has nothing
/// to show.
fn panel(s: &State, k: &Kit, p: Palette, overlay: bool, layer: &mut Vec<El>) -> Option<El> {
    let (title, (body, foot)) = match &s.tasks.panel {
        Some(Panel::Add) => ("New task", add_panel(s, k, p, layer)),
        Some(Panel::Task(_)) => {
            let t = s.panel_task()?;
            ("Task", task_panel(s, k, t, p, layer))
        }
        None => return None,
    };
    Some(
        col()
            .key("tasks-panel")
            .h_full()
            .min_w(0.0)
            .min_h(0.0)
            .fill(p.card)
            .child(panel_top(title, overlay, p))
            .child(scroll(body, "tasks-panel-scroll"))
            .child(hairline(p))
            .child(foot),
    )
}

// ---- the page ------------------------------------------------------------------------------

/// The list column's width with the panel open, and how the panel sits: beside the list, or
/// over it when there is not enough room for both.
fn geometry(s: &State, narrow: bool) -> (f32, f32, bool) {
    let pane = s.app_w.get() - super::nav::width(s);
    let overlay = narrow || pane < 780.0;
    let panel = (pane * 0.38).clamp(340.0, 400.0);
    let list = if s.tasks.panel.is_some() && !overlay {
        pane - panel - 1.0
    } else {
        pane
    };
    (list, panel, overlay)
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let t = &s.tasks;
    let (list_w, panel_w, overlay) = geometry(s, narrow);
    let mut pane = col()
        .key("tasks-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .fill(p.bg);
    if let Some(note) = &t.notice {
        pane = pane.child(
            row()
                .px(24.0)
                .py(8.0)
                .gap(12.0)
                .items_center()
                .fill(p.warnbg)
                .child(roles::SMALL.medium().wrap(note, p.warn).grow(1.0))
                .child(
                    button("Dismiss", "tasks-notice-dismiss", false, p)
                        .on_press(|s: &mut State| s.tasks.notice = None),
                ),
        );
    }
    let panel_el = panel(s, k, p, overlay, layer);
    let body = row().w_full().grow(1.0).min_h(0.0).min_w(0.0);
    let Some(panel_el) = panel_el else {
        return pane.child(body.child(list_column(s, k, p, narrow, list_w, layer)));
    };
    if overlay {
        return pane.child(body.child(panel_el.grow(1.0)));
    }
    pane.child(
        body.child(list_column(s, k, p, narrow, list_w, layer))
            .child(el().w(1.0).h_full().fill(p.line))
            .child(panel_el.w(panel_w)),
    )
}

fn list_column(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    list_w: f32,
    layer: &mut Vec<El>,
) -> El {
    let t = &s.tasks;
    let side = if narrow {
        16.0
    } else if t.panel.is_some() {
        24.0
    } else {
        40.0
    };
    // Rows give the title what is left after the properties. The project column is narrower
    // beside the panel and gone when grouping by project, where the heading says it.
    let several_projects = t
        .items
        .first()
        .is_some_and(|first| t.items.iter().any(|item| item.project != first.project));
    let project_w = if !s.config.features.projects
        || !several_projects
        || t.grouping == TaskGrouping::Project
        || list_w < 620.0
    {
        0.0
    } else if t.grouping == TaskGrouping::Status {
        14.0
    } else if list_w < 700.0 {
        104.0
    } else {
        176.0
    };
    let lay = RowLayout { project_w };
    let v = t.arranged();
    let mut doc = col()
        .key("tasks-top")
        .w_full()
        .max_w(920.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0)
        .child(title_row(s, k, narrow, p, layer))
        .children((!t.items.is_empty()).then(|| redo_older_control(s, k, p, layer)))
        .children(t.titles_notice.then(|| row().w_full().gap(8.0).items_center()
            .child(roles::SMALL.wrap("Long task titles are now shown short. The full text is in each task's description.", p.soft).grow(1.0))
            .child(button("Dismiss", "titles-notice-dismiss", false, p).on_press(|s: &mut State| { s.tasks.titles_notice = false; s.save_title_prefs(); }))))
        .child(button(if t.show_full_titles { "Show full titles: on" } else { "Show full titles: off" }, "show-full-titles", false, p)
            .role("switch").checked(t.show_full_titles).on_press(|s: &mut State| { s.toggle_full_titles(); }))
        .child(summary_line(s, p).mb(16.0))
        .child(toolbar(s, k, p, layer, list_w.min(920.0) - 2.0 * side).mb(10.0));
    if v.groups.is_empty() && v.done.is_empty() {
        doc = doc.child(empty_note(s, p));
    }
    for (n, g) in v.groups.iter().enumerate() {
        doc = doc.child(group_card(s, k, g, n, lay, p));
    }
    if !v.done.is_empty() {
        doc = doc.child(done_section(s, &v, lay, p));
    }
    if !t.items.is_empty() || t.cutoff.is_some() {
        doc = doc.child(cleared_line(s, k, p, layer));
    }
    if !narrow && t.panel.is_none() {
        doc = doc.child(key_hints(p));
    }
    col().grow(1.0).min_w(0.0).min_h(0.0).child(scroll(
        col().w_full().items_center().child(doc),
        "tasks-scroll",
    ))
}

fn v2_controls(s: &State, k: &Kit, t: &TaskItem, p: Palette, layer: &mut Vec<El>) -> El {
    let mut out = col().w_full().gap(10.0);
    out = out.child(roles::META.wrap(
        if t.team == "No team" {
            "Team: none".into()
        } else {
            format!("Team: {} (from owner)", t.team)
        },
        p.soft,
    ));
    out = out.child(roles::GROUP.wrap(if t.done { "Status: Done" } else { "Status" }, p.ink));
    let mut choices = row().w_full().gap(6.0).flex_wrap();
    for status in [
        core::TaskStatus::Todo,
        core::TaskStatus::Waiting,
        core::TaskStatus::Review,
    ] {
        let id = t.meeting_id.clone();
        let index = t.index;
        let waiting =
            if status == core::TaskStatus::Waiting && t.status != core::TaskStatus::Waiting {
                None
            } else {
                t.waiting_on.clone()
            };
        choices = choices.child(
            button(
                status.label(),
                format!("status-{status:?}"),
                t.status == status && !t.done,
                p,
            )
            .on_press(move |s: &mut State| {
                s.edit_task_v2(
                    &id,
                    index,
                    core::TaskV2Edit::Status(status, waiting.clone()),
                )
            }),
        );
    }
    out = out.child(choices);
    if t.status == core::TaskStatus::Waiting {
        if s.tasks.ui.waiting_text {
            let (id, index) = (t.meeting_id.clone(), t.index);
            let submit_id = id.clone();
            out = out.child(
                row()
                    .w_full()
                    .gap(6.0)
                    .items_center()
                    .child(
                        TextInput::new(&s.tasks.waiting_input)
                            .placeholder("Person or event")
                            .on_edit(|s: &mut State| &mut s.tasks.waiting_input)
                            .on_submit(move |s: &mut State, _| s.save_waiting(&submit_id, index))
                            .height(34.0)
                            .build(k)
                            .key("waiting-text")
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(
                        button("Save", "waiting-save", true, p)
                            .on_press(move |s: &mut State| s.save_waiting(&id, index)),
                    )
                    .child(
                        button("Back", "waiting-back", false, p).on_press(|s: &mut State| {
                            s.tasks.ui.waiting_text = false;
                            s.tasks.waiting_input = TextEdit::default();
                        }),
                    ),
            );
        } else {
            let mut menu = Menu::new("waiting-tasks")
                .trigger(
                    super::area::button(
                        format!(
                            "Waiting on: {}",
                            t.waiting_label
                                .as_deref()
                                .unwrap_or("choose a person, event or task")
                        ),
                        "waiting-choose",
                        p,
                    )
                    .label("Waiting on: choose a person, event or task")
                    .on_press(|s: &mut State| s.tasks.ui.waiting_menu = !s.tasks.ui.waiting_menu),
                )
                .open(s.tasks.ui.waiting_menu)
                .placement(Placement::BottomStart)
                .min_width(240.0);
            if s.tasks.ui.waiting_menu {
                menu = menu
                    .on_close(|s: &mut State| s.tasks.ui.waiting_menu = false)
                    .item(
                        MenuItem::new("Person or event...").on_select(|s: &mut State| {
                            s.tasks.ui.waiting_menu = false;
                            s.tasks.ui.waiting_text = true;
                        }),
                    );
                for linked in s.tasks.all.iter().filter(|l| {
                    l.meeting_id == t.meeting_id && !l.stable_id.is_empty() && l.index != t.index
                }) {
                    let (id, index, target) =
                        (t.meeting_id.clone(), t.index, linked.stable_id.clone());
                    menu = menu.item(
                        MenuItem::new(&linked.title)
                            .checked(
                                t.waiting_on.as_ref().and_then(|w| w.task_id.as_ref())
                                    == Some(&target),
                            )
                            .on_select(move |s: &mut State| {
                                s.tasks.ui.waiting_menu = false;
                                s.edit_task_v2(
                                    &id,
                                    index,
                                    core::TaskV2Edit::Status(
                                        core::TaskStatus::Waiting,
                                        Some(core::WaitingOn {
                                            task_id: Some(target.clone()),
                                            text: None,
                                        }),
                                    ),
                                );
                            }),
                    );
                }
            }
            out = out.child(menu.build(k, layer));
        }
    }

    out = out.child(
        roles::GROUP
            .wrap(
                if t.subtasks.is_empty() {
                    "Sub-tasks".into()
                } else {
                    format!("Sub-tasks {}", t.progress())
                },
                p.ink,
            )
            .mt(16.0),
    );
    for sub in &t.subtasks {
        let (id, index, key, done) = (
            t.meeting_id.clone(),
            t.index,
            core::subtask_tick_key(&t.stable_id, &sub.id),
            sub.done,
        );
        out = out.child(
            pressable(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(status_dot(done, 16.0, p))
                    .child(
                        roles::SMALL
                            .wrap(&sub.title, p.soft)
                            .max_lines(2)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            )
            .key(format!("panel-sub-{}", sub.id))
            .label(format!("Sub-task: {}", sub.title))
            .role("checkbox")
            .checked(done)
            .py(5.0)
            .on_press(move |s: &mut State| {
                s.edit_task_v2(
                    &id,
                    index,
                    core::TaskV2Edit::SubtaskTick {
                        key: key.clone(),
                        done: !done,
                    },
                )
            }),
        );
    }
    let (id, index) = (t.meeting_id.clone(), t.index);
    let submit_id = id.clone();
    out = out.child(
        row()
            .w_full()
            .gap(6.0)
            .items_center()
            .child(
                TextInput::new(&s.tasks.subtask_input)
                    .placeholder("Add a sub-task")
                    .on_edit(|s: &mut State| &mut s.tasks.subtask_input)
                    .on_submit(move |s: &mut State, _| s.add_panel_subtask(&submit_id, index))
                    .height(34.0)
                    .build(k)
                    .key("subtask-title")
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                button("Add", "subtask-add", true, p)
                    .on_press(move |s: &mut State| s.add_panel_subtask(&id, index)),
            ),
    );

    out
}

fn redo_older_menu(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let older: Vec<_> = s
        .meetings
        .iter()
        .filter(|m| core::needs_task_redo(m))
        .collect();
    if older.is_empty() {
        return el();
    }
    let mut menu = Menu::new("redo-older-tasks")
        .trigger(
            super::area::button("...", "redo-meeting-open", p)
                .label("Task list options")
                .on_press(|s: &mut State| s.tasks.redo_menu = !s.tasks.redo_menu),
        )
        .open(s.tasks.redo_menu)
        .placement(Placement::BottomStart)
        .min_width(280.0);
    if s.tasks.redo_menu {
        menu = menu
            .on_close(|s: &mut State| s.tasks.redo_menu = false)
            .group_label("Rebuild tasks from old summaries");
        for m in older {
            let id = m.id.clone();
            let date = m
                .started
                .map(|d| d.format("%-d %b %Y").to_string())
                .unwrap_or_default();
            menu = menu.item(MenuItem::new(format!("{} ({date})", m.title)).on_select(
                move |s: &mut State| {
                    s.tasks.redo_menu = false;
                    s.tasks.redo_confirm = Some(id.clone());
                    s.tasks.redo_problem = None;
                },
            ));
        }
    }
    menu.build(k, layer)
}

fn redo_older_control(s: &State, _k: &Kit, p: Palette, _layer: &mut Vec<El>) -> El {
    let mut out = col().w_full().gap(10.0);

    if let Some(problem) = &s.tasks.redo_problem {
        out = out.child(
            super::widgets::banner(
                "Tasks were not rebuilt",
                problem,
                super::widgets::BannerKind::Problem,
                p,
            )
            .child(
                super::area::button("Open Meeting summary settings", "redo-open-settings", p)
                    .on_press(|s: &mut State| s.open_ai_notes()),
            )
            .child(
                button("Dismiss", "redo-problem-dismiss", false, p)
                    .on_press(|s: &mut State| s.tasks.redo_problem = None),
            ),
        );
    }
    if let Some(id) = &s.tasks.redo_confirm {
        let title = s
            .meetings
            .iter()
            .find(|m| m.id == *id)
            .map(|m| m.title.as_str())
            .unwrap_or("this meeting");
        let id = id.clone();
        out = out.child(super::widgets::banner("Rebuild tasks?", format!("Rebuild tasks for {title}? The transcript, summary and your typed notes go to your helper again. Your changes are kept only when tasks can be matched."), super::widgets::BannerKind::Info, p)
            .child(row().flex_wrap().gaps(8.0, 8.0)
                .child(button("Keep tasks", "redo-cancel", true, p).on_press(|s: &mut State| s.tasks.redo_confirm = None))
                .child(button("Rebuild tasks", "redo-confirm", false, p).on_press(move |s: &mut State| {
                    s.tasks.redo_confirm = None;
                    if let Some(problem) = s.notes_request_problem() {
                        s.tasks.redo_problem = Some(problem);
                        return;
                    }
                    s.publish_edit(super::edit_worker::Job::Redo { id: id.clone() });
                    s.tasks.notice = Some("Tasks queued. Edits are kept where tasks can be matched.".into());
                }))));
    }
    out
}

impl State {
    fn add_panel_subtask(&mut self, id: &str, index: usize) {
        let title = self.tasks.subtask_input.text.trim().to_string();
        if !title.is_empty() {
            self.edit_task_v2(id, index, core::TaskV2Edit::AddSubtask(title));
        }
    }
    fn save_waiting(&mut self, id: &str, index: usize) {
        let text = self.tasks.waiting_input.text.trim().to_string();
        if !text.is_empty() {
            self.tasks.ui.waiting_text = false;
            self.edit_task_v2(
                id,
                index,
                core::TaskV2Edit::Status(
                    core::TaskStatus::Waiting,
                    Some(core::WaitingOn {
                        task_id: None,
                        text: Some(text),
                    }),
                ),
            );
        }
    }
}

pub(super) fn shown_title<'a>(s: &State, t: &'a TaskItem) -> &'a str {
    let authored = if t.manual {
        true
    } else {
        s.tasks
            .overrides
            .match_task(&t.meeting_id, &t.stable_id, &t.task, t.index)
            .is_some_and(|o| o.title.is_some())
            || s.meetings
                .iter()
                .find(|m| m.id == t.meeting_id)
                .and_then(|m| m.summary.todo.get(t.index))
                .is_some_and(|todo| !todo.title.trim().is_empty())
    };
    if s.tasks.show_full_titles && !authored {
        &t.task
    } else {
        &t.title
    }
}
impl State {
    fn toggle_full_titles(&mut self) {
        self.tasks.show_full_titles = !self.tasks.show_full_titles;
        self.tasks.titles_notice = false;
        if self.tasks.inputs.title.text == self.tasks.edit_title
            && let Some(item) = self.panel_task()
        {
            let display = shown_title(self, item).to_string();
            self.tasks.inputs.title = TextEdit::new(&display);
            self.tasks.edit_title = display;
        }
        self.save_title_prefs();
    }
    fn save_title_prefs(&self) {
        let mut prefs = core::UiPrefs::load(&self.store.root);
        prefs.show_full_titles = self.tasks.show_full_titles;
        prefs.task_titles_notice_seen = !self.tasks.titles_notice;
        let _ = prefs.save(&self.store.root);
    }
}
