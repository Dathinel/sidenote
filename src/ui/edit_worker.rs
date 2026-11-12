//! At most one editor publication runs at a time; completion targets are immutable session
//! identities. A save that arrives while one runs waits its turn in `SaveQueue` and is never
//! dropped.
use super::{Event, State};
use serde_json::Value;
use sidenote::core::{
    self, TaskStore,
    memory::{MemoryRecord, MemoryStore},
};
use std::{collections::VecDeque, io, path::PathBuf, sync::mpsc::Sender};

/// One save runs; the rest wait in the order they were made. Each job already carries what it
/// needs and merges against the file when it runs, so running them one after another gives the
/// same result as if each had waited for the one before.
pub struct SaveQueue<J> {
    busy: bool,
    waiting: VecDeque<J>,
}
impl<J> Default for SaveQueue<J> {
    fn default() -> Self {
        Self {
            busy: false,
            waiting: VecDeque::new(),
        }
    }
}
impl<J> SaveQueue<J> {
    /// Hands back the job to start now, or keeps it for later and returns None.
    pub fn submit(&mut self, job: J) -> Option<J> {
        if self.busy {
            self.waiting.push_back(job);
            None
        } else {
            self.busy = true;
            Some(job)
        }
    }
    /// The running save ended. Hands back the next job, which becomes the running one, or
    /// None when the queue is empty and the worker is idle again.
    pub fn finish(&mut self) -> Option<J> {
        let next = self.waiting.pop_front();
        self.busy = next.is_some();
        next
    }
    #[cfg(test)]
    fn waiting(&self) -> usize {
        self.waiting.len()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveError {
    Busy,
    Moved,
    Missing,
    Access,
    Invalid,
    Failed,
    WorkerStopped,
}
impl SaveError {
    pub fn from(e: &io::Error) -> Self {
        if matches!(e.raw_os_error(), Some(32 | 33)) {
            return Self::Busy;
        }
        match e.kind() {
            io::ErrorKind::WouldBlock => Self::Busy,
            io::ErrorKind::InvalidInput => Self::Moved,
            io::ErrorKind::NotFound => Self::Missing,
            io::ErrorKind::PermissionDenied => Self::Access,
            io::ErrorKind::InvalidData => Self::Invalid,
            _ => Self::Failed,
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Busy => "Changes are still arriving. Your changes are kept. Try Save again.",
            Self::Moved => core::live_edit::MOVED,
            Self::Missing => "This item is no longer available. Your changes are kept.",
            Self::Access => "Could not write to the data folder. Your changes are kept.",
            Self::Invalid => "Could not read the saved data. Your changes are kept.",
            Self::Failed => "Could not save. Your changes are kept. Try again.",
            Self::WorkerStopped => "The save stopped. Your changes are kept. Try again.",
        }
    }
}
#[derive(Clone, Copy)]
pub enum Field {
    Title,
    Owner,
    Due,
    Project,
    Mine,
}
#[derive(Clone)]
pub enum Target {
    Tick,
    Task(String, usize, u64),
    Memory(String, u64),
}
impl Target {
    fn matches(&self, s: &State) -> bool {
        match self {
            Self::Tick => true,
            Self::Memory(id, session) => {
                s.memory.editor_generation == *session && s.memory.cursor.as_ref() == Some(id)
            }
            Self::Task(row, index, session) => {
                s.tasks.editor_generation == *session
                    && s.tasks.panel == Some(super::tasks::Panel::Task((row.clone(), *index)))
            }
        }
    }
}
pub enum Job {
    Company {
        root: PathBuf,
        edit: super::companies::Edit,
    },
    V2 {
        store: TaskStore,
        task: Box<core::TaskItem>,
        source: PathBuf,
        edit: core::TaskV2Edit,
    },
    /// A board move. The steps run one after the other in this one save, so the page takes in
    /// both at once.
    Move {
        store: TaskStore,
        ticks: core::StateStore,
        task: Box<core::TaskItem>,
        source: PathBuf,
        plan: core::MovePlan,
    },
    Grouping {
        store: TaskStore,
        by: core::TaskGrouping,
    },
    TeamEdit {
        store: TaskStore,
        original: Option<core::Team>,
        replacement: Option<core::Team>,
    },
    Redo {
        id: String,
    },
    Tick {
        store: core::StateStore,
        id: String,
        task: String,
        stable_id: String,
        on: bool,
    },
    ManualTick {
        store: TaskStore,
        id: String,
    },
    Task {
        store: TaskStore,
        row: String,
        index: usize,
        base: Value,
        mine: Value,
        source: Option<PathBuf>,
        field: Field,
        typed: String,
        close: bool,
        session: u64,
    },
    Memory {
        store: MemoryStore,
        base: Box<MemoryRecord>,
        text: String,
        session: u64,
    },
}
impl Job {
    fn target(&self) -> Target {
        match self {
            Self::Company { .. }
            | Self::Tick { .. }
            | Self::ManualTick { .. }
            | Self::V2 { .. }
            | Self::Move { .. }
            | Self::Grouping { .. }
            | Self::TeamEdit { .. }
            | Self::Redo { .. } => Target::Tick,
            Self::Task {
                row,
                index,
                session,
                ..
            } => Target::Task(row.clone(), *index, *session),
            Self::Memory { base, session, .. } => Target::Memory(base.id.clone(), *session),
        }
    }
}
pub enum Completion {
    Company(Result<Box<core::companies::Companies>, String>),
    Redo(Result<(), SaveError>),
    V2 {
        result: Result<Box<core::EditView<core::Overrides>>, SaveError>,
        added: Option<String>,
    },
    Move {
        id: String,
        moved: Box<core::Moved>,
        error: Option<SaveError>,
    },
    Preferences {
        teams: Option<core::Teams>,
        result: Result<(), SaveError>,
    },
    Tick {
        id: String,
        result: Result<Box<core::EditView<core::Saved>>, SaveError>,
    },
    ManualTick(Result<Box<core::EditView<core::Overrides>>, SaveError>),
    Task {
        row: String,
        index: usize,
        mine: Value,
        field: Field,
        typed: String,
        close: bool,
        session: u64,
        result: Result<Box<core::EditView<core::Overrides>>, SaveError>,
    },
    Memory {
        id: String,
        text: String,
        session: u64,
        result: Result<Box<core::EditView<MemoryRecord>>, SaveError>,
    },
    Failed(Target),
}
impl Completion {
    fn target(&self) -> Target {
        match self {
            Self::Company(_)
            | Self::Tick { .. }
            | Self::ManualTick(_)
            | Self::Preferences { .. }
            | Self::V2 { .. }
            | Self::Move { .. }
            | Self::Redo(_) => Target::Tick,
            Self::Task {
                row,
                index,
                session,
                ..
            } => Target::Task(row.clone(), *index, *session),
            Self::Memory { id, session, .. } => Target::Memory(id.clone(), *session),
            Self::Failed(t) => t.clone(),
        }
    }
}
struct Guard {
    target: Option<Target>,
    events: Sender<Event>,
    waker: Option<super::Waker>,
}
impl Guard {
    fn finish(&mut self, result: Completion) {
        self.target.take();
        let _ = self.events.send(Event::Edited(result));
        if let Some(w) = &self.waker {
            w();
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(target) = self.target.take() {
            let _ = self.events.send(Event::Edited(Completion::Failed(target)));
            if let Some(w) = &self.waker {
                w();
            }
        }
    }
}
impl State {
    pub(super) fn publish_edit(&mut self, job: Job) {
        if let Some(job) = self.edit_queue.submit(job) {
            self.edit_busy = true;
            self.start_edit(job);
        }
    }
    fn start_edit(&mut self, job: Job) {
        let mut guard = Guard {
            target: Some(job.target()),
            events: self.events_tx.clone(),
            waker: self.waker.clone(),
        };
        let _ = std::thread::Builder::new()
            .name("save-edit".into())
            .spawn(move || {
                let completion = match job {
                    Job::Company { root, edit } => {
                        Completion::Company(edit.run(&root).map(Box::new).map_err(|e| {
                            core::validation_message(&e)
                                .map(str::to_string)
                                .unwrap_or_else(|| SaveError::from(&e).message().into())
                        }))
                    }
                    Job::V2 {
                        store,
                        task,
                        source,
                        edit,
                    } => {
                        let added = if let core::TaskV2Edit::AddSubtask(title) = &edit {
                            Some(title.clone())
                        } else {
                            None
                        };
                        Completion::V2 {
                            result: store
                                .edit_v2(&task, &source, edit)
                                .map(Box::new)
                                .map_err(|e| SaveError::from(&e)),
                            added,
                        }
                    }
                    Job::Move {
                        store,
                        ticks,
                        task,
                        source,
                        plan,
                    } => {
                        let (moved, error) = core::run_move(&store, &ticks, &task, &source, plan);
                        Completion::Move {
                            id: task.meeting_id.clone(),
                            moved: Box::new(moved),
                            error: error.map(|e| SaveError::from(&e)),
                        }
                    }
                    Job::Grouping { store, by } => Completion::Preferences {
                        teams: None,
                        result: store.save_grouping(by).map_err(|e| SaveError::from(&e)),
                    },
                    Job::TeamEdit {
                        store,
                        original,
                        replacement,
                    } => match store.edit_team(original.as_ref(), replacement.as_ref()) {
                        Ok(teams) => Completion::Preferences {
                            teams: Some(teams),
                            result: Ok(()),
                        },
                        Err(e) => Completion::Preferences {
                            teams: None,
                            result: Err(SaveError::from(&e)),
                        },
                    },
                    Job::Redo { id } => Completion::Redo(
                        core::notes::request(&id, core::notes::Request::RedoTasks)
                            .map_err(|e| SaveError::from(&e)),
                    ),
                    Job::Tick {
                        store,
                        id,
                        task,
                        stable_id,
                        on,
                    } => Completion::Tick {
                        result: store
                            .set_task_tick_view(&id, &task, &stable_id, on)
                            .map(Box::new)
                            .map_err(|e| SaveError::from(&e)),
                        id,
                    },
                    Job::ManualTick { store, id } => Completion::ManualTick(
                        store
                            .toggle_manual_view(&id)
                            .map(Box::new)
                            .map_err(|e| SaveError::from(&e)),
                    ),
                    Job::Task {
                        store,
                        row,
                        index,
                        base,
                        mine,
                        source,
                        field,
                        typed,
                        close,
                        session,
                    } => {
                        let result = store
                            .merge_task(&row, index, &base, &mine, source.as_deref())
                            .map(Box::new)
                            .map_err(|e| SaveError::from(&e));
                        Completion::Task {
                            row,
                            index,
                            mine,
                            field,
                            typed,
                            close,
                            session,
                            result,
                        }
                    }
                    Job::Memory {
                        store,
                        base,
                        text,
                        session,
                    } => {
                        let result = store
                            .merge_edit(&base, &text)
                            .map(Box::new)
                            .map_err(|e| SaveError::from(&e));
                        Completion::Memory {
                            id: base.id.clone(),
                            text,
                            session,
                            result,
                        }
                    }
                };
                guard.finish(completion);
            });
    }
    pub(super) fn edit_finished(&mut self, completion: Completion) {
        self.edit_busy = false;
        self.apply_completion(completion);
        match self.edit_queue.finish() {
            Some(next) => {
                self.edit_busy = true;
                self.start_edit(next);
            }
            // Nothing waits, so the page now shows what every move did.
            None => self.tasks.board_pending.clear(),
        }
    }
    fn apply_completion(&mut self, completion: Completion) {
        if !completion.target().matches(self) {
            self.request_refresh();
            return;
        }
        match completion {
            Completion::Company(result) => match result {
                Ok(data) => {
                    self.companies.data = *data;
                    if let Some(id) = &self.companies.selected
                        && !self.companies.data.catalog.records.contains_key(id)
                    {
                        self.companies.selected = self.companies.merge_into.clone();
                        self.question = Default::default();
                        self.load_saved();
                    }
                    self.companies.editing = false;
                    self.companies.merging = false;
                    self.companies.person = None;
                    self.companies.notice = None;
                }
                Err(e) => self.companies.notice = Some(e),
            },
            Completion::Redo(result) => {
                self.tasks.notice = Some(
                    match result {
                        Ok(()) => "Tasks queued. Your changes will be kept.",
                        Err(_) => {
                            "Could not queue tasks. Check Meeting summary in Settings, then try again."
                        }
                    }
                    .into(),
                );
            }
            Completion::Preferences { teams, result } => match result {
                Ok(()) => {
                    self.hex.shell.saved =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                    if let Some(teams) = teams {
                        self.tasks.teams = teams;
                        self.tasks.teams_notice = Some("Teams saved".into());
                        self.people.team_editor = None;
                        self.tasks.team_name = rune::TextEdit::default();
                        self.tasks.team_members = rune::TextEdit::default();
                        self.tasks.team_aliases = rune::TextEdit::default();
                    }
                    self.rebuild_tasks();
                }
                Err(e) => {
                    self.tasks.teams_notice = Some(e.message().into());
                    self.tasks.notice = Some(e.message().into());
                }
            },
            Completion::Tick { id, result } => match result {
                Ok(view) => {
                    if view.generation == core::mutation_generation() {
                        self.tasks
                            .checked
                            .insert(id.clone(), view.value.checked.clone());
                        if self.selected.as_deref() == Some(&id) {
                            self.saved = view.value;
                        }
                        self.rebuild_tasks();
                    }
                }
                Err(e) => self.notice = Some(e.message().into()),
            },
            Completion::V2 { result, added } => match result {
                Ok(view) => {
                    if view.generation == core::mutation_generation() {
                        self.tasks.overrides = view.value;
                        self.rebuild_tasks();
                    }
                    if added.is_some_and(|title| self.tasks.subtask_input.text.trim() == title) {
                        self.tasks.subtask_input = Default::default();
                    }
                }
                Err(e) => self.tasks.notice = Some(e.message().into()),
            },
            Completion::Move { id, moved, error } => {
                // Both steps wrote, so only the newest view is current. The tick view is
                // older than the status view by one write and is taken with it.
                let newest = moved
                    .overrides
                    .as_ref()
                    .map(|v| v.generation)
                    .into_iter()
                    .chain(moved.ticks.as_ref().map(|v| v.generation))
                    .max();
                if newest == Some(core::mutation_generation()) {
                    let core::Moved { ticks, overrides } = *moved;
                    if let Some(view) = ticks {
                        self.tasks
                            .checked
                            .insert(id.clone(), view.value.checked.clone());
                        if self.selected.as_deref() == Some(&id) {
                            self.saved = view.value;
                        }
                    }
                    if let Some(view) = overrides {
                        self.tasks.overrides = view.value;
                    }
                }
                if let Some(e) = error {
                    self.tasks.notice = Some(e.message().into());
                }
                self.rebuild_tasks();
            }
            Completion::ManualTick(result) => match result {
                Ok(view) => {
                    if view.generation == core::mutation_generation() {
                        self.tasks.overrides = view.value;
                        self.rebuild_tasks();
                    }
                }
                Err(e) => self.notice = Some(e.message().into()),
            },
            Completion::Failed(Target::Tick) => {
                self.notice = Some(SaveError::WorkerStopped.message().into())
            }
            Completion::Task {
                row,
                index,
                mine,
                field,
                typed,
                close,
                result,
                ..
            } => match result {
                Ok(view) => {
                    let saved = if let Some(item) = self
                        .tasks
                        .all
                        .iter()
                        .find(|t| t.meeting_id == row && t.index == index && !t.manual)
                    {
                        serde_json::to_value(view.value.match_task(
                            &row,
                            &item.stable_id,
                            &item.task,
                            index,
                        ))
                        .unwrap_or_default()
                    } else {
                        core::live_edit::task_value(&view.value, &row, index)
                    };
                    core::live_edit::advance_base(&mut self.tasks.edit_base, &mine, &saved);
                    let current = view.generation == core::mutation_generation();
                    if current {
                        self.tasks.overrides = view.value;
                        self.rebuild_tasks();
                    }
                    self.tasks.edit_failed = false;
                    self.tasks.notice = None;
                    let description_unchanged = !matches!(field, Field::Title)
                        || self.tasks.inputs.description.text
                            == mine["description"]
                                .as_str()
                                .unwrap_or(&self.tasks.edit_description);
                    let input = match field {
                        Field::Title => &mut self.tasks.inputs.title,
                        Field::Owner | Field::Mine => &mut self.tasks.inputs.owner,
                        Field::Due => &mut self.tasks.inputs.due,
                        Field::Project => &mut self.tasks.inputs.project,
                    };
                    let unchanged = input.text == typed && description_unchanged;
                    if matches!(field, Field::Title) {
                        if unchanged {
                            self.tasks.ui.title_edit = false;
                            self.tasks.ui.description_edit = false;
                        }
                        if let Some(title) = mine["title"].as_str() {
                            self.tasks.edit_title = title.into();
                        }
                        if let Some(description) = mine["description"].as_str() {
                            self.tasks.edit_description = description.into();
                        }
                    } else if unchanged {
                        *input = Default::default();
                        match field {
                            Field::Owner | Field::Mine => self.tasks.ui.owner_edit = false,
                            Field::Project => self.tasks.ui.new_project = false,
                            _ => {}
                        }
                    }
                    if close && unchanged {
                        self.close_panel();
                    }
                }
                Err(e) => {
                    self.tasks.edit_failed = true;
                    self.tasks.notice = Some(e.message().into());
                }
            },
            Completion::Memory {
                id, text, result, ..
            } => match result {
                Ok(view) => {
                    let record = view.value;
                    if view.generation == core::mutation_generation()
                        && let Some(r) = self.memory.records.iter_mut().find(|r| r.id == id)
                    {
                        *r = record.clone();
                    }
                    self.memory.edit_base = Some(record);
                    self.memory.errors.remove(&id);
                    self.memory.notice = None;
                    if self.memory.edit.text == text {
                        self.memory.editing = false;
                        self.memory.edit = Default::default();
                    }
                }
                Err(e) => {
                    self.memory.errors.insert(id, e.message().into());
                }
            },
            Completion::Failed(Target::Memory(id, _)) => {
                self.memory
                    .errors
                    .insert(id, SaveError::WorkerStopped.message().into());
            }
            Completion::Failed(Target::Task(..)) => {
                self.tasks.edit_failed = true;
                self.tasks.notice = Some(SaveError::WorkerStopped.message().into());
            }
        }
        self.request_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::SaveQueue;

    #[test]
    fn a_second_save_waits_instead_of_being_dropped() {
        let mut q = SaveQueue::default();
        assert_eq!(q.submit("first"), Some("first"));
        assert_eq!(q.submit("second"), None);
        assert_eq!(q.submit("third"), None);
        assert_eq!(q.waiting(), 2);
        // Each finish hands over the next job in the order they were made.
        assert_eq!(q.finish(), Some("second"));
        assert_eq!(q.finish(), Some("third"));
        assert_eq!(q.finish(), None);
        assert_eq!(q.waiting(), 0);
    }

    #[test]
    fn an_idle_queue_starts_the_next_save_at_once() {
        let mut q = SaveQueue::default();
        assert_eq!(q.submit(1), Some(1));
        assert_eq!(q.finish(), None);
        assert_eq!(q.submit(2), Some(2));
        // A save made while the running one finishes still runs: the queue stays busy until
        // it is empty.
        assert_eq!(q.submit(3), None);
        assert_eq!(q.finish(), Some(3));
        assert_eq!(q.submit(4), None);
        assert_eq!(q.finish(), Some(4));
        assert_eq!(q.finish(), None);
    }
}
