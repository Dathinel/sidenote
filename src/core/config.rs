//! Device-local user choices. Only state/** is synchronized.
use super::{StateStore, TaskStore, memory::MemoryStore, state_dir, store_io};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::Path, sync::Mutex, time::Instant};

fn operation<T>(result: io::Result<T>, operation: &str, path_kind: &str) -> io::Result<T> {
    let log_busy = if path_kind == "relay_profile" {
        static BUSY: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
        let mut state = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let busy = result.as_ref().err().and_then(store_io::relay_reason) == Some("relay_lock");
        relay_busy(&mut state, busy, Instant::now())
    } else {
        false
    };
    result.inspect_err(|error| {
        if error.kind() != io::ErrorKind::WouldBlock || log_busy {
            crate::listener::log(&diagnostic(operation, path_kind, error));
        }
    })
}
fn relay_busy(state: &mut Option<(Instant, bool)>, busy: bool, now: Instant) -> bool {
    if !busy {
        *state = None;
        return false;
    }
    let (since, reported) = state.get_or_insert((now, false));
    let log = !*reported && now.duration_since(*since).as_secs() >= 120;
    *reported |= log;
    log
}
pub(super) fn diagnostic(operation: &str, path_kind: &str, error: &io::Error) -> String {
    let mut line = format!(
        "config operation={operation} path_kind={path_kind} kind={:?} os={:?}",
        error.kind(),
        store_io::os_code(error)
    );
    if let Some(reason) = store_io::relay_reason(error) {
        line.push_str(&format!(" reason={reason}"));
    }
    if error.kind() == io::ErrorKind::WouldBlock {
        line.push_str(" status=busy");
    }
    line
}
pub(super) fn read(root: &Path) -> io::Result<Vec<u8>> {
    let path = root.join("config.json");
    store_io::retry_sharing_violation(|| match std::fs::read(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(error),
        result => operation(result, "read", "config.json"),
    })
}

pub(super) fn lock(root: &Path) -> io::Result<std::fs::File> {
    operation(
        store_io::lock(&root.join("local/config.lock")).map_err(|error| {
            if cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)) {
                io::Error::new(io::ErrorKind::WouldBlock, error)
            } else {
                error
            }
        }),
        "lock",
        "config.lock",
    )
}

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

#[derive(Debug)]
pub struct KnowledgeBaseSaveError(&'static str);

impl std::fmt::Display for KnowledgeBaseSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Could not save knowledge base after merging other saved changes. {} Reload Settings and try again.",
            self.0
        )
    }
}

impl std::error::Error for KnowledgeBaseSaveError {}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub engine_settings: super::engine_settings::EngineSettings,
    pub version: u32,
    pub features: Features,
    #[serde(default)]
    pub labs: super::meeting_watch::Labs,
    #[serde(default)]
    pub meeting_icon_in_captures: bool,
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
            engine_settings: super::engine_settings::EngineSettings::default(),
            features: Features::default(),
            labs: super::meeting_watch::Labs::default(),
            meeting_icon_in_captures: false,
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
        let mut config = Self::read_config(root)?;
        if config.as_ref().is_none_or(|c| c.notes.is_none()) {
            let _lock = lock(root)?;
            // Another writer may have published or migrated while we waited.
            config = Self::read_config(root)?;
            if config.as_ref().is_none_or(|c| c.notes.is_none()) {
                let mut migrated = match config {
                    Some(config) => config,
                    None => operation(Self::migrate(root), "migration_read", "legacy_settings")?,
                };
                if migrated.notes.is_none() {
                    migrated.notes = Some(super::notes::migrated_settings(
                        &super::live::notes_pipeline(),
                    ));
                }
                operation(
                    store_io::write(&root.join("config.json"), &migrated),
                    "migration_write",
                    "config.json",
                )?;
                config = Some(migrated);
            }
        }
        Self::effective(
            config.expect("config published under the writer lock"),
            root,
        )
    }

    pub(crate) fn load_existing(root: &Path) -> io::Result<Self> {
        let config = Self::read_config(root)?.ok_or(io::ErrorKind::NotFound)?;
        if config.notes.is_none() {
            return Err(store_io::invalid("Missing notes settings"));
        }
        Self::effective(config, root)
    }

    fn effective(mut config: Self, root: &Path) -> io::Result<Self> {
        operation(
            config.overlay_profile(root),
            "overlay_read",
            "relay_profile",
        )?;
        config.baseline = Some(serde_json::to_value(&config)?);
        Ok(config)
    }

    pub(crate) fn read_config(root: &Path) -> io::Result<Option<Self>> {
        match read(root) {
            Ok(bytes) => {
                let config: Self = operation(
                    Self::decode(&bytes).map_err(io::Error::from),
                    "decode",
                    "config.json",
                )?;
                if config.version != 1 {
                    return Err(store_io::invalid("Unsupported config.json version"));
                }
                Ok(Some(config))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn decode(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
    }

    #[cfg(all(windows, feature = "desktop"))]
    pub(crate) fn meeting_icon_capture_allowed(root: &Path) -> bool {
        // The native window timer must fail closed on sharing errors without retrying.
        std::fs::read(root.join("config.json"))
            .ok()
            .and_then(|bytes| Self::decode(&bytes).ok())
            .is_some_and(|config| config.version == 1 && config.meeting_icon_in_captures)
    }

    pub fn save(&mut self, root: &Path) -> io::Result<()> {
        let _lock = lock(root)?;
        let path = root.join("config.json");
        let bytes = read(root)?;
        let mut latest: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?;
        if latest["version"] != 1 {
            return Err(store_io::invalid("Unsupported config.json version"));
        }
        let baseline = self
            .baseline
            .as_ref()
            .ok_or_else(|| store_io::invalid("Reload settings before saving"))?;
        let edited = serde_json::to_value(&*self)?;
        merge_edits(&mut latest, baseline, &edited);
        super::engine_settings::prepare_save(&mut latest, baseline, &edited)?;
        let merged: Self = serde_json::from_value(latest.clone())?;
        if baseline.get("knowledge_base") != edited.get("knowledge_base")
            && !merged.knowledge_base.is_empty()
        {
            merged.knowledge_base.validate().map_err(|reason| {
                io::Error::new(io::ErrorKind::InvalidData, KnowledgeBaseSaveError(reason))
            })?;
        }
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

    /// Reconcile a delayed notes readback without replacing newer settings or their baseline.
    pub fn refresh_notes_from(&mut self, saved: &Self) {
        self.notes = saved.notes.clone();
        if let Some(baseline) = self
            .baseline
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            if let Some(notes) = saved.baseline.as_ref().and_then(|b| b.get("notes")) {
                baseline.insert("notes".into(), notes.clone());
            } else {
                baseline.remove("notes");
            }
        }
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

    pub(crate) fn save_notes(
        root: &Path,
        baseline: &super::notes::NotesSettings,
        edited: &super::notes::NotesSettings,
    ) -> io::Result<()> {
        // Re-read under the lock so another settings card's changes survive this save.
        let _lock = lock(root)?;
        let path = root.join("config.json");
        let mut config: Self = match read(root) {
            Ok(bytes) => {
                serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e),
        };
        if config.version != 1 {
            return Err(store_io::invalid("Unsupported config.json version"));
        }
        let mut value = serde_json::to_value(config.notes.clone().unwrap_or_default())?;
        merge_edits(
            &mut value,
            &serde_json::to_value(baseline)?,
            &serde_json::to_value(edited)?,
        );
        let mut notes: super::notes::NotesSettings = serde_json::from_value(value)?;
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
            let name_base = baseline
                .relay_name_base
                .as_ref()
                .unwrap_or(&baseline.your_name);
            super::relay_preview::shared::edit(
                root,
                "profile.json",
                &serde_json::json!({"your_name":name_base}),
                &serde_json::json!({"your_name":edited.your_name}),
            )?;
            notes.your_name = original;
        }
        config.notes = Some(notes);
        store_io::write(&path, &config)
    }
}

// Apply only explicit edits; a stale card must not restore values in other cards.
pub(super) fn merge_edits(
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
    fn relay_busy_reports_once_after_bound_and_resets() {
        let mut state = None;
        let now = Instant::now();
        assert!(!relay_busy(&mut state, true, now));
        let almost = now + std::time::Duration::from_secs(119);
        assert!(!relay_busy(&mut state, true, almost));
        let later = now + std::time::Duration::from_secs(120);
        assert!(relay_busy(&mut state, true, later));
        assert!(!relay_busy(&mut state, true, later));
        assert!(!relay_busy(&mut state, false, later));
        assert!(!relay_busy(&mut state, true, later));
    }
    #[cfg(windows)]
    fn relay_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("local/relay")).unwrap();
        store_io::write(
            &root.join("local/relay/settings.json"),
            &super::super::relay_preview::Settings {
                enabled: true,
                backup: "synthetic".into(),
                shared_backup: "synthetic".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn held_replica_lock_defers_notes_overlay_then_retries() {
        let root = tempfile::tempdir().unwrap();
        relay_fixture(root.path());
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.path().join("local/relay/replica.sqlite.sync-lock"))
            .unwrap();
        lock.lock().unwrap();
        let mut config = Config {
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Default::default()
        };
        let error = config.overlay_profile(root.path()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(
            store_io::defer_busy(config.overlay_profile(root.path()))
                .unwrap()
                .is_none()
        );
        drop(lock);
        assert!(
            store_io::defer_busy(config.overlay_profile(root.path()))
                .unwrap()
                .is_some()
        );
        super::super::relay_preview::disconnect(root.path()).unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn corrupt_replica_does_not_defer_or_use_stale_notes_name() {
        let root = tempfile::tempdir().unwrap();
        relay_fixture(root.path());
        std::fs::write(
            root.path().join("local/relay/replica.sqlite"),
            b"synthetic corrupt database",
        )
        .unwrap();
        let mut config = Config {
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Default::default()
        };
        let error = store_io::defer_busy(config.overlay_profile(root.path())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "relay_database");
        std::fs::write(root.path().join("local/relay/settings.json"), b"{").unwrap();
        let error = config.overlay_profile(root.path()).unwrap_err();
        assert_eq!(store_io::relay_reason(&error), Some("relay_decode"));
    }
    #[test]
    fn diagnostics_keep_only_operation_path_kind_and_os_code() {
        let error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "C:/synthetic-private/config.json private text",
        );
        assert_eq!(
            diagnostic("read", "config.json", &error),
            "config operation=read path_kind=config.json kind=PermissionDenied os=None"
        );
        for (op, kind) in [
            ("lock", "config.lock"),
            ("migration_write", "config.json"),
            ("overlay_read", "relay_profile"),
        ] {
            let error = io::Error::from_raw_os_error(32);
            let line = diagnostic(op, kind, &error);
            assert!(line.contains(&format!("operation={op} path_kind={kind}")));
            assert!(line.ends_with("os=Some(32)"));
        }
    }
    #[cfg(windows)]
    #[test]
    fn config_read_contention_is_bounded_and_config_lock_returns_busy() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, b"{}").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let start = std::time::Instant::now();
        let error = Config::load_in(root.path()).err().unwrap();
        assert_eq!(error.raw_os_error(), Some(32));
        assert!(start.elapsed() >= std::time::Duration::from_millis(200));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        drop(held);
        store_io::write(&path, &Config::default()).unwrap();
        let _lock = store_io::lock(&root.path().join("local/config.lock")).unwrap();
        let error = Config::load_in(root.path()).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn published_config_reads_ignore_writer_lock_but_saves_are_busy() {
        let root = tempfile::tempdir().unwrap();
        let initial = Config {
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Config::default()
        };
        store_io::write(&root.path().join("config.json"), &initial).unwrap();
        let _held = store_io::lock(&root.path().join("local/config.lock")).unwrap();
        let mut loaded = Config::load_in(root.path()).unwrap();
        loaded.features.tasks = true;
        assert_eq!(
            loaded.save(root.path()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(
            Config::save_notes(
                root.path(),
                initial.notes.as_ref().unwrap(),
                loaded.notes.as_ref().unwrap()
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!Config::load_in(root.path()).unwrap().features.tasks);
    }

    #[test]
    fn concurrent_config_readers_observe_complete_published_versions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let initial = Config {
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Config::default()
        };
        store_io::write(&path.join("config.json"), &initial).unwrap();
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..3 {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..100 {
                        let config = Config::load_in(path).unwrap();
                        let notes = config.notes.unwrap();
                        assert_eq!(notes.model, notes.your_name);
                    }
                });
            }
            barrier.wait();
            let mut config = Config::load_in(path).unwrap();
            for version in 0..40 {
                let notes = config.notes.as_mut().unwrap();
                notes.model = format!("synthetic-{version}");
                notes.your_name = notes.model.clone();
                config.save(path).unwrap();
            }
        });
    }

    #[cfg(windows)]
    #[test]
    fn migration_rechecks_under_lock_and_preserves_winning_publication() {
        for missing in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path();
            if !missing {
                store_io::write(&path.join("config.json"), &Config::default()).unwrap();
            }
            let held = store_io::lock(&path.join("local/config.lock")).unwrap();
            std::thread::scope(|scope| {
                let reader = scope.spawn(move || Config::load_in(path).unwrap());
                std::thread::sleep(std::time::Duration::from_millis(75));
                let winner = Config {
                    notes: Some(super::super::notes::NotesSettings::default()),
                    features: Features {
                        tasks: true,
                        ..Features::default()
                    },
                    ..Config::default()
                };
                store_io::write(&path.join("config.json"), &winner).unwrap();
                let bytes = std::fs::read(path.join("config.json")).unwrap();
                drop(held);
                assert!(reader.join().unwrap().features.tasks);
                assert_eq!(std::fs::read(path.join("config.json")).unwrap(), bytes);
            });
            if !missing {
                store_io::write(&path.join("config.json"), &Config::default()).unwrap();
            } else {
                std::fs::remove_file(path.join("config.json")).unwrap();
            }
            Config::load_in(path).unwrap();
            let bytes = std::fs::read(path.join("config.json")).unwrap();
            let _held = store_io::lock(&path.join("local/config.lock")).unwrap();
            assert!(Config::load_in(path).unwrap().notes.is_some());
            assert_eq!(std::fs::read(path.join("config.json")).unwrap(), bytes);
        }
    }

    fn valid_kb() -> KnowledgeBase {
        KnowledgeBase {
            repo_path: "E:/synthetic/kb".into(),
            revision: "origin/main".into(),
            source_roots: vec!["docs".into()],
            company_name: "Synthetic company".into(),
            ..KnowledgeBase::default()
        }
    }

    #[test]
    fn delayed_notes_readback_preserves_newer_settings_and_their_baseline() {
        let root = tempfile::tempdir().unwrap();
        let initial = Config {
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Config::default()
        };
        store_io::write(&root.path().join("config.json"), &initial).unwrap();
        let mut current = Config::load_in(root.path()).unwrap();
        let baseline = current.notes.as_ref().unwrap().clone();
        let mut edited = baseline.clone();
        edited.your_name = "Synthetic owner".into();
        Config::save_notes(root.path(), &baseline, &edited).unwrap();
        let delayed = Config::load_in(root.path()).unwrap();

        current.features.tasks = true;
        current.labs.measure_recording_files = true;
        current.pipeline.language_mode = "forced:hi".into();
        current.knowledge_base = valid_kb();
        current.save(root.path()).unwrap();
        let mut expected = serde_json::to_value(&current).unwrap();
        expected["notes"] = serde_json::to_value(&delayed.notes).unwrap();
        current.refresh_notes_from(&delayed);
        assert_eq!(serde_json::to_value(&current).unwrap(), expected);
        assert_eq!(current.baseline.as_ref().unwrap(), &expected);

        let baseline = current.notes.as_ref().unwrap().clone();
        let mut edited = baseline.clone();
        edited.model = "synthetic-readback-model".into();
        Config::save_notes(root.path(), &baseline, &edited).unwrap();
        current.refresh_notes_from(&Config::load_in(root.path()).unwrap());
        assert_eq!(
            current.baseline.as_ref().unwrap(),
            &serde_json::to_value(&current).unwrap()
        );

        let mut other = Config::load_in(root.path()).unwrap();
        other.notes.as_mut().unwrap().model = "synthetic-new-model".into();
        other.pipeline.language_mode = "forced:en".into();
        other.save(root.path()).unwrap();
        current.features.memory = true;
        current.save(root.path()).unwrap();
        assert!(current.features.tasks);
        assert_eq!(current.pipeline.language_mode, "forced:en");
        assert_eq!(current.notes.as_ref().unwrap().model, "synthetic-new-model");
        assert_eq!(current.notes.as_ref().unwrap().your_name, "Synthetic owner");
    }

    #[test]
    fn kb_save_rejects_invalid_merged_results_without_writing_or_refreshing_draft() {
        for change in ["clear", "remove", "invalid revision"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("config.json");
            let initial = Config {
                knowledge_base: valid_kb(),
                notes: Some(super::super::notes::NotesSettings::default()),
                ..Config::default()
            };
            store_io::write(&path, &initial).unwrap();
            let mut stale = Config::load_in(root.path()).unwrap();
            let baseline = stale.baseline.clone();
            stale.knowledge_base.company_name = "Synthetic edited company".into();
            assert!(stale.knowledge_base.validate().is_ok());
            if change == "clear" {
                let mut other = Config::load_in(root.path()).unwrap();
                other.knowledge_base = KnowledgeBase::default();
                other.save(root.path()).unwrap();
            } else {
                let mut latest = serde_json::to_value(&initial).unwrap();
                if change == "remove" {
                    latest.as_object_mut().unwrap().remove("knowledge_base");
                } else {
                    latest["knowledge_base"]["revision"] = json!("main");
                }
                store_io::write(&path, &latest).unwrap();
            }
            let before = std::fs::read(&path).unwrap();
            let error = stale.save(root.path()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.get_ref().unwrap().is::<KnowledgeBaseSaveError>());
            assert!(error.to_string().contains("Reload Settings"));
            assert_eq!(std::fs::read(&path).unwrap(), before);
            assert_eq!(stale.baseline, baseline);
            assert_eq!(
                stale.knowledge_base.company_name,
                "Synthetic edited company"
            );
            stale.knowledge_base = valid_kb();
            stale.features.tasks = true;
            stale.save(root.path()).unwrap();
            assert!(stale.features.tasks);
            let before: serde_json::Value = serde_json::from_slice(&before).unwrap();
            let after: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(after.get("knowledge_base"), before.get("knowledge_base"));
        }
    }

    #[test]
    fn kb_save_keeps_valid_concurrent_fields_and_allows_clearing() {
        let root = tempfile::tempdir().unwrap();
        let initial = Config {
            knowledge_base: valid_kb(),
            notes: Some(super::super::notes::NotesSettings::default()),
            ..Config::default()
        };
        store_io::write(&root.path().join("config.json"), &initial).unwrap();
        let mut first = Config::load_in(root.path()).unwrap();
        let mut second = first.clone();
        second.knowledge_base.repo_path = "E:/synthetic/other-kb".into();
        second.save(root.path()).unwrap();
        first.knowledge_base.company_name = "Synthetic new company".into();
        first.save(root.path()).unwrap();
        assert_eq!(first.knowledge_base.repo_path, "E:/synthetic/other-kb");
        assert!(first.knowledge_base.validate().is_ok());
        first.knowledge_base = KnowledgeBase::default();
        first.save(root.path()).unwrap();
        assert!(
            Config::load_in(root.path())
                .unwrap()
                .knowledge_base
                .is_empty()
        );
    }

    #[test]
    fn notes_saves_merge_fields_and_preserve_latest_consent() {
        use super::super::notes::{Helper, NotesSettings};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let initial = Config {
            notes: Some(NotesSettings {
                helper: Some(Helper::Claude),
                enabled_at_ms: Some(123),
                model: "synthetic-original".into(),
                ..NotesSettings::default()
            }),
            ..Config::default()
        };
        store_io::write(&path, &initial).unwrap();
        let baseline = Config::load_in(root.path()).unwrap().notes.unwrap();
        let mut model_edit = baseline.clone();
        model_edit.model = "synthetic-new".into();
        Config::save_notes(root.path(), &baseline, &model_edit).unwrap();
        let mut name_edit = baseline.clone();
        name_edit.your_name = "Synthetic owner".into();
        Config::save_notes(root.path(), &baseline, &name_edit).unwrap();
        let saved = Config::load_in(root.path()).unwrap().notes.unwrap();
        assert_eq!(saved.model, "synthetic-new");
        assert_eq!(saved.your_name, "Synthetic owner");
        assert_eq!(saved.enabled_at_ms, Some(123));

        let mut off = saved.clone();
        off.engine = "off".into();
        Config::save_notes(root.path(), &saved, &off).unwrap();
        let mut auto_edit = saved.clone();
        auto_edit.auto = false;
        Config::save_notes(root.path(), &saved, &auto_edit).unwrap();
        let saved = Config::load_in(root.path()).unwrap().notes.unwrap();
        assert_eq!(saved.engine, "off");
        assert_eq!(saved.enabled_at_ms, None);
        assert!(!saved.auto);
        assert_eq!(saved.model, "synthetic-new");
        assert_eq!(saved.your_name, "Synthetic owner");
        let mut on = saved.clone();
        on.engine = "app".into();
        Config::save_notes(root.path(), &saved, &on).unwrap();
        let saved = Config::load_in(root.path()).unwrap().notes.unwrap();
        assert!(saved.enabled_at_ms.unwrap() > 123);
        assert_eq!(saved.model, "synthetic-new");
    }

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
        Config::save_notes(
            root.path(),
            saved.notes.as_ref().unwrap(),
            &super::super::notes::NotesSettings::default(),
        )
        .unwrap();
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
        first.knowledge_base = valid_kb();
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
