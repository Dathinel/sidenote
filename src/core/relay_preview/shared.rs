//! Field registers for explicitly selected app stores. No folder projection is published.
use super::*;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Node {
    kind: char,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    order: usize,
}
type Fields = BTreeMap<Vec<String>, Node>;

pub(crate) fn domain(store: &str) -> Option<String> {
    Some(match store {
        "projects.json" => "projects".into(),
        "memory/records.json" => "memory".into(),
        "task-overrides.json" => "tasks".into(),
        s if s.starts_with("$manual:") && s.len() <= 264 => "tasks".into(),
        "teams.json" | "people.json" => "people".into(),
        "profile.json" => "settings".into(),
        _ => {
            let id = store.strip_suffix(".json")?;
            let (a, b) = id.split_once('-')?;
            if a.is_empty()
                || b.is_empty()
                || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
            {
                return None;
            }
            format!("meeting:{id}")
        }
    })
}

pub(crate) fn defaults(store: &str) -> io::Result<Value> {
    Ok(selected(
        store,
        match store {
            "projects.json" => serde_json::to_value(super::super::Projects::default())?,
            "memory/records.json" => {
                serde_json::to_value(super::super::memory::MemoryFile::default())?
            }
            "task-overrides.json" => serde_json::to_value(super::super::Overrides::default())?,
            "teams.json" => serde_json::json!({"teams":[]}),
            "people.json" => serde_json::json!({"sides":{}}),
            "profile.json" => serde_json::json!({"your_name":""}),
            _ => serde_json::to_value(super::super::Saved::default())?,
        },
    ))
}
fn receipt_owner(id: &str) -> Option<String> {
    let parts: Vec<_> = id.split('-').collect();
    parts
        .windows(2)
        .find(|p| super::super::device::valid_id(p[0]) && super::super::device::valid_id(p[1]))
        .map(|p| p[0].into())
}
pub(crate) fn selected(store: &str, mut value: Value) -> Value {
    fn receipt(value: &mut Value) {
        if let Some(id) = value.get("id").and_then(Value::as_str).map(str::to_owned) {
            if let Some(owner) = receipt_owner(&id) {
                value["device"] = Value::String(owner);
            }
            if value.get("backup").is_some() {
                value["backup"] = Value::String(format!("{id}/backup.json"));
            }
        }
        if let Some(previous) = value.get_mut("previous") {
            receipt(previous);
        }
    }
    if store == "projects.json"
        && let Some(merges) = value.get_mut("merges").and_then(Value::as_object_mut)
    {
        for merge in merges.values_mut() {
            receipt(merge);
        }
    }
    if store == "memory/records.json" && value.get("migration").is_some_and(Value::is_object) {
        value["migration"]["source_path"] = Value::String("memory/company.md".into());
    }
    if store == "task-overrides.json" {
        if let Some(object) = value.as_object_mut() {
            object.remove("manual");
        }
    } else if store.ends_with(".json") && domain(store).is_some_and(|d| d.starts_with("meeting:")) {
        value = serde_json::json!({"checked":value.get("checked").cloned().unwrap_or_else(|| serde_json::json!([]))});
    }
    value
}

fn prefix(store: &str) -> String {
    format!("$shared:{store}")
}
fn key(store: &str, path: &[String]) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&[
        prefix(store),
        serde_json::to_string(path)?,
    ])?)
}
fn ready(store: &str) -> String {
    serde_json::to_string(&[prefix(store), "$ready".into()]).expect("strings")
}

fn identity(value: &Value) -> anyhow::Result<String> {
    if let Some(id) = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(format!("id:{id}"));
    }
    if let (Some(record), Some(at)) = (value.get("record_id"), value.get("at")) {
        return Ok(format!(
            "event:{}",
            serde_json::to_string(&(record, at, value.get("operation"), value.get("proposal_id")))?
        ));
    }
    Ok(format!(
        "value:{}",
        blake3::hash(&serde_json::to_vec(value)?).to_hex()
    ))
}
fn flatten(value: &Value) -> anyhow::Result<Fields> {
    fn visit(
        path: Vec<String>,
        value: &Value,
        order: usize,
        out: &mut Fields,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            path.len() <= 32 && out.len() < 100_000,
            "shared store limit"
        );
        let atomic = path.iter().any(|p| p.starts_with("o:area-operation:"))
            || (path.len() >= 2 && path[path.len() - 2] == "o:merges")
            || path.iter().any(|p| p == "o:undo");
        let kind = if atomic {
            'v'
        } else {
            match value {
                Value::Object(_) => 'o',
                Value::Array(_) => 'a',
                _ => 'v',
            }
        };
        out.insert(
            path.clone(),
            Node {
                kind,
                value: if kind == 'v' {
                    value.clone()
                } else {
                    Value::Null
                },
                order,
            },
        );
        if kind == 'o' {
            for (name, value) in value.as_object().expect("object") {
                let mut next = path.clone();
                let name =
                    if path.len() == 2 && path[0] == "o:tasks" && !name.starts_with("id:relay:") {
                        value
                            .get("task")
                            .and_then(Value::as_str)
                            .map(|task| {
                                format!(
                                    "id:relay:{}:{}",
                                    blake3::hash(name.as_bytes()).to_hex(),
                                    blake3::hash(task.as_bytes()).to_hex()
                                )
                            })
                            .unwrap_or_else(|| name.clone())
                    } else {
                        name.clone()
                    };
                next.push(format!("o:{name}"));
                visit(next, value, 0, out)?;
            }
        } else if kind == 'a' {
            let mut seen = BTreeSet::new();
            for (index, value) in value.as_array().expect("array").iter().enumerate() {
                let id = identity(value)?;
                // Sets collapse equal values; distinct records must have distinct stable IDs.
                if !seen.insert(id.clone()) {
                    anyhow::ensure!(!id.starts_with("id:"), "duplicate shared identity");
                    continue;
                }
                let mut next = path.clone();
                next.push(format!("a:{id}"));
                visit(next, value, index, out)?;
            }
        }
        Ok(())
    }
    let mut fields = Fields::new();
    visit(Vec::new(), value, 0, &mut fields)?;
    Ok(fields)
}
fn node(doc: &Automerge, store: &str, path: &[String]) -> anyhow::Result<Option<Node>> {
    let key = key(store, path)?;
    let values = doc.get_all(ROOT, &key)?;
    let mut nodes = Vec::new();
    for (value, _) in &values {
        let choice = versions::decode(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("shared field"))?,
        )?;
        nodes.push(serde_json::from_value::<Node>(choice.value)?);
    }
    // A removed collection entry cannot be resurrected by an unrelated stale field edit.
    if nodes.iter().any(|n| n.kind == 'x') && nodes.iter().any(|n| n.kind == 'o' || n.kind == 'a') {
        return Ok(None);
    }
    let Some((value, _)) = doc.get(ROOT, key)? else {
        return Ok(None);
    };
    let choice = versions::decode(
        value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("shared field"))?,
    )?;
    let node: Node = serde_json::from_value(choice.value)?;
    Ok((node.kind != 'x').then_some(node))
}
pub(crate) fn project(doc: &Automerge, store: &str, default: Value) -> anyhow::Result<Value> {
    if doc.get(ROOT, ready(store))?.is_none() {
        return Ok(default);
    }
    let mut paths = BTreeSet::new();
    for key in doc.keys(ROOT) {
        let [scope, path]: [String; 2] = serde_json::from_str(&key)?;
        if scope == prefix(store) && path != "$ready" {
            paths.insert(serde_json::from_str::<Vec<String>>(&path)?);
        }
    }
    fn build(
        doc: &Automerge,
        store: &str,
        path: Vec<String>,
        paths: &BTreeSet<Vec<String>>,
    ) -> anyhow::Result<Option<(usize, Value)>> {
        let Some(node) = node(doc, store, &path)? else {
            return Ok(None);
        };
        let value = match node.kind {
            'v' => node.value,
            'o' | 'a' => {
                let mut object = Map::new();
                let mut array = Vec::new();
                for child in paths
                    .iter()
                    .filter(|p| p.len() == path.len() + 1 && p.starts_with(&path))
                {
                    if let Some((order, value)) = build(doc, store, child.clone(), paths)? {
                        let name = child.last().expect("child");
                        if node.kind == 'o' {
                            object.insert(
                                name.strip_prefix("o:")
                                    .ok_or_else(|| anyhow::anyhow!("object path"))?
                                    .into(),
                                value,
                            );
                        } else {
                            array.push((order, name.clone(), value));
                        }
                    }
                }
                if node.kind == 'o' {
                    Value::Object(object)
                } else {
                    array.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
                    Value::Array(array.into_iter().map(|(_, _, v)| v).collect())
                }
            }
            _ => anyhow::bail!("shared node kind"),
        };
        Ok(Some((node.order, value)))
    }
    Ok(build(doc, store, Vec::new(), &paths)?
        .map(|(_, v)| v)
        .unwrap_or(default))
}

pub(crate) fn apply(
    doc: &mut Automerge,
    store: &str,
    before: &Value,
    after: &Value,
    name: &str,
    author: &str,
    import: bool,
) -> anyhow::Result<()> {
    let before = flatten(before)?;
    let after = flatten(after)?;
    let mut changes = Vec::new();
    for path in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let old = before.get(path);
        let new = after.get(path);
        if old.map(|n| (&n.kind, &n.value)) == new.map(|n| (&n.kind, &n.value)) {
            continue;
        }
        let key = key(store, path)?;
        if import && doc.get(ROOT, &key)?.is_some() {
            continue;
        }
        let mut next = new.cloned().unwrap_or(Node {
            kind: 'x',
            value: Value::Null,
            order: 0,
        });
        if import && next.kind == 'v' && (next.value.is_null() || next.value.as_str() == Some("")) {
            continue;
        }
        let current = node(doc, store, path)?;
        fn semantic(n: Option<&Node>) -> Option<(char, &Value)> {
            n.filter(|n| !(n.kind == 'v' && (n.value.is_null() || n.value.as_str() == Some(""))))
                .map(|n| (n.kind, &n.value))
        }
        if !import
            && semantic(old) != semantic(current.as_ref())
            && semantic(new) != semantic(current.as_ref())
        {
            anyhow::bail!("shared field changed since the editor opened");
        }
        if let Some(old) = current {
            next.order = old.order;
        }
        changes.push((key, next));
    }
    let mut tx = doc.transaction();
    for (key, next) in changes {
        let choice = Choice {
            value: serde_json::to_value(next)?,
            device: name.into(),
            at: chrono::Utc::now().to_rfc3339(),
            source: format!("{}:{author}", if import { "import" } else { "device" }),
        };
        tx.put(
            ROOT,
            key,
            format!("sidenote-field-v2:{}", serde_json::to_string(&choice)?),
        )?;
    }
    tx.put(ROOT, ready(store), true)?;
    tx.commit();
    Ok(())
}

impl Preview {
    pub(crate) fn shared_value(&self, store: &str, default: Value) -> io::Result<Value> {
        let domain = domain(store).ok_or_else(|| failed("shared domain"))?;
        project(&self.device.load(&domain).map_err(failed)?, store, default).map_err(failed)
    }
    pub(crate) fn shared_edit(&self, store: &str, before: &Value, after: &Value) -> io::Result<()> {
        let domain = domain(store).ok_or_else(|| failed("shared domain"))?;
        let own = super::super::DeviceSettings::read_in(&self.root)?;
        let before = selected(store, before.clone());
        let after = selected(store, after.clone());
        let operation = |doc: &mut Automerge| {
            if doc.get(ROOT, ready(store))?.is_none() {
                apply(doc, store, &Value::Null, &before, &own.name, &own.id, true)?;
            }
            apply(doc, store, &before, &after, &own.name, &own.id, false)
        };
        let connection = self.connection.lock().map_err(failed)?;
        if let Some(c) = connection.as_ref() {
            c.client.edit(&domain, operation).map_err(failed)
        } else {
            self.device
                .store(|s| s.edit(&domain, operation))
                .map_err(failed)
        }
    }
    pub(crate) fn check_stamps(
        &self,
        store: &str,
    ) -> io::Result<BTreeMap<Vec<String>, Option<u64>>> {
        let doc = self
            .device
            .load(&domain(store).ok_or_else(|| failed("domain"))?)
            .map_err(failed)?;
        let mut out = BTreeMap::new();
        for key in doc.keys(ROOT) {
            let [scope, path]: [String; 2] = serde_json::from_str(&key)?;
            if scope != prefix(store) || path == "$ready" {
                continue;
            }
            let path: Vec<String> = serde_json::from_str(&path)?;
            if path.len() != 2 || path[0] != "o:checked" {
                continue;
            }
            let Some(node) = node(&doc, store, &path).map_err(failed)? else {
                continue;
            };
            let Some(task) = node.value.as_str() else {
                continue;
            };
            let choice = versions::decode(
                doc.get(ROOT, &key)
                    .map_err(failed)?
                    .ok_or_else(|| failed("field"))?
                    .0
                    .as_str()
                    .ok_or_else(|| failed("field"))?,
            )
            .map_err(failed)?;
            let at = if choice.source.starts_with("import:") {
                None
            } else {
                chrono::DateTime::parse_from_rfc3339(&choice.at)
                    .ok()
                    .and_then(|d| d.timestamp_nanos_opt())
                    .and_then(|n| u64::try_from(n).ok())
            };
            out.insert(vec!["checked".into(), task.into()], at);
        }
        Ok(out)
    }
    pub(crate) fn shared_writer(
        &self,
        store: &str,
        path: Vec<String>,
    ) -> io::Result<Option<String>> {
        let doc = self
            .device
            .load(&domain(store).ok_or_else(|| failed("domain"))?)
            .map_err(failed)?;
        let Some((v, _)) = doc
            .get(ROOT, key(store, &path).map_err(failed)?)
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let choice =
            versions::decode(v.as_str().ok_or_else(|| failed("field"))?).map_err(failed)?;
        let node: Node = serde_json::from_value(choice.value)?;
        if store == "projects.json"
            && let Some(owner) = node
                .value
                .get("device")
                .and_then(Value::as_str)
                .filter(|s| super::super::device::valid_id(s))
        {
            return Ok(Some(owner.into()));
        }
        Ok(choice
            .source
            .strip_prefix("device:")
            .or_else(|| choice.source.strip_prefix("import:"))
            .map(str::to_owned))
    }
}

pub(crate) fn load(root: &Path, store: &str, default: Value) -> io::Result<Value> {
    match get(root)? {
        Some(p) => p.shared_value(store, default),
        None => Ok(default),
    }
}
pub(crate) fn edit(root: &Path, store: &str, before: &Value, after: &Value) -> io::Result<bool> {
    let _mode = store_io::lock(&root.join("local/relay/mode.lock"))?;
    if let Some(preview) = get(root)? {
        if before != after {
            preview.shared_edit(store, before, after)?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(crate) fn local_profile(root: &Path) -> io::Result<Value> {
    let raw: Value = store_io::read(&root.join("config.json"))?;
    Ok(
        serde_json::json!({"your_name":raw.get("notes").and_then(|n|n.get("your_name")).and_then(Value::as_str).unwrap_or("")}),
    )
}
pub(crate) fn local_people(root: &Path) -> io::Result<Value> {
    let raw: Value = store_io::read(&root.join("local/ui.json"))?;
    Ok(
        serde_json::json!({"sides":raw.get("people_sides").cloned().unwrap_or_else(||serde_json::json!({}))}),
    )
}
pub(crate) fn team_ids(teams: &mut super::super::Teams) {
    for team in &mut teams.teams {
        if team.id.is_empty() {
            team.id = format!(
                "team-{}",
                blake3::hash(super::super::people::key(&team.name).as_bytes()).to_hex()
            );
        }
    }
}

fn alternatives(doc: &Automerge, key: &str) -> anyhow::Result<Vec<(Choice, Node)>> {
    let mut out = BTreeMap::new();
    for (value, _) in doc.get_all(ROOT, key)? {
        let mut choice = versions::decode(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("shared field"))?,
        )?;
        let node: Node = serde_json::from_value(choice.value)?;
        if node.kind != 'v' && node.kind != 'x' {
            continue;
        }
        choice.value = if node.kind == 'x' {
            Value::Null
        } else {
            node.value.clone()
        };
        out.entry(serde_json::to_string(&choice.value)?)
            .or_insert((choice, node));
    }
    Ok(out.into_values().collect())
}
impl Preview {
    pub(super) fn shared_conflicts(&self) -> io::Result<Vec<Conflict>> {
        let mut out = Vec::new();
        for doc_domain in self.device.store(|s| s.inventory()).map_err(failed)? {
            let doc = self.device.load(&doc_domain).map_err(failed)?;
            for key in doc.keys(ROOT) {
                let [scope, field]: [String; 2] = serde_json::from_str(&key)?;
                let Some(store) = scope.strip_prefix("$shared:") else {
                    continue;
                };
                if field == "$ready" || domain(store).as_deref() != Some(&doc_domain) {
                    continue;
                }
                let path: Vec<String> = serde_json::from_str(&field)?;
                let mut visible = true;
                for depth in 0..path.len() {
                    if node(&doc, store, &path[..depth]).map_err(failed)?.is_none() {
                        visible = false;
                        break;
                    }
                }
                if !visible {
                    continue;
                }
                if path.last().is_some_and(|p| {
                    matches!(
                        p.as_str(),
                        "o:updated_at" | "o:decided_at" | "o:last_confirmed" | "o:first_seen"
                    )
                }) {
                    continue;
                }
                let choices = alternatives(&doc, &key).map_err(failed)?;
                if choices.len() > 1 {
                    let title = match store {
                        "projects.json" => "Projects and areas",
                        "memory/records.json" => "Memory",
                        "teams.json" => "Teams",
                        "people.json" => "People",
                        "profile.json" => "You",
                        "task-overrides.json" => "Meeting task edits",
                        s if s.starts_with("$manual:") => "Task subtasks",
                        _ => "Meeting checks",
                    };
                    out.push(Conflict {
                        domain: doc_domain.clone(),
                        task_id: store.into(),
                        title: title.into(),
                        field,
                        choices: choices.into_iter().map(|(c, _)| c).collect(),
                    });
                }
            }
        }
        Ok(out)
    }
    pub fn resolve_conflict(&self, conflict: &Conflict, chosen: &Choice) -> io::Result<()> {
        if conflict.domain.is_empty() {
            return self.resolve(&conflict.task_id, &conflict.field, chosen);
        }
        if domain(&conflict.task_id).as_deref() != Some(&conflict.domain) {
            return Err(store_io::invalid("Unknown shared conflict domain"));
        }
        let own = super::super::DeviceSettings::read_in(&self.root)?;
        let record = super::super::device::random_id()?;
        let operation = |doc: &mut Automerge| -> anyhow::Result<()> {
            let path: Vec<String> = serde_json::from_str(&conflict.field)?;
            let key = key(&conflict.task_id, &path)?;
            let current = alternatives(doc, &key)?;
            let selected = current
                .iter()
                .find(|(choice, _)| choice == chosen)
                .ok_or_else(|| anyhow::anyhow!("versions changed"))?;
            anyhow::ensure!(current.len() > 1, "versions changed");
            let log = Resolution {
                task_id: conflict.task_id.clone(),
                field: conflict.field.clone(),
                alternatives: current.iter().map(|(c, _)| c.clone()).collect(),
                chosen: chosen.clone(),
                device: own.name.clone(),
                at: chrono::Utc::now().to_rfc3339(),
            };
            let value = Choice {
                value: serde_json::to_value(&selected.1)?,
                device: own.name.clone(),
                at: log.at.clone(),
                source: format!("device:{}", own.id),
            };
            let mut tx = doc.transaction();
            tx.put(
                ROOT,
                key,
                format!("sidenote-field-v2:{}", serde_json::to_string(&value)?),
            )?;
            tx.put(
                ROOT,
                serde_json::to_string(&["$resolution", &record])?,
                serde_json::to_string(&log)?,
            )?;
            tx.commit();
            if conflict.task_id == "memory/records.json"
                && path.first().is_some_and(|v| v == "o:proposals")
                && path.last().is_some_and(|v| v == "o:status")
                && chosen.value.as_str() != Some("accepted")
            {
                let proposal = path
                    .get(1)
                    .and_then(|p| p.strip_prefix("a:id:"))
                    .ok_or_else(|| anyhow::anyhow!("proposal identity"))?;
                let original =
                    project(doc, "memory/records.json", defaults("memory/records.json")?)?;
                let file: super::super::memory::MemoryFile =
                    serde_json::from_value(original.clone())?;
                let mut restored = file.clone();
                for event in &file.events {
                    if event.proposal_id.as_deref() != Some(proposal) {
                        continue;
                    }
                    let Some(undo) = &event.undo else {
                        continue;
                    };
                    if undo
                        .proposal_after
                        .as_ref()
                        .is_none_or(|p| p.status != super::super::memory::ProposalStatus::Accepted)
                    {
                        continue;
                    }
                    for after in &undo.after {
                        if let Some(index) = restored.records.iter().position(|r| r.id == after.id)
                        {
                            let current = serde_json::to_value(&restored.records[index])?;
                            let prior = undo.before.iter().find(|r| r.id == after.id);
                            anyhow::ensure!(
                                current == serde_json::to_value(after)?
                                    || prior
                                        .is_some_and(|p| serde_json::to_value(p).ok().as_ref()
                                            == Some(&current)),
                                "memory record changed after decision"
                            );
                            restored.records.remove(index);
                            if let Some(prior) = prior {
                                restored.records.push(prior.clone());
                            }
                        }
                    }
                }
                apply(
                    doc,
                    "memory/records.json",
                    &original,
                    &serde_json::to_value(restored)?,
                    &own.name,
                    &own.id,
                    false,
                )?;
            }
            Ok(())
        };
        let connection = self.connection.lock().map_err(failed)?;
        if let Some(c) = connection.as_ref() {
            c.client.edit(&conflict.domain, operation).map_err(failed)
        } else {
            self.device
                .store(|s| s.edit(&conflict.domain, operation))
                .map_err(failed)
        }
    }
}

pub(super) fn nested_manual(
    doc: &mut Automerge,
    id: &str,
    before: &Value,
    after: &Value,
    name: &str,
    author: &str,
) -> anyhow::Result<()> {
    let store = format!("$manual:{id}");
    if doc.get(ROOT, ready(&store))?.is_none() {
        apply(doc, &store, &Value::Null, before, name, author, true)?;
    }
    apply(doc, &store, before, after, name, author, false)
}
pub(super) fn manual_fields(
    doc: &Automerge,
    id: &str,
    row: &mut Map<String, Value>,
) -> anyhow::Result<()> {
    let value = project(doc, &format!("$manual:{id}"), Value::Null)?;
    if let Some(fields) = value.as_object() {
        row.extend(fields.clone());
    }
    Ok(())
}
impl Preview {
    pub(crate) fn edit_overrides(&self, before: &Value, after: &Value) -> io::Result<()> {
        let own = super::super::DeviceSettings::read_in(&self.root)?;
        let old: Vec<ManualTask> = serde_json::from_value(
            before
                .get("manual")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )?;
        let new: Vec<ManualTask> = serde_json::from_value(
            after
                .get("manual")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )?;
        let operation = |doc: &mut Automerge| -> anyhow::Result<()> {
            versions::apply(doc, &old, &new, &own.name, &own.id)?;
            apply(
                doc,
                "task-overrides.json",
                &selected("task-overrides.json", before.clone()),
                &selected("task-overrides.json", after.clone()),
                &own.name,
                &own.id,
                false,
            )
        };
        let connection = self.connection.lock().map_err(failed)?;
        if let Some(c) = connection.as_ref() {
            c.client.edit("tasks", operation).map_err(failed)
        } else {
            self.device
                .store(|s| s.edit("tasks", operation))
                .map_err(failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn seed(store: &str, value: &Value) -> anyhow::Result<Automerge> {
        let mut doc = Automerge::new();
        apply(
            &mut doc,
            store,
            &Value::Null,
            value,
            "Synthetic PC",
            "synthetic",
            true,
        )?;
        Ok(doc)
    }
    #[test]
    fn independent_fields_members_and_aliases_merge_without_array_winners() -> anyhow::Result<()> {
        let store = "teams.json";
        let original = json!({"teams":[{"id":"synthetic-team","name":"Research","members":["Alex"],"aliases":{}}]});
        let mut a = seed(store, &original)?;
        let mut b = a.fork();
        let mut mine = original.clone();
        mine["teams"][0]["members"] = json!(["Alex", "Bailey"]);
        let mut theirs = original.clone();
        theirs["teams"][0]["aliases"] = json!({"A.":"Alex"});
        theirs["teams"][0]["members"] = json!(["Alex", "Casey"]);
        apply(&mut a, store, &original, &mine, "A", "a", false)?;
        apply(&mut b, store, &original, &theirs, "B", "b", false)?;
        a.merge(&mut b)?;
        let value = project(&a, store, Value::Null)?;
        assert_eq!(value["teams"][0]["members"].as_array().unwrap().len(), 3);
        assert_eq!(value["teams"][0]["aliases"]["A."], "Alex");
        let mut changed = value.clone();
        changed["teams"][0]["name"] = json!("Field research");
        let mut offline = a.fork();
        apply(&mut a, store, &value, &json!({"teams":[]}), "A", "a", false)?;
        apply(&mut offline, store, &value, &changed, "B", "b", false)?;
        a.merge(&mut offline)?;
        let mut stale = seed(store, &original)?;
        a.merge(&mut stale)?;
        assert_eq!(project(&a, store, Value::Null)?["teams"], json!([]));
        Ok(())
    }
    #[test]
    fn stale_editor_refuses_overwrite_and_opaque_receipts_keep_independent_entries()
    -> anyhow::Result<()> {
        let original = json!({"projects":["A","B"],"merges":{},"area:one":{"schema_version":1,"id":"one","name":"Planning","status":"active","aliases":[]}});
        let mut a = seed("projects.json", &original)?;
        let mut b = a.fork();
        let mut mine = original.clone();
        mine["area:one"]["name"] = json!("Field planning");
        mine["merges"]["A"] = json!({"id":"receipt-a","active":true});
        let mut theirs = original.clone();
        theirs["area:one"]["status"] = json!("retired");
        theirs["merges"]["B"] = json!({"id":"receipt-b","active":true});
        apply(&mut a, "projects.json", &original, &mine, "A", "a", false)?;
        apply(&mut b, "projects.json", &original, &theirs, "B", "b", false)?;
        a.merge(&mut b)?;
        let value = project(&a, "projects.json", Value::Null)?;
        assert_eq!(value["area:one"]["name"], "Field planning");
        assert_eq!(value["area:one"]["status"], "retired");
        assert_eq!(value["merges"].as_object().unwrap().len(), 2);
        let mut stale = original.clone();
        stale["area:one"]["name"] = json!("Stale overwrite");
        let heads = a.get_heads();
        assert!(apply(&mut a, "projects.json", &original, &stale, "B", "b", false).is_err());
        assert_eq!(a.get_heads(), heads);
        Ok(())
    }
    #[test]
    fn manual_subtasks_keep_different_fields_on_the_same_stable_id() -> anyhow::Result<()> {
        let original = ManualTask {
            id: "synthetic-task".into(),
            task: "Synthetic task".into(),
            subtasks: vec![super::super::super::Subtask {
                id: "synthetic-subtask".into(),
                title: "Initial".into(),
                done: false,
            }],
            ..Default::default()
        };
        let mut a = Automerge::new();
        versions::apply(&mut a, &[], std::slice::from_ref(&original), "A", "a")?;
        let mut b = a.fork();
        let mut mine = original.clone();
        mine.subtasks[0].title = "Edited title".into();
        let mut theirs = original.clone();
        theirs.subtasks[0].done = true;
        versions::apply(&mut a, std::slice::from_ref(&original), &[mine], "A", "a")?;
        versions::apply(&mut b, &[original], &[theirs], "B", "b")?;
        a.merge(&mut b)?;
        let tasks = super::super::project(&a)?;
        assert_eq!(tasks[0].subtasks[0].title, "Edited title");
        assert!(tasks[0].subtasks[0].done);
        Ok(())
    }
}
