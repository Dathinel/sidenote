use chrono::{Datelike, Days, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Team = crate::core::Team;

#[derive(Deserialize, Serialize)]
pub(super) struct Summary {
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub people: Vec<String>,
    pub context: Vec<String>,
    pub topics: Vec<Topic>,
    pub todo: Vec<Todo>,
    pub to_settle: Vec<Unsettled>,
    #[serde(default)]
    pub verify: Vec<String>,
    #[serde(default)]
    pub schema: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_by: Option<serde_json::Value>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Topic {
    pub title: String,
    pub points: Vec<Point>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Point {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub who: Option<String>,
    #[serde(default)]
    pub at: Vec<String>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Unsettled {
    pub question: String,
    pub waits_on: Option<String>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Todo {
    pub title: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    pub waiting_on: Option<Waiting>,
    #[serde(default)]
    pub subtasks: Vec<Subtask>,
    pub team: Option<String>,
    pub owner: Option<String>,
    pub due: Option<String>,
    #[serde(default)]
    pub due_default: bool,
    #[serde(default)]
    pub at: Vec<String>,
    #[serde(default)]
    pub span: Vec<String>,
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "ref", skip_serializing)]
    pub reference: String,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Subtask {
    pub title: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub done: bool,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Waiting {
    #[serde(default, rename = "ref", skip_serializing)]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Reasons are static: helper output and meeting text must never reach state or logs.
pub(super) fn object<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, &'static str> {
    let start = raw.find('{').ok_or("not a JSON object")?;
    let end = raw
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or("not a JSON object")?;
    serde_json::from_str(&raw[start..=end]).map_err(|_| "missing field or wrong field type")
}

pub(super) fn title(text: &str, cap: usize) -> String {
    let mut out = String::new();
    for word in text.split_whitespace().take(8) {
        if out.chars().count() + usize::from(!out.is_empty()) + word.chars().count() > cap {
            if out.is_empty() {
                out.extend(word.chars().take(cap));
            }
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}
fn hash(parts: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_hex()[..12].into()
}
fn next_id(base: String, seen: &mut BTreeSet<String>) -> String {
    let mut id = base.clone();
    let mut suffix = 2;
    while !seen.insert(id.clone()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}
fn default_due(mut date: NaiveDate) -> String {
    let mut days = 0;
    while days < 3 {
        let Some(next) = date.checked_add_days(Days::new(1)) else {
            break;
        };
        date = next;
        if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            days += 1;
        }
    }
    date.format("%Y-%m-%d").to_string()
}
pub(super) fn timestamps(transcript: &str) -> BTreeSet<String> {
    let mut stamps = BTreeSet::new();
    for line in crate::core::parse_transcript(transcript) {
        if !line.at.is_empty() {
            stamps.insert(line.at);
        }
    }
    for line in transcript.lines() {
        if let Some(stamp) = line
            .strip_prefix('[')
            .and_then(|s| s.split_once(']'))
            .map(|(s, _)| s)
        {
            stamps.insert(stamp.into());
        }
    }
    stamps
}
fn repair_stamps(stamps: &mut Vec<String>, valid: &BTreeSet<String>, cap: usize) {
    stamps.retain(|s| valid.contains(s));
    stamps.truncate(cap);
}
pub(super) fn parse(
    raw: &str,
    meeting_id: &str,
    date: NaiveDate,
    transcript: &str,
    teams: &[Team],
) -> Result<Summary, &'static str> {
    let mut notes: Summary = object(raw)?;
    notes.title = title(&notes.title, 100);
    if notes.title.is_empty() {
        return Err("empty title");
    }
    let valid = timestamps(transcript);
    for topic in &mut notes.topics {
        for point in &mut topic.points {
            repair_stamps(&mut point.at, &valid, 2);
            if !["fact", "decided", "concern", "response", "open"].contains(&point.kind.as_str()) {
                point.kind = "fact".into();
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut references = BTreeMap::new();
    for todo in &mut notes.todo {
        todo.title = title(&todo.title, 60);
        if todo.title.is_empty() {
            return Err("empty task title");
        }
        todo.task = todo.title.clone();
        if !["todo", "waiting"].contains(&todo.status.as_str()) {
            todo.status = "todo".into();
        }
        repair_stamps(&mut todo.at, &valid, 1);
        repair_stamps(&mut todo.span, &valid, 2);
        if todo.span.len() != 2 {
            todo.span.clear();
        }
        let normalized = todo
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let stamp = todo
            .at
            .first()
            .map(|s| s.split('.').next().unwrap_or(s))
            .unwrap_or("");
        todo.id = next_id(hash(&[meeting_id, &normalized, stamp]), &mut seen);
        if !todo.reference.is_empty() {
            references
                .entry(todo.reference.clone())
                .or_insert_with(|| todo.id.clone());
        }
        todo.subtasks.truncate(7);
        let mut child_ids = BTreeSet::new();
        for child in &mut todo.subtasks {
            child.title = title(&child.title, 60);
            child.id = next_id(
                hash(&[&todo.id, &child.title.to_lowercase()]),
                &mut child_ids,
            );
            child.done = false;
        }
        let resolved = crate::core::Teams {
            teams: teams.to_vec(),
        }
        .resolve(todo.owner.as_deref(), todo.team.as_deref());
        todo.team = (resolved != "No team").then_some(resolved);
        if todo
            .due
            .as_ref()
            .is_none_or(|d| d.len() != 10 || NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err())
        {
            todo.due = Some(default_due(date));
            todo.due_default = true;
        } else {
            todo.due_default = false;
        }
    }
    for todo in &mut notes.todo {
        if let Some(waiting) = &mut todo.waiting_on {
            if let Some(reference) = waiting.reference.take() {
                if let Some(id) = references.get(&reference).filter(|id| **id != todo.id) {
                    waiting.task_id = Some(id.clone());
                    waiting.text = None;
                } else {
                    waiting.task_id = None;
                    waiting.text = Some(reference);
                }
            } else if waiting
                .task_id
                .as_ref()
                .is_some_and(|id| !seen.contains(id) || *id == todo.id)
            {
                waiting.text = waiting.task_id.take();
            }
            if waiting.task_id.is_none()
                && waiting.text.as_ref().is_none_or(|s| s.trim().is_empty())
            {
                todo.waiting_on = None;
            }
        }
    }
    notes.schema = 2;
    notes.made_by = None;
    Ok(notes)
}

#[derive(Deserialize)]
struct Sorting {
    meeting_project: Option<String>,
    tasks: Vec<SortTag>,
}
#[derive(Deserialize)]
struct SortTag {
    i: usize,
    project: String,
    #[serde(rename = "new")]
    _new: bool,
}
pub(super) fn sorting(
    raw: &str,
    summary: &Summary,
    projects: &[String],
    your_name: &str,
) -> Result<crate::core::TasksFile, &'static str> {
    let sorted: Sorting = object(raw)?;
    let mut tags = BTreeMap::new();
    for tag in sorted.tasks {
        if tag.i >= summary.todo.len() || tags.insert(tag.i, tag).is_some() {
            return Err("invalid task index");
        }
    }
    let fallback = sorted
        .meeting_project
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(crate::core::UNSORTED);
    let tasks = summary
        .todo
        .iter()
        .enumerate()
        .map(|(i, todo)| {
            let tag = tags.get(&i);
            let project = tag
                .map(|t| t.project.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(fallback)
                .to_string();
            let new = project != crate::core::UNSORTED && !projects.contains(&project);
            crate::core::TaskTag {
                i,
                project,
                new,
                mine: Some(todo.owner.as_deref().is_some_and(|owner| {
                    owner == "You" || (!your_name.is_empty() && owner == your_name)
                })),
            }
        })
        .collect();
    Ok(crate::core::TasksFile {
        meeting_project: sorted.meeting_project,
        tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const TRANSCRIPT: &str =
        "[00:00:01.000] You (English): Send the report.\n[00:00:02.000] Them (English): Yes.\n";
    pub(super) fn fixture() -> serde_json::Value {
        serde_json::json!({"title":"Plan the launch", "context":[], "topics":[], "todo":[{"ref":"t1","title":"Send the report","description":"Share the final figures.","owner":"You","due":null,"at":["00:00:01"]}], "to_settle":[]})
    }
    fn check(v: &serde_json::Value) -> Summary {
        parse(
            &v.to_string(),
            "123-456",
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            TRANSCRIPT,
            &[],
        )
        .unwrap()
    }
    #[test]
    fn helper_can_only_create_todo_or_waiting_tasks() {
        for status in ["todo", "waiting", "done", "review", "", "unknown"] {
            let mut v = fixture();
            v["todo"][0]["status"] = serde_json::json!(status);
            assert_eq!(
                check(&v).todo[0].status,
                if status == "waiting" {
                    "waiting"
                } else {
                    "todo"
                }
            );
        }
    }
    #[test]
    fn valid_fenced_output_is_backward_compatible() {
        let v = fixture();
        let notes = parse(
            &format!("answer\n```json\n{v}\n```"),
            "123-456",
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            TRANSCRIPT,
            &[],
        )
        .unwrap();
        let old: crate::core::Summary =
            serde_json::from_value(serde_json::to_value(&notes).unwrap()).unwrap();
        assert_eq!(old.todo[0].task, "Send the report");
        assert_eq!(notes.schema, 2);
    }
    #[test]
    fn missing_required_fields_and_wrong_types_are_hard_failures() {
        for key in ["title", "context", "topics", "todo", "to_settle"] {
            let mut v = fixture();
            v.as_object_mut().unwrap().remove(key);
            assert!(
                parse(&v.to_string(), "1-2", NaiveDate::MIN, TRANSCRIPT, &[]).is_err(),
                "{key}"
            );
            let mut v = fixture();
            v[key] = serde_json::json!(42);
            assert!(
                parse(&v.to_string(), "1-2", NaiveDate::MIN, TRANSCRIPT, &[]).is_err(),
                "{key}"
            );
        }
        let mut v = fixture();
        v["title"] = serde_json::json!("  ");
        assert!(parse(&v.to_string(), "1-2", NaiveDate::MIN, TRANSCRIPT, &[]).is_err());
    }
    #[test]
    fn repairs_status_team_dates_timestamps_and_subtasks() {
        let mut v = fixture();
        let todo = &mut v["todo"][0];
        todo["status"] = serde_json::json!("nonsense");
        todo["team"] = serde_json::json!("Unknown");
        todo["due"] = serde_json::json!("2026-02-30");
        todo["at"] = serde_json::json!(["23:59:59", "00:00:01"]);
        todo["subtasks"] = serde_json::json!(
            (0..9)
                .map(|_| serde_json::json!({"title":"Check the figures", "done":true}))
                .collect::<Vec<_>>()
        );
        todo["title"] = serde_json::json!(
            "Send the extremely comprehensive finalized consolidated detailed financial reconciliation report"
        );
        let n = check(&v);
        let t = &n.todo[0];
        assert_eq!(t.status, "todo");
        assert!(t.team.is_none());
        assert_eq!(t.due.as_deref(), Some("2026-11-11"));
        assert!(t.due_default);
        assert_eq!(t.at, ["00:00:01"]);
        assert_eq!(t.subtasks.len(), 7);
        assert!(t.subtasks.iter().all(|s| !s.done));
        assert!(t.title.chars().count() <= 60);
        assert_eq!(
            t.title,
            "Send the extremely comprehensive finalized consolidated"
        );
    }
    #[test]
    fn stable_ids_collisions_and_blocker_refs() {
        let mut v = fixture();
        let mut second = v["todo"][0].clone();
        second["ref"] = serde_json::json!("t2");
        second["waiting_on"] = serde_json::json!({"ref":"t1"});
        v["todo"].as_array_mut().unwrap().push(second);
        let a = check(&v);
        let b = check(&v);
        assert_eq!(a.todo[0].id, b.todo[0].id);
        assert_eq!(a.todo[1].id, format!("{}-2", a.todo[0].id));
        assert_eq!(
            a.todo[1].waiting_on.as_ref().unwrap().task_id.as_ref(),
            Some(&a.todo[0].id)
        );
        v["todo"][0]["waiting_on"] = serde_json::json!({"ref":"missing"});
        assert_eq!(
            check(&v).todo[0]
                .waiting_on
                .as_ref()
                .unwrap()
                .text
                .as_deref(),
            Some("missing")
        );
    }
    #[test]
    fn team_owner_match_wins_only_when_unambiguous() {
        let v = fixture();
        let teams = vec![Team {
            name: "Delivery".into(),
            members: vec!["You".into()],
            ..Default::default()
        }];
        let n = parse(&v.to_string(), "1-2", NaiveDate::MIN, TRANSCRIPT, &teams).unwrap();
        assert_eq!(n.todo[0].team.as_deref(), Some("Delivery"));
    }
    #[test]
    fn sorting_fills_missing_tags_and_computes_mine() {
        let notes = check(&fixture());
        let projects = vec!["Launch".into()];
        let tags = sorting(
            r#"{"meeting_project":"Launch","tasks":[]}"#,
            &notes,
            &projects,
            "",
        )
        .unwrap();
        assert_eq!(tags.tasks[0].mine, Some(true));
        assert_eq!(tags.tasks[0].project, "Launch");
        assert!(!tags.tasks[0].new);
    }
}
