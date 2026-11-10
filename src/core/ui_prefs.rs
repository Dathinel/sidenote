//! Window preferences that belong to this installation only, such as whether the sidebar is
//! folded. They live in `local/ui.json`, which the state share's ignore file leaves out, so a
//! preference never travels to another device.
use super::store_io;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub show_full_titles: bool,
    pub task_titles_notice_seen: bool,
    /// None until the owner folds or opens the sidebar once.
    pub sidebar_collapsed: Option<bool>,
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
