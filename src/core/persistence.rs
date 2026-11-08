use super::state_dir;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};
static WRITES: Mutex<()> = Mutex::new(());
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_order: Option<usize>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Saved {
    pub checked: BTreeSet<String>,
    pub messages: Vec<Message>,
}
#[derive(Clone, Debug)]
pub struct StateStore {
    pub root: PathBuf,
}
impl Default for StateStore {
    fn default() -> Self {
        Self { root: state_dir() }
    }
}
/// The key of the Ask thread across all meetings, which is not a meeting id.
pub const ALL_KEY: &str = "all";
fn checked_path(root: &Path, id: &str) -> io::Result<PathBuf> {
    let valid = id == ALL_KEY
        || id.split_once('-').is_some_and(|(a, b)| {
            !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
        });
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid meeting id",
        ));
    }
    Ok(root.join(format!("{id}.json")))
}
impl StateStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    /// Reads the merged view. Peer files and legacy inputs are never rewritten.
    pub fn load(&self, id: &str) -> io::Result<Saved> {
        checked_path(&self.root, id)?;
        super::sync_state::load(&self.root, &format!("{id}.json"))
    }
    /// Import snapshot additions. Explicit tick/history operations handle removals.
    pub fn save(&self, id: &str, saved: &Saved) -> io::Result<()> {
        self.update(id, |current| {
            current.checked.extend(saved.checked.iter().cloned());
            for message in &saved.messages {
                if !current.messages.iter().any(|m| match (&m.id, &message.id) {
                    (Some(a), Some(b)) => a == b,
                    _ => m.role == message.role && m.text == message.text,
                }) {
                    current.messages.push(message.clone());
                }
            }
        })
        .map(|_| ())
    }
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Saved)) -> io::Result<Saved> {
        let _guard = WRITES
            .lock()
            .map_err(|_| io::Error::other("State lock poisoned"))?;
        checked_path(&self.root, id)?;
        super::sync_state::update(&self.root, &format!("{id}.json"), change)
    }
    pub fn set_tick(&self, id: &str, task: &str, checked: bool) -> io::Result<Saved> {
        self.update(id, |s| {
            if checked {
                s.checked.insert(task.into());
            } else {
                s.checked.remove(task);
            }
        })
    }
    pub fn append_message(&self, id: &str, message: Message) -> io::Result<Saved> {
        self.append_message_identified(id, message)
            .map(|(saved, _)| saved)
    }
    /// Returns the exact submitted identity even if another device appends during the save.
    pub fn append_message_identified(
        &self,
        id: &str,
        mut message: Message,
    ) -> io::Result<(Saved, String)> {
        let message_id = match message.id.as_ref().filter(|s| !s.is_empty()) {
            Some(id) => id.clone(),
            None => super::sync_state::new_id(&self.root)?,
        };
        message.id = Some(message_id.clone());
        message.at.get_or_insert_with(|| {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        });
        let saved = self.update(id, |s| s.messages.push(message))?;
        Ok((saved, message_id))
    }
    pub fn clear_history(&self, id: &str) -> io::Result<Saved> {
        self.update(id, |s| s.messages.clear())
    }
}
pub fn read_saved(id: &str) -> io::Result<Saved> {
    StateStore::default().load(id)
}
pub fn save(id: &str, saved: &Saved) -> io::Result<()> {
    StateStore::default().save(id, saved)
}
