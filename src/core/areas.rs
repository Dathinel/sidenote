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
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Area {
    pub id: String,
    pub name: String,
    pub aliases: BTreeSet<String>,
    pub status: Status,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub area_id: Option<String>,
    pub status: Status,
    pub converted_to: Option<String>,
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
    pub target: Target,
    pub baseline: Value,
}
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub areas: BTreeMap<String, Area>,
    pub projects: BTreeMap<String, Project>,
    pub legacy_aliases: BTreeMap<String, LegacyAlias>,
    pub links: BTreeMap<String, Link>,
}
pub(super) fn read_entries<T: DeserializeOwned>(
    p: &Projects,
    prefix: &str,
) -> io::Result<BTreeMap<String, T>> {
    p.other
        .iter()
        .filter_map(|(k, v)| k.strip_prefix(prefix).map(|id| (id, v)))
        .map(|(id, v)| Ok((id.into(), serde_json::from_value(v.clone())?)))
        .collect()
}
pub(super) fn put(
    p: &mut Projects,
    prefix: &str,
    id: &str,
    value: &impl Serialize,
) -> io::Result<()> {
    p.other
        .insert(format!("{prefix}{id}"), serde_json::to_value(value)?);
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
        return json!(
            overrides
                .manual
                .iter()
                .find(|m| format!("manual/{}", m.id) == t.meeting_id)
                .and_then(|m| m.project.as_ref())
        );
    }
    let edit = overrides.get(&t.meeting_id, t.index);
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
    ])
}
pub fn project_id(name: &str) -> String {
    format!("project-{}", blake3::hash(name.as_bytes()).to_hex())
}
impl Projects {
    pub fn is_area_alias(&self, name: &str) -> bool {
        let normalized = normalized_name(name);
        self.other
            .keys()
            .filter_map(|k| k.strip_prefix("area-alias:"))
            .any(|n| normalized_name(n) == normalized)
    }
    pub fn area_catalog(&self) -> io::Result<Catalog> {
        let mut catalog = Catalog {
            areas: read_entries(self, "area:")?,
            projects: read_entries(self, "area-project:")?,
            legacy_aliases: read_entries(self, "area-alias:")?,
            links: read_entries(self, "area-link:")?,
        };
        catalog
            .projects
            .retain(|_, p| self.resolve(&p.name) == p.name);
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
                        id,
                        name,
                        area_id: None,
                        status: Status::Active,
                        converted_to: None,
                    },
                );
            }
        }
        for p in catalog.projects.values_mut() {
            p.name = self.resolve(&p.name);
        }
        catalog.validate()?;
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
        for project in self.area_catalog()?.projects.values() {
            put(self, "area-project:", &project.id, project)?;
        }
        Ok(())
    }
}
impl Catalog {
    fn validate(&self) -> io::Result<()> {
        for (id, a) in &self.areas {
            if *id != a.id || a.name.trim().is_empty() {
                return Err(store_io::invalid("Invalid area registry"));
            }
        }
        for (id, p) in &self.projects {
            if *id != p.id
                || p.area_id
                    .iter()
                    .chain(p.converted_to.iter())
                    .any(|a| !self.areas.contains_key(a))
            {
                return Err(store_io::invalid("A project refers to a missing area"));
            }
        }
        for link in self.links.values() {
            self.validate_target(&link.target)?;
        }
        Ok(())
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
                std::iter::once(&a.name)
                    .chain(a.aliases.iter())
                    .any(|n| normalized_name(n) == key)
            })
            .collect()
    }
    pub fn validate_target(&self, target: &Target) -> io::Result<()> {
        if !self.areas.contains_key(&target.area_id)
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
    pub fn target(
        &self,
        p: &Projects,
        key: &str,
        baseline: &Value,
        raw_project: &str,
    ) -> Option<Target> {
        if let Some(link) = self.links.get(key).filter(|l| &l.baseline == baseline) {
            return Some(link.target.clone());
        }
        if let Some(LegacyAlias::Area { area_id }) = self
            .legacy_aliases
            .iter()
            .find(|(name, _)| normalized_name(name) == normalized_name(raw_project))
            .map(|(_, alias)| alias)
        {
            return Some(Target {
                area_id: area_id.clone(),
                project_id: None,
            });
        }
        let project = self.project(p, raw_project)?;
        if let Some(area_id) = &project.converted_to {
            Some(Target {
                area_id: area_id.clone(),
                project_id: None,
            })
        } else {
            project.area_id.as_ref().map(|a| Target {
                area_id: a.clone(),
                project_id: Some(project.id.clone()),
            })
        }
    }
    pub fn label(&self, target: &Target) -> String {
        let area = &self.areas[&target.area_id].name;
        target
            .project_id
            .as_ref()
            .and_then(|id| self.projects.get(id))
            .map_or_else(|| area.clone(), |p| format!("{area} › {}", p.name))
    }
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
                super::dominant_project(tasks.iter().filter(|t| t.meeting_id == meeting.id))
            })
            .or_else(|| file.and_then(|f| f.meeting_project.clone()));
        let baseline = json!([
            p.meeting_projects.get(&meeting.id),
            file.and_then(|f| f.meeting_project.as_ref())
        ]);
        if let Some(target) = catalog.target(
            p,
            &meeting_key(&meeting.id),
            &baseline,
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
            out.meetings.insert(meeting.id.clone(), target);
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
