//! Additive area registers live beside project registers. Legacy names and source bytes stay intact.
use super::{Meeting, Pipeline, Projects, TaskItem, project_merge::normalized_name, store_io};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Inactive,
    Retired,
}
pub fn schema_one() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectChoice {
    pub project: String,
    pub revision: String,
}
pub fn new_assignment_revision(root: &std::path::Path) -> io::Result<String> {
    super::sync_state::new_id(root)
}
pub fn assignment_baseline(legacy: Value, assignment: Option<&impl Serialize>) -> Value {
    match assignment {
        Some(intent) => json!({"legacy":legacy,"assignment":intent}),
        None => legacy,
    }
}
pub fn meeting_baseline(p: &Projects, pipeline: &Pipeline, id: &str) -> Value {
    assignment_baseline(
        json!([
            p.meeting_projects.get(id),
            pipeline.get(id).and_then(|f| f.meeting_project.as_ref())
        ]),
        p.other.get(&format!("assignment:{}", meeting_key(id))),
    )
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Area {
    #[serde(default = "schema_one")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub aliases: BTreeSet<String>,
    pub status: Status,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    #[serde(default = "schema_one")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub area_id: Option<String>,
    pub status: Status,
    pub converted_to: Option<String>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    pub area_id: String,
    pub project_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyAlias {
    Area { area_id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    #[serde(default = "schema_one")]
    pub schema_version: u32,
    pub target: Target,
    pub baseline: Value,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub areas: BTreeMap<String, Area>,
    pub projects: BTreeMap<String, Project>,
    pub legacy_aliases: BTreeMap<String, LegacyAlias>,
    pub links: BTreeMap<String, Link>,
    pub problems: Vec<String>,
    pub read_only: bool,
    pub redirects: BTreeMap<String, String>,
    pub name_conflicts: BTreeMap<String, BTreeSet<String>>,
    pub quarantined: usize,
    pub unreadable_receipts: usize,
}
fn read_catalog_entries<T: DeserializeOwned>(
    p: &Projects,
    prefix: &str,
    problems: &mut Vec<String>,
    read_only: &mut bool,
) -> BTreeMap<String, T> {
    let mut entries = BTreeMap::new();
    for (key, value) in &p.other {
        let Some(id) = key.strip_prefix(prefix) else {
            continue;
        };
        if value
            .get("schema_version")
            .and_then(Value::as_u64)
            .is_some_and(|v| v > 1)
        {
            *read_only = true;
            problems.push("Some area data uses a newer schema and is read-only.".into());
        }
        if !valid_item(prefix, id, value) {
            problems.push("Could not read an area item".into());
            continue;
        }
        match serde_json::from_value(value.clone()) {
            Ok(value) => {
                entries.insert(id.into(), value);
            }
            Err(_) => {
                problems.push("Could not read an area item".into());
            }
        }
    }
    entries
}
pub(super) fn put(
    p: &mut Projects,
    prefix: &str,
    id: &str,
    value: &impl Serialize,
) -> io::Result<()> {
    let key = format!("{prefix}{id}");
    if p.other
        .get(&key)
        .and_then(|v| v.get("schema_version"))
        .and_then(Value::as_u64)
        .is_some_and(|v| v > 1)
    {
        return Err(store_io::invalid(
            "This area data comes from a newer version. Update the app before changing it.",
        ));
    }
    let mut next = serde_json::to_value(value)?;
    if let Some(old) = p.other.get(&key) {
        retain_unknown(prefix, old, &mut next)?;
    }
    p.other.insert(key, next);
    Ok(())
}
const EXTRA_CAP: usize = 64 * 1024;
fn known_fields(prefix: &str) -> &'static [&'static str] {
    match prefix {
        "area:" => &["schema_version", "id", "name", "aliases", "status"],
        "area-project:" => &[
            "schema_version",
            "id",
            "name",
            "area_id",
            "status",
            "converted_to",
        ],
        "area-link:" => &["schema_version", "target", "baseline"],
        "area-alias:" => &["kind", "area_id"],
        "area-operation:" => &[
            "schema_version",
            "device",
            "id",
            "action",
            "backup",
            "checksum",
            "after",
            "baseline",
            "project_baseline",
            "active",
            "skipped",
        ],
        _ => &[],
    }
}
pub(super) fn valid_item(prefix: &str, id: &str, value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    if matches!(prefix, "area:" | "area-project:")
        && (value["id"].as_str() != Some(id)
            || value["name"].as_str().is_none_or(|n| {
                n.trim().is_empty() || (prefix == "area:" && n.chars().count() > 80)
            }))
    {
        return false;
    }
    if prefix == "area-link:"
        && value["target"]["area_id"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        return false;
    }
    let extra: BTreeMap<_, _> = obj
        .iter()
        .filter(|(k, _)| !known_fields(prefix).contains(&k.as_str()))
        .collect();
    serde_json::to_vec(&extra).is_ok_and(|b| b.len() <= EXTRA_CAP)
}
fn retain_unknown(prefix: &str, old: &Value, new: &mut Value) -> io::Result<()> {
    if let (Some(old), Some(new)) = (old.as_object(), new.as_object_mut()) {
        for (key, value) in old {
            if !known_fields(prefix).contains(&key.as_str()) && !new.contains_key(key) {
                new.insert(key.clone(), value.clone());
            }
        }
        let extra: BTreeMap<_, _> = new
            .iter()
            .filter(|(k, _)| !known_fields(prefix).contains(&k.as_str()))
            .collect();
        if serde_json::to_vec(&extra)?.len() > EXTRA_CAP {
            return Err(store_io::invalid("Extra area fields exceed 64 KB"));
        }
    }
    Ok(())
}
pub fn task_key(t: &TaskItem) -> String {
    if t.manual {
        json!(["manual", t.meeting_id]).to_string()
    } else {
        json!(["task", t.meeting_id, t.index, t.task]).to_string()
    }
}
pub fn meeting_key(id: &str) -> String {
    json!(["meeting", id]).to_string()
}
pub fn task_baseline(t: &TaskItem, pipeline: &Pipeline, overrides: &super::Overrides) -> Value {
    if t.manual {
        let task = overrides
            .manual
            .iter()
            .find(|m| format!("manual/{}", m.id) == t.meeting_id);
        return assignment_baseline(
            json!(task.and_then(|m| m.project.as_ref())),
            task.and_then(|m| m.assignment.as_ref()),
        );
    }
    let edit = overrides.get(&t.meeting_id, t.index);
    assignment_baseline(
        json!([
            pipeline
                .get(&t.meeting_id)
                .and_then(|f| f.tasks.iter().find(|tag| tag.i == t.index))
                .map(|tag| &tag.project),
            edit.and_then(|o| o.project.as_ref()),
            edit.filter(|o| o.project.is_some())
                .and_then(|o| o.task.as_ref()),
            pipeline
                .get(&t.meeting_id)
                .and_then(|f| f.meeting_project.as_ref())
        ]),
        edit.and_then(|e| e.assignment.as_ref()),
    )
}
pub fn project_id(name: &str) -> String {
    format!("project-{}", blake3::hash(name.as_bytes()).to_hex())
}
impl Projects {
    pub(super) fn prepare_area_merge(&mut self, from: &str, into: &str) -> io::Result<()> {
        let c = self.area_catalog()?;
        c.ensure_writable()?;
        if c.areas.is_empty() && c.links.is_empty() {
            return Ok(());
        }
        let source = c
            .project(self, from)
            .ok_or_else(|| store_io::invalid("Source project is missing"))?;
        let mut target = c
            .project(self, into)
            .cloned()
            .ok_or_else(|| store_io::invalid("Target project is missing"))?;
        if source.area_id.is_some() && target.area_id.is_some() && source.area_id != target.area_id
        {
            return Err(store_io::invalid(
                "These projects belong to different areas. Move them into the same area before merging.",
            ));
        }
        if target.area_id.is_none() && source.area_id.is_some() {
            target.area_id = source.area_id.clone();
            put(self, "area-project:", &target.id, &target)?;
        }
        for (key, link) in &c.links {
            if link.target.project_id.as_ref() == Some(&source.id) {
                let mut link = link.clone();
                link.target.project_id = Some(target.id.clone());
                if let Some(area) = &target.area_id {
                    link.target.area_id = area.clone();
                }
                put(self, "area-link:", key, &link)?;
            }
        }
        self.retain_project_identity()
    }
    pub fn is_area_alias(&self, name: &str) -> bool {
        let normalized = normalized_name(name);
        self.other
            .keys()
            .filter_map(|k| k.strip_prefix("area-alias:"))
            .any(|n| normalized_name(n) == normalized)
    }
    pub fn area_catalog(&self) -> io::Result<Catalog> {
        let mut catalog = Catalog::default();
        catalog.areas =
            read_catalog_entries(self, "area:", &mut catalog.problems, &mut catalog.read_only);
        catalog.projects = read_catalog_entries(
            self,
            "area-project:",
            &mut catalog.problems,
            &mut catalog.read_only,
        );
        catalog.legacy_aliases = read_catalog_entries(
            self,
            "area-alias:",
            &mut catalog.problems,
            &mut catalog.read_only,
        );
        catalog.links = read_catalog_entries(
            self,
            "area-link:",
            &mut catalog.problems,
            &mut catalog.read_only,
        );
        let identities = catalog.projects.clone();
        catalog.unreadable_receipts = self
            .other
            .keys()
            .filter(|k| k.starts_with("area-operation:"))
            .count()
            - self.area_receipts()?.len();
        catalog.quarantined = catalog
            .problems
            .iter()
            .filter(|p| p.as_str() == "Could not read an area item")
            .count();
        catalog
            .problems
            .retain(|p| p != "Could not read an area item");
        if catalog.quarantined > 0 {
            catalog
                .problems
                .push(format!("Could not read {} items", catalog.quarantined));
        }
        if catalog.unreadable_receipts > 0 {
            catalog.problems.push(format!(
                "Could not read {} undo records",
                catalog.unreadable_receipts
            ));
        }
        for p in catalog.projects.values_mut() {
            p.name = self.resolve(&p.name);
        }
        for (key, value) in &self.other {
            let Some(name) = key.strip_prefix("area-identity:") else {
                continue;
            };
            let Some(id) = value.as_str() else { continue };
            let name = self.resolve(name);
            if let Some(project) = catalog.projects.get(id) {
                if project.name != name {
                    let choices = catalog.name_conflicts.entry(id.into()).or_default();
                    choices.insert(project.name.clone());
                    choices.insert(name.clone());
                }
            } else if !catalog.projects.values().any(|p| p.name == name) {
                catalog.projects.insert(
                    id.into(),
                    Project {
                        schema_version: 1,
                        id: id.into(),
                        name,
                        area_id: None,
                        status: Status::Active,
                        converted_to: None,
                        extra: Default::default(),
                    },
                );
            }
        }
        for name in super::project_names(self, &[]) {
            if !catalog
                .projects
                .values()
                .any(|p| self.resolve(&p.name) == name)
            {
                let id = project_id(&name);
                catalog.projects.insert(
                    id.clone(),
                    Project {
                        schema_version: 1,
                        id,
                        name,
                        area_id: None,
                        status: Status::Active,
                        converted_to: None,
                        extra: Default::default(),
                    },
                );
            }
        }
        for p in catalog.projects.values_mut() {
            p.name = self.resolve(&p.name);
        }
        // A rename keeps its identity. A recorded merge chooses the target identity explicitly.
        let mut redirects = BTreeMap::new();
        for (from, merge) in self.merges.iter().filter(|(_, m)| m.active) {
            let name = self.resolve(&merge.into);
            let mut before_merge = self.clone();
            before_merge.aliases.remove(from);
            let survivor = identities
                .values()
                .find(|p| {
                    p.name == merge.into
                        || before_merge.resolve(&p.name) == before_merge.resolve(&merge.into)
                })
                .map(|p| p.id.clone())
                .or_else(|| {
                    self.other
                        .get(&format!("area-identity:{}", merge.into))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| project_id(&merge.into));
            let mut target = catalog.projects.get(&survivor).cloned().unwrap_or(Project {
                schema_version: 1,
                id: survivor.clone(),
                name: name.clone(),
                area_id: None,
                status: Status::Active,
                converted_to: None,
                extra: Default::default(),
            });
            for source in identities
                .values()
                .filter(|p| self.resolve(&p.name) == name && p.id != survivor)
            {
                if target.area_id.is_none() {
                    target.area_id = source.area_id.clone();
                }
                redirects.insert(source.id.clone(), survivor.clone());
            }
            catalog
                .projects
                .retain(|id, p| p.name != name || *id == survivor);
            catalog.projects.insert(survivor, target);
        }
        for link in catalog.links.values_mut() {
            if let Some(id) = link
                .target
                .project_id
                .as_ref()
                .and_then(|id| redirects.get(id))
            {
                link.target.project_id = Some(id.clone());
            }
            if let Some(id) = &link.target.project_id
                && !catalog.projects.contains_key(id)
                && let Some(old) = identities.get(id)
                && let Some(project) = catalog
                    .projects
                    .values()
                    .find(|p| p.name == self.resolve(&old.name))
            {
                link.target.project_id = Some(project.id.clone());
            }
        }
        catalog.redirects = redirects;
        if !catalog.name_conflicts.is_empty() {
            catalog
                .problems
                .push("Renamed on both laptops. Pick the project name to keep.".into());
        }
        catalog.repair_missing();
        Ok(catalog)
    }
    pub fn project_area_label(&self, name: &str) -> String {
        let Ok(c) = self.area_catalog() else {
            return name.into();
        };
        if let Some(project) = c.project(self, name)
            && let Some(id) = project.converted_to.as_ref().or(project.area_id.as_ref())
            && let Some(area) = c.areas.get(id)
        {
            return if project.converted_to.is_some() {
                area.name.clone()
            } else {
                format!("{} › {}", area.name, project.name)
            };
        }
        name.into()
    }
    pub(super) fn retain_project_identity(&mut self) -> io::Result<()> {
        let catalog = self.area_catalog()?;
        catalog.ensure_writable()?;
        for project in catalog.projects.values() {
            self.other
                .entry(format!("area-identity:{}", project.name))
                .or_insert_with(|| Value::String(project.id.clone()));
        }
        Ok(())
    }
}
impl Catalog {
    pub fn ensure_writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(store_io::invalid(
                "Area data from a newer version is read-only. Update the app before changing it.",
            ));
        }
        Ok(())
    }
    fn repair_missing(&mut self) {
        for (id, project) in &mut self.projects {
            if project.id != *id {
                project.id = id.clone();
                self.problems
                    .push("A project identity needs review. Its work is still available.".into());
            }
        }
        let required: BTreeSet<_> = self
            .projects
            .values()
            .flat_map(|p| p.area_id.iter().chain(p.converted_to.iter()))
            .chain(self.links.values().map(|l| &l.target.area_id))
            .chain(self.legacy_aliases.values().map(|a| {
                let LegacyAlias::Area { area_id } = a;
                area_id
            }))
            .cloned()
            .collect();
        for id in required {
            if !self.areas.contains_key(&id) {
                self.problems.push(
                    "Some work refers to a Missing area. The other areas are still available."
                        .into(),
                );
                self.areas.insert(
                    id.clone(),
                    Area {
                        schema_version: 1,
                        id,
                        name: "Missing area".into(),
                        aliases: Default::default(),
                        status: Status::Retired,
                        extra: Default::default(),
                    },
                );
            }
        }
        for (id, area) in &mut self.areas {
            if area.id != *id || area.name.trim().is_empty() {
                area.id = id.clone();
                area.name = "Missing area".into();
                self.problems
                    .push("An unreadable area is shown as Missing area.".into());
            }
        }
        for link in self.links.values() {
            if self
                .areas
                .get(&link.target.area_id)
                .is_some_and(|a| a.status == Status::Retired)
            {
                self.problems.push("Retired area has linked work from another device. Restore the area or reassign that work.".into());
            }
        }
        if self.projects.values().any(|p| {
            p.area_id.iter().chain(p.converted_to.iter()).any(|id| {
                self.areas
                    .get(id)
                    .is_some_and(|a| a.status == Status::Retired)
            })
        }) {
            self.problems.push("Retired area has linked work from another device. Restore the area or reassign that work.".into());
        }
        self.problems.sort();
        self.problems.dedup();
    }
    pub fn project<'a>(&'a self, p: &Projects, name: &str) -> Option<&'a Project> {
        self.projects
            .values()
            .find(|v| p.resolve(&v.name) == p.resolve(name))
    }
    pub fn area_matches(&self, name: &str) -> Vec<&Area> {
        let key = normalized_name(name);
        self.areas
            .values()
            .filter(|a| {
                if a.status == Status::Retired {
                    return false;
                }
                std::iter::once(&a.name)
                    .chain(a.aliases.iter())
                    .any(|n| normalized_name(n) == key)
            })
            .collect()
    }
    pub fn validate_target(&self, target: &Target) -> io::Result<()> {
        self.ensure_writable()?;
        if self
            .areas
            .get(&target.area_id)
            .is_none_or(|a| a.status == Status::Retired)
            || target.project_id.as_ref().is_some_and(|id| {
                self.projects.get(id).is_none_or(|p| {
                    p.converted_to.is_some() || p.area_id.as_ref() != Some(&target.area_id)
                })
            })
        {
            return Err(store_io::invalid(
                "Choose a project inside this area, or area-only work",
            ));
        }
        Ok(())
    }
    /// The only assignment precedence rule. Baselines include the complete retained intent.
    pub fn resolve_target(
        &self,
        p: &Projects,
        key: &str,
        baseline: &Value,
        retained: Option<Assignment<'_>>,
        raw_project: &str,
    ) -> Option<Target> {
        let target = if let Some(link) = self.links.get(key).filter(|l| &l.baseline == baseline) {
            Some(link.target.clone())
        } else {
            match retained {
                Some(Assignment::Target(target)) => Some(target.clone()),
                Some(Assignment::Project(name)) => self.named_target(p, name),
                None => self.named_target(p, raw_project),
            }
        }?;
        Some(self.current_target(&target))
    }
    fn named_target(&self, p: &Projects, name: &str) -> Option<Target> {
        if let Some(LegacyAlias::Area { area_id }) = self
            .legacy_aliases
            .iter()
            .find(|(n, _)| normalized_name(n) == normalized_name(name))
            .map(|(_, a)| a)
        {
            return Some(Target {
                area_id: area_id.clone(),
                project_id: None,
            });
        }
        let project = self.project(p, name)?;
        Some(Target {
            area_id: project.area_id.clone().unwrap_or_default(),
            project_id: Some(project.id.clone()),
        })
    }
    fn current_target(&self, target: &Target) -> Target {
        let mut target = target.clone();
        if let Some(mut id) = target.project_id.take() {
            let mut seen = BTreeSet::new();
            while seen.insert(id.clone()) {
                let Some(next) = self.redirects.get(&id) else {
                    break;
                };
                id = next.clone();
            }
            if let Some(project) = self.projects.get(&id) {
                if let Some(area) = &project.converted_to {
                    return Target {
                        area_id: area.clone(),
                        project_id: None,
                    };
                }
                target.area_id = project.area_id.clone().unwrap_or_default();
            }
            target.project_id = Some(id);
        }
        target
    }
    pub fn label(&self, target: &Target) -> String {
        let area = self
            .areas
            .get(&target.area_id)
            .map(|a| a.name.as_str())
            .unwrap_or("Missing area");
        target
            .project_id
            .as_ref()
            .and_then(|id| self.projects.get(id))
            .map_or_else(|| area.to_string(), |p| format!("{area} › {}", p.name))
    }
}

pub enum Assignment<'a> {
    Project(&'a str),
    Target(&'a Target),
}
#[derive(Clone, Debug, Default)]
pub struct Activity {
    pub projects: BTreeSet<String>,
    pub areas: BTreeSet<String>,
    pub meetings: BTreeMap<String, Target>,
}
/// Called by the area worker. Its inputs are an immutable UI snapshot, never shared mutable state.
pub fn activity(
    p: &Projects,
    meetings: &[Meeting],
    pipeline: &Pipeline,
    tasks: &[TaskItem],
    running_meetings: &BTreeSet<String>,
    now: chrono::DateTime<chrono::Utc>,
) -> io::Result<Activity> {
    let catalog = p.area_catalog()?;
    let mut out = Activity::default();
    let mut grouped: std::collections::HashMap<&str, Vec<&TaskItem>> = Default::default();
    for task in tasks.iter().filter(|t| !t.manual) {
        grouped.entry(&task.meeting_id).or_default().push(task);
    }
    for task in tasks.iter().filter(|t| !t.done) {
        if let Some(area) = &task.area_id {
            out.areas.insert(area.clone());
        }
        if let Some(project) = catalog
            .project(p, &task.project)
            .filter(|p| p.converted_to.is_none())
        {
            out.projects.insert(project.id.clone());
        }
    }
    for meeting in meetings {
        let file = pipeline.get(&meeting.id);
        let raw = p
            .meeting_projects
            .get(&meeting.id)
            .cloned()
            .or_else(|| {
                grouped
                    .get(meeting.id.as_str())
                    .and_then(|items| super::dominant_project(items.iter().copied()))
            })
            .or_else(|| file.and_then(|f| f.meeting_project.clone()));
        let baseline = meeting_baseline(p, pipeline, &meeting.id);
        if let Some(target) = catalog.resolve_target(
            p,
            &meeting_key(&meeting.id),
            &baseline,
            p.other
                .get(&format!("assignment:{}", meeting_key(&meeting.id)))
                .and_then(|v| v.get("project"))
                .and_then(Value::as_str)
                .map(Assignment::Project),
            raw.as_deref().unwrap_or(super::UNSORTED),
        ) {
            if meeting.started.is_some_and(|s| {
                s.with_timezone(&chrono::Utc) >= now - chrono::Duration::days(30)
                    && s.with_timezone(&chrono::Utc) <= now
            }) || running_meetings.contains(&meeting.id)
            {
                if let Some(id) = &target.project_id {
                    out.projects.insert(id.clone());
                }
                if target.project_id.is_some() || running_meetings.contains(&meeting.id) {
                    out.areas.insert(target.area_id.clone());
                }
            }
            if !target.area_id.is_empty() {
                out.meetings.insert(meeting.id.clone(), target);
            }
        } else if let Some(project) = raw.as_deref().and_then(|n| catalog.project(p, n))
            && (running_meetings.contains(&meeting.id)
                || meeting.started.is_some_and(|s| {
                    s.with_timezone(&chrono::Utc) >= now - chrono::Duration::days(30)
                        && s.with_timezone(&chrono::Utc) <= now
                }))
        {
            out.projects.insert(project.id.clone());
        }
    }
    for project in catalog
        .projects
        .values()
        .filter(|p| out.projects.contains(&p.id))
    {
        if let Some(id) = &project.area_id {
            out.areas.insert(id.clone());
        }
    }
    Ok(out)
}

pub fn running_meetings(root: &std::path::Path) -> io::Result<BTreeSet<String>> {
    let path = root.join("local/router-work");
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e),
    };
    let mut active = BTreeSet::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let marker = entry.path().join("active.json");
        let bytes = match std::fs::read(marker) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        if value["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .is_some_and(crate::platform::process_running)
        {
            active.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(active)
}
