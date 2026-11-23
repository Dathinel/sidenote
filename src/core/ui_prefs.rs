//! Window preferences that belong to this installation only, such as whether the sidebar is
//! folded. They live in `local/ui.json`, which the state share's ignore file leaves out, so a
//! preference never travels to another device.
use super::store_io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How the Tasks page lays its tasks out. The list stays the default, so a missing key changes
/// nothing for anyone who has not chosen the board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TasksLayout {
    #[default]
    List,
    Board,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub memory_kind: Option<super::memory::RecordType>,
    pub memory_pending: bool,
    pub memory_scope: String,
    pub memory_project: Option<String>,
    pub memory_meeting: Option<String>,
    pub meeting_tabs: bool,
    pub meeting_summary_templates: std::collections::BTreeMap<String, String>,
    pub people_since_ms: Option<i64>,
    pub people_existing_meetings: Option<std::collections::BTreeSet<String>>,
    pub people_sides: std::collections::BTreeMap<String, super::people::Side>,
    #[serde(skip)]
    pub relay_people_base: Option<std::collections::BTreeMap<String, super::people::Side>>,
    pub show_full_titles: bool,
    /// Task filters belong to this PC. Missing keys keep the first visit on Everyone/Open.
    pub tasks_mine: bool,
    pub tasks_include_unassigned: bool,
    pub tasks_done: super::DoneFilter,
    /// List or board, remembered for this PC only.
    pub tasks_layout: TasksLayout,
    /// Capture view index: 0 for Blocks, 1 for Filmstrip.
    pub annotations_view: usize,
    pub task_titles_notice_seen: bool,
    /// None until the owner folds or opens the sidebar once.
    pub sidebar_collapsed: Option<bool>,
    pub settings_tabs: std::collections::BTreeMap<String, usize>,
    #[serde(default = "super::ask_options::meeting_thinking_default")]
    pub ask_meeting_thinking: super::ask_options::Thinking,
    pub ask_chat_thinking: super::ask_options::Thinking,
    pub ask_models: std::collections::BTreeMap<String, String>,
    pub ask_meeting_agent: String,
    pub ask_meeting_model: Option<String>,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            memory_kind: None,
            memory_pending: false,
            memory_scope: String::new(),
            memory_project: None,
            memory_meeting: None,
            meeting_tabs: false,
            meeting_summary_templates: Default::default(),
            people_since_ms: None,
            people_existing_meetings: None,
            people_sides: Default::default(),
            relay_people_base: None,
            show_full_titles: false,
            tasks_mine: false,
            tasks_include_unassigned: false,
            tasks_done: Default::default(),
            tasks_layout: Default::default(),
            annotations_view: 0,
            task_titles_notice_seen: false,
            sidebar_collapsed: None,
            settings_tabs: Default::default(),
            ask_meeting_thinking: super::ask_options::Thinking::Low,
            ask_chat_thinking: super::ask_options::Thinking::Medium,
            ask_models: Default::default(),
            ask_meeting_agent: super::ask_options::meeting_agent_default(),
            ask_meeting_model: None,
        }
    }
}

fn path(root: &Path) -> PathBuf {
    root.join("local").join("ui.json")
}

impl UiPrefs {
    /// A missing or unreadable file is the defaults. A preference is never worth an error.
    pub fn load(root: &Path) -> Self {
        let mut prefs: Self = store_io::read(&path(root)).unwrap_or_default();
        if let Ok(Some(preview)) = super::relay_preview::get(root) {
            let before = serde_json::json!({"sides":prefs.people_sides});
            if let Ok(value) = preview.shared_value("people.json", before)
                && let Ok(sides) = serde_json::from_value(value["sides"].clone())
            {
                prefs.people_sides = sides;
                prefs.relay_people_base = Some(prefs.people_sides.clone());
            }
        }
        prefs
    }

    /// Publish only this person's side delta, preserving other settings and side edits.
    pub fn save_person_side(
        &mut self,
        root: &Path,
        person: &str,
        teams: &super::Teams,
        side: Option<super::people::Side>,
    ) -> std::io::Result<()> {
        let _lock = store_io::lock(&root.join("local/ui.lock"))?;
        let prior = Self::load(root);
        let mut edited = prior.people_sides.clone();
        let canonical = super::people::key(&teams.owner_name(person));
        edited.retain(|name, _| super::people::key(&teams.owner_name(name)) != canonical);
        if let Some(side) = side {
            edited.insert(canonical, side);
        }
        if super::relay_preview::Settings::load(root)?.enabled {
            super::relay_preview::shared::edit(
                root,
                "people.json",
                &serde_json::json!({"sides":prior.people_sides}),
                &serde_json::json!({"sides":edited}),
            )?;
        } else {
            let mut latest: serde_json::Value = store_io::read(&path(root))?;
            if latest.is_null() {
                latest = serde_json::json!({});
            }
            super::config::merge_edits(
                &mut latest,
                &serde_json::json!({"people_sides":prior.people_sides}),
                &serde_json::json!({"people_sides":edited}),
            );
            store_io::write(&path(root), &latest)?;
        }
        self.people_sides = edited;
        self.relay_people_base = prior.relay_people_base.map(|_| self.people_sides.clone());
        Ok(())
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let mut local = self.clone();
        if super::relay_preview::Settings::load(root)?.enabled {
            let prior: Self = store_io::read(&path(root))?;
            let before = self
                .relay_people_base
                .as_ref()
                .unwrap_or(&prior.people_sides);
            super::relay_preview::shared::edit(
                root,
                "people.json",
                &serde_json::json!({"sides":before}),
                &serde_json::json!({"sides":self.people_sides}),
            )?;
            local.people_sides = prior.people_sides;
        }
        store_io::write(&path(root), &local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirm_and_reset_merge_only_the_person_side() {
        let root = tempfile::tempdir().unwrap();
        let path = path(root.path());
        store_io::write(&path, &serde_json::json!({"show_full_titles":true,"unknown_setting":42,"people_sides":{"guest person":"external"}})).unwrap();
        let mut prefs = UiPrefs::load(root.path());
        prefs
            .save_person_side(
                root.path(),
                "Avery",
                &super::super::Teams::default(),
                Some(super::super::people::Side::OurTeam),
            )
            .unwrap();
        assert_eq!(
            UiPrefs::load(root.path()).people_sides["avery"],
            super::super::people::Side::OurTeam
        );
        prefs
            .save_person_side(root.path(), "Avery", &super::super::Teams::default(), None)
            .unwrap();
        let saved: serde_json::Value = store_io::read(&path).unwrap();
        assert!(saved["people_sides"].get("avery").is_none());
        assert_eq!(saved["people_sides"]["guest person"], "external");
        assert_eq!(saved["unknown_setting"], 42);
        assert_eq!(saved["show_full_titles"], true);
    }

    #[test]
    fn memory_filters_round_trip_and_leave_old_settings_intact() {
        let mut prefs: UiPrefs =
            serde_json::from_str(r#"{"show_full_titles":true,"settings_tabs":{"Meetings":1}}"#)
                .unwrap();
        assert!(prefs.memory_kind.is_none() && prefs.memory_project.is_none());
        assert!(!prefs.memory_pending);
        prefs.memory_kind = Some(super::super::memory::RecordType::Person);
        prefs.memory_project = Some("Synthetic workshop".into());
        prefs.memory_scope = "review".into();
        prefs.memory_meeting = Some("synthetic-meeting".into());
        let restored: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(prefs, restored);
        assert!(restored.show_full_titles);
        assert_eq!(restored.settings_tabs["Meetings"], 1);
    }

    #[test]
    fn task_filters_default_to_everyone_and_round_trip_with_existing_keys() {
        let mut prefs: UiPrefs =
            serde_json::from_str(r#"{"show_full_titles":true,"settings_tabs":{"Meetings":1}}"#)
                .unwrap();
        assert!(!prefs.meeting_tabs);
        assert!(prefs.meeting_summary_templates.is_empty());
        assert_eq!(prefs.people_since_ms, None);
        assert_eq!(prefs.people_existing_meetings, None);
        assert!(prefs.people_sides.is_empty());
        assert!(!prefs.tasks_mine);
        assert!(!prefs.tasks_include_unassigned);
        assert_eq!(prefs.tasks_done, super::super::DoneFilter::Open);
        prefs.tasks_mine = true;
        prefs.tasks_include_unassigned = true;
        prefs.tasks_done = super::super::DoneFilter::All;
        let restored: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(restored, prefs);
        assert!(restored.show_full_titles);
        assert_eq!(restored.settings_tabs["Meetings"], 1);
    }
    #[test]
    fn tasks_layout_defaults_to_list_and_round_trips_beside_old_keys() {
        let mut prefs: UiPrefs =
            serde_json::from_str(r#"{"show_full_titles":true,"tasks_mine":true}"#).unwrap();
        assert_eq!(prefs.tasks_layout, TasksLayout::List);
        prefs.tasks_layout = TasksLayout::Board;
        let text = serde_json::to_string(&prefs).unwrap();
        assert!(text.contains(r#""tasks_layout":"board""#));
        let restored: UiPrefs = serde_json::from_str(&text).unwrap();
        assert_eq!(restored.tasks_layout, TasksLayout::Board);
        assert!(restored.show_full_titles && restored.tasks_mine);
    }
    #[test]
    fn settings_tabs_round_trip_without_changing_existing_preferences() {
        let mut prefs: UiPrefs = serde_json::from_str(
            r#"{"show_full_titles":true,"task_titles_notice_seen":true,"sidebar_collapsed":false}"#,
        )
        .unwrap();
        assert!(prefs.settings_tabs.is_empty());
        assert_eq!(
            prefs.ask_meeting_thinking,
            super::super::ask_options::Thinking::Low
        );
        assert_eq!(
            prefs.ask_chat_thinking,
            super::super::ask_options::Thinking::Medium
        );
        prefs.ask_chat_thinking = super::super::ask_options::Thinking::High;
        prefs
            .ask_models
            .insert("Codex".into(), "chosen-model".into());
        prefs.settings_tabs.insert("Meetings".into(), 1);
        prefs.settings_tabs.insert("AI helpers".into(), 2);
        let round_trip: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(round_trip, prefs);
        assert!(round_trip.show_full_titles && round_trip.task_titles_notice_seen);
        assert_eq!(round_trip.sidebar_collapsed, Some(false));
    }
}
