//! The task list across every meeting: each to-do with its project, owner, due date and tick,
//! the files that hold the user's edits, and the filters and groups the Tasks view shows.
//!
//! Three files feed it, none of them inside a meeting's own summary:
//! `<meeting folder>/tasks.json` (written by the pipeline, read only), `projects.json` (the
//! user registers names and renames) and `task-overrides.json` (the app's edits).
//! Ticks are the existing `StateStore` ticks, so the To do panel and this list stay in step.
use super::{Meeting, StateStore, state_dir};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Where a to-do goes when its meeting has no `tasks.json`.
pub const UNSORTED: &str = "Unsorted";

static WRITES: Mutex<()> = Mutex::new(());

fn read_json<T: Default + DeserializeOwned>(path: &Path) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => {
            let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            serde_json::from_slice(body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

// ---- the pipeline's tasks.json -------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskTag {
    /// Index into the meeting's `todo` list.
    pub i: usize,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub new: bool,
    #[serde(default)]
    pub mine: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TasksFile {
    pub meeting_project: Option<String>,
    pub tasks: Vec<TaskTag>,
}
/// Meeting id to its `tasks.json`. A meeting without a readable file has no entry.
pub type Pipeline = HashMap<String, TasksFile>;

pub fn load_pipeline(meetings: &[Meeting]) -> Pipeline {
    meetings
        .iter()
        .filter_map(|m| {
            let file = read_json::<TasksFile>(&m.folder.join("tasks.json")).ok()?;
            Some((m.id.clone(), file))
        })
        .collect()
}
pub fn load_pipeline_in(root: &Path, meetings: &[Meeting]) -> Pipeline {
    meetings
        .iter()
        .filter_map(|m| {
            let file = read_json::<TasksFile>(&root.join(&m.id).join("tasks.json")).ok()?;
            Some((m.id.clone(), file))
        })
        .collect()
}

// ---- projects.json -------------------------------------------------------------------------

/// `projects.json` in the selected state folder. Unknown keys survive a write.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Projects {
    pub projects: Vec<String>,
    /// Old name to new name. Chains are followed, so a renamed project stays merged.
    pub aliases: BTreeMap<String, String>,
    pub merges: BTreeMap<String, super::project_merge::ProjectMerge>,
    /// Stable assignments also cover meetings with no tasks. Source files remain untouched.
    pub meeting_projects: BTreeMap<String, String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
impl Projects {
    /// The name a project now goes by, after following its aliases.
    pub fn resolve(&self, name: &str) -> String {
        let matches = self.matches(name);
        if matches.len() == 1 {
            matches[0].clone()
        } else {
            self.resolve_exact(name)
        }
    }
    fn resolve_exact(&self, name: &str) -> String {
        let mut name = name.trim();
        let mut seen = BTreeSet::new();
        while seen.insert(name) {
            let next = self
                .merges
                .get(name)
                .filter(|m| m.active)
                .map(|m| &m.into)
                .or_else(|| self.aliases.get(name));
            match next {
                Some(next) if next != name => name = next.trim(),
                _ => return name.to_string(),
            }
        }
        // Concurrent peer merges can form a cycle. Keep every item in one stable folder.
        seen.into_iter().min().unwrap_or(name).to_string()
    }
    pub fn matches(&self, name: &str) -> Vec<String> {
        let key = super::project_merge::normalized_name(name);
        let mut matches = BTreeSet::new();
        for known in self
            .projects
            .iter()
            .chain(self.aliases.keys())
            .chain(self.merges.keys())
        {
            if super::project_merge::normalized_name(known) == key {
                matches.insert(self.resolve_exact(known));
            }
        }
        matches.into_iter().collect()
    }
    /// Points `old` at `new`, so every task of either name lands under `new`.
    fn rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        // `new` becomes the final name, so it must not point anywhere itself.
        self.aliases.remove(new);
        for target in self.aliases.values_mut() {
            if target == old {
                *target = new.to_string();
            }
        }
        self.aliases.insert(old.to_string(), new.to_string());
        self.add(new);
    }
    fn add(&mut self, name: &str) {
        if !name.is_empty() && self.matches(name).is_empty() {
            self.projects.push(name.to_string());
        }
    }
}

// ---- task-overrides.json -------------------------------------------------------------------

/// One task's edits. A field that is `None` keeps what the pipeline said.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// `YYYY-MM-DD`, or an empty string for "no date".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// The to-do's text when the edit was made. A summary that is regenerated can move the
    /// indexes, so an edit whose text no longer matches is ignored rather than misapplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
impl TaskOverride {
    fn is_empty(&self) -> bool {
        self.project.is_none() && self.mine.is_none() && self.owner.is_none() && self.due.is_none()
    }
}
/// The meeting id that hand-added tasks carry. Real meeting ids are `digits-digits`.
pub const MANUAL: &str = "manual";

/// A task the user added by hand. It has no meeting, so it keeps its own tick.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ManualTask {
    /// Numeric legacy metadata, never used for merge/edit/delete identity.
    #[serde(skip_serializing_if = "is_zero")]
    pub n: usize,
    pub id: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// `YYYY-MM-DD`; missing or empty means no date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<bool>,
    pub done: bool,
    /// `YYYY-MM-DD`, the day it was added.
    pub added: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl ManualTask {
    /// The same edit paths as a meeting task: change a `TaskOverride` view of it, then copy back.
    pub fn edit(&mut self, change: impl FnOnce(&mut TaskOverride)) {
        let mut o = TaskOverride {
            project: self.project.clone(),
            mine: self.mine,
            owner: self.owner.clone(),
            due: self.due.clone(),
            ..TaskOverride::default()
        };
        change(&mut o);
        self.project = o.project.filter(|p| !p.trim().is_empty());
        self.mine = o.mine;
        self.owner = o.owner.filter(|p| !p.trim().is_empty());
        self.due = o.due.filter(|d| !d.trim().is_empty());
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    /// Meeting id, then to-do index.
    pub tasks: BTreeMap<String, BTreeMap<String, TaskOverride>>,
    /// Tasks added by hand.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub manual: Vec<ManualTask>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
impl Overrides {
    pub fn get(&self, meeting_id: &str, index: usize) -> Option<&TaskOverride> {
        self.tasks.get(meeting_id)?.get(&index.to_string())
    }
}

// ---- tasks-settings.json -------------------------------------------------------------------

/// `tasks-settings.json` in the selected state folder. A missing file means no cutoff.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TasksSettings {
    /// `YYYY-MM-DD`. Tasks from meetings that started before this day are left off the Tasks
    /// page. The meetings themselves are untouched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleared_before: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
impl TasksSettings {
    /// None when no cutoff is set, or when the date cannot be read.
    pub fn cutoff(&self) -> Option<NaiveDate> {
        self.cleared_before
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok())
    }
}

/// Reads and writes the app's own task files in one folder (the selected state folder).
#[derive(Clone, Debug)]
pub struct TaskStore {
    pub root: PathBuf,
}
impl Default for TaskStore {
    fn default() -> Self {
        Self { root: state_dir() }
    }
}
impl TaskStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn projects_path(&self) -> PathBuf {
        self.root.join("projects.json")
    }
    fn overrides_path(&self) -> PathBuf {
        self.root.join("task-overrides.json")
    }
    /// A missing file is empty. A malformed one is an error, so it is never overwritten.
    pub fn load_projects(&self) -> io::Result<Projects> {
        super::sync_state::load(&self.root, "projects.json")
    }
    pub fn load_overrides(&self) -> io::Result<Overrides> {
        super::sync_state::load(&self.root, "task-overrides.json")
    }
    fn update<T: Default + DeserializeOwned + Serialize>(
        path: &Path,
        change: impl FnOnce(&mut T),
    ) -> io::Result<T> {
        let _guard = WRITES
            .lock()
            .map_err(|_| io::Error::other("Task lock poisoned"))?;
        let root = path
            .parent()
            .ok_or_else(|| super::store_io::invalid("Missing task state root"))?;
        let store = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| super::store_io::invalid("Invalid task store"))?;
        super::sync_state::update(root, store, change)
    }
    fn settings_path(&self) -> PathBuf {
        self.root.join("tasks-settings.json")
    }
    pub fn load_settings(&self) -> io::Result<TasksSettings> {
        super::sync_state::load(&self.root, "tasks-settings.json")
    }
    /// Sets the day before which meetings' tasks are cleared, or None to keep all.
    pub fn set_cutoff(&self, day: Option<NaiveDate>) -> io::Result<TasksSettings> {
        Self::update(&self.settings_path(), |s: &mut TasksSettings| {
            s.cleared_before = day.map(|d| d.format("%Y-%m-%d").to_string());
        })
    }
    pub fn update_projects(&self, change: impl FnOnce(&mut Projects)) -> io::Result<Projects> {
        Self::update(&self.projects_path(), change)
    }
    pub fn update_overrides(&self, change: impl FnOnce(&mut Overrides)) -> io::Result<Overrides> {
        Self::update(&self.overrides_path(), change)
    }
    /// Adds a project name to the list the pipeline reads.
    pub fn add_project(&self, name: &str) -> io::Result<Projects> {
        super::sync_state::update_checked(&self.root, "projects.json", |p: &mut Projects| {
            if p.matches(name).len() > 1 {
                return Err(super::store_io::invalid("Choose an existing project"));
            }
            p.add(name.trim());
            Ok(())
        })
    }
    /// Renames a project by writing an alias `old -> new`. Naming an existing project merges.
    pub fn rename_project(&self, old: &str, new: &str) -> io::Result<Projects> {
        super::sync_state::update_checked(&self.root, "projects.json", |p: &mut Projects| {
            if !p.matches(new).is_empty() && p.resolve(old) != p.resolve(new) {
                return Err(super::store_io::invalid(
                    "Use Merge into to preview this change",
                ));
            }
            p.rename(old.trim(), new.trim());
            Ok(())
        })
    }
    pub fn assign_meeting(&self, id: &str, name: &str) -> io::Result<Projects> {
        super::sync_state::update_checked(&self.root, "projects.json", |p: &mut Projects| {
            if name != UNSORTED && p.matches(name).len() != 1 {
                return Err(super::store_io::invalid(
                    "Choose or explicitly create a project first",
                ));
            }
            p.meeting_projects.insert(id.into(), p.resolve(name));
            Ok(())
        })
    }
    /// Changes one task's override. The entry is removed when nothing is left in it.
    pub fn update_override(
        &self,
        meeting_id: &str,
        index: usize,
        task: &str,
        change: impl FnOnce(&mut TaskOverride),
    ) -> io::Result<Overrides> {
        self.update_overrides(|o| {
            let per_meeting = o.tasks.entry(meeting_id.to_string()).or_default();
            let entry = per_meeting.entry(index.to_string()).or_default();
            // An override made against other text belongs to a different to-do.
            if entry.task.as_deref().is_some_and(|t| t != task) {
                *entry = TaskOverride::default();
            }
            change(entry);
            entry.task = Some(task.to_string());
            entry.other.insert(
                "updated_at".into(),
                Value::String(chrono::Utc::now().to_rfc3339()),
            );
            if entry.is_empty() {
                per_meeting.remove(&index.to_string());
            }
            if per_meeting.is_empty() {
                o.tasks.remove(meeting_id);
            }
        })
    }
    /// Drops every edit made to one task.
    pub fn clear_override(&self, meeting_id: &str, index: usize) -> io::Result<Overrides> {
        self.update_overrides(|o| {
            if let Some(per_meeting) = o.tasks.get_mut(meeting_id) {
                per_meeting.remove(&index.to_string());
                if per_meeting.is_empty() {
                    o.tasks.remove(meeting_id);
                }
            }
        })
    }
    /// Adds a task with a globally unique installation/random identity.
    pub fn add_manual(&self, mut task: ManualTask) -> io::Result<(Overrides, String)> {
        let id = super::sync_state::new_id(&self.root)?;
        task.id = id.clone();
        task.n = 0;
        let o = self.update_overrides(|o| o.manual.push(task))?;
        Ok((o, id))
    }
    pub fn update_manual(
        &self,
        id: &str,
        change: impl FnOnce(&mut ManualTask),
    ) -> io::Result<Overrides> {
        self.update_overrides(|o| {
            if let Some(m) = o.manual.iter_mut().find(|m| m.id == id) {
                change(m);
            }
        })
    }
    pub fn delete_manual(&self, id: &str) -> io::Result<Overrides> {
        self.update_overrides(|o| o.manual.retain(|m| m.id != id))
    }
    /// The ticked task texts of one meeting, from the same files the To do panel writes.
    pub fn load_ticks(&self, meeting_id: &str) -> BTreeSet<String> {
        StateStore::new(self.root.clone())
            .load(meeting_id)
            .map(|s| s.checked)
            .unwrap_or_default()
    }
}

// ---- the task list -------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TaskItem {
    pub meeting_id: String,
    pub meeting_title: String,
    pub meeting_date: Option<NaiveDate>,
    /// Position of the meeting in the catalog, newest first.
    pub meeting_rank: usize,
    /// Index into the meeting's `todo` list.
    pub index: usize,
    pub task: String,
    pub owner: Option<String>,
    /// As written: `YYYY-MM-DD`, or whatever the summary said.
    pub due: Option<String>,
    pub due_date: Option<NaiveDate>,
    pub due_default: bool,
    /// The first transcript timestamp the summary gave.
    pub stamp: Option<String>,
    pub project: String,
    pub mine: Option<bool>,
    pub done: bool,
    /// The user has edited this task.
    pub edited: bool,
    /// Hand-added rows carry their global id in `meeting_id` as `manual/<id>`.
    pub manual: bool,
}
impl TaskItem {
    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        !self.done && self.due_date.is_some_and(|d| d < today)
    }
}

/// Hand-made tasks first, then every to-do of every meeting, newest meeting first, in the
/// order the summary lists them.
pub fn build_tasks(
    meetings: &[Meeting],
    pipeline: &Pipeline,
    projects: &Projects,
    overrides: &Overrides,
    checked: &HashMap<String, BTreeSet<String>>,
) -> Vec<TaskItem> {
    let mut out = Vec::new();
    for m in &overrides.manual {
        let due = m.due.clone().filter(|d| !d.trim().is_empty());
        let project = m
            .project
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(UNSORTED);
        out.push(TaskItem {
            meeting_id: format!("{MANUAL}/{}", m.id),
            meeting_title: "Added by hand".to_string(),
            meeting_date: NaiveDate::parse_from_str(&m.added, "%Y-%m-%d").ok(),
            meeting_rank: 0,
            index: 0,
            task: m.task.clone(),
            owner: m.owner.clone().filter(|o| !o.trim().is_empty()),
            due_date: due
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            due,
            due_default: false,
            stamp: None,
            project: projects.resolve(project),
            mine: m.mine,
            done: m.done,
            edited: false,
            manual: true,
        });
    }
    for (rank, m) in meetings.iter().enumerate() {
        let rank = rank + 1;
        let file = pipeline.get(&m.id);
        for (index, todo) in m.summary.todo.iter().enumerate() {
            let tag = file.and_then(|f| f.tasks.iter().find(|t| t.i == index));
            let edit = overrides
                .get(&m.id, index)
                .filter(|o| o.task.as_deref().is_none_or(|t| t == todo.task));
            let project = edit
                .and_then(|o| o.project.clone())
                .or_else(|| tag.map(|t| t.project.clone()))
                .or_else(|| file.and_then(|f| f.meeting_project.clone()))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| UNSORTED.to_string());
            let project = projects.resolve(&project);
            let project = if edit.and_then(|o| o.project.as_ref()).is_some()
                || projects
                    .projects
                    .iter()
                    .any(|p| projects.resolve(p).eq_ignore_ascii_case(&project))
            {
                project
            } else {
                UNSORTED.to_string()
            };
            let owner = match edit.and_then(|o| o.owner.as_deref()) {
                Some(o) => Some(o),
                None => todo.owner.as_deref(),
            }
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(str::to_string);
            let (due, due_default) = match edit.and_then(|o| o.due.as_deref()) {
                Some(d) => (Some(d.to_string()), false),
                None => (todo.due.clone(), todo.due_default),
            };
            let due = due.filter(|d| !d.trim().is_empty());
            out.push(TaskItem {
                meeting_id: m.id.clone(),
                meeting_title: m.title.clone(),
                meeting_date: m.started.map(|s| s.date_naive()),
                meeting_rank: rank,
                index,
                task: todo.task.clone(),
                owner,
                due_date: due
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
                due,
                due_default,
                stamp: todo.at.first().cloned(),
                project: projects.resolve(&project),
                mine: edit
                    .and_then(|o| o.mine)
                    .or_else(|| tag.and_then(|t| t.mine)),
                done: checked.get(&m.id).is_some_and(|c| c.contains(&todo.task)),
                edited: edit.is_some(),
                manual: false,
            });
        }
    }
    out
}

/// The one cutoff rule. A task is cleared when its meeting started before `cutoff`, unless the
/// owner added it by hand or edited it (owner, due date, project or Mine). A meeting with no
/// start time has no age, so its tasks stay. A tick does not exempt a task.
pub fn is_cleared(t: &TaskItem, cutoff: Option<NaiveDate>) -> bool {
    !t.manual && !t.edited && matches!((cutoff, t.meeting_date), (Some(c), Some(d)) if d < c)
}

/// The tasks the cutoff leaves on the page, in the same order.
pub fn without_cleared(tasks: Vec<TaskItem>, cutoff: Option<NaiveDate>) -> Vec<TaskItem> {
    tasks
        .into_iter()
        .filter(|t| !is_cleared(t, cutoff))
        .collect()
}

/// The project most of a meeting's to-dos are in, ignoring Unsorted. Ties go to the project of
/// the earliest to-do. None when every to-do is unsorted, or there are none.
pub fn dominant_project<'a>(items: impl Iterator<Item = &'a TaskItem>) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for t in items.filter(|t| t.project != UNSORTED) {
        match counts.iter_mut().find(|(n, _)| *n == t.project) {
            Some((_, c)) => *c += 1,
            None => counts.push((&t.project, 1)),
        }
    }
    counts
        .iter()
        .fold(None::<(&str, usize)>, |best, &(n, c)| match best {
            Some((_, b)) if b >= c => best,
            _ => Some((n, c)),
        })
        .map(|(n, _)| n.to_string())
}

/// Every project name to offer in a menu, without the Unsorted bucket, sorted for reading.
pub fn project_names(projects: &Projects, _tasks: &[TaskItem]) -> Vec<String> {
    let mut names: Vec<String> = projects
        .projects
        .iter()
        .map(|p| projects.resolve(p))
        .filter(|p| !p.is_empty() && p != UNSORTED)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskCounts {
    pub open: usize,
    /// Open tasks marked as mine.
    pub mine_open: usize,
    pub done: usize,
}
pub fn count_tasks(tasks: &[TaskItem]) -> TaskCounts {
    let mut c = TaskCounts::default();
    for t in tasks {
        if t.done {
            c.done += 1;
        } else {
            c.open += 1;
            if t.mine == Some(true) {
                c.mine_open += 1;
            }
        }
    }
    c
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DoneFilter {
    #[default]
    Open,
    Done,
    All,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Who {
    #[default]
    Everyone,
    Mine,
}
#[derive(Clone, Debug, Default)]
pub struct TaskFilter {
    pub done: DoneFilter,
    pub who: Who,
    /// With `Who::Mine`: also show tasks nobody has been matched to.
    pub include_unassigned: bool,
    pub query: String,
}
impl TaskFilter {
    fn passes_done(&self, t: &TaskItem) -> bool {
        match self.done {
            DoneFilter::Open => !t.done,
            DoneFilter::Done => t.done,
            DoneFilter::All => true,
        }
    }
    /// Mine / Everyone and the search words; the Open / Done choice is applied separately.
    fn passes_rest(&self, t: &TaskItem) -> bool {
        let who = match self.who {
            Who::Everyone => true,
            Who::Mine => t.mine == Some(true) || (self.include_unassigned && t.mine.is_none()),
        };
        if !who {
            return false;
        }
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let hay = format!(
            "{}\n{}\n{}",
            t.task,
            t.owner.as_deref().unwrap_or(""),
            t.project
        )
        .to_lowercase();
        query.split_whitespace().all(|w| hay.contains(w))
    }
    pub fn passes(&self, t: &TaskItem) -> bool {
        self.passes_done(t) && self.passes_rest(t)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskGrouping {
    #[default]
    Project,
    Meeting,
    Due,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueBucket {
    Overdue,
    Today,
    ThisWeek,
    Later,
    NoDate,
    /// Finished tasks whose date has passed. They are not overdue.
    Earlier,
}
impl DueBucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::Today => "Today",
            Self::ThisWeek => "This week",
            Self::Later => "Later",
            Self::NoDate => "No date",
            Self::Earlier => "Earlier",
        }
    }
}
/// The Sunday that ends the week `today` is in.
fn week_end(today: NaiveDate) -> NaiveDate {
    today + Duration::days(6 - i64::from(today.weekday().num_days_from_monday()))
}
pub fn due_bucket(t: &TaskItem, today: NaiveDate) -> DueBucket {
    match t.due_date {
        None => DueBucket::NoDate,
        Some(d) if d < today => {
            if t.done {
                DueBucket::Earlier
            } else {
                DueBucket::Overdue
            }
        }
        Some(d) if d == today => DueBucket::Today,
        Some(d) if d <= week_end(today) => DueBucket::ThisWeek,
        Some(_) => DueBucket::Later,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupKind {
    Project(String),
    Meeting(String),
    Due(DueBucket),
}
#[derive(Clone, Debug)]
pub struct TaskGroup {
    pub kind: GroupKind,
    pub label: String,
    /// A meeting group's date.
    pub detail: Option<String>,
    /// Indexes into the task slice, in display order.
    pub rows: Vec<usize>,
    /// Open tasks in the group that pass the Mine and search choices, whatever Open / Done says.
    pub open: usize,
    /// Finished tasks counted the same way, for the group's progress.
    pub done: usize,
    /// Stable identity for collapsing: the project name, meeting id or bucket label.
    pub key: String,
}

pub fn group_tasks(
    tasks: &[TaskItem],
    filter: &TaskFilter,
    by: TaskGrouping,
    today: NaiveDate,
) -> Vec<TaskGroup> {
    let mut groups: Vec<TaskGroup> = Vec::new();
    let mut slot: HashMap<String, usize> = HashMap::new();
    for (i, t) in tasks.iter().enumerate() {
        if !filter.passes_rest(t) {
            continue;
        }
        let (key, kind, label, detail) = match by {
            TaskGrouping::Project => (
                t.project.clone(),
                GroupKind::Project(t.project.clone()),
                t.project.clone(),
                None,
            ),
            TaskGrouping::Meeting => (
                if t.manual {
                    MANUAL.into()
                } else {
                    t.meeting_id.clone()
                },
                GroupKind::Meeting(if t.manual {
                    MANUAL.into()
                } else {
                    t.meeting_id.clone()
                }),
                t.meeting_title.clone(),
                t.meeting_date.map(|d| d.format("%-d %b %Y").to_string()),
            ),
            TaskGrouping::Due => {
                let b = due_bucket(t, today);
                (
                    b.label().to_string(),
                    GroupKind::Due(b),
                    b.label().to_string(),
                    None,
                )
            }
        };
        let at = *slot.entry(key.clone()).or_insert_with(|| {
            groups.push(TaskGroup {
                kind,
                label,
                detail,
                rows: Vec::new(),
                open: 0,
                done: 0,
                key,
            });
            groups.len() - 1
        });
        if t.done {
            groups[at].done += 1;
        } else {
            groups[at].open += 1;
        }
        if filter.passes_done(t) {
            groups[at].rows.push(i);
        }
    }
    groups.retain(|g| !g.rows.is_empty());
    // Date first, tasks without one last.
    let due_key = |t: &TaskItem| (t.due_date.is_none(), t.due_date);
    for g in &mut groups {
        match by {
            // With Show All, finished tasks sit under the open ones.
            TaskGrouping::Project => g.rows.sort_by_key(|&i| {
                let t = &tasks[i];
                (t.done, mine_rank(t), due_key(t), t.meeting_rank, t.index)
            }),
            // Hand-made tasks share one group; they run by due date like a project.
            TaskGrouping::Meeting => g.rows.sort_by_key(|&i| {
                let t = &tasks[i];
                (t.done, t.manual.then(|| due_key(t)), t.index)
            }),
            TaskGrouping::Due => g.rows.sort_by_key(|&i| {
                let t = &tasks[i];
                (
                    t.done,
                    mine_rank(t),
                    due_key(t),
                    t.project.to_lowercase(),
                    t.meeting_rank,
                    t.index,
                )
            }),
        }
    }
    match by {
        TaskGrouping::Project => {
            groups.sort_by_key(|g| (g.label == UNSORTED, g.label.to_lowercase()))
        }
        TaskGrouping::Meeting => {
            groups.sort_by_key(|g| g.rows.first().map(|&i| tasks[i].meeting_rank))
        }
        TaskGrouping::Due => groups.sort_by_key(|g| match g.kind {
            GroupKind::Due(DueBucket::Overdue) => 0,
            GroupKind::Due(DueBucket::Today) => 1,
            GroupKind::Due(DueBucket::ThisWeek) => 2,
            GroupKind::Due(DueBucket::Later) => 3,
            GroupKind::Due(DueBucket::NoDate) => 4,
            _ => 5,
        }),
    }
    groups
}

/// Reads a due date typed by a person: `2026-11-12`, `12 Nov`, `12 Nov 2026`, `today`,
/// `tomorrow`, or a weekday (the next one after today).
pub fn parse_due(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "" => return None,
        "today" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        _ => {}
    }
    if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Some(d);
    }
    // "Sat 7 Nov", as the date buttons write it: the weekday is only a reading aid.
    if let Some((first, rest)) = s.split_once(' ')
        && first
            .trim_end_matches(',')
            .parse::<chrono::Weekday>()
            .is_ok()
        && !rest.trim().is_empty()
    {
        return parse_due(rest, today);
    }
    if let Ok(day) = s.parse::<chrono::Weekday>() {
        let ahead = (7 + day.num_days_from_monday() as i64
            - today.weekday().num_days_from_monday() as i64)
            % 7;
        return Some(today + Duration::days(if ahead == 0 { 7 } else { ahead }));
    }
    let with_year = |s: &str| {
        ["%d %b %Y", "%d %B %Y"]
            .iter()
            .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
    };
    with_year(&s).or_else(|| {
        // Without a year, a date well in the past means next year: "5 Jan" typed in December.
        let d = with_year(&format!("{s} {}", today.year()))?;
        if d < today - Duration::days(60) {
            with_year(&format!("{s} {}", today.year() + 1))
        } else {
            Some(d)
        }
    })
}

/// The Monday after the week `today` is in.
pub fn next_week(today: NaiveDate) -> NaiveDate {
    week_end(today) + Duration::days(1)
}

// ---- the page: one grouped list, then what is done ----------------------------------------

/// What the Tasks page shows, top to bottom. Every index points into the task slice and each
/// task appears at most once.
#[derive(Clone, Debug, Default)]
pub struct TaskView {
    /// The groups, in order. Group counts cover every task that passes Mine and search.
    pub groups: Vec<TaskGroup>,
    /// With Show Open: the finished tasks, for the folded Done section at the end.
    pub done: Vec<usize>,
}
impl TaskView {
    /// Every row in reading order, for the keyboard cursor. `done_open` and `collapsed` say
    /// which folded parts are open.
    pub fn rows(&self, done_open: bool, collapsed: &BTreeSet<String>) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for g in &self.groups {
            if !collapsed.contains(&g.key) {
                out.extend(&g.rows);
            }
        }
        if done_open {
            out.extend(&self.done);
        }
        out
    }
}

/// Mine first, then nobody's, then someone else's.
fn mine_rank(t: &TaskItem) -> u8 {
    match t.mine {
        Some(true) => 0,
        None => 1,
        Some(false) => 2,
    }
}

/// Lays out the page: the groups, mine first in each, and with Show Open a folded Done
/// section after them.
pub fn arrange(
    tasks: &[TaskItem],
    filter: &TaskFilter,
    by: TaskGrouping,
    today: NaiveDate,
) -> TaskView {
    let fold_done = filter.done == DoneFilter::Open;
    let mut view = TaskView {
        groups: group_tasks(tasks, filter, by, today),
        done: Vec::new(),
    };
    if fold_done {
        view.done = (0..tasks.len())
            .filter(|&i| tasks[i].done && filter.passes_rest(&tasks[i]))
            .collect();
        view.done
            .sort_by_key(|&i| (tasks[i].meeting_rank, tasks[i].index));
    }
    view
}

/// How urgent a due date reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Late,
    Today,
    Soon,
    Calm,
}

/// A short, relative due date: "2 days late", "Due today", "Due tomorrow", "Due Sunday",
/// "Due 20 Nov". Finished tasks say "Was due" instead of late.
pub fn due_phrase(t: &TaskItem, today: NaiveDate) -> Option<(String, Urgency)> {
    let raw = t.due.as_deref()?;
    let Some(d) = t.due_date else {
        return Some((format!("Due {raw}"), Urgency::Calm));
    };
    let days = (d - today).num_days();
    let short = if d.year() == today.year() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%-d %b %Y").to_string()
    };
    if t.done {
        let word = if days < 0 { "Was due" } else { "Due" };
        return Some((format!("{word} {short}"), Urgency::Calm));
    }
    Some(match days {
        ..=-2 => (format!("{} days late", -days), Urgency::Late),
        -1 => ("1 day late".to_string(), Urgency::Late),
        0 => ("Due today".to_string(), Urgency::Today),
        1 => ("Due tomorrow".to_string(), Urgency::Soon),
        2..=6 => (format!("Due {}", d.format("%A")), Urgency::Soon),
        _ => (format!("Due {short}"), Urgency::Calm),
    })
}

/// Where a to-do came from: the transcript lines around its timestamp and the decision that
/// most likely led to it.
#[derive(Clone, Default)]
pub struct TaskSource {
    /// Indexes into the meeting's lines, in order, and which one the timestamp names.
    pub lines: Vec<usize>,
    pub at: Option<usize>,
    pub decision: Option<super::Point>,
}

fn words(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .map(str::to_lowercase)
        .collect()
}

pub fn task_source(m: &Meeting, index: usize) -> TaskSource {
    let Some(todo) = m.summary.todo.get(index) else {
        return TaskSource::default();
    };
    let mut out = TaskSource::default();
    let stamp = todo.at.first();
    if let Some(at) = stamp.and_then(|s| m.target(s)) {
        let first = at.saturating_sub(1);
        let last = (at + 1).min(m.lines.len().saturating_sub(1));
        out.lines = (first..=last).collect();
        out.at = Some(at);
    }
    // A decision shares words with the task and was usually made a little before it was
    // assigned. Both count; a weak match shows nothing rather than a wrong decision.
    let task_words = words(&todo.task);
    let when = stamp.and_then(|s| super::seconds(s));
    let mut best: Option<(usize, &super::Point)> = None;
    for p in m.points().filter(|p| p.kind == "decided") {
        let shared = words(&p.text).intersection(&task_words).count();
        let near = match (when, p.at.first().and_then(|s| super::seconds(s))) {
            (Some(a), Some(b)) if a.abs_diff(b) <= 120 => 2,
            (Some(a), Some(b)) if a.abs_diff(b) <= 300 => 1,
            _ => 0,
        };
        let score = shared + near;
        if score >= 3 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, p));
        }
    }
    out.decision = best.map(|(_, p)| p.clone());
    out
}
