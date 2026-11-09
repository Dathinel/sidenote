//! Whole-field merge seam. A later text CRDT can replace `merge` without changing editors.
use super::{ManualTask, Overrides, TaskOverride, TaskStore, sync_state};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io;
pub const MOVED: &str = "This task moved after the summary was updated. Your changes are kept.";
pub fn task_value(o: &Overrides, row: &str, index: usize) -> Value {
    if let Some(id) = row.strip_prefix("manual/") {
        serde_json::to_value(o.manual.iter().find(|m| m.id == id)).unwrap_or(Value::Null)
    } else {
        serde_json::to_value(o.get(row, index)).unwrap_or(Value::Null)
    }
}
/// Missing and null are equivalent for optional fields. Only mine's changed fields overwrite.
pub fn merge<T: Serialize + DeserializeOwned>(base: &T, mine: &T, theirs: &T) -> io::Result<T> {
    let (base, mine, mut out) = (
        serde_json::to_value(base)?,
        serde_json::to_value(mine)?,
        serde_json::to_value(theirs)?,
    );
    let b = base
        .as_object()
        .ok_or_else(|| io::Error::other("Invalid edit base"))?;
    let m = mine
        .as_object()
        .ok_or_else(|| io::Error::other("Invalid edit"))?;
    let o = out
        .as_object_mut()
        .ok_or_else(|| io::Error::other("Invalid current item"))?;
    for key in b
        .keys()
        .chain(m.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if key == "updated_at" {
            continue;
        }
        let (bv, mv) = (
            b.get(key).unwrap_or(&Value::Null),
            m.get(key).unwrap_or(&Value::Null),
        );
        if mv == bv {
            continue;
        }
        if mv.is_null() {
            o.remove(key);
        } else {
            o.insert(key.clone(), mv.clone());
        }
    }
    // Each pair records one choice, even when only one component changed.
    for fields in [["project", "assignment"], ["owner", "mine"]] {
        if fields
            .iter()
            .any(|key| b.get(*key).unwrap_or(&Value::Null) != m.get(*key).unwrap_or(&Value::Null))
        {
            for key in fields {
                match m.get(key).filter(|value| !value.is_null()) {
                    Some(value) => {
                        o.insert(key.into(), value.clone());
                    }
                    None => {
                        o.remove(key);
                    }
                }
            }
        }
    }
    Ok(serde_json::from_value(out)?)
}
/// Rebase only saved fields. Drafts in the other boxes still use their edit-start values.
pub fn advance_base(base: &mut Value, mine: &Value, saved: &Value) {
    let mut next = base.as_object().cloned().unwrap_or_default();
    if let Some(m) = mine.as_object() {
        let keys: std::collections::BTreeSet<_> = next.keys().chain(m.keys()).cloned().collect();
        for key in keys {
            if base.get(&key).unwrap_or(&Value::Null) != m.get(&key).unwrap_or(&Value::Null) {
                if let Some(value) = saved.get(&key) {
                    next.insert(key, value.clone());
                } else {
                    next.remove(&key);
                }
            }
        }
    }
    for fields in [["project", "assignment"], ["owner", "mine"]] {
        if fields.iter().any(|key| {
            base.get(*key).unwrap_or(&Value::Null) != mine.get(*key).unwrap_or(&Value::Null)
        }) {
            for key in fields {
                match saved.get(key) {
                    Some(value) => {
                        next.insert(key.into(), value.clone());
                    }
                    None => {
                        next.remove(key);
                    }
                }
            }
        }
    }
    *base = Value::Object(next);
}
impl TaskStore {
    pub fn toggle_manual_view(&self, id: &str) -> io::Result<super::EditView<Overrides>> {
        sync_state::update_merged(&self.root, "task-overrides.json", |o: &mut Overrides| {
            let task = o
                .manual
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Task removed"))?;
            task.done = !task.done;
            Ok(())
        })
    }
    pub fn merge_task(
        &self,
        row: &str,
        index: usize,
        base: &Value,
        mine: &Value,
        source: Option<&std::path::Path>,
    ) -> io::Result<super::EditView<Overrides>> {
        let moved = || io::Error::new(io::ErrorKind::InvalidInput, MOVED);
        let source_stamp = if !row.starts_with("manual/") {
            let path = source.ok_or_else(moved)?.join("english-summary.json");
            let meta = super::file_reads::checked_metadata(&path)?;
            let raw = super::file_reads::read(&path, super::file_reads::SUMMARY_MAX)?;
            let summary: super::Summary = serde_json::from_slice(&raw)?;
            if summary.todo.get(index).map(|t| t.task.as_str())
                != mine.get("task").and_then(Value::as_str)
            {
                return Err(moved());
            }
            Some((path, meta.len(), meta.modified().ok()))
        } else {
            None
        };
        let identity = || -> io::Result<()> {
            if let Some((path, len, modified)) = &source_stamp {
                let meta = super::file_reads::checked_metadata(path)?;
                if meta.len() != *len || meta.modified().ok() != *modified {
                    return Err(moved());
                }
            }
            Ok(())
        };
        sync_state::update_merged_validated(
            &self.root,
            "task-overrides.json",
            |o: &mut Overrides| {
                identity()?;
                if let Some(id) = row.strip_prefix("manual/") {
                    let t =
                        o.manual.iter_mut().find(|m| m.id == id).ok_or_else(|| {
                            io::Error::new(io::ErrorKind::NotFound, "Task removed")
                        })?;
                    *t = merge(
                        &serde_json::from_value::<ManualTask>(base.clone())?,
                        &serde_json::from_value(mine.clone())?,
                        t,
                    )?;
                } else {
                    let t = o
                        .tasks
                        .entry(row.into())
                        .or_default()
                        .entry(index.to_string())
                        .or_default();
                    let expected = mine.get("task").and_then(Value::as_str);
                    if t.task.as_deref().is_some_and(|text| Some(text) != expected) {
                        *t = TaskOverride::default();
                    }
                    let b: TaskOverride = serde_json::from_value(if base.is_null() {
                        json!({})
                    } else {
                        base.clone()
                    })?;
                    *t = merge(&b, &serde_json::from_value(mine.clone())?, t)?;
                    t.other.insert(
                        "updated_at".into(),
                        Value::String(chrono::Utc::now().to_rfc3339()),
                    );
                    if t.assignment.is_none()
                        && t.project.is_none()
                        && t.owner.is_none()
                        && t.due.is_none()
                        && t.mine.is_none()
                    {
                        let per = o.tasks.get_mut(row).unwrap();
                        per.remove(&index.to_string());
                        if per.is_empty() {
                            o.tasks.remove(row);
                        }
                    }
                }
                Ok(())
            },
            identity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_choice_replaces_a_peers_mine_flag_and_rebases_both_fields() {
        let base = TaskOverride {
            mine: Some(false),
            ..Default::default()
        };
        let mine = TaskOverride {
            owner: Some("Alex".into()),
            ..base.clone()
        };
        let theirs = TaskOverride {
            mine: Some(true),
            due: Some("2026-11-12".into()),
            ..base.clone()
        };
        let saved = merge(&base, &mine, &theirs).unwrap();
        assert_eq!(saved.owner.as_deref(), Some("Alex"));
        assert_eq!(saved.mine, Some(false));
        assert_eq!(saved.due, theirs.due);

        let mut rebased = serde_json::to_value(&base).unwrap();
        advance_base(
            &mut rebased,
            &serde_json::to_value(&mine).unwrap(),
            &serde_json::to_value(&saved).unwrap(),
        );
        assert_eq!(rebased["owner"], "Alex");
        assert_eq!(rebased["mine"], false);
        assert!(rebased.get("due").is_none());
    }

    #[test]
    fn mine_choice_and_owner_reset_replace_the_whole_pair() {
        let base = ManualTask {
            owner: Some("Alex".into()),
            mine: Some(false),
            ..Default::default()
        };
        let theirs = ManualTask {
            owner: Some("Sam".into()),
            ..base.clone()
        };
        let mine = ManualTask {
            mine: Some(true),
            ..base.clone()
        };
        let saved = merge(&base, &mine, &theirs).unwrap();
        assert_eq!(saved.owner, base.owner);
        assert_eq!(saved.mine, Some(true));

        let reset = ManualTask {
            owner: None,
            mine: None,
            ..base.clone()
        };
        let saved = merge(&base, &reset, &theirs).unwrap();
        assert_eq!(saved.owner, None);
        assert_eq!(saved.mine, None);
        let mut rebased = serde_json::to_value(&base).unwrap();
        advance_base(
            &mut rebased,
            &serde_json::to_value(&reset).unwrap(),
            &serde_json::to_value(&saved).unwrap(),
        );
        assert!(rebased.get("owner").is_none());
        assert!(rebased.get("mine").is_none());
    }
}
