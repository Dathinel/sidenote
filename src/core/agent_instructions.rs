//! Synced base instructions; locked safety and output rules are compiled into the app.
use super::{store_io, sync_state};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::Path};

pub const PROMPT_VERSION: u32 = 1;
pub const MAX_CHARS: usize = 16_000;
const STORE: &str = "agent-instructions.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Notes,
    Tasks,
    Title,
    Ask,
    AskAll,
}
impl Agent {
    pub const ALL: [Agent; 5] = [
        Agent::Notes,
        Agent::Tasks,
        Agent::Title,
        Agent::Ask,
        Agent::AskAll,
    ];
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub text: String,
    pub based_on: u32,
    pub edited_at: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(skip)]
    pub using_defaults_after_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_all: Option<Entry>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
impl Store {
    pub fn get(&self, agent: Agent) -> Option<&Entry> {
        match agent {
            Agent::Notes => self.notes.as_ref(),
            Agent::Tasks => self.tasks.as_ref(),
            Agent::Title => self.title.as_ref(),
            Agent::Ask => self.ask.as_ref(),
            Agent::AskAll => self.ask_all.as_ref(),
        }
    }
    fn slot(&mut self, agent: Agent) -> &mut Option<Entry> {
        match agent {
            Agent::Notes => &mut self.notes,
            Agent::Tasks => &mut self.tasks,
            Agent::Title => &mut self.title,
            Agent::Ask => &mut self.ask,
            Agent::AskAll => &mut self.ask_all,
        }
    }
    fn validate(&self) -> io::Result<()> {
        for agent in Agent::ALL {
            if let Some(entry) = self.get(agent) {
                validate_text(&entry.text)?;
                if entry.based_on == 0
                    || chrono::DateTime::parse_from_rfc3339(&entry.edited_at).is_err()
                {
                    return Err(store_io::invalid(
                        "Invalid agent instruction version or edit time",
                    ));
                }
            }
        }
        Ok(())
    }
}
fn validate_text(text: &str) -> io::Result<()> {
    if text.chars().count() > MAX_CHARS {
        return Err(store_io::invalid(
            "Agent instructions exceed 16,000 characters",
        ));
    }
    if text.contains('\0') {
        return Err(store_io::invalid(
            "Agent instructions cannot contain NUL characters",
        ));
    }
    Ok(())
}
pub(crate) fn validate_store(value: &serde_json::Value) -> io::Result<()> {
    let store: Store = serde_json::from_value(value.clone())?;
    store.validate()
}
pub fn core_head(a: Agent) -> &'static str {
    match a {
        Agent::Notes => include_str!("agent_instructions/notes_core_head.txt"),
        Agent::Tasks => include_str!("agent_instructions/tasks_core_head.txt"),
        Agent::Title => include_str!("agent_instructions/title_core_head.txt"),
        Agent::Ask => include_str!("agent_instructions/ask_core_head.txt"),
        Agent::AskAll => include_str!("agent_instructions/ask_all_core_head.txt"),
    }
}
pub fn core_tail(a: Agent) -> &'static str {
    match a {
        Agent::Notes => include_str!("agent_instructions/notes_core_tail.txt"),
        Agent::Tasks => include_str!("agent_instructions/tasks_core_tail.txt"),
        Agent::Title => include_str!("agent_instructions/title_core_tail.txt"),
        Agent::Ask => include_str!("agent_instructions/ask_core_tail.txt"),
        Agent::AskAll => include_str!("agent_instructions/ask_all_core_tail.txt"),
    }
}
pub fn default_base(a: Agent) -> &'static str {
    match a {
        Agent::Notes => include_str!("agent_instructions/notes_base.txt"),
        Agent::Tasks => include_str!("agent_instructions/tasks_base.txt"),
        Agent::Title => include_str!("agent_instructions/title_base.txt"),
        Agent::Ask => include_str!("agent_instructions/ask_base.txt"),
        Agent::AskAll => include_str!("agent_instructions/ask_all_base.txt"),
    }
}
/// The prompt text without the meeting material: fixed head, your base, fixed tail. Notes and
/// Tasks add their material after this, with the same layout the helper ladder uses.
pub fn compose(a: Agent, base: &str) -> String {
    let (head, tail) = (core_head(a), core_tail(a));
    match a {
        Agent::Notes | Agent::Tasks => format!("{head}\n{base}\n{tail}\n"),
        Agent::Title => format!("{}\n\n{}\n{}\n", head.trim(), base.trim(), tail.trim()),
        // Ask rules are one paragraph: Ask adds its live-meeting lines after the first blank line.
        Agent::Ask | Agent::AskAll => [head.trim(), &one_paragraph(base), tail.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}
fn one_paragraph(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
/// The prompt text with the shipped default base.
pub fn default_prompt(a: Agent) -> String {
    compose(a, default_base(a))
}
/// The prompt text with your saved base, or the default base when you have none or the store
/// cannot be read. Used by Ask and meeting titles, which must never fail because of a bad file.
pub fn effective(root: &Path, a: Agent) -> String {
    match base(root, a) {
        Ok((text, _)) => compose(a, &text),
        Err(_) => default_prompt(a),
    }
}
pub fn load(root: &Path) -> io::Result<Store> {
    match sync_state::load_report::<Store>(root, STORE) {
        Ok((mut store, warning)) if store.validate().is_ok() => {
            store.using_defaults_after_error = warning;
            Ok(store)
        }
        _ => Ok(Store {
            using_defaults_after_error: true,
            ..Store::default()
        }),
    }
}
pub fn base(root: &Path, a: Agent) -> io::Result<(String, bool)> {
    let store = load(root)?;
    Ok(store
        .get(a)
        .map(|e| (e.text.clone(), true))
        .unwrap_or_else(|| (default_base(a).into(), false)))
}
pub fn save(root: &Path, a: Agent, text: &str) -> io::Result<()> {
    validate_text(text)?;
    let entry = Entry {
        text: text.into(),
        based_on: PROMPT_VERSION,
        edited_at: chrono::Utc::now().to_rfc3339(),
    };
    sync_state::update::<Store>(root, STORE, |store| {
        *store.slot(a) = Some(entry);
    })?;
    Ok(())
}
pub fn reset(root: &Path, a: Agent) -> io::Result<()> {
    sync_state::update::<Store>(root, STORE, |store| {
        *store.slot(a) = None;
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sidenote-instructions-{}-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn defaults_save_load_reset_and_metadata() {
        let root = Temp::new();
        assert_eq!(
            base(&root.0, Agent::Notes).unwrap(),
            (default_base(Agent::Notes).into(), false)
        );
        save(&root.0, Agent::Notes, "Keep exact decisions.").unwrap();
        assert_eq!(
            base(&root.0, Agent::Notes).unwrap(),
            ("Keep exact decisions.".into(), true)
        );
        let store = load(&root.0).unwrap();
        assert_eq!(store.notes.unwrap().based_on, 1);
        reset(&root.0, Agent::Notes).unwrap();
        assert!(!base(&root.0, Agent::Notes).unwrap().1);
        assert!(
            base(&root.0, Agent::Tasks)
                .unwrap()
                .0
                .contains("existing project")
        );
    }
    #[test]
    fn limits_count_characters_and_reject_nul_before_writing() {
        let root = Temp::new();
        save(&root.0, Agent::Notes, &"é".repeat(MAX_CHARS)).unwrap();
        assert!(save(&root.0, Agent::Notes, &"é".repeat(MAX_CHARS + 1)).is_err());
        assert!(save(&root.0, Agent::Tasks, "bad\0text").is_err());
        assert_eq!(
            base(&root.0, Agent::Notes).unwrap().0.chars().count(),
            MAX_CHARS
        );
    }
    #[test]
    fn legacy_store_is_registered_and_validated() {
        let root = Temp::new();
        let v = serde_json::json!({"notes":{"text":"Plain notes","based_on":1,"edited_at":"2026-11-10T04:00:00Z"}});
        fs::write(root.0.join(STORE), v.to_string()).unwrap();
        assert_eq!(
            base(&root.0, Agent::Notes).unwrap(),
            ("Plain notes".into(), true)
        );
        let mut bad = v;
        bad["notes"]["text"] = serde_json::json!("bad\0text");
        assert!(validate_store(&bad).is_err());
    }
    #[test]
    fn invalid_synced_instructions_load_defaults_and_expose_warning() {
        let root = Temp::new();
        for raw in ["invalid JSON".to_owned(), serde_json::json!({"notes":{"text":"bad\u{0000}text","based_on":1,"edited_at":"2026-11-10T04:00:00Z"}}).to_string()] {
            fs::write(root.0.join(STORE), &raw).unwrap();
            let store = load(&root.0).unwrap();
            assert!(store.using_defaults_after_error);
            assert!(store.notes.is_none());
            assert_eq!(base(&root.0, Agent::Notes).unwrap(), (default_base(Agent::Notes).into(), false));
            assert_eq!(fs::read_to_string(root.0.join(STORE)).unwrap(), raw);
        }
        save(&root.0, Agent::Notes, "Repaired instructions").unwrap();
        assert_eq!(
            base(&root.0, Agent::Notes).unwrap(),
            ("Repaired instructions".into(), true)
        );
    }
    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let to = target.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }
    #[test]
    fn two_devices_editing_different_agents_merge_and_reset_independently() {
        let a = Temp::new();
        let b = Temp::new();
        save(&a.0, Agent::Notes, "Notes from device one").unwrap();
        save(&b.0, Agent::Tasks, "Tasks from device two").unwrap();
        copy_tree(&a.0.join("state"), &b.0.join("state"));
        copy_tree(&b.0.join("state"), &a.0.join("state"));
        for root in [&a.0, &b.0] {
            assert_eq!(base(root, Agent::Notes).unwrap().0, "Notes from device one");
            assert_eq!(base(root, Agent::Tasks).unwrap().0, "Tasks from device two");
        }
        reset(&a.0, Agent::Notes).unwrap();
        copy_tree(&a.0.join("state"), &b.0.join("state"));
        assert!(!base(&b.0, Agent::Notes).unwrap().1);
        assert!(base(&b.0, Agent::Tasks).unwrap().1);
    }
}
