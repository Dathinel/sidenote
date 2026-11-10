//! Device-local user choices. Only state/** is synchronized.
use super::{StateStore, TaskStore, memory::MemoryStore, state_dir, store_io};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Copy)]
pub enum Feature {
    Tasks,
    Memory,
    DailyCheck,
    Projects,
    OtherDevices,
}

impl Feature {
    pub const ALL: [Self; 5] = [
        Self::Tasks,
        Self::Memory,
        Self::DailyCheck,
        Self::Projects,
        Self::OtherDevices,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::Memory => "Memory",
            Self::DailyCheck => "Daily check",
            Self::Projects => "Projects",
            Self::OtherDevices => "Other devices",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Tasks => "See all tasks in one place and their count in the sidebar.",
            Self::Memory => "Review memory, proposals and memory badges.",
            Self::DailyCheck => "Build memory each day and review the alignment report.",
            Self::Projects => "Group meetings with project chips and sidebar filters.",
            Self::OtherDevices => "Show the Devices section in the sidebar.",
        }
    }

    pub fn for_tool(name: &str) -> Option<Self> {
        match name {
            "list_tasks" => Some(Self::Tasks),
            "get_memory" | "propose_memory" => Some(Self::Memory),
            "get_daily_check" => Some(Self::DailyCheck),
            "list_projects" => Some(Self::Projects),
            _ => None,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    pub tasks: bool,
    pub memory: bool,
    pub daily_check: bool,
    pub projects: bool,
    pub other_devices: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

impl Features {
    pub fn enabled(&self, feature: Feature) -> bool {
        match feature {
            Feature::Tasks => self.tasks,
            Feature::Memory => self.memory,
            Feature::DailyCheck => self.daily_check,
            Feature::Projects => self.projects,
            Feature::OtherDevices => self.other_devices,
        }
    }

    pub fn toggle(&mut self, feature: Feature) {
        let slot = match feature {
            Feature::Tasks => &mut self.tasks,
            Feature::Memory => &mut self.memory,
            Feature::DailyCheck => &mut self.daily_check,
            Feature::Projects => &mut self.projects,
            Feature::OtherDevices => &mut self.other_devices,
        };
        *slot = !*slot;
    }
}

/// Optional private business context, kept only in device-local config.json.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeBase {
    pub repo_path: String,
    pub revision: String,
    pub source_roots: Vec<String>,
    pub company_name: String,
    pub people_roster_source: String,
    pub glossary: String,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub features: Features,
    #[serde(default)]
    pub knowledge_base: KnowledgeBase,
    #[serde(default)]
    pub pipeline: super::meeting_router::RoutingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<super::notes::NotesSettings>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            features: Features::default(),
            knowledge_base: KnowledgeBase::default(),
            pipeline: super::meeting_router::RoutingConfig::default(),
            notes: None,
            extra: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn load() -> io::Result<Self> {
        Self::load_in(&state_dir())
    }

    pub fn load_in(root: &Path) -> io::Result<Self> {
        let _lock = store_io::lock(&root.join("local/config.lock"))?;
        let path = root.join("config.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut config: Self =
                    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
                if config.version != 1 {
                    return Err(store_io::invalid("Unsupported config.json version"));
                }
                if config.notes.is_none() {
                    config.notes = Some(super::notes::migrated_settings(
                        &super::live::notes_pipeline(),
                    ));
                    store_io::write(&path, &config)?;
                }
                Ok(config)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let config = Self::migrate(root)?;
                store_io::write(&path, &config)?;
                Ok(config)
            }
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, root: &Path) -> io::Result<()> {
        let _lock = store_io::lock(&root.join("local/config.lock"))?;
        store_io::write(&root.join("config.json"), self)
    }

    fn migrate(root: &Path) -> io::Result<Self> {
        let (meetings, errors) = super::scan();
        if !errors.is_empty() {
            return Err(store_io::invalid(
                "Could not inspect meetings for feature migration",
            ));
        }
        let store = TaskStore {
            root: root.to_owned(),
        };
        let projects = store.load_projects()?;
        let overrides = store.load_overrides()?;
        let stores = super::sync_state::stores(root)?;
        let saved = StateStore::new(root.to_owned());
        let mut checked = std::collections::HashMap::new();
        for meeting in &meetings {
            checked.insert(meeting.id.clone(), saved.load(&meeting.id)?.checked);
        }
        // Retain opt-in for ticks whose source meeting is no longer on this device.
        for name in &stores {
            if let Some(id) = name.strip_suffix(".json")
                && id.split_once('-').is_some_and(|(a, b)| {
                    !a.is_empty()
                        && !b.is_empty()
                        && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
                })
                && !checked.contains_key(id)
            {
                checked.insert(id.to_owned(), saved.load(id)?.checked);
            }
        }
        let pipeline = super::load_pipeline(&meetings);
        let tasks = super::build_tasks(&meetings, &pipeline, &projects, &overrides, &checked);
        let memory = MemoryStore::new(root.to_owned());
        let records = memory.load()?;
        let mut report_proposals = false;
        let mut daily_check = false;
        for name in &stores {
            if name
                .strip_prefix("checks/")
                .and_then(|s| s.strip_suffix(".json"))
                .is_some_and(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
            {
                daily_check = true;
                let report: serde_json::Value = super::sync_state::load(root, name)?;
                report_proposals |= report
                    .get("memory_proposals")
                    .and_then(|p| p.as_array())
                    .is_some_and(|p| !p.is_empty());
            }
        }
        Ok(Self {
            notes: Some(super::notes::migrated_settings(
                &super::live::notes_pipeline(),
            )),
            features: Features {
                tasks: !tasks.is_empty()
                    || !overrides.tasks.is_empty()
                    || !overrides.manual.is_empty()
                    || checked.values().any(|v| !v.is_empty()),
                memory: report_proposals
                    || !records.records.is_empty()
                    || !records.proposals.is_empty()
                    || !memory.legacy_text()?.trim().is_empty(),
                daily_check,
                projects: !projects.projects.is_empty()
                    || !projects.aliases.is_empty()
                    || tasks.iter().any(|t| t.project != super::UNSORTED),
                other_devices: meetings.iter().any(|m| m.device.is_some()),
                ..Features::default()
            },
            ..Self::default()
        })
    }

    pub(crate) fn save_notes(root: &Path, notes: &super::notes::NotesSettings) -> io::Result<()> {
        // Re-read under the lock so another settings card's changes survive this save.
        let _lock = store_io::lock(&root.join("local/config.lock"))?;
        let path = root.join("config.json");
        let mut config: Self = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e),
        };
        if config.version != 1 {
            return Err(store_io::invalid("Unsupported config.json version"));
        }
        let mut notes = notes.clone();
        let was_on = config.notes.as_ref().is_some_and(|s| {
            s.engine == "app" && s.effective_helper().is_some() && s.enabled_at_ms.is_some()
        });
        if notes.engine == "app" && notes.effective_helper().is_some() {
            if !was_on {
                notes.enabled_at_ms = Some(chrono::Utc::now().timestamp_millis());
            } else {
                notes.enabled_at_ms = config.notes.as_ref().and_then(|s| s.enabled_at_ms);
            }
        } else {
            notes.enabled_at_ms = None;
        }
        config.notes = Some(notes);
        store_io::write(&path, &config)
    }
}
