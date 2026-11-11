use serde_json::Value;
use sidenote::core::settle::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Icon {
    Owner,
    Date,
    Project,
    Team,
    Waiting,
    Ours,
    Tasks,
}
pub(super) struct Change {
    pub text: String,
    pub icon: Icon,
}
fn name(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}
fn date(value: &Value) -> String {
    name(value)
        .map(|value| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map(|date| date.format("%a %-d %b").to_string())
                .unwrap_or_else(|_| value.into())
        })
        .unwrap_or_default()
}
pub(super) fn describe(
    kind: Kind,
    before: &Value,
    after: &Value,
    task_title: impl Fn(&str) -> Option<String>,
) -> Change {
    let mut changes = Vec::new();
    match kind {
        Kind::TaskProperties => {
            if before["owner"] != after["owner"] {
                changes.push((
                    Icon::Owner,
                    name(&after["owner"])
                        .map(|s| format!("owner set to {s}"))
                        .unwrap_or_else(|| "owner cleared".into()),
                ));
            }
            if before["due"] != after["due"] || before["due_default"] != after["due_default"] {
                changes.push((
                    Icon::Date,
                    if name(&after["due"]).is_some() {
                        format!(
                            "due {}{}",
                            date(&after["due"]),
                            if after["due_default"].as_bool() == Some(true) {
                                " (guessed)"
                            } else {
                                ""
                            }
                        )
                    } else {
                        "due date cleared".into()
                    },
                ));
            }
            if before["team"] != after["team"] {
                changes.push((
                    Icon::Team,
                    name(&after["team"])
                        .map(|s| format!("team set to {s}"))
                        .unwrap_or_else(|| "team cleared".into()),
                ));
            }
            let waiting = after["status"].as_str() == Some("waiting");
            if before["waiting_on"] != after["waiting_on"]
                || (before["status"] != after["status"] && waiting)
            {
                let target = name(&after["waiting_on"]["text"])
                    .map(str::to_string)
                    .or_else(|| name(&after["waiting_on"]["task_id"]).and_then(&task_title));
                changes.push((
                    Icon::Waiting,
                    target
                        .map(|s| format!("waiting on {s}"))
                        .unwrap_or_else(|| {
                            if waiting {
                                "marked as waiting".into()
                            } else {
                                "no longer waiting".into()
                            }
                        }),
                ));
            }
            if before["status"] != after["status"] && !waiting {
                let status = match after["status"].as_str() {
                    Some("done") => "marked done",
                    Some("todo") if before["status"].as_str() == Some("waiting") => {
                        "no longer waiting"
                    }
                    Some("todo") => "marked open",
                    _ => "status updated",
                };
                if !changes.iter().any(|(_, s)| s == status) {
                    changes.push((
                        if before["status"].as_str() == Some("waiting") {
                            Icon::Waiting
                        } else {
                            Icon::Tasks
                        },
                        status.into(),
                    ));
                }
            }
            if before["subtasks"] != after["subtasks"] {
                let old = before["subtasks"].as_array().map_or(0, Vec::len);
                let new = after["subtasks"].as_array().map_or(0, Vec::len);
                changes.push((
                    Icon::Tasks,
                    if old == 0 && new > 0 {
                        format!(
                            "added {new} {}",
                            if new == 1 { "subtask" } else { "subtasks" }
                        )
                    } else if new == 0 {
                        "subtasks removed".into()
                    } else {
                        "subtasks updated".into()
                    },
                ));
            }
        }
        Kind::ProjectAssignment => {
            let project_changed = before["meeting_project"] != after["meeting_project"];
            if project_changed {
                changes.push((
                    Icon::Project,
                    name(&after["meeting_project"])
                        .map(|s| format!("added to {s}"))
                        .unwrap_or_else(|| {
                            name(&before["meeting_project"])
                                .map(|s| format!("removed from {s}"))
                                .unwrap_or_else(|| "project cleared".into())
                        }),
                ));
            }
            let mut ours = 0;
            let mut not_ours = 0;
            let old_tasks = before["tasks"].as_array();
            for task in after["tasks"].as_array().into_iter().flatten() {
                let old = old_tasks
                    .and_then(|tasks| tasks.iter().find(|old| old["i"] == task["i"]))
                    .unwrap_or(&Value::Null);
                let number = task["i"].as_u64().unwrap_or(0) + 1;
                if old["project"] != task["project"]
                    && !(project_changed && task["project"] == after["meeting_project"])
                {
                    changes.push((
                        Icon::Project,
                        name(&task["project"])
                            .map(|s| format!("task {number} added to {s}"))
                            .unwrap_or_else(|| format!("project cleared for task {number}")),
                    ));
                }
                if old["mine"] != task["mine"] {
                    match task["mine"].as_bool() {
                        Some(true) => ours += 1,
                        Some(false) => not_ours += 1,
                        None => changes.push((
                            Icon::Ours,
                            format!("our team choice cleared for task {number}"),
                        )),
                    }
                }
            }
            if ours > 0 {
                changes.push((
                    Icon::Ours,
                    if ours == 1 {
                        "marked as ours".into()
                    } else {
                        format!("marked {ours} tasks as ours")
                    },
                ));
            }
            if not_ours > 0 {
                changes.push((
                    Icon::Ours,
                    if not_ours == 1 {
                        "marked as not ours".into()
                    } else {
                        format!("marked {not_ours} tasks as not ours")
                    },
                ));
            }
            if before["tasks"].as_array().is_some_and(|tasks| {
                tasks.iter().any(|old| {
                    !after["tasks"]
                        .as_array()
                        .is_some_and(|tasks| tasks.iter().any(|new| old["i"] == new["i"]))
                })
            }) {
                changes.push((Icon::Project, "task assignments cleared".into()));
            }
        }
    }
    Change {
        icon: changes.first().map_or(Icon::Tasks, |(icon, _)| *icon),
        text: if changes.is_empty() {
            "details updated".into()
        } else {
            changes
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join(" \u{b7} ")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn describe_task(before: Value, after: Value) -> Change {
        describe(Kind::TaskProperties, &before, &after, |id| {
            (id == "t1").then(|| "Send the sample pack".into())
        })
    }
    #[test]
    fn task_changes_use_actual_values_and_first_changed_icon() {
        let change = describe_task(
            json!({"owner":"Alex","due":"2026-11-16","due_default":false}),
            json!({"owner":"Robin","due":"2026-11-17","due_default":true}),
        );
        assert_eq!(
            change.text,
            "owner set to Robin \u{b7} due Tue 17 Nov (guessed)"
        );
        assert_eq!(change.icon, Icon::Owner);
        let date = describe_task(
            json!({"owner":"Robin","due":"2026-11-16"}),
            json!({"owner":"Robin","due":"2026-11-17"}),
        );
        assert_eq!(date.text, "due Tue 17 Nov");
        assert_eq!(date.icon, Icon::Date);
    }
    #[test]
    fn waiting_uses_person_or_source_task_title_without_raw_ids() {
        let change = describe_task(
            json!({"status":"todo"}),
            json!({"status":"waiting","waiting_on":{"text":"Morgan Lee"}}),
        );
        assert_eq!(change.text, "waiting on Morgan Lee");
        assert_eq!(change.icon, Icon::Waiting);
        let change = describe_task(
            json!({"status":"todo"}),
            json!({"status":"waiting","waiting_on":{"task_id":"t1"}}),
        );
        assert_eq!(change.text, "waiting on Send the sample pack");
    }
    #[test]
    fn team_and_cleared_values_are_explicit() {
        let change = describe_task(json!({"team":"Delivery"}), json!({"team":"Design"}));
        assert_eq!(change.text, "team set to Design");
        assert_eq!(change.icon, Icon::Team);
        let change = describe_task(
            json!({"owner":"Robin","due":"2026-11-17"}),
            json!({"owner":null,"due":null}),
        );
        assert_eq!(change.text, "owner cleared \u{b7} due date cleared");
    }
    #[test]
    fn projects_and_mine_compare_task_ids_instead_of_array_order() {
        let change = describe(
            Kind::ProjectAssignment,
            &json!({"meeting_project":null,"tasks":[{"i":1,"mine":true,"project":"Other"}]}),
            &json!({"meeting_project":"Northwind Field Guide","tasks":[{"i":0,"mine":true,"project":"Northwind Field Guide"},{"i":1,"mine":true,"project":"Other"}]}),
            |_| None,
        );
        assert_eq!(
            change.text,
            "added to Northwind Field Guide \u{b7} marked as ours"
        );
        assert_eq!(change.icon, Icon::Project);
        let change = describe(
            Kind::ProjectAssignment,
            &json!({"tasks":[{"i":0,"mine":false}]}),
            &json!({"tasks":[{"i":0,"mine":true}]}),
            |_| None,
        );
        assert_eq!(change.text, "marked as ours");
        assert_eq!(change.icon, Icon::Ours);
        let change = describe(
            Kind::ProjectAssignment,
            &json!({"tasks":[{"i":0,"mine":true}]}),
            &json!({"tasks":[{"i":0,"mine":false}]}),
            |_| None,
        );
        assert_eq!(change.text, "marked as not ours");
    }
}
