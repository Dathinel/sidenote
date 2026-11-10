//! Window preferences that belong to this installation only, such as whether the sidebar is
//! folded. They live in `local/ui.json`, which the state share's ignore file leaves out, so a
//! preference never travels to another device.
use super::store_io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
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
    fn task_filters_default_to_everyone_and_round_trip_with_existing_keys() {
        let mut prefs: UiPrefs =
            serde_json::from_str(r#"{"show_full_titles":true,"settings_tabs":{"Meetings":1}}"#)
                .unwrap();
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
        prefs.settings_tabs.insert("Meetings".into(), 1);
        prefs.settings_tabs.insert("AI helpers".into(), 2);
        let round_trip: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(round_trip, prefs);
        assert!(round_trip.show_full_titles && round_trip.task_titles_notice_seen);
        assert_eq!(round_trip.sidebar_collapsed, Some(false));
    }
}
