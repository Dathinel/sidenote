use super::*;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Choice {
    pub value: Value,
    pub device: String,
    pub at: String,
    pub source: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Conflict {
    #[serde(default)]
    pub domain: String,
    pub task_id: String,
    pub title: String,
    pub field: String,
    pub choices: Vec<Choice>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub task_id: String,
    pub field: String,
    pub alternatives: Vec<Choice>,
    pub chosen: Choice,
    pub device: String,
    pub at: String,
}

pub(super) fn decode(raw: &str) -> anyhow::Result<Choice> {
    if let Some(json) = raw.strip_prefix("sidenote-field-v2:") {
        return Ok(serde_json::from_str(json)?);
    }
    Ok(Choice {
        value: serde_json::from_str(raw)?,
        device: "Earlier preview".into(),
        at: String::new(),
        source: "legacy preview".into(),
    })
}
fn encoded(value: Value, device: &str, source: &str) -> anyhow::Result<String> {
    let choice = Choice {
        value,
        device: device.into(),
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        source: source.into(),
    };
    Ok(format!(
        "sidenote-field-v2:{}",
        serde_json::to_string(&choice)?
    ))
}
pub(super) fn present(doc: &Automerge, key: &str) -> anyhow::Result<bool> {
    let values = choices(doc, key)?;
    Ok(values.iter().any(|v| v.value == Value::Bool(true))
        && !values.iter().any(|v| v.value == Value::Bool(false)))
}

pub(super) fn apply(
    doc: &mut Automerge,
    before: &[ManualTask],
    after: &[ManualTask],
    device: &str,
    source: &str,
) -> anyhow::Result<()> {
    let (before, after) = (fields(before)?, fields(after)?);
    let mut nested = Vec::new();
    for (id, new) in &after {
        let old = before.get(id);
        let view = |fields: Option<&Map<String, Value>>| {
            serde_json::json!({
                "subtasks":fields.and_then(|v|v.get("subtasks")).cloned().unwrap_or_else(||serde_json::json!([])),
                "subtask_ticks":fields.and_then(|v|v.get("subtask_ticks")).cloned().unwrap_or_else(||serde_json::json!({})),
            })
        };
        let (a, b) = (view(old), view(Some(new)));
        if a != b {
            nested.push((id.clone(), a, b));
        }
    }
    let mut tx = doc.transaction();
    for id in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let (old, new) = (before.get(id), after.get(id));
        if old.is_none() || new.is_none() {
            tx.put(
                ROOT,
                serde_json::to_string(&[id.as_str(), "$present"])?,
                encoded(Value::Bool(new.is_some()), device, source)?,
            )?;
        }
        let names: BTreeSet<_> = old
            .into_iter()
            .flat_map(|f| f.keys())
            .chain(new.into_iter().flat_map(|f| f.keys()))
            .collect();
        for name in names {
            anyhow::ensure!(name.len() <= 128 && name != "$present", "invalid field");
            let (a, b) = (old.and_then(|f| f.get(name)), new.and_then(|f| f.get(name)));
            if a != b && new.is_some() && !matches!(name.as_str(), "subtasks" | "subtask_ticks") {
                tx.put(
                    ROOT,
                    serde_json::to_string(&[id, name])?,
                    encoded(b.cloned().unwrap_or(Value::Null), device, source)?,
                )?;
            }
        }
    }
    tx.commit();
    for (id, before, after) in nested {
        super::shared::nested_manual(doc, &id, &before, &after, device, source)?;
    }
    Ok(())
}

fn choices(doc: &Automerge, key: &str) -> anyhow::Result<Vec<Choice>> {
    let mut distinct = BTreeMap::new();
    for (value, _) in doc.get_all(ROOT, key)? {
        let choice = decode(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid task field"))?,
        )?;
        distinct
            .entry(serde_json::to_string(&choice.value)?)
            .or_insert(choice);
    }
    Ok(distinct.into_values().collect())
}

pub(super) fn conflicts(doc: &Automerge) -> anyhow::Result<Vec<Conflict>> {
    let tasks = project(doc)?;
    let mut conflicts = Vec::new();
    for key in doc.keys(ROOT) {
        let [id, field]: [String; 2] = serde_json::from_str(&key)?;
        if id.starts_with('$') || field.starts_with('$') || field == "id" {
            continue;
        }
        let Some(task) = tasks.iter().find(|t| t.id == id) else {
            continue;
        };
        let choices = choices(doc, &key)?;
        if choices.len() > 1 {
            conflicts.push(Conflict {
                domain: String::new(),
                task_id: id,
                title: task.title.clone().unwrap_or_else(|| task.task.clone()),
                field,
                choices,
            });
        }
    }
    Ok(conflicts)
}

impl Preview {
    pub fn conflicts(&self) -> io::Result<Vec<Conflict>> {
        let mut out = conflicts(&self.device.load("tasks").map_err(failed)?).map_err(failed)?;
        out.extend(self.shared_conflicts()?);
        Ok(out)
    }
    pub fn resolutions(&self) -> io::Result<Vec<Resolution>> {
        let domains = self.device.store(|s| s.inventory()).map_err(failed)?;
        let mut out: Vec<Resolution> = Vec::new();
        for domain in domains {
            let doc = self.device.load(&domain).map_err(failed)?;
            for key in doc.keys(ROOT) {
                let [kind, _]: [String; 2] = serde_json::from_str(&key)?;
                if kind == "$resolution" {
                    let value = doc
                        .get(ROOT, key)
                        .map_err(failed)?
                        .ok_or_else(|| failed("missing resolution"))?
                        .0;
                    out.push(
                        serde_json::from_str(
                            value.as_str().ok_or_else(|| failed("invalid resolution"))?,
                        )
                        .map_err(failed)?,
                    );
                }
            }
        }
        out.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(out)
    }
    pub fn resolve(&self, task_id: &str, field: &str, chosen: &Choice) -> io::Result<()> {
        let device = super::super::DeviceSettings::read_in(&self.root)?.name;
        let record_id = super::super::device::random_id()?;
        let operation = |doc: &mut Automerge| -> anyhow::Result<()> {
            let key = serde_json::to_string(&[task_id, field])?;
            let alternatives = choices(doc, &key)?;
            anyhow::ensure!(
                alternatives.len() > 1 && alternatives.iter().any(|c| c == chosen),
                "versions changed"
            );
            let log = Resolution {
                task_id: task_id.into(),
                field: field.into(),
                alternatives,
                chosen: chosen.clone(),
                device: device.clone(),
                at: chrono::Utc::now().to_rfc3339(),
            };
            let mut tx = doc.transaction();
            tx.put(
                ROOT,
                key,
                encoded(chosen.value.clone(), &device, "resolution")?,
            )?;
            tx.put(
                ROOT,
                serde_json::to_string(&["$resolution", &record_id])?,
                serde_json::to_string(&log)?,
            )?;
            tx.commit();
            Ok(())
        };
        let connection = self.connection.lock().map_err(|_| failed("mutex"))?;
        if let Some(connection) = connection.as_ref() {
            connection.client.edit("tasks", operation).map_err(failed)
        } else {
            self.device
                .store(|s| s.edit("tasks", operation))
                .map_err(failed)
        }
    }
}

pub fn current(root: &Path) -> io::Result<Vec<Conflict>> {
    get(root)?
        .map(|preview| preview.conflicts())
        .unwrap_or_else(|| Ok(Vec::new()))
}
