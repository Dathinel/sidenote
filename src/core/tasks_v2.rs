//! Tasks v2 resolution and edits. Source summaries remain owned by the notes engine.
use super::{Overrides, Subtask, TaskItem, TaskOverride, TaskStore, Todo, WaitingOn};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

pub fn needs_task_redo(meeting: &super::Meeting) -> bool {
    meeting.device.is_none()
        && (!meeting.summary.title.trim().is_empty() || !meeting.summary.todo.is_empty())
        && (meeting.summary.schema < 2
            || meeting
                .summary
                .todo
                .iter()
                .any(|t| t.id.is_empty() || t.title.is_empty()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Todo,
    Waiting,
    Review,
}
impl TaskStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "waiting" => Self::Waiting,
            "review" => Self::Review,
            _ => Self::Todo,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "To do",
            Self::Waiting => "Waiting on",
            Self::Review => "Review",
        }
    }
}
impl TaskItem {
    pub fn status_label(&self) -> &'static str {
        if self.done {
            "Done"
        } else {
            self.status.label()
        }
    }
    pub fn progress(&self) -> String {
        format!(
            "{}/{}",
            self.subtasks.iter().filter(|s| s.done).count(),
            self.subtasks.len()
        )
    }
}
pub fn task_tick_key(id: &str) -> String {
    format!("task:{id}")
}
pub fn subtask_tick_key(parent: &str, id: &str) -> String {
    // Length delimiters keep user and agent ids from aliasing each other.
    format!("sub:{}:{parent}:{}:{id}", parent.len(), id.len())
}
impl Overrides {
    pub fn match_task(
        &self,
        meeting: &str,
        id: &str,
        text: &str,
        index: usize,
    ) -> Option<&TaskOverride> {
        let entries = self.tasks.get(meeting)?;
        let guarded = |o: &&TaskOverride| o.task.as_deref() == Some(text);
        if !id.is_empty()
            && let Some(o) = entries.get(&format!("id:{id}")).filter(guarded)
        {
            return Some(o);
        }
        entries
            .get(&index.to_string())
            .filter(|o| o.task.as_deref().is_none_or(|t| t == text))
            .or_else(|| entries.values().find(|o| o.task.as_deref() == Some(text)))
    }
}
pub fn effective_subtasks(todo: &Todo, edit: Option<&TaskOverride>) -> Vec<Subtask> {
    let mut out = todo.subtasks.clone();
    if let Some(edit) = edit {
        for sub in &edit.added_subtasks {
            if !out.iter().any(|s| s.id == sub.id) {
                out.push(sub.clone());
            }
        }
        for sub in &mut out {
            let key = subtask_tick_key(&todo.id, &sub.id);
            if let Some(done) = edit.subtask_ticks.get(&key) {
                sub.done = *done;
            }
        }
    }
    out
}
pub fn resolve_waiting(tasks: &mut [TaskItem]) {
    let linked: BTreeMap<_, _> = tasks
        .iter()
        .filter(|t| !t.stable_id.is_empty())
        .map(|t| {
            (
                (t.meeting_id.clone(), t.stable_id.clone()),
                (t.done, t.title.clone()),
            )
        })
        .collect();
    for task in tasks {
        if task.status != TaskStatus::Waiting {
            continue;
        }
        if let Some(wait) = &task.waiting_on {
            let target = wait
                .task_id
                .as_ref()
                .and_then(|id| linked.get(&(task.meeting_id.clone(), id.clone())));
            task.waiting_label = target
                .map(|(_, title)| title.clone())
                .or_else(|| wait.text.clone())
                .or_else(|| {
                    wait.task_id
                        .as_ref()
                        .map(|_| "Task no longer in this meeting".into())
                });
            if task.status == TaskStatus::Waiting && target.is_some_and(|(done, _)| *done) {
                task.status = TaskStatus::Todo;
                task.waiting_label = None;
            }
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Teams {
    pub teams: Vec<Team>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Team {
    pub name: String,
    pub members: Vec<String>,
    /// Alias spelling to member name.
    pub aliases: BTreeMap<String, String>,
}
fn name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
impl Teams {
    pub fn resolve(&self, owner: Option<&str>, suggested: Option<&str>) -> String {
        if let Some(owner) = owner.map(name).filter(|n| !n.is_empty()) {
            let matches: Vec<_> = self
                .teams
                .iter()
                .filter(|t| {
                    t.members.iter().any(|m| name(m) == owner)
                        || t.aliases.iter().any(|(a, m)| {
                            name(a) == owner
                                && t.members.iter().any(|member| name(member) == name(m))
                        })
                })
                .collect();
            if matches.len() == 1 {
                return matches[0].name.clone();
            }
        }
        suggested
            .and_then(|s| self.teams.iter().find(|t| name(&t.name) == name(s)))
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "No team".into())
    }
    pub fn apply(&self, tasks: &mut [TaskItem]) {
        for t in tasks {
            t.team = self.resolve(t.owner.as_deref(), Some(&t.team));
        }
    }
}
#[derive(Clone, Debug)]
pub enum TaskV2Edit {
    Reset,
    Status(TaskStatus, Option<WaitingOn>),
    SubtaskTick { key: String, done: bool },
    AddSubtask(String),
}
impl TaskStore {
    pub fn load_teams(&self) -> io::Result<Teams> {
        let path = self.root.join("teams.json");
        match super::store_io::read_bounded(&path, super::file_reads::JOURNAL_MAX) {
            Ok(bytes) => {
                serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                    .map_err(io::Error::from)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Teams::default()),
            Err(e) => Err(e),
        }
    }
    pub fn save_teams(&self, teams: &Teams) -> io::Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        if teams
            .teams
            .iter()
            .any(|t| t.name.trim().is_empty() || !seen.insert(name(&t.name)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Use a different name for each team",
            ));
        }
        super::store_io::write(&self.root.join("teams.json"), teams)
    }
    pub fn load_grouping(&self) -> io::Result<super::TaskGrouping> {
        match super::store_io::read_bounded(&self.root.join("local/task-grouping.json"), 4096) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::from),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(super::TaskGrouping::Due),
            Err(e) => Err(e),
        }
    }
    pub fn save_grouping(&self, by: super::TaskGrouping) -> io::Result<()> {
        super::store_io::write(&self.root.join("local/task-grouping.json"), &by)
    }
    pub fn edit_v2(
        &self,
        task: &TaskItem,
        source: &std::path::Path,
        edit: TaskV2Edit,
    ) -> io::Result<super::EditView<Overrides>> {
        if task.manual {
            let added_id = if matches!(edit, TaskV2Edit::AddSubtask(_)) {
                super::sync_state::new_id(&self.root)?
            } else {
                String::new()
            };
            return super::sync_state::update_merged(
                &self.root,
                "task-overrides.json",
                |o: &mut Overrides| {
                    let manual = o
                        .manual
                        .iter_mut()
                        .find(|m| m.id == task.stable_id)
                        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Task removed"))?;
                    match &edit {
                        TaskV2Edit::Status(status, waiting) => {
                            manual.status = Some(*status);
                            manual.waiting_on = waiting.clone();
                        }
                        TaskV2Edit::SubtaskTick { key, done } => {
                            manual.subtask_ticks.insert(key.clone(), *done);
                        }
                        TaskV2Edit::AddSubtask(title) => {
                            let title = title.trim();
                            if title.is_empty() || title.chars().count() > 120 {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "Use 1 to 120 characters",
                                ));
                            }
                            manual.subtasks.push(Subtask {
                                id: added_id.clone(),
                                title: title.into(),
                                done: false,
                            });
                        }
                        TaskV2Edit::Reset => {
                            manual.status = None;
                            manual.waiting_on = None;
                            manual.subtasks.clear();
                            manual.subtask_ticks.clear();
                        }
                    }
                    Ok(())
                },
            );
        }
        let (meeting, index, expected_id, expected_text) = (
            task.meeting_id.as_str(),
            task.index,
            task.stable_id.as_str(),
            task.task.as_str(),
        );
        let path = source.join("english-summary.json");
        let raw = super::file_reads::read(&path, super::file_reads::SUMMARY_MAX)?;
        let summary: super::Summary = serde_json::from_slice(&raw)?;
        let todo = summary
            .todo
            .iter()
            .find(|t| !expected_id.is_empty() && t.id == expected_id)
            .or_else(|| summary.todo.iter().find(|t| t.text() == expected_text))
            .filter(|t| t.text() == expected_text)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, super::live_edit::MOVED))?;
        let key = if todo.id.is_empty() {
            index.to_string()
        } else {
            format!("id:{}", todo.id)
        };
        let added_id = if matches!(edit, TaskV2Edit::AddSubtask(_)) {
            super::sync_state::new_id(&self.root)?
        } else {
            String::new()
        };
        super::sync_state::update_merged_validated(
            &self.root,
            "task-overrides.json",
            |o: &mut Overrides| {
                let mut entry = o
                    .match_task(meeting, &todo.id, todo.text(), index)
                    .cloned()
                    .unwrap_or_default();
                entry.task = Some(todo.text().into());
                if matches!(edit, TaskV2Edit::Reset) {
                    if let Some(entries) = o.tasks.get_mut(meeting) {
                        let unique = summary
                            .todo
                            .iter()
                            .filter(|t| t.text() == todo.text())
                            .count()
                            == 1;
                        entries.retain(|k, v| {
                            k != &key
                                && !(v.task.as_deref() == Some(todo.text())
                                    && (unique || k == &index.to_string()))
                        });
                    }
                    return Ok(());
                }
                match &edit {
                    TaskV2Edit::Reset => unreachable!(),
                    TaskV2Edit::Status(status, waiting) => {
                        entry.status = Some(*status);
                        entry.waiting_on = waiting.clone();
                    }
                    TaskV2Edit::SubtaskTick { key, done } => {
                        entry.subtask_ticks.insert(key.clone(), *done);
                    }
                    TaskV2Edit::AddSubtask(title) => {
                        let title = title.trim();
                        if title.is_empty() || title.chars().count() > 120 {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "Use 1 to 120 characters",
                            ));
                        }
                        entry.added_subtasks.push(Subtask {
                            id: added_id.clone(),
                            title: title.into(),
                            done: false,
                        });
                    }
                }
                o.tasks
                    .entry(meeting.into())
                    .or_default()
                    .insert(key.clone(), entry);
                Ok(())
            },
            || {
                if super::file_reads::read(&path, super::file_reads::SUMMARY_MAX)? != raw {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        super::live_edit::MOVED,
                    ));
                }
                Ok(())
            },
        )
    }
}

pub fn todo_done(todo: &Todo, checked: Option<&std::collections::BTreeSet<String>>) -> bool {
    let has = |key: &str| checked.is_some_and(|c| c.contains(key));
    has(todo.text())
        || (!todo.id.is_empty() && has(&task_tick_key(&todo.id)))
        || (todo.status == "done" && !has(&format!("open:{}", todo.id)))
}
impl super::StateStore {
    pub fn set_task_tick_view(
        &self,
        meeting: &str,
        text: &str,
        stable: &str,
        on: bool,
    ) -> io::Result<super::EditView<super::Saved>> {
        if stable.is_empty() {
            return self.set_tick_view(meeting, text, on);
        }
        // Validate the meeting through the existing tick API before the merged write.
        if !meeting.split_once('-').is_some_and(|(a, b)| {
            !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid meeting id",
            ));
        }
        super::sync_state::update_merged(
            &self.root,
            &format!("{meeting}.json"),
            |s: &mut super::Saved| {
                for key in [text.to_string(), task_tick_key(stable)] {
                    if on {
                        s.checked.insert(key);
                    } else {
                        s.checked.remove(&key);
                    }
                }
                if on {
                    s.checked.remove(&format!("open:{stable}"));
                } else {
                    s.checked.insert(format!("open:{stable}"));
                }
                Ok(())
            },
        )
    }
}

/// Match redo identities before comparing text. Ambiguous text never transfers an edit.
pub fn redo_matches(old: &[Todo], new: &[Todo]) -> Vec<Option<usize>> {
    let mut used = std::collections::BTreeSet::new();
    new.iter()
        .map(|next| {
            let exact = old
                .iter()
                .position(|t| !next.id.is_empty() && t.id == next.id);
            let text: Vec<_> = old
                .iter()
                .enumerate()
                .filter(|(_, t)| t.text() == next.text())
                .map(|(i, _)| i)
                .collect();
            let matched = exact.or_else(|| (text.len() == 1).then(|| text[0]));
            matched.filter(|i| used.insert(*i))
        })
        .collect()
}
impl TaskStore {
    pub fn preserve_redo(&self, meeting: &str, old: &[Todo], new: &[Todo]) -> io::Result<()> {
        let matches = redo_matches(old, new);
        let ids: BTreeMap<_, _> = new
            .iter()
            .zip(&matches)
            .filter_map(|(next, matched)| matched.map(|i| (old[i].id.clone(), next.id.clone())))
            .filter(|(id, _)| !id.is_empty())
            .collect();
        self.update_overrides(|overrides| {
            let before = overrides.clone();
            for (next, matched) in new.iter().zip(&matches) {
                let Some(i) = matched else { continue };
                if let Some(mut edit) = before
                    .match_task(meeting, &old[*i].id, old[*i].text(), *i)
                    .cloned()
                {
                    edit.task = Some(next.text().into());
                    if let Some(waiting) = edit.waiting_on.as_mut()
                        && let Some(id) = waiting.task_id.as_mut()
                        && let Some(next) = ids.get(id)
                    {
                        *id = next.clone();
                    }
                    for sub in &next.subtasks {
                        if let Some(previous) = old[*i]
                            .subtasks
                            .iter()
                            .find(|s| s.id == sub.id)
                            .or_else(|| old[*i].subtasks.iter().find(|s| s.title == sub.title))
                            && let Some(done) = edit
                                .subtask_ticks
                                .get(&subtask_tick_key(&old[*i].id, &previous.id))
                                .copied()
                        {
                            edit.subtask_ticks
                                .insert(subtask_tick_key(&next.id, &sub.id), done);
                        }
                    }
                    // Added sub-tasks carry user ids and also need their parent key remapped.
                    for sub in &edit.added_subtasks {
                        if let Some(done) = edit
                            .subtask_ticks
                            .get(&subtask_tick_key(&old[*i].id, &sub.id))
                            .copied()
                        {
                            edit.subtask_ticks
                                .insert(subtask_tick_key(&next.id, &sub.id), done);
                        }
                    }
                    overrides
                        .tasks
                        .entry(meeting.into())
                        .or_default()
                        .insert(format!("id:{}", next.id), edit);
                }
            }
        })?;
        let store = super::StateStore::new(self.root.clone());
        let before = store.load(meeting)?;
        store.update(meeting, |saved| {
            for (next, matched) in new.iter().zip(&matches) {
                let Some(i) = matched else { continue };
                if todo_done(&old[*i], Some(&before.checked)) {
                    saved.checked.insert(next.text().into());
                    saved.checked.insert(task_tick_key(&next.id));
                } else if before.checked.contains(&format!("open:{}", old[*i].id)) {
                    saved.checked.insert(format!("open:{}", next.id));
                }
            }
        })?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{self, TaskGrouping};
    use serde_json::json;
    use std::{
        collections::HashMap,
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        root: std::path::PathBuf,
        meeting: core::Meeting,
    }
    impl Fixture {
        fn new(todo: serde_json::Value) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tv2-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let folder = root.join("meetings/1800000000000-1");
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("english-summary.json"),
                json!({"title":"Sample planning","todo":todo}).to_string(),
            )
            .unwrap();
            let meeting = core::Meeting::load(&folder).unwrap();
            Self { root, meeting }
        }
        fn store(&self) -> TaskStore {
            TaskStore::new(self.root.clone())
        }
        fn tasks(&self, overrides: &Overrides) -> Vec<TaskItem> {
            core::build_tasks(
                std::slice::from_ref(&self.meeting),
                &HashMap::new(),
                &core::Projects::default(),
                overrides,
                &HashMap::new(),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn legacy_and_title_only_tasks_display_and_search() {
        let f = Fixture::new(
            json!([{"task":"Send the letter"},{"id":"b","title":"Check the sample","description":"Use paper","subtasks":[{"id":"s","title":"Read the proof"}]}]),
        );
        let tasks = f.tasks(&Overrides::default());
        assert_eq!(tasks[0].title, "Send the letter");
        assert_eq!(tasks[1].task, "Check the sample");
        let texts = f.meeting.summary.texts().collect::<Vec<_>>();
        assert!(texts.contains(&"Use paper"));
        assert!(texts.contains(&"Read the proof"));
    }
    #[test]
    fn status_override_is_guarded_by_text_and_survives_empty_check() {
        let f = Fixture::new(json!([{"id":"a","title":"Review the copy","status":"waiting"}]));
        let task = f.tasks(&Overrides::default()).remove(0);
        let view = f
            .store()
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Review, None),
            )
            .unwrap();
        assert_eq!(f.tasks(&view.value)[0].status, TaskStatus::Review);
        let mut wrong = view.value;
        wrong
            .tasks
            .get_mut(&f.meeting.id)
            .unwrap()
            .get_mut("id:a")
            .unwrap()
            .task = Some("Other task".into());
        assert_eq!(f.tasks(&wrong)[0].status, TaskStatus::Waiting);
        assert!(
            !TaskOverride {
                status: Some(TaskStatus::Todo),
                ..Default::default()
            }
            .is_empty()
        );
        assert!(serde_json::from_value::<TaskOverride>(json!({"status":"done"})).is_err());
    }
    #[test]
    fn waiting_auto_todo_is_read_only_and_scoped_to_meeting() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send sample","status":"waiting","waiting_on":{"task_id":"b"}},{"id":"b","title":"Approve sample"}]),
        );
        let checked = HashMap::from([(
            f.meeting.id.clone(),
            std::collections::BTreeSet::from(["Approve sample".into()]),
        )]);
        let tasks = core::build_tasks(
            std::slice::from_ref(&f.meeting),
            &HashMap::new(),
            &core::Projects::default(),
            &Overrides::default(),
            &checked,
        );
        assert_eq!(tasks[0].status, TaskStatus::Todo);
        assert!(!tasks[0].done);
        assert_eq!(f.meeting.summary.todo[0].status, "waiting");
        let mut tasks = f.tasks(&Overrides::default());
        tasks[1].meeting_id = "other".into();
        tasks[1].done = true;
        resolve_waiting(&mut tasks);
        assert_eq!(tasks[0].status, TaskStatus::Waiting);
    }
    #[test]
    fn waiting_person_and_missing_task_are_plain_text() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send sample","status":"waiting","waiting_on":{"text":"Alex's reply"}},{"id":"b","title":"Check copy","status":"waiting","waiting_on":{"task_id":"gone"}}]),
        );
        let tasks = f.tasks(&Overrides::default());
        assert_eq!(tasks[0].waiting_label.as_deref(), Some("Alex's reply"));
        assert_eq!(
            tasks[1].waiting_label.as_deref(),
            Some("Task no longer in this meeting")
        );
    }
    #[test]
    fn subtask_ticks_and_additions_persist_without_closing_parent() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send sample","subtasks":[{"id":"s","title":"Read the proof"}]}]),
        );
        let task = f.tasks(&Overrides::default()).remove(0);
        let store = f.store();
        store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::SubtaskTick {
                    key: subtask_tick_key("a", "s"),
                    done: true,
                },
            )
            .unwrap();
        let view = store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::AddSubtask("Print the proof".into()),
            )
            .unwrap();
        let tasks = f.tasks(&view.value);
        assert_eq!(tasks[0].progress(), "1/2");
        assert!(!tasks[0].done);
        let id = &tasks[0].subtasks[1].id;
        let view = store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::SubtaskTick {
                    key: subtask_tick_key("a", id),
                    done: true,
                },
            )
            .unwrap();
        assert_eq!(f.tasks(&view.value)[0].progress(), "2/2");
        assert!(!f.tasks(&view.value)[0].done);
        assert_ne!(subtask_tick_key("a", "bc"), subtask_tick_key("ab", "c"));
        assert_ne!(subtask_tick_key("a", "s"), task_tick_key("a"));
    }
    #[test]
    fn teams_resolve_owner_alias_then_defined_suggestion() {
        let teams = Teams {
            teams: vec![
                Team {
                    name: "Design".into(),
                    members: vec!["Alex Chen".into()],
                    aliases: BTreeMap::from([("Alex C".into(), "Alex Chen".into())]),
                },
                Team {
                    name: "Delivery".into(),
                    ..Default::default()
                },
            ],
        };
        assert_eq!(teams.resolve(Some(" alex  c "), Some("Delivery")), "Design");
        assert_eq!(teams.resolve(Some("Someone"), Some("delivery")), "Delivery");
        assert_eq!(teams.resolve(None, Some("Imaginary")), "No team");
        assert_eq!(teams.resolve(None, None), "No team");
    }
    #[test]
    fn each_grouping_keeps_subtasks_with_parent() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send sample","owner":"Alex","team":"Design","status":"review","subtasks":[{"id":"s","title":"Read proof"}]},{"id":"b","title":"Read letter","owner":"Morgan","status":"waiting"}]),
        );
        let tasks = f.tasks(&Overrides::default());
        for grouping in [
            TaskGrouping::Project,
            TaskGrouping::Team,
            TaskGrouping::Status,
            TaskGrouping::Owner,
            TaskGrouping::Due,
        ] {
            let groups = core::group_tasks(
                &tasks,
                &core::TaskFilter::default(),
                grouping,
                chrono::NaiveDate::MIN,
            );
            assert_eq!(groups.iter().map(|g| g.rows.len()).sum::<usize>(), 2);
            assert_eq!(tasks[0].subtasks.len(), 1);
        }
        let status = core::group_tasks(
            &tasks,
            &core::TaskFilter::default(),
            TaskGrouping::Status,
            chrono::NaiveDate::MIN,
        );
        assert_eq!(
            status.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(),
            ["Waiting on", "Review"]
        );
    }
    #[test]
    fn grouping_is_device_local_and_round_trips() {
        let f = Fixture::new(json!([]));
        let store = f.store();
        store.save_grouping(TaskGrouping::Team).unwrap();
        assert_eq!(store.load_grouping().unwrap(), TaskGrouping::Team);
        assert!(f.root.join("local/task-grouping.json").is_file());
        assert!(!f.root.join("tasks-settings.json").exists());
    }
    #[test]
    fn redo_matches_id_first_text_second_without_ambiguous_transfer() {
        let todo = |id: &str, text: &str| Todo {
            id: id.into(),
            task: text.into(),
            ..Default::default()
        };
        let old = vec![
            todo("a", "Old name"),
            todo("b", "Same text"),
            todo("c", "Duplicate"),
            todo("d", "Duplicate"),
        ];
        let new = vec![
            todo("x", "Same text"),
            todo("a", "New name"),
            todo("z", "Duplicate"),
        ];
        assert_eq!(redo_matches(&old, &new), vec![Some(1), Some(0), None]);
    }
    #[test]
    fn redo_preserves_status_subtasks_properties_and_parent_tick() {
        let f = Fixture::new(
            json!([{"id":"a","task":"Old title","subtasks":[{"id":"s","title":"Read proof"}]}]),
        );
        let store = f.store();
        let task = f.tasks(&Overrides::default()).remove(0);
        store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Review, None),
            )
            .unwrap();
        store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::SubtaskTick {
                    key: subtask_tick_key("a", "s"),
                    done: true,
                },
            )
            .unwrap();
        core::StateStore::new(f.root.clone())
            .set_task_tick_view(&f.meeting.id, "Old title", "a", true)
            .unwrap();
        let mut next = f.meeting.summary.todo.clone();
        next[0].task = "New title".into();
        store
            .preserve_redo(&f.meeting.id, &f.meeting.summary.todo, &next)
            .unwrap();
        let overrides = store.load_overrides().unwrap();
        let edit = overrides
            .match_task(&f.meeting.id, "a", "New title", 0)
            .unwrap();
        assert_eq!(edit.status, Some(TaskStatus::Review));
        assert!(edit.subtask_ticks[&subtask_tick_key("a", "s")]);
        assert!(store.load_ticks(&f.meeting.id).contains("New title"));
    }
    #[test]
    fn done_uses_existing_ticks_and_can_reopen_agent_done() {
        let f = Fixture::new(json!([{"id":"a","title":"Read proof","status":"done"}]));
        let todo = &f.meeting.summary.todo[0];
        assert!(todo_done(todo, None));
        let store = core::StateStore::new(f.root.clone());
        let view = store
            .set_task_tick_view(&f.meeting.id, todo.text(), "a", false)
            .unwrap();
        assert!(!todo_done(todo, Some(&view.value.checked)));
        let view = store
            .set_task_tick_view(&f.meeting.id, todo.text(), "a", true)
            .unwrap();
        assert!(todo_done(todo, Some(&view.value.checked)));
    }
    #[test]
    fn source_change_rejects_stale_edit() {
        let f = Fixture::new(json!([{"id":"a","title":"Read proof"}]));
        let task = f.tasks(&Overrides::default()).remove(0);
        fs::write(
            f.meeting.folder.join("english-summary.json"),
            json!({"todo":[{"id":"a","title":"Different work"}]}).to_string(),
        )
        .unwrap();
        assert!(
            f.store()
                .edit_v2(
                    &task,
                    &f.meeting.folder,
                    TaskV2Edit::Status(TaskStatus::Review, None)
                )
                .is_err()
        );
    }
    #[test]
    fn manual_tasks_have_status_and_independent_subtasks() {
        let f = Fixture::new(json!([]));
        let store = f.store();
        let (overrides, _) = store
            .add_manual(core::ManualTask {
                task: "Print the cards".into(),
                ..Default::default()
            })
            .unwrap();
        let task = f.tasks(&overrides).remove(0);
        store
            .edit_v2(&task, &f.root, TaskV2Edit::Status(TaskStatus::Review, None))
            .unwrap();
        let view = store
            .edit_v2(
                &task,
                &f.root,
                TaskV2Edit::AddSubtask("Read the proof".into()),
            )
            .unwrap();
        let tasks = f.tasks(&view.value);
        assert_eq!(tasks[0].status, TaskStatus::Review);
        let view = store
            .edit_v2(
                &task,
                &f.root,
                TaskV2Edit::SubtaskTick {
                    key: subtask_tick_key(&task.stable_id, &tasks[0].subtasks[0].id),
                    done: true,
                },
            )
            .unwrap();
        assert_eq!(f.tasks(&view.value)[0].progress(), "1/1");
        assert!(!f.tasks(&view.value)[0].done);
    }
    #[test]
    fn reset_removes_stable_and_legacy_edits_and_todo_hides_waiting_reason() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Read proof","status":"waiting","waiting_on":{"text":"A reply"}}]),
        );
        let store = f.store();
        let task = f.tasks(&Overrides::default()).remove(0);
        store
            .update_override(&f.meeting.id, 0, task.task.as_str(), |o| {
                o.owner = Some("Alex".into())
            })
            .unwrap();
        let view = store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Todo, None),
            )
            .unwrap();
        assert!(f.tasks(&view.value)[0].waiting_label.is_none());
        let view = store
            .edit_v2(&task, &f.meeting.folder, TaskV2Edit::Reset)
            .unwrap();
        assert!(
            view.value
                .match_task(&f.meeting.id, "a", &task.task, 0)
                .is_none()
        );
    }
    #[test]
    fn existing_property_edits_share_the_stable_status_entry() {
        let f = Fixture::new(json!([{"id":"a","title":"Read proof"}]));
        let store = f.store();
        let task = f.tasks(&Overrides::default()).remove(0);
        store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Review, None),
            )
            .unwrap();
        let overrides = store
            .update_override(&f.meeting.id, 0, &task.task, |o| {
                o.due = Some("2026-11-12".into())
            })
            .unwrap();
        let tasks = f.tasks(&overrides);
        assert_eq!(tasks[0].status, TaskStatus::Review);
        assert_eq!(tasks[0].due.as_deref(), Some("2026-11-12"));
    }
    #[test]
    fn redo_remaps_a_user_waiting_link_when_task_ids_change() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send proof"},{"id":"b","title":"Approve proof"}]),
        );
        let store = f.store();
        let task = f.tasks(&Overrides::default()).remove(0);
        store
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(
                    TaskStatus::Waiting,
                    Some(WaitingOn {
                        task_id: Some("b".into()),
                        text: None,
                    }),
                ),
            )
            .unwrap();
        let mut next = f.meeting.summary.todo.clone();
        next[0].id = "aa".into();
        next[1].id = "bb".into();
        store
            .preserve_redo(&f.meeting.id, &f.meeting.summary.todo, &next)
            .unwrap();
        let overrides = store.load_overrides().unwrap();
        assert_eq!(
            overrides
                .match_task(&f.meeting.id, "aa", "Send proof", 0)
                .unwrap()
                .waiting_on
                .as_ref()
                .unwrap()
                .task_id
                .as_deref(),
            Some("bb")
        );
    }
    #[test]
    fn resetting_a_task_does_not_clear_another_with_the_same_title() {
        let f =
            Fixture::new(json!([{"id":"a","title":"Read proof"},{"id":"b","title":"Read proof"}]));
        let store = f.store();
        let tasks = f.tasks(&Overrides::default());
        store
            .edit_v2(
                &tasks[0],
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Review, None),
            )
            .unwrap();
        store
            .edit_v2(
                &tasks[1],
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Waiting, None),
            )
            .unwrap();
        let view = store
            .edit_v2(&tasks[0], &f.meeting.folder, TaskV2Edit::Reset)
            .unwrap();
        assert_eq!(f.tasks(&view.value)[1].status, TaskStatus::Waiting);
    }
    #[test]
    fn older_meeting_without_tasks_can_still_request_redo() {
        let mut f = Fixture::new(json!([]));
        assert!(needs_task_redo(&f.meeting));
        f.meeting.summary.schema = 2;
        assert!(!needs_task_redo(&f.meeting));
        f.meeting.summary.schema = 0;
        f.meeting.device = Some("Other PC".into());
        assert!(!needs_task_redo(&f.meeting));
    }
    #[test]
    fn a_released_waiting_task_can_wait_on_a_new_blocker() {
        let f = Fixture::new(
            json!([{"id":"a","title":"Send proof","status":"waiting","waiting_on":{"task_id":"b"}},{"id":"b","title":"Approve proof","status":"done"}]),
        );
        let task = f.tasks(&Overrides::default()).remove(0);
        assert_eq!(task.status, TaskStatus::Todo);
        let view = f
            .store()
            .edit_v2(
                &task,
                &f.meeting.folder,
                TaskV2Edit::Status(TaskStatus::Waiting, None),
            )
            .unwrap();
        let next = f.tasks(&view.value);
        assert_eq!(next[0].status, TaskStatus::Waiting);
        assert!(next[0].waiting_on.is_none());
    }
}
