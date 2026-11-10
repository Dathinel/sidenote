use super::{
    State,
    hex::settings::{card, setting_row},
    roles,
    theme::Palette,
    widgets::{hairline, pressable},
};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Select, SelectAppearance, TextInput};
use sidenote::core::{
    self, UiPrefs,
    ask_options::{self, Thinking},
};
use std::collections::{BTreeMap, HashMap};

pub const OPENCODE_THINKING: &str = "OpenCode has no separate effort flag. Choose a thinking variant with provider/model#variant in Models and agents.";

pub struct AskSettings {
    pub meeting: Thinking,
    pub chat: Thinking,
    pub models: BTreeMap<String, String>,
    pub model_input: TextEdit,
    pub thinking_open: bool,
    pub select_open: Option<String>,
    pub last_end_ms: Option<i64>,
    pub last_live: Option<ask_options::LiveSource>,
    pub overrides: HashMap<(String, bool), Thinking>,
}
impl AskSettings {
    pub fn new(prefs: &UiPrefs) -> Self {
        let recent = ask_options::recent_meeting(
            &core::meetings_dir(),
            chrono::Local::now().timestamp_millis(),
        );
        Self {
            meeting: prefs.ask_meeting_thinking,
            chat: prefs.ask_chat_thinking,
            models: prefs.ask_models.clone(),
            model_input: TextEdit::default(),
            thinking_open: false,
            select_open: None,
            last_end_ms: recent.as_ref().map(|(end, _)| *end),
            last_live: recent.map(|(_, source)| source),
            overrides: HashMap::new(),
        }
    }
}
impl State {
    pub fn ask_in_meeting(&self) -> bool {
        ask_options::in_meeting(
            self.hex_recording(),
            self.ask_settings.last_end_ms,
            chrono::Local::now().timestamp_millis(),
        )
    }
    pub fn ask_thinking(&self) -> Thinking {
        let meeting = self.ask_in_meeting();
        self.ask_settings
            .overrides
            .get(&(self.ask_key(), meeting))
            .copied()
            .unwrap_or(if meeting {
                self.ask_settings.meeting
            } else {
                self.ask_settings.chat
            })
    }
    pub fn set_ask_thinking(&mut self, thinking: Thinking) {
        let meeting = self.ask_in_meeting();
        self.ask_settings
            .overrides
            .insert((self.ask_key(), meeting), thinking);
        self.ask_settings.thinking_open = false;
        if !meeting {
            self.ask_settings.chat = thinking;
            self.save_ask_preferences();
        }
    }
    pub fn save_ask_preferences(&mut self) {
        let mut prefs = UiPrefs::load(&self.store.root);
        prefs.ask_meeting_thinking = self.ask_settings.meeting;
        prefs.ask_chat_thinking = self.ask_settings.chat;
        prefs.ask_models = self.ask_settings.models.clone();
        if prefs.save(&self.store.root).is_err() {
            self.notice = Some(
                "Could not save your Ask preferences. Check the app data folder and try again."
                    .into(),
            );
        }
    }
    pub fn choose_ask_runner(&mut self, name: String) {
        self.runner = Some(name);
        self.runner_open = false;
        if let Some(name) = &self.runner
            && core::runners::remember_runner(name).is_err()
        {
            self.notice = Some("Could not remember your agent choice.".into());
        }
        self.refresh_ask_model_input();
    }
    pub fn refresh_ask_model_input(&mut self) {
        self.ask_settings.model_input = TextEdit::new(self.ask_model().as_deref().unwrap_or(""));
    }
    pub fn ask_model(&self) -> Option<String> {
        let name = self.runner.as_ref()?;
        self.ask_settings
            .models
            .get(name)
            .cloned()
            .or_else(|| self.runners.iter().find(|r| &r.name == name)?.model.clone())
    }
    pub fn save_ask_model(&mut self) {
        let Some(name) = self.runner.clone() else {
            return;
        };
        let model = self.ask_settings.model_input.text.trim().to_string();
        if model.is_empty() {
            self.ask_settings.models.remove(&name);
        } else {
            self.ask_settings.models.insert(name, model);
        }
        self.save_ask_preferences();
        self.refresh_ask_model_input();
    }
}

fn thinking(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, meeting: bool) -> El {
    if s.runners
        .iter()
        .find(|r| Some(&r.name) == s.runner.as_ref())
        .is_some_and(|r| !ask_options::supports_thinking(r))
    {
        return roles::META
            .wrap("Managed by OpenCode", p.faint)
            .help(OPENCODE_THINKING);
    }
    let key = if meeting {
        "ask-meeting-thinking"
    } else {
        "ask-chat-thinking"
    };
    let value = if meeting {
        s.ask_settings.meeting
    } else {
        s.ask_settings.chat
    };
    let options = Thinking::ALL.map(|v| v.label().to_string());
    Select::new(key, &options)
        .label(if meeting {
            "Thinking during meetings"
        } else {
            "Thinking in chats"
        })
        .current(Some(value.label()))
        .appearance(SelectAppearance::Field)
        .width(180.0)
        .open(s.ask_settings.select_open.as_deref() == Some(key))
        .on_toggle(move |s: &mut State| {
            s.ask_settings.select_open = if s.ask_settings.select_open.as_deref() == Some(key) {
                None
            } else {
                Some(key.into())
            };
        })
        .on_close(|s: &mut State| s.ask_settings.select_open = None)
        .on_choose(move |s: &mut State, value: &str, _| {
            if let Some(value) = Thinking::ALL.into_iter().find(|v| v.label() == value) {
                if meeting {
                    s.ask_settings.meeting = value;
                } else {
                    s.ask_settings.chat = value;
                }
                s.ask_settings
                    .overrides
                    .retain(|(_, during_meeting), _| *during_meeting != meeting);
                s.save_ask_preferences();
            }
            s.ask_settings.select_open = None;
        })
        .build(k, layer)
}

pub fn section(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let options: Vec<String> = s
        .runners
        .iter()
        .filter(|r| s.runner_found.contains(&r.name))
        .map(ask_options::agent_name)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    let current = s
        .runners
        .iter()
        .find(|r| Some(&r.name) == s.runner.as_ref())
        .map(ask_options::agent_name)
        .map(str::to_string);
    let agent =
        Select::new("ask-agent", &options)
            .label("Ask agent")
            .current(current.as_deref())
            .placeholder("No agent found")
            .disabled(options.is_empty())
            .appearance(SelectAppearance::Field)
            .width(180.0)
            .open(s.ask_settings.select_open.as_deref() == Some("ask-agent"))
            .on_toggle(|s: &mut State| {
                s.ask_settings.select_open =
                    if s.ask_settings.select_open.as_deref() == Some("ask-agent") {
                        None
                    } else {
                        Some("ask-agent".into())
                    };
            })
            .on_close(|s: &mut State| s.ask_settings.select_open = None)
            .on_choose(|s: &mut State, value: &str, _| {
                if let Some(r) = s.runners.iter().find(|r| {
                    ask_options::agent_name(r) == value && s.runner_found.contains(&r.name)
                }) {
                    s.choose_ask_runner(r.name.clone());
                }
                s.ask_settings.select_open = None;
            })
            .build(k, layer);
    let model = row()
        .gap(8.0)
        .items_center()
        .flex_wrap()
        .child(
            TextInput::new(&s.ask_settings.model_input)
                .label("Ask model")
                .placeholder("Agent default")
                .height(34.0)
                .on_edit(|s: &mut State| &mut s.ask_settings.model_input)
                .on_submit(|s: &mut State, _| s.save_ask_model())
                .build(k)
                .key("ask-settings-model-input")
                .w(205.0),
        )
        .child(
            pressable(roles::CONTROL.wrap("Save", p.accent))
                .key("ask-settings-model-save")
                .px(8.0)
                .py(7.0)
                .on_press(|s: &mut State| s.save_ask_model()),
        );
    let body = col()
        .w_full()
        .child(setting_row(
            s,
            "ask-agent-row",
            "Agent",
            "Answers Ask using an agent installed on this PC.",
            agent,
            true,
            p,
        ))
        .child(hairline(p))
        .child(setting_row(
            s,
            "ask-model-row",
            "Model",
            "Enter a model available in your agent account. Press Save or Enter.",
            model,
            true,
            p,
        ))
        .child(hairline(p))
        .child(setting_row(
            s,
            "ask-meeting-thinking-row",
            "Thinking during meetings",
            "Used while recording and for 10 minutes after. You can override it in Ask.",
            thinking(s, k, p, layer, true),
            true,
            p,
        ))
        .child(hairline(p))
        .child(setting_row(
            s,
            "ask-chat-thinking-row",
            "Thinking in chats",
            "The last level you choose outside meetings is remembered.",
            thinking(s, k, p, layer, false),
            true,
            p,
        ));
    card("Ask", body, p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_overrides_and_models_persist_without_changing_meeting_defaults_or_summary_settings() {
        use std::{fs, os::windows::process::CommandExt, process::Command};
        if std::env::var_os("SIDENOTE_ASK_PREFS_CHILD").is_none() {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("ask-prefs-{}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "ui::ask_settings::tests::chat_overrides_and_models_persist_without_changing_meeting_defaults_or_summary_settings", "--nocapture"])
                .env("SIDENOTE_ASK_PREFS_CHILD", "1")
                .env("APPDATA", root.join("appdata"))
                .env("LOCALAPPDATA", root.join("localappdata"))
                .env("SIDENOTE_DATA_DIR", root.join("state"))
                .env("SIDENOTE_STATE_DIR", root.join("state"))
                .env("SIDENOTE_HEX_DIR", root.join("meetings"))
                .env("SIDENOTE_PEERS_DIR", root.join("peers"))
                .creation_flags(0x08000000).status().unwrap();
            assert!(status.success());
            return;
        }
        let mut state = State::new();
        let root = state.store.root.clone();
        let settings = core::notes::NotesSettings::default();
        core::notes::save_settings(&settings).unwrap();
        let summary_before = serde_json::to_value(&settings).unwrap();
        assert_eq!(state.ask_thinking(), Thinking::Medium);
        state.set_ask_thinking(Thinking::High);
        assert_eq!(UiPrefs::load(&root).ask_chat_thinking, Thinking::High);
        state.selected = Some("1234-1".into());
        assert_eq!(state.ask_thinking(), Thinking::High);
        state.ask_settings.last_end_ms = Some(chrono::Local::now().timestamp_millis());
        assert_eq!(state.ask_thinking(), Thinking::Low);
        state.set_ask_thinking(Thinking::Medium);
        assert_eq!(state.ask_thinking(), Thinking::Medium);
        state.selected = Some("1234-2".into());
        assert_eq!(state.ask_thinking(), Thinking::Low);
        state.ask_settings.last_end_ms = Some(chrono::Local::now().timestamp_millis() - 600001);
        assert_eq!(state.ask_thinking(), Thinking::High);
        state.choose_ask_runner("Codex".into());
        state.ask_settings.model_input = TextEdit::new("chosen-model");
        state.save_ask_model();
        assert_eq!(state.ask_model().as_deref(), Some("chosen-model"));
        let prefs = UiPrefs::load(&root);
        assert_eq!(prefs.ask_meeting_thinking, Thinking::Low);
        assert_eq!(prefs.ask_chat_thinking, Thinking::High);
        assert_eq!(
            prefs.ask_models.get("Codex").map(String::as_str),
            Some("chosen-model")
        );
        assert_eq!(core::last_used_runner().unwrap().as_deref(), Some("Codex"));
        assert_eq!(
            serde_json::to_value(core::notes::settings().unwrap()).unwrap(),
            summary_before
        );
    }
}
