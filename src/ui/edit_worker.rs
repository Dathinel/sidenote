//! At most one editor publication; completion targets are immutable session identities.
use super::{Event, State};
use serde_json::Value;
use sidenote::core::{
    self, TaskStore,
    memory::{MemoryRecord, MemoryStore},
};
use std::{io, path::PathBuf, sync::mpsc::Sender};
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
    V2 {
        store: TaskStore,
        task: Box<core::TaskItem>,
        source: PathBuf,
        edit: core::TaskV2Edit,
    },
    Grouping {
        store: TaskStore,
        by: core::TaskGrouping,
    },
    Teams {
        store: TaskStore,
        teams: core::Teams,
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
            Self::Tick { .. }
            | Self::ManualTick { .. }
            | Self::V2 { .. }
            | Self::Grouping { .. }
            | Self::Teams { .. }
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
    V2 {
        result: Result<Box<core::EditView<core::Overrides>>, SaveError>,
        added: Option<String>,
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
            Self::Tick { .. }
            | Self::ManualTick(_)
            | Self::Preferences { .. }
            | Self::V2 { .. } => Target::Tick,
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
        if self.edit_busy {
            return;
        }
        self.edit_busy = true;
        let mut guard = Guard {
            target: Some(job.target()),
            events: self.events_tx.clone(),
            waker: self.waker.clone(),
        };
        let _ = std::thread::Builder::new()
            .name("save-edit".into())
            .spawn(move || {
                let completion = match job {
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
                    Job::Grouping { store, by } => Completion::Preferences {
                        teams: None,
                        result: store.save_grouping(by).map_err(|e| SaveError::from(&e)),
                    },
                    Job::Teams { store, teams } => Completion::Preferences {
                        result: store.save_teams(&teams).map_err(|e| SaveError::from(&e)),
                        teams: Some(teams),
                    },
                    Job::Redo { id } => Completion::Preferences {
                        teams: None,
                        result: core::notes::request(&id, core::notes::Request::RedoTasks)
                            .map_err(|e| SaveError::from(&e)),
                    },
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
        if !completion.target().matches(self) {
            self.request_refresh();
            return;
        }
        match completion {
            Completion::Preferences { teams, result } => match result {
                Ok(()) => {
                    if let Some(teams) = teams {
                        self.tasks.teams = teams;
                        self.tasks.teams_notice = Some("Teams saved".into());
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
                    let input = match field {
                        Field::Title => &mut self.tasks.inputs.title,
                        Field::Owner | Field::Mine => &mut self.tasks.inputs.owner,
                        Field::Due => &mut self.tasks.inputs.due,
                        Field::Project => &mut self.tasks.inputs.project,
                    };
                    let unchanged = input.text == typed;
                    if matches!(field, Field::Title) {
                        self.tasks.edit_title = mine["task"].as_str().unwrap_or_default().into();
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
