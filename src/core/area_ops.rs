//! Area operations share project merge's backup, assignment inventory and guarded publication.
use super::{
    Projects, TaskStore, areas::*, memory::MemoryFile, project_merge, store_io, sync_state,
};
use project_merge::{digest, raw_assignments};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationRow {
    pub project_id: String,
    pub area_name: String,
    pub convert: bool,
    #[serde(default)]
    pub children: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Action {
    Create {
        name: String,
    },
    Rename {
        area_id: String,
        name: String,
    },
    Move {
        project_id: String,
        area_id: String,
    },
    Convert {
        project_id: String,
        area_name: String,
        children: BTreeMap<String, String>,
    },
    Migrate {
        rows: Vec<MigrationRow>,
    },
    Assign {
        key: String,
        target: Target,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub project_id: Option<String>,
    pub area_id: Option<String>,
    pub baseline: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preview {
    pub action: Action,
    pub references: Vec<Reference>,
    pub changes: BTreeMap<String, Option<Value>>,
    pub fingerprint: String,
    pub project_count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: u32,
    #[serde(default)]
    pub device: String,
    pub id: String,
    pub action: Action,
    pub backup: PathBuf,
    pub checksum: String,
    pub after: BTreeMap<String, Option<Value>>,
    pub baseline: BTreeMap<String, Value>,
    pub project_baseline: BTreeMap<String, Value>,
    pub active: bool,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    projects: Projects,
    preview: Preview,
    files: BTreeMap<PathBuf, Vec<u8>>,
}
fn invalid(message: &str) -> io::Error {
    store_io::invalid(message)
}
fn area_for_name(p: &mut Projects, name: &str) -> io::Result<String> {
    let name = name.trim();
    if name.is_empty() || name == super::UNSORTED {
        return Err(invalid("Enter an area name for every project"));
    }
    let c = p.area_catalog()?;
    let matches = c.area_matches(name);
    if matches.len() > 1 {
        return Err(invalid(
            "This area name is ambiguous. Rename an area first.",
        ));
    }
    if let Some(a) = matches.first() {
        return Ok(a.id.clone());
    }
    let id = format!(
        "area-{}",
        blake3::hash(super::project_merge::normalized_name(name).as_bytes()).to_hex()
    );
    put(
        p,
        "area:",
        &id,
        &Area {
            id: id.clone(),
            name: name.into(),
            aliases: Default::default(),
            status: Status::Active,
        },
    )?;
    Ok(id)
}
fn move_project(p: &mut Projects, id: &str, area: &str) -> io::Result<()> {
    let c = p.area_catalog()?;
    if !c.areas.contains_key(area) {
        return Err(invalid("Choose an existing area"));
    }
    let mut project = c
        .projects
        .get(id)
        .filter(|p| p.converted_to.is_none())
        .cloned()
        .ok_or_else(|| invalid("Choose an existing project"))?;
    project.area_id = Some(area.into());
    put(p, "area-project:", id, &project)
}
fn convert(
    p: &mut Projects,
    id: &str,
    area: &str,
    children: &BTreeMap<String, String>,
    refs: &[Reference],
) -> io::Result<()> {
    move_project(p, id, area)?;
    let c = p.area_catalog()?;
    let mut project = c.projects[id].clone();
    for (key, child) in children {
        let reference = refs
            .iter()
            .find(|r| r.key == *key && r.project_id.as_deref() == Some(id))
            .ok_or_else(|| invalid("A conversion choice no longer matches this project"))?;
        let target = Target {
            area_id: area.into(),
            project_id: Some(child.clone()),
        };
        if child == id {
            return Err(invalid(
                "Choose a child project, not the project being converted",
            ));
        }
        c.validate_target(&target)?;
        put(
            p,
            "area-link:",
            key,
            &Link {
                target,
                baseline: reference.baseline.clone(),
            },
        )?;
    }
    let aliases: Vec<_> = std::iter::once(project.name.clone())
        .chain(
            p.aliases
                .keys()
                .filter(|name| p.resolve(name) == project.name)
                .cloned(),
        )
        .collect();
    for name in aliases {
        put(
            p,
            "area-alias:",
            &name,
            &LegacyAlias::Area {
                area_id: area.into(),
            },
        )?;
    }
    project.converted_to = Some(area.into());
    put(p, "area-project:", id, &project)
}
fn simulate(p: &Projects, action: &Action, refs: &[Reference]) -> io::Result<Projects> {
    let mut next = p.clone();
    next.retain_project_identity()?;
    match action {
        Action::Create { name } => {
            if !next.area_catalog()?.area_matches(name).is_empty() {
                return Err(invalid("An area already uses that name or alias"));
            }
            area_for_name(&mut next, name)?;
        }
        Action::Rename { area_id, name } => {
            let c = next.area_catalog()?;
            let mut area = c
                .areas
                .get(area_id)
                .cloned()
                .ok_or_else(|| invalid("This area no longer exists"))?;
            if name.trim().is_empty() || c.area_matches(name).iter().any(|a| a.id != *area_id) {
                return Err(invalid("Choose a unique, nonempty area name"));
            }
            area.aliases.insert(area.name.clone());
            area.name = name.trim().into();
            put(&mut next, "area:", area_id, &area)?;
        }
        Action::Move {
            project_id,
            area_id,
        } => move_project(&mut next, project_id, area_id)?,
        Action::Convert {
            project_id,
            area_name,
            children,
        } => {
            let area = area_for_name(&mut next, area_name)?;
            convert(&mut next, project_id, &area, children, refs)?;
        }
        Action::Migrate { rows } => {
            let c = next.area_catalog()?;
            let expected: std::collections::BTreeSet<_> = c
                .projects
                .values()
                .filter(|p| p.converted_to.is_none())
                .map(|p| p.id.clone())
                .collect();
            let supplied: std::collections::BTreeSet<_> =
                rows.iter().map(|r| r.project_id.clone()).collect();
            if expected != supplied || supplied.len() != rows.len() {
                return Err(invalid(
                    "Review every current project once before applying migration",
                ));
            }
            // Establish all parents before resolving any child-project choices.
            for row in rows {
                let area = area_for_name(&mut next, &row.area_name)?;
                move_project(&mut next, &row.project_id, &area)?;
            }
            for row in rows.iter().filter(|r| r.convert) {
                let area = area_for_name(&mut next, &row.area_name)?;
                convert(&mut next, &row.project_id, &area, &row.children, refs)?;
            }
        }
        Action::Assign { key, target } => {
            next.area_catalog()?.validate_target(target)?;
            let reference = refs
                .iter()
                .find(|r| r.key == *key)
                .ok_or_else(|| invalid("This item changed. Reload before assigning it."))?;
            put(
                &mut next,
                "area-link:",
                key,
                &Link {
                    target: target.clone(),
                    baseline: reference.baseline.clone(),
                },
            )?;
        }
    }
    next.area_catalog()?;
    Ok(next)
}
impl Projects {
    pub fn area_receipts(&self) -> io::Result<BTreeMap<String, Receipt>> {
        read_entries(self, "area-operation:")
    }
}
impl TaskStore {
    fn area_inventory(&self, p: &Projects) -> io::Result<(String, Vec<Reference>)> {
        let meetings = project_merge::scan()?;
        for m in &meetings {
            let path = m.folder.join("tasks.json");
            if path.try_exists()? {
                let _: super::TasksFile = store_io::read(&path)?;
            }
        }
        let pipeline = super::load_pipeline(&meetings);
        let overrides = self.load_overrides()?;
        let memory: MemoryFile = sync_state::load(&self.root, "memory/records.json")?;
        let mut checked = std::collections::HashMap::new();
        for m in &meetings {
            checked.insert(
                m.id.clone(),
                super::StateStore::new(self.root.clone())
                    .load(&m.id)?
                    .checked,
            );
        }
        let tasks = super::build_tasks(&meetings, &pipeline, p, &overrides, &checked);
        let c = p.area_catalog()?;
        let raw = raw_assignments(p, &meetings, &pipeline, &overrides, &memory);
        let activity = activity(
            p,
            &meetings,
            &pipeline,
            &tasks,
            &Default::default(),
            chrono::Utc::now(),
        )?;
        let mut refs = Vec::new();
        let mut push = |key: String,
                        kind: &str,
                        label: String,
                        project: Option<String>,
                        area: Option<String>| {
            refs.push(Reference {
                baseline: raw.get(&key).cloned().unwrap_or(Value::Null),
                key,
                kind: kind.into(),
                label,
                project_id: project,
                area_id: area,
            });
        };
        for t in &tasks {
            push(
                task_key(t),
                "task",
                format!("{}: {}", t.meeting_title, t.task),
                c.project(p, &t.project)
                    .filter(|p| p.converted_to.is_none())
                    .map(|p| p.id.clone()),
                t.area_id.clone(),
            );
        }
        for m in &meetings {
            let target = activity.meetings.get(&m.id);
            let project = target.and_then(|t| t.project_id.clone()).or_else(|| {
                let name = p
                    .meeting_projects
                    .get(&m.id)
                    .cloned()
                    .or_else(|| {
                        super::dominant_project(tasks.iter().filter(|t| t.meeting_id == m.id))
                    })
                    .or_else(|| pipeline.get(&m.id).and_then(|f| f.meeting_project.clone()))?;
                c.project(p, &name)
                    .filter(|p| p.converted_to.is_none())
                    .map(|p| p.id.clone())
            });
            push(
                meeting_key(&m.id),
                "meeting",
                m.title.clone(),
                project,
                target.map(|t| t.area_id.clone()),
            );
        }
        for (kind, id, text, project) in memory
            .records
            .iter()
            .map(|r| {
                (
                    "memory",
                    &r.id,
                    r.text.as_deref().unwrap_or("Retained record"),
                    r.project.as_deref(),
                )
            })
            .chain(memory.proposals.iter().map(|r| {
                (
                    "proposal",
                    &r.proposal.id,
                    r.proposal.text.as_str(),
                    r.proposal.project.as_deref(),
                )
            }))
        {
            let key = json!([kind, id]).to_string();
            let target = project.and_then(|n| c.target(p, &key, &raw[&key], n));
            let project = target
                .as_ref()
                .and_then(|t| t.project_id.clone())
                .or_else(|| {
                    project
                        .and_then(|n| c.project(p, n))
                        .filter(|p| p.converted_to.is_none())
                        .map(|p| p.id.clone())
                });
            push(key, kind, text.into(), project, target.map(|t| t.area_id));
        }
        refs.sort_by(|a, b| a.key.cmp(&b.key));
        let sources: Vec<_> = meetings.iter().map(|m| (&m.id, &m.summary)).collect();
        let pipeline: BTreeMap<_, _> = pipeline.into_iter().collect();
        let checked: BTreeMap<_, _> = checked.into_iter().collect();
        Ok((
            digest(&(
                p,
                sources,
                pipeline,
                overrides,
                memory,
                checked,
                project_merge::project_files(&self.root)?,
            ))?,
            refs,
        ))
    }
    pub fn preview_area(&self, action: Action) -> io::Result<Preview> {
        self.area_preview(&self.load_projects()?, action)
    }
    fn area_preview(&self, p: &Projects, action: Action) -> io::Result<Preview> {
        let (fingerprint, refs) = self.area_inventory(p)?;
        let next = simulate(p, &action, &refs)?;
        let keys: std::collections::BTreeSet<_> =
            p.other.keys().chain(next.other.keys()).cloned().collect();
        let changes: BTreeMap<_, _> = keys
            .into_iter()
            .filter(|k| p.other.get(k) != next.other.get(k))
            .map(|k| (k.clone(), next.other.get(&k).cloned()))
            .collect();
        let c = p.area_catalog()?;
        let affected = |r: &Reference| match &action {
            Action::Create { .. } => false,
            Action::Rename { area_id, .. } => r.area_id.as_ref() == Some(area_id),
            Action::Move { project_id, .. } | Action::Convert { project_id, .. } => {
                r.project_id.as_ref() == Some(project_id)
            }
            Action::Migrate { rows } => rows
                .iter()
                .any(|row| r.project_id.as_ref() == Some(&row.project_id)),
            Action::Assign { key, .. } => r.key == *key,
        };
        let references = refs.into_iter().filter(affected).collect();
        let project_count = c
            .projects
            .values()
            .filter(|p| p.converted_to.is_none())
            .count();
        Ok(Preview {
            action,
            references,
            changes,
            fingerprint,
            project_count,
        })
    }
    pub fn apply_area(&self, preview: &Preview) -> io::Result<Projects> {
        let projects = sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                if self.area_preview(p, preview.action.clone())? != *preview {
                    return Err(invalid("Items changed. Review a fresh preview."));
                }
                let id = format!(
                    "{}-{}",
                    chrono::Utc::now().format("%Y%m%dT%H%M%S"),
                    sync_state::new_id(&self.root)?
                );
                let backup = self
                    .root
                    .join("local/area-operations")
                    .join(&id)
                    .join("backup.json");
                let snapshot = Backup {
                    projects: p.clone(),
                    preview: preview.clone(),
                    files: project_merge::project_files(&self.root)?,
                };
                store_io::write(&backup, &snapshot)?;
                verify_backup(&backup, &snapshot)?;
                proof_boundary(&self.root, "backup")?;
                if self.area_preview(p, preview.action.clone())? != *preview {
                    return Err(invalid(
                        "Items changed during backup. Review a fresh preview.",
                    ));
                }
                let mut baseline: BTreeMap<_, _> = preview
                    .references
                    .iter()
                    .map(|r| (r.key.clone(), r.baseline.clone()))
                    .collect();
                let mut project_baseline = BTreeMap::from([("$aliases".into(), json!(p.aliases))]);
                for r in &preview.references {
                    if let Some(id) = &r.project_id {
                        project_baseline.insert(
                            format!("area-project:{id}"),
                            p.other
                                .get(&format!("area-project:{id}"))
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                }
                for (key, value) in &preview.changes {
                    match value {
                        Some(v) => {
                            p.other.insert(key.clone(), v.clone());
                        }
                        None => {
                            p.other.remove(key);
                        }
                    }
                }
                // Include conversion targets and parent records in the Undo guard.
                for key in preview
                    .changes
                    .keys()
                    .filter(|k| k.starts_with("area-project:"))
                {
                    project_baseline.insert(
                        key.clone(),
                        p.other.get(key).cloned().unwrap_or(Value::Null),
                    );
                }
                for key in baseline.clone().keys() {
                    baseline.insert(
                        format!("link:{key}"),
                        p.other
                            .get(&format!("area-link:{key}"))
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                for link in p.area_catalog()?.links.values() {
                    if let Some(id) = &link.target.project_id {
                        let key = format!("area-project:{id}");
                        project_baseline.insert(
                            key.clone(),
                            p.other.get(&key).cloned().unwrap_or(Value::Null),
                        );
                    }
                }
                let receipt = Receipt {
                    schema_version: 1,
                    device: super::DeviceSettings::read_in(&self.root)?.id,
                    id: id.clone(),
                    action: preview.action.clone(),
                    backup,
                    checksum: digest(&snapshot)?,
                    after: preview.changes.clone(),
                    baseline,
                    project_baseline,
                    active: true,
                };
                put(p, "area-operation:", &id, &receipt)?;
                proof_boundary(&self.root, "staged")?;
                Ok(())
            },
            |before| {
                proof_boundary(&self.root, "publication")?;
                if self.area_preview(before, preview.action.clone())? != *preview {
                    return Err(invalid(
                        "Items changed before publication. Review a fresh preview.",
                    ));
                }
                Ok(())
            },
        )?;
        proof_boundary(&self.root, "published")?;
        Ok(projects)
    }
    pub fn area_receipts(&self) -> io::Result<BTreeMap<String, Receipt>> {
        self.load_projects()?.area_receipts()
    }
    pub fn undo_area(&self, id: &str) -> io::Result<Projects> {
        let parts: Vec<_> = id.split('-').collect();
        if parts.len() != 3
            || !super::device::valid_id(parts[1])
            || !super::device::valid_id(parts[2])
        {
            return Err(invalid("Invalid area operation. Reload before Undo."));
        }
        let projects = self.load_projects()?;
        let dependencies = self.area_inventory(&projects)?.0;
        sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                let mut receipt: Receipt = read_entries(p, "area-operation:")?
                    .remove(id)
                    .filter(|r: &Receipt| r.active)
                    .ok_or_else(|| invalid("This operation changed. Reload before Undo."))?;
                let backup = self
                    .root
                    .join("local/area-operations")
                    .join(id)
                    .join("backup.json");
                if receipt.id != id
                    || receipt.device != super::DeviceSettings::read_in(&self.root)?.id
                    || receipt.backup != backup
                {
                    return Err(invalid(
                        "This backup belongs to another device. Undo was not applied.",
                    ));
                }
                let snapshot: Backup = serde_json::from_slice(&fs::read(&backup)?)?;
                if digest(&snapshot)? != receipt.checksum {
                    return Err(invalid("The backup changed. Undo was not applied."));
                }
                verify_backup(&backup, &snapshot)?;
                let raw = self.undo_state(p)?.1;
                for (key, expected) in &receipt.baseline {
                    let current = if let Some(key) = key.strip_prefix("link:") {
                        p.other
                            .get(&format!("area-link:{key}"))
                            .cloned()
                            .unwrap_or(Value::Null)
                    } else {
                        raw.get(key).cloned().unwrap_or(Value::Null)
                    };
                    if current != *expected {
                        return Err(invalid(
                            "Some items were moved later. Undo was not applied. Keep the change or move them back by hand.",
                        ));
                    }
                }
                if receipt
                    .after
                    .iter()
                    .any(|(key, value)| p.other.get(key) != value.as_ref())
                    || receipt.project_baseline.iter().any(|(key, value)| {
                        (if key == "$aliases" {
                            json!(p.aliases)
                        } else {
                            p.other.get(key).cloned().unwrap_or(Value::Null)
                        }) != *value
                    })
                {
                    return Err(invalid(
                        "An area or project changed later. Undo was not applied.",
                    ));
                }
                for key in receipt.after.keys() {
                    match snapshot.projects.other.get(key) {
                        Some(v) => {
                            p.other.insert(key.clone(), v.clone());
                        }
                        None => {
                            p.other.remove(key);
                        }
                    }
                }
                receipt.active = false;
                put(p, "area-operation:", id, &receipt)?;
                p.area_catalog()?;
                Ok(())
            },
            |before| {
                proof_boundary(&self.root, "undo-publication")?;
                if self.area_inventory(before)?.0 != dependencies {
                    return Err(invalid("Items changed before Undo. Reload and try again."));
                }
                Ok(())
            },
        )
    }
}
fn verify_backup(path: &Path, expected: &Backup) -> io::Result<()> {
    let actual: Backup = serde_json::from_slice(&fs::read(path)?)?;
    if digest(&actual)? != digest(expected)? {
        return Err(invalid("Backup verification failed"));
    }
    for bytes in actual.files.values() {
        let _: Value =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))?;
    }
    Ok(())
}
/// Debug-only crash and peer-edit rendezvous, confined to the synthetic proof folder.
pub fn proof_boundary(root: &Path, stage: &str) -> io::Result<()> {
    if !cfg!(debug_assertions)
        || !root.starts_with(Path::new("E:/Personal-Projects/reviews/areas-proof"))
    {
        return Ok(());
    }
    if std::env::var("SIDENOTE_AREA_CRASH").ok().as_deref() == Some(stage) {
        std::process::exit(86);
    }
    let gate = root.join(format!("local/area-{stage}.pause"));
    if !gate.try_exists()? {
        return Ok(());
    }
    fs::write(gate.with_extension("reached"), b"ready")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while gate.try_exists()? {
        if std::time::Instant::now() >= deadline {
            return Err(invalid("Area proof rendezvous timed out"));
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    Ok(())
}
