//! At most one editor publication per window; only the worker touches storage.
use super::{Event, State};
use serde_json::Value;
use sidenote::core::{
    self, TaskStore,
    memory::{MemoryRecord, MemoryStore},
};
use std::path::PathBuf;
#[derive(Clone, Copy)]
pub enum Field {
    Title,
    Owner,
    Due,
    Project,
    Mine,
}
pub enum Job {
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
    },
    Memory {
        store: MemoryStore,
        base: Box<MemoryRecord>,
        text: String,
    },
}
pub enum Completion {
    Task {
        row: String,
        index: usize,
        mine: Value,
        field: Field,
        typed: String,
        close: bool,
        result: Result<core::Overrides, String>,
    },
    Memory {
        id: String,
        text: String,
        result: Result<Box<MemoryRecord>, String>,
    },
}
impl State {
    pub(super) fn publish_edit(&mut self, job: Job) {
        if self.edit_busy {
            return;
        }
        self.edit_busy = true;
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = match job {
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
                } => {
                    let result = store
                        .merge_task(&row, index, &base, &mine, source.as_deref())
                        .map_err(|e| e.to_string());
                    Completion::Task {
                        row,
                        index,
                        mine,
                        field,
                        typed,
                        close,
                        result,
                    }
                }
                Job::Memory { store, base, text } => {
                    let result = store
                        .merge_edit(&base, &text)
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    Completion::Memory {
                        id: base.id.clone(),
                        text,
                        result,
                    }
                }
            };
            let _ = events.send(Event::Edited(result));
            if let Some(waker) = waker {
                waker();
            }
        });
    }
    pub(super) fn edit_finished(&mut self, result: Completion) {
        self.edit_busy = false;
        match result {
            Completion::Task {
                row,
                index,
                mine,
                field,
                typed,
                close,
                result,
            } => match result {
                Ok(o) => {
                    let saved = core::live_edit::task_value(&o, &row, index);
                    core::live_edit::advance_base(&mut self.tasks.edit_base, &mine, &saved);
                    self.tasks.overrides = o;
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
                    self.rebuild_tasks();
                    if close && unchanged {
                        self.close_panel();
                    }
                }
                Err(e) => {
                    self.tasks.edit_failed = true;
                    self.tasks.notice = Some(e);
                }
            },
            Completion::Memory { id, text, result } => match result {
                Ok(record) => {
                    if let Some(r) = self.memory.records.iter_mut().find(|r| r.id == id) {
                        *r = (*record).clone();
                    }
                    self.memory.edit_base = Some(*record);
                    self.memory.errors.remove(&id);
                    self.memory.notice = None;
                    if self.memory.edit.text == text {
                        self.memory.editing = false;
                        self.memory.edit = Default::default();
                    }
                }
                Err(e) => {
                    self.memory.errors.insert(id, e);
                }
            },
        }
        self.rescan();
    }
}
