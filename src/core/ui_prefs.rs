//! Window preferences that belong to this installation only, such as whether the sidebar is
//! folded. They live in `local/ui.json`, which the state share's ignore file leaves out, so a
//! preference never travels to another device.
use super::store_io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub show_full_titles: bool,
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
            show_full_titles: false,
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
