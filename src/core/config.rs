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
            "list_tasks" | "list_teams" => Some(Self::Tasks),
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

impl KnowledgeBase {
    pub fn is_empty(&self) -> bool {
        self.repo_path.is_empty()
            && self.revision.is_empty()
            && self.source_roots.is_empty()
            && self.company_name.is_empty()
            && self.people_roster_source.is_empty()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.repo_path.trim().is_empty() {
            return Err("Enter the repository folder.");
        }
        if self.repo_path.chars().any(char::is_control) {
            return Err("Repository folder cannot contain control characters.");
        }
        if self.revision.trim().is_empty() {
            return Err("Enter a revision starting with origin/.");
        }
        if !self.revision.starts_with("origin/")
            || self.revision.len() == 7
            || self
                .revision
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("Revision must start with origin/ and contain no whitespace.");
        }
        if self.source_roots.is_empty() {
            return Err("Enter at least one source folder, one per line.");
        }
        if self
            .source_roots
            .iter()
            .any(|p| p.is_empty() || !super::daily_check::safe_kb_path(&format!("{p}/source.md")))
        {
            return Err(
                "Source folders must be relative, use /, and exclude parent paths and .env files.",
            );
        }
        if !self.people_roster_source.is_empty()
            && !super::daily_check::safe_kb_path(&self.people_roster_source)
        {
            return Err(
                "Roster must be a relative .md, .yaml, .yml, .json or .txt file, without parent paths or .env files.",
            );
        }
        Ok(())
    }
    pub(crate) fn config_stage(&self) -> super::failure::Stage {
        if self.repo_path.trim().is_empty()
            || self.revision.trim().is_empty()
            || self.source_roots.is_empty()
        {
            super::failure::Stage::KbConfigEmpty
        } else {
            super::failure::Stage::KbConfigInvalid
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub features: Features,
    #[serde(default)]
    pub labs: super::meeting_watch::Labs,
    #[serde(default)]
    pub knowledge_base: KnowledgeBase,
    #[serde(default)]
    pub pipeline: super::meeting_router::RoutingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<super::notes::NotesSettings>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
    #[serde(skip)]
    baseline: Option<serde_json::Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            features: Features::default(),
            labs: super::meeting_watch::Labs::default(),
            knowledge_base: KnowledgeBase::default(),
            pipeline: super::meeting_router::RoutingConfig::default(),
            notes: None,
            baseline: None,
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
                config.overlay_profile(root)?;
                config.baseline = Some(serde_json::to_value(&config)?);
                Ok(config)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut config = Self::migrate(root)?;
                store_io::write(&path, &config)?;
                config.overlay_profile(root)?;
                config.baseline = Some(serde_json::to_value(&config)?);
                Ok(config)
            }
            Err(e) => Err(e),
        }
    }

    pub fn save(&mut self, root: &Path) -> io::Result<()> {
        let _lock = store_io::lock(&root.join("local/config.lock"))?;
        let path = root.join("config.json");
        let bytes = std::fs::read(&path)?;
        let mut latest: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
        if latest["version"] != 1 {
            return Err(store_io::invalid("Unsupported config.json version"));
        }
        let baseline = self
            .baseline
            .as_ref()
            .ok_or_else(|| store_io::invalid("Reload settings before saving"))?;
        merge_edits(&mut latest, baseline, &serde_json::to_value(&*self)?);
        let _: Self = serde_json::from_value(latest.clone())?;
        if super::relay_preview::Settings::load(root)?.enabled
            && let Some(notes) = latest
                .get_mut("notes")
                .and_then(serde_json::Value::as_object_mut)
        {
            notes.insert(
                "your_name".into(),
                super::relay_preview::shared::local_profile(root)?["your_name"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
            );
        }
        let mut saved: Self = serde_json::from_value(latest.clone())?;
        saved.overlay_profile(root)?;
        saved.baseline = Some(serde_json::to_value(&saved)?);
        store_io::write(&path, &latest)?;
        *self = saved;
        Ok(())
    }

    fn overlay_profile(&mut self, root: &Path) -> io::Result<()> {
        if let Some(notes) = &mut self.notes
            && let Some(preview) = super::relay_preview::get(root)?
        {
            let value = preview.shared_value(
                "profile.json",
                serde_json::json!({"your_name":notes.your_name}),
            )?;
            notes.your_name = value["your_name"].as_str().unwrap_or("").into();
            notes.relay_name_base = Some(notes.your_name.clone());
        }
        Ok(())
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
        if super::relay_preview::Settings::load(root)?.enabled {
            let original = config
                .notes
                .as_ref()
                .map(|n| n.your_name.clone())
                .unwrap_or_default();
            let baseline = notes.relay_name_base.as_ref().unwrap_or(&original);
            super::relay_preview::shared::edit(
                root,
                "profile.json",
                &serde_json::json!({"your_name":baseline}),
                &serde_json::json!({"your_name":notes.your_name}),
            )?;
            notes.your_name = original;
        }
        config.notes = Some(notes);
        store_io::write(&path, &config)
    }
}

// Apply only explicit edits; a stale card must not restore values in other cards.
fn merge_edits(
    latest: &mut serde_json::Value,
    baseline: &serde_json::Value,
    edited: &serde_json::Value,
) {
    if baseline == edited {
        return;
    }
    if let (Some(before), Some(after)) = (baseline.as_object(), edited.as_object()) {
        if !latest.is_object() {
            *latest = serde_json::json!({});
        }
        let latest = latest.as_object_mut().unwrap();
        for (key, value) in after {
            if before.get(key) == Some(value) {
                continue;
            }
            merge_edits(
                latest.entry(key.clone()).or_insert(serde_json::Value::Null),
                before.get(key).unwrap_or(&serde_json::Value::Null),
                value,
            );
        }
        for key in before.keys().filter(|key| !after.contains_key(*key)) {
            latest.remove(key);
        }
    } else {
        *latest = edited.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_save_preserves_unedited_kb_and_other_fields() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        store_io::write(&path, &Config::default()).unwrap();
        let mut stale = Config::load_in(root.path()).unwrap();
        let mut current = Config::load_in(root.path()).unwrap();
        current.knowledge_base.repo_path = "E:/example/kb".into();
        current.knowledge_base.revision = "origin/main".into();
        current.knowledge_base.source_roots = vec!["docs".into()];
        current.knowledge_base.company_name = "Example company".into();
        current.knowledge_base.people_roster_source = "people/roster.md".into();
        current.save(root.path()).unwrap();
        stale.features.daily_check = true;
        stale.save(root.path()).unwrap();
        let saved = Config::load_in(root.path()).unwrap();
        assert_eq!(
            serde_json::to_value(&saved.knowledge_base).unwrap(),
            serde_json::to_value(&current.knowledge_base).unwrap()
        );
        assert!(saved.features.daily_check);
        let mut kb_card = saved.clone();
        let mut other = saved.clone();
        other.features.tasks = true;
        other.knowledge_base.glossary = "Synthetic term".into();
        other.save(root.path()).unwrap();
        kb_card.knowledge_base.company_name.clear();
        kb_card.save(root.path()).unwrap();
        let saved = Config::load_in(root.path()).unwrap();
        assert!(saved.features.tasks);
        assert_eq!(saved.knowledge_base.glossary, "Synthetic term");
        assert!(saved.knowledge_base.company_name.is_empty());
        assert!(Config::default().save(root.path()).is_err());
        Config::save_notes(root.path(), &super::super::notes::NotesSettings::default()).unwrap();
        assert_eq!(
            Config::load_in(root.path())
                .unwrap()
                .knowledge_base
                .repo_path,
            "E:/example/kb"
        );
    }

    #[test]
    fn successful_saves_refresh_values_and_baseline_before_unrelated_edits() {
        let root = tempfile::tempdir().unwrap();
        store_io::write(&root.path().join("config.json"), &Config::default()).unwrap();
        let mut first = Config::load_in(root.path()).unwrap();
        first.pipeline.language_mode = "forced:en".into();
        first.save(root.path()).unwrap();
        assert_eq!(
            first.baseline.as_ref().unwrap(),
            &serde_json::to_value(&first).unwrap()
        );
        let mut second = Config::load_in(root.path()).unwrap();
        second.pipeline.language_mode = "forced:hi".into();
        second.save(root.path()).unwrap();
        first.knowledge_base.company_name = "Example company".into();
        first.save(root.path()).unwrap();
        assert_eq!(first.pipeline.language_mode, "forced:hi");
        first.features.tasks = true;
        first.labs.measure_recording_files = true;
        first.save(root.path()).unwrap();
        let mut second = Config::load_in(root.path()).unwrap();
        second.features.tasks = false;
        second.labs.measure_recording_files = false;
        second.save(root.path()).unwrap();
        first.knowledge_base.company_name.clear();
        first.save(root.path()).unwrap();
        let saved = Config::load_in(root.path()).unwrap();
        assert_eq!(saved.pipeline.language_mode, "forced:hi");
        assert!(!saved.features.tasks);
        assert!(!saved.labs.measure_recording_files);
        assert_eq!(
            first.baseline.as_ref().unwrap(),
            &serde_json::to_value(&first).unwrap()
        );
    }

    #[test]
    fn failed_save_keeps_the_unsaved_edit_and_its_original_baseline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        store_io::write(&path, &Config::default()).unwrap();
        let mut config = Config::load_in(root.path()).unwrap();
        let baseline = config.baseline.clone();
        config.features.tasks = true;
        std::fs::write(&path, b"invalid synthetic JSON").unwrap();
        assert!(config.save(root.path()).is_err());
        assert!(config.features.tasks);
        assert_eq!(config.baseline, baseline);
    }

    #[test]
    fn missing_or_retyped_parent_receives_only_edited_leaves() {
        let before = json!({"knowledge_base": {"repo_path": "E:/example/kb", "revision": "origin/main", "company_name": "Example company"}});
        let mut edited = before.clone();
        edited["knowledge_base"]["company_name"] = json!("Example revision");
        for mut latest in [
            json!({}),
            json!({"knowledge_base": null}),
            json!({"knowledge_base": "removed"}),
        ] {
            merge_edits(&mut latest, &before, &edited);
            assert_eq!(
                latest,
                json!({"knowledge_base": {"company_name": "Example revision"}})
            );
        }
    }

    #[test]
    fn merge_keeps_unknown_fields_and_changes_only_edited_leaves() {
        let before = json!({"knowledge_base": {"repo_path": "", "revision": ""}, "features": {"tasks": false}});
        let edited = json!({"knowledge_base": {"repo_path": "", "revision": ""}, "features": {"tasks": true}});
        let mut latest = json!({"knowledge_base": {"repo_path": "E:/example/kb", "revision": "origin/main", "future": true}, "features": {"tasks": false}, "future": 1});
        merge_edits(&mut latest, &before, &edited);
        assert_eq!(latest["knowledge_base"]["repo_path"], "E:/example/kb");
        assert_eq!(latest["knowledge_base"]["future"], true);
        assert_eq!(latest["future"], 1);
        assert_eq!(latest["features"]["tasks"], true);
        let mut missing = json!({"features": {"tasks": false}});
        merge_edits(&mut missing, &before, &edited);
        assert!(missing.get("knowledge_base").is_none());
    }

    #[test]
    fn knowledge_base_rejects_unsafe_paths_and_unscoped_revision() {
        let mut kb = KnowledgeBase {
            repo_path: "E:/example/kb".into(),
            revision: "origin/main".into(),
            source_roots: vec!["docs".into()],
            ..KnowledgeBase::default()
        };
        assert!(kb.validate().is_ok());
        for path in ["../private", "E:/private", "docs\\private", ".env", ""] {
            kb.source_roots = vec![path.into()];
            assert!(kb.validate().is_err());
        }
        kb.source_roots = vec!["docs".into()];
        for path in ["../roster.md", "roster.exe", ".env.json"] {
            kb.people_roster_source = path.into();
            assert!(kb.validate().is_err());
        }
        kb.people_roster_source.clear();
        for revision in ["main", "origin/", "origin/main bad"] {
            kb.revision = revision.into();
            assert!(kb.validate().is_err());
        }
    }
}
