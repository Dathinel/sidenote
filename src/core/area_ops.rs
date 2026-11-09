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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assignments: BTreeMap<String, Option<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Action {
    ResolveName {
        project_id: String,
        name: String,
    },
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
#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub references: Vec<Reference>,
    pub by_project: BTreeMap<String, Vec<usize>>,
    pub counts: [usize; 4],
    pub project_counts: BTreeMap<String, [usize; 4]>,
}
impl Inventory {
    pub fn new(references: Vec<Reference>) -> Self {
        let mut by_project: BTreeMap<String, Vec<usize>> = Default::default();
        let mut counts = [0; 4];
        let mut project_counts: BTreeMap<String, [usize; 4]> = Default::default();
        for (i, reference) in references.iter().enumerate() {
            let kind = match reference.kind.as_str() {
                "meeting" => 0,
                "task" => 1,
                "memory" => 2,
                _ => 3,
            };
            counts[kind] += 1;
            if let Some(id) = &reference.project_id {
                by_project.entry(id.clone()).or_default().push(i);
                project_counts.entry(id.clone()).or_default()[kind] += 1;
            }
        }
        Self {
            references,
            by_project,
            counts,
            project_counts,
        }
    }
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Backup {
    projects: Projects,
    preview: Preview,
    files: BTreeMap<PathBuf, FileCopy>,
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum FileCopy {
    Text(String),
    LegacyBytes(Vec<u8>),
}
impl FileCopy {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::LegacyBytes(bytes) => bytes,
        }
    }
}
fn invalid(message: &str) -> io::Error {
    store_io::invalid(message)
}
fn area_for_name(p: &mut Projects, name: &str) -> io::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name == super::UNSORTED {
        return Err(invalid("Enter an area name of 1 to 80 characters"));
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
    if let Some(area) = c.areas.values().find(|a| {
        a.status == Status::Retired
            && std::iter::once(&a.name)
                .chain(a.aliases.iter())
                .any(|n| project_merge::normalized_name(n) == project_merge::normalized_name(name))
    }) {
        let mut area = area.clone();
        area.status = Status::Active;
        put(p, "area:", &area.id, &area)?;
        return Ok(area.id);
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
            schema_version: 1,
            id: id.clone(),
            name: name.into(),
            aliases: Default::default(),
            status: Status::Active,
            extra: Default::default(),
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
    if children.keys().any(|key| {
        !refs
            .iter()
            .any(|r| r.key == *key && r.project_id.as_deref() == Some(id))
    }) {
        return Err(invalid(
            "A conversion choice no longer matches this project",
        ));
    }
    for reference in refs.iter().filter(|r| r.project_id.as_deref() == Some(id)) {
        let child = children.get(&reference.key);
        let target = Target {
            area_id: area.into(),
            project_id: child.cloned(),
        };
        if child.is_some_and(|child| child == id) {
            return Err(invalid(
                "Choose a child project, not the project being converted",
            ));
        }
        c.validate_target(&target)?;
        put(
            p,
            "area-link:",
            &reference.key,
            &Link {
                schema_version: 1,
                target,
                baseline: reference.baseline.clone(),
                extra: Default::default(),
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
    p.area_catalog()?.ensure_writable()?;
    let mut next = p.clone();
    next.retain_project_identity()?;
    match action {
        Action::ResolveName { project_id, name } => {
            let catalog = next.area_catalog()?;
            let choices = catalog
                .name_conflicts
                .get(project_id)
                .ok_or_else(|| invalid("This rename was already resolved. Reload."))?;
            if !choices.contains(name) {
                return Err(invalid("Choose one of the conflicting project names"));
            }
            let mut project = catalog.projects[project_id].clone();
            project.name = name.clone();
            next.aliases.remove(name);
            for candidate in choices.iter().filter(|n| *n != name) {
                next.aliases.insert(candidate.clone(), name.clone());
            }
            put(&mut next, "area-project:", project_id, &project)?;
        }
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
            if name.trim().is_empty()
                || name.chars().count() > 80
                || c.area_matches(name).iter().any(|a| a.id != *area_id)
            {
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
            let converted: std::collections::BTreeSet<_> = rows
                .iter()
                .filter(|r| r.convert)
                .map(|r| r.project_id.as_str())
                .collect();
            if rows.iter().any(|r| {
                r.children
                    .values()
                    .any(|child| converted.contains(child.as_str()))
                    || r.assignments
                        .values()
                        .flatten()
                        .any(|child| converted.contains(child.as_str()))
            }) {
                return Err(invalid(
                    "A selected child project is also being turned into an area. Keep it as a project or change the per-item choice.",
                ));
            }
            // Establish all parents before resolving any child-project choices.
            for row in rows {
                let area = area_for_name(&mut next, &row.area_name)?;
                move_project(&mut next, &row.project_id, &area)?;
            }
            for row in rows.iter().filter(|r| r.convert) {
                let area = area_for_name(&mut next, &row.area_name)?;
                let mut children = row.children.clone();
                for (key, target) in &row.assignments {
                    if let Some(target) = target {
                        children.insert(key.clone(), target.clone());
                    } else {
                        children.remove(key);
                    }
                }
                convert(&mut next, &row.project_id, &area, &children, refs)?;
            }
            for row in rows.iter().filter(|r| !r.convert) {
                let area = area_for_name(&mut next, &row.area_name)?;
                for (key, target) in &row.assignments {
                    let reference = refs
                        .iter()
                        .find(|r| r.key == *key && r.project_id.as_ref() == Some(&row.project_id))
                        .ok_or_else(|| {
                            invalid("An item choice no longer belongs to this project")
                        })?;
                    let target = Target {
                        area_id: area.clone(),
                        project_id: target.clone(),
                    };
                    next.area_catalog()?.validate_target(&target)?;
                    put(
                        &mut next,
                        "area-link:",
                        key,
                        &Link {
                            schema_version: 1,
                            target,
                            baseline: reference.baseline.clone(),
                            extra: Default::default(),
                        },
                    )?;
                }
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
                    schema_version: 1,
                    target: target.clone(),
                    baseline: reference.baseline.clone(),
                    extra: Default::default(),
                },
            )?;
        }
    }
    next.area_catalog()?;
    Ok(next)
}
impl Projects {
    pub fn area_receipts(&self) -> io::Result<BTreeMap<String, Receipt>> {
        Ok(self
            .other
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix("area-operation:").and_then(|id| {
                    serde_json::from_value::<Receipt>(value.clone())
                        .ok()
                        .filter(|r| {
                            valid_receipt_id(id, &r.device)
                                && r.id == id
                                && r.backup == PathBuf::from(id).join("backup.json")
                                && valid_item("area-operation:", id, value)
                        })
                        .map(|receipt| (id.into(), receipt))
                })
            })
            .collect())
    }
}
impl TaskStore {
    pub fn area_references(&self) -> io::Result<Inventory> {
        self.area_inventory(&self.load_projects()?)
            .map(|(_, refs)| Inventory::new(refs))
    }
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
            let project = match target {
                Some(target) => target.project_id.clone(),
                None => (|| {
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
                })(),
            };
            push(
                meeting_key(&m.id),
                "meeting",
                m.title.clone(),
                project,
                target.map(|t| t.area_id.clone()),
            );
        }
        for (kind, id, text, project, assignment) in memory
            .records
            .iter()
            .map(|r| {
                (
                    "memory",
                    &r.id,
                    r.text.as_deref().unwrap_or("Retained record"),
                    r.project.as_deref(),
                    r.assignment.as_ref(),
                )
            })
            .chain(memory.proposals.iter().map(|r| {
                (
                    "proposal",
                    &r.proposal.id,
                    r.proposal.text.as_str(),
                    r.proposal.project.as_deref(),
                    None,
                )
            }))
        {
            let key = json!([kind, id]).to_string();
            let target = c.resolve_target(
                p,
                &key,
                &raw[&key],
                assignment.map(Assignment::Target),
                project.unwrap_or(super::UNSORTED),
            );
            let project = match target.as_ref() {
                Some(target) => target.project_id.clone(),
                None => project
                    .and_then(|n| c.project(p, n))
                    .filter(|p| p.converted_to.is_none())
                    .map(|p| p.id.clone()),
            };
            push(
                key,
                kind,
                text.into(),
                project,
                target.and_then(|t| (!t.area_id.is_empty()).then_some(t.area_id)),
            );
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
        super::area_version::ensure(&self.root)?;
        let (fingerprint, refs) = self.area_inventory(p)?;
        let next = simulate(p, &action, &refs)?;
        let keys: std::collections::BTreeSet<_> =
            p.other.keys().chain(next.other.keys()).cloned().collect();
        let mut changes: BTreeMap<_, _> = keys
            .into_iter()
            .filter(|k| p.other.get(k) != next.other.get(k))
            .map(|k| (k.clone(), next.other.get(&k).cloned()))
            .collect();
        for name in p.aliases.keys().chain(next.aliases.keys()) {
            if p.aliases.get(name) != next.aliases.get(name) {
                changes.insert(
                    format!("$alias:{name}"),
                    next.aliases.get(name).map(|v| json!(v)),
                );
            }
        }
        let c = p.area_catalog()?;
        let affected = |r: &Reference| match &action {
            Action::Create { name } => c.areas.values().any(|a| {
                a.status == Status::Retired
                    && project_merge::normalized_name(&a.name)
                        == project_merge::normalized_name(name)
                    && r.area_id.as_ref() == Some(&a.id)
            }),
            Action::Rename { area_id, .. } => r.area_id.as_ref() == Some(area_id),
            Action::ResolveName { project_id, .. }
            | Action::Move { project_id, .. }
            | Action::Convert { project_id, .. } => r.project_id.as_ref() == Some(project_id),
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
        super::area_version::ensure(&self.root)?;
        let original = self.load_projects()?;
        if self.area_preview(&original, preview.action.clone())? != *preview {
            return Err(invalid("Items changed. Review a fresh preview."));
        }
        let mut next = original.clone();
        for (key, value) in &preview.changes {
            if let Some(name) = key.strip_prefix("$alias:") {
                if let Some(value) = value.as_ref().and_then(Value::as_str) {
                    next.aliases.insert(name.into(), value.into());
                } else {
                    next.aliases.remove(name);
                }
                continue;
            }
            match value {
                Some(v) => {
                    next.other.insert(key.clone(), v.clone());
                }
                None => {
                    next.other.remove(key);
                }
            }
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
        store_io::reject_reparse(&backup)?;
        let snapshot = Backup {
            projects: original.clone(),
            preview: preview.clone(),
            files: project_merge::project_files(&self.root)?
                .into_iter()
                .map(|(path, bytes)| {
                    String::from_utf8(bytes)
                        .map(|text| (path, FileCopy::Text(text)))
                        .map_err(|_| {
                            invalid("A project input is not readable UTF-8. It was kept unchanged.")
                        })
                })
                .collect::<io::Result<_>>()?,
        };
        let receipt = Receipt {
            schema_version: 1,
            device: super::DeviceSettings::read_in(&self.root)?.id,
            id: id.clone(),
            action: preview.action.clone(),
            backup: PathBuf::from(&id).join("backup.json"),
            checksum: digest(&snapshot)?,
            after: preview.changes.clone(),
            baseline: preview
                .references
                .iter()
                .map(|r| (r.key.clone(), r.baseline.clone()))
                .collect(),
            project_baseline: Default::default(),
            active: true,
            skipped: Vec::new(),
        };
        put(&mut next, "area-operation:", &id, &receipt)?;
        if self.area_preview(&self.load_projects()?, preview.action.clone())? != *preview {
            return Err(invalid(
                "Items changed before publication. Review a fresh preview.",
            ));
        }
        let stamps = dependency_stamps(&self.root)?;
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(invalid(
                "This backup exceeds the 16 MB limit. Migrate fewer projects at a time.",
            ));
        }
        let result = (|| {
            store_io::atomic_write(&backup, &bytes)?;
            verify_backup(&backup, &snapshot)?;
            proof_boundary(&self.root, "backup")?;
            proof_boundary(&self.root, "staged")?;
            sync_state::update_checked_validated(
                &self.root,
                "projects.json",
                |p: &mut Projects| {
                    if serde_json::to_value(&*p)? != serde_json::to_value(&original)? {
                        return Err(invalid("Items changed. Review a fresh preview."));
                    }
                    *p = next;
                    Ok(())
                },
                |_| {
                    proof_boundary(&self.root, "publication")?;
                    if dependency_stamps(&self.root)? != stamps {
                        return Err(invalid(
                            "Items changed before publication. Review a fresh preview.",
                        ));
                    }
                    Ok(())
                },
            )
        })();
        if result.is_err() {
            let _ = fs::remove_file(&backup);
            let _ = fs::remove_dir(
                backup
                    .parent()
                    .ok_or_else(|| invalid("Invalid backup folder"))?,
            );
        }
        let projects = result?;
        proof_boundary(&self.root, "published")?;
        let _ = cleanup_backups(&self.root, &projects);
        Ok(projects)
    }
    pub fn area_receipts(&self) -> io::Result<BTreeMap<String, Receipt>> {
        self.load_projects()?.area_receipts()
    }
    pub fn local_area_backup(&self, id: &str, receipt: &Receipt) -> io::Result<PathBuf> {
        let own = super::DeviceSettings::read_in(&self.root)?.id;
        if sync_state::writer(
            &self.root,
            "projects.json",
            vec![format!("area-operation:{id}")],
        )?
        .as_deref()
            != Some(own.as_str())
        {
            return Err(invalid(
                "This change was done on another device. Undo is available there.",
            ));
        }
        receipt_backup(&self.root, id, receipt, &own)
    }
    pub fn undo_area(&self, id: &str) -> io::Result<Projects> {
        super::area_version::ensure(&self.root)?;
        let original = self.load_projects()?;
        let mut receipt: Receipt = serde_json::from_value(
            original
                .other
                .get(&format!("area-operation:{id}"))
                .cloned()
                .ok_or_else(|| invalid("This operation changed. Reload before Undo."))?,
        )?;
        if !receipt.active {
            return Err(invalid("This operation was already undone."));
        }
        if receipt.schema_version != 1 {
            return Err(invalid(
                "This area operation uses a newer schema. Update the app before Undo.",
            ));
        }
        let backup = self.local_area_backup(id, &receipt)?;
        let snapshot: Backup =
            serde_json::from_slice(&store_io::read_bounded(&backup, 16 * 1024 * 1024)?)?;
        if digest(&snapshot)? != receipt.checksum {
            return Err(invalid("The backup changed. Undo was not applied."));
        }
        verify_backup(&backup, &snapshot)?;
        if receipt.after != snapshot.preview.changes || receipt.action != snapshot.preview.action {
            return Err(invalid("The Undo record changed. Undo was not applied."));
        }
        let operation_baseline: BTreeMap<_, _> = snapshot
            .preview
            .references
            .iter()
            .map(|r| (r.key.clone(), r.baseline.clone()))
            .collect();
        original.area_catalog()?.ensure_writable()?;
        let (_, refs) = self.area_inventory(&original)?;
        let raw: BTreeMap<_, _> = refs
            .iter()
            .map(|r| (r.key.clone(), r.baseline.clone()))
            .collect();
        let mut next = original.clone();
        for (key, written) in &receipt.after {
            if key.starts_with("area-identity:") {
                continue;
            }
            let reference = key.strip_prefix("area-link:");
            let current = key
                .strip_prefix("$alias:")
                .map(|name| original.aliases.get(name).map(|v| json!(v)))
                .unwrap_or_else(|| original.other.get(key).cloned());
            let changed = current != *written
                || reference.is_some_and(|r| operation_baseline.get(r) != raw.get(r));
            if changed {
                receipt.skipped.push(
                    reference
                        .and_then(|r| {
                            refs.iter()
                                .chain(snapshot.preview.references.iter())
                                .find(|item| item.key == r)
                        })
                        .map(|r| r.label.clone())
                        .unwrap_or_else(|| register_label(&original, key)),
                );
                continue;
            }
            if let Some(name) = key.strip_prefix("$alias:") {
                if let Some(value) = snapshot.projects.aliases.get(name) {
                    next.aliases.insert(name.into(), value.clone());
                } else {
                    next.aliases.remove(name);
                }
                continue;
            }
            match snapshot.projects.other.get(key) {
                Some(value) => {
                    next.other.insert(key.clone(), value.clone());
                }
                None if key.starts_with("area:") => {
                    if let Some(value) = next.other.get_mut(key).and_then(Value::as_object_mut) {
                        value.insert("status".into(), json!("retired"));
                    }
                }
                None => {
                    next.other.remove(key);
                }
            }
        }
        receipt.skipped.sort();
        receipt.active = false;
        put(&mut next, "area-operation:", id, &receipt)?;
        let before_check = self.area_inventory(&original)?.0;
        let stamps = dependency_stamps(&self.root)?;
        if before_check != self.area_inventory(&self.load_projects()?)?.0 {
            return Err(invalid("Items changed before Undo. Reload and try again."));
        }
        proof_boundary(&self.root, "undo-staged")?;
        let result = sync_state::update_checked_validated(
            &self.root,
            "projects.json",
            |p: &mut Projects| {
                if serde_json::to_value(&*p)? != serde_json::to_value(&original)? {
                    return Err(invalid("Items changed before Undo. Reload and try again."));
                }
                *p = next;
                Ok(())
            },
            |_| {
                proof_boundary(&self.root, "undo-publication")?;
                if dependency_stamps(&self.root)? != stamps {
                    return Err(invalid("Items changed before Undo. Reload and try again."));
                }
                Ok(())
            },
        )?;
        proof_boundary(&self.root, "undo-published")?;
        Ok(result)
    }
}
pub fn valid_receipt_id(id: &str, device: &str) -> bool {
    let parts: Vec<_> = id.split('-').collect();
    parts.len() == 3
        && parts[1] == device
        && super::device::valid_id(parts[1])
        && super::device::valid_id(parts[2])
        && parts[0].len() == 15
        && chrono::NaiveDateTime::parse_from_str(parts[0], "%Y%m%dT%H%M%S")
            .is_ok_and(|t| t.format("%Y%m%dT%H%M%S").to_string() == parts[0])
}
pub fn receipt_backup(root: &Path, id: &str, receipt: &Receipt, own: &str) -> io::Result<PathBuf> {
    let relative = PathBuf::from(id).join("backup.json");
    if receipt.device != own
        || receipt.id != id
        || !valid_receipt_id(id, own)
        || receipt.backup != relative
    {
        return Err(invalid(
            "This backup belongs to another device or has an invalid location. Undo was not applied.",
        ));
    }
    let base = root.join("local/area-operations");
    let path = base.join(relative);
    store_io::reject_reparse(&path)?;
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(base.canonicalize()?) {
        return Err(invalid("Invalid Undo backup location"));
    }
    Ok(path)
}
fn register_label(p: &Projects, key: &str) -> String {
    if let Some(name) = key.strip_prefix("$alias:") {
        return name.into();
    }
    p.other
        .get(key)
        .and_then(|v| v.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Area assignment")
        .to_string()
}
fn dependency_stamps(root: &Path) -> io::Result<BTreeMap<PathBuf, (u64, std::time::SystemTime)>> {
    fn visit(
        path: &Path,
        out: &mut BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
    ) -> io::Result<()> {
        let entries = match fs::read_dir(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(&entry.path(), out)?;
            } else if entry.path().extension().is_some_and(|e| e == "json") {
                let meta = entry.metadata()?;
                out.insert(entry.path(), (meta.len(), meta.modified()?));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    for path in [
        root.join("state"),
        super::meetings_dir(),
        super::peers_dir(),
    ] {
        visit(&path, &mut out)?;
    }
    Ok(out)
}
fn cleanup_backups(root: &Path, p: &Projects) -> io::Result<()> {
    let own = super::DeviceSettings::read_in(root)?.id;
    let receipts = p.area_receipts()?;
    let base = root.join("local/area-operations");
    store_io::reject_reparse(&base)?;
    let mut folders = Vec::new();
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !valid_receipt_id(&id, &own) || !entry.file_type()?.is_dir() {
            continue;
        }
        store_io::reject_reparse(&entry.path())?;
        folders.push((entry.metadata()?.modified()?, id, entry.path()));
    }
    folders.sort();
    let keep: std::collections::BTreeSet<_> = folders
        .iter()
        .rev()
        .filter(|(_, id, _)| receipts.contains_key(id))
        .take(10)
        .map(|(_, id, _)| id.clone())
        .collect();
    for (_, id, path) in folders {
        let orphan_old = !receipts.contains_key(&id)
            && path
                .metadata()?
                .modified()?
                .elapsed()
                .is_ok_and(|age| age > std::time::Duration::from_secs(3600));
        if (!keep.contains(&id) && receipts.contains_key(&id)) || orphan_old {
            let _ = fs::remove_file(path.join("backup.json"));
            let _ = fs::remove_dir(path);
        }
    }
    Ok(())
}
fn verify_backup(path: &Path, expected: &Backup) -> io::Result<()> {
    let actual: Backup = serde_json::from_slice(&store_io::read_bounded(path, 16 * 1024 * 1024)?)?;
    if digest(&actual)? != digest(expected)? {
        return Err(invalid("Backup verification failed"));
    }
    for file in actual.files.values() {
        let bytes = file.bytes();
        let _: Value =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))?;
    }
    Ok(())
}
/// Debug-only crash and peer-edit rendezvous, confined to the synthetic proof folder.
pub fn proof_boundary(root: &Path, stage: &str) -> io::Result<()> {
    if !cfg!(debug_assertions) || !super::is_proof_path(root, "areas-proof") {
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
