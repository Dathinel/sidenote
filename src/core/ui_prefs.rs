//! Window preferences that belong to this installation only, such as whether the sidebar is
//! folded. They live in `local/ui.json`, which the state share's ignore file leaves out, so a
//! preference never travels to another device.
use super::store_io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    pub show_full_titles: bool,
    /// Task filters belong to this PC. Missing keys keep the first visit on Everyone/Open.
    pub tasks_mine: bool,
    pub tasks_include_unassigned: bool,
    pub tasks_done: super::DoneFilter,
    pub task_titles_notice_seen: bool,
    /// None until the owner folds or opens the sidebar once.
    pub sidebar_collapsed: Option<bool>,
    pub settings_tabs: std::collections::BTreeMap<String, usize>,
    #[serde(default = "super::ask_options::meeting_thinking_default")]
    pub ask_meeting_thinking: super::ask_options::Thinking,
    pub ask_chat_thinking: super::ask_options::Thinking,
    pub ask_models: std::collections::BTreeMap<String, String>,
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
            show_full_titles: false,
            tasks_mine: false,
            tasks_include_unassigned: false,
            tasks_done: Default::default(),
            task_titles_notice_seen: false,
            sidebar_collapsed: None,
            settings_tabs: Default::default(),
            ask_meeting_thinking: super::ask_options::Thinking::Low,
            ask_chat_thinking: super::ask_options::Thinking::Medium,
            ask_models: Default::default(),
        }
    }
}

fn path(root: &Path) -> PathBuf {
    root.join("local").join("ui.json")
}

impl UiPrefs {
    /// A missing or unreadable file is the defaults. A preference is never worth an error.
    pub fn load(root: &Path) -> Self {
        store_io::read(&path(root)).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        store_io::write(&path(root), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
