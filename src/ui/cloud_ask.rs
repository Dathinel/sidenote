use super::{
    State,
    hex::settings::{card, setting_row},
    roles,
    theme::Palette,
    widgets::{self, ButtonRole},
};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Select, SelectAppearance, TextInput};
use sidenote::core::provider::{self, Metrics, Settings};
use std::collections::{HashMap, HashSet};

pub struct Connection {
    pub settings: Settings,
    url: TextEdit,
    model: TextEdit,
    key: TextEdit,
    key_saved: bool,
    pub status: Option<String>,
    pub catalogue: Vec<String>,
    pub catalogue_busy: bool,
    pub metrics: HashMap<String, Metrics>,
    pub labels: HashMap<String, String>,
    pub slow: HashSet<String>,
}
impl Connection {
    pub fn new() -> Self {
        let loaded = Settings::load();
        let status = loaded.as_ref().err().cloned();
        let settings = loaded.unwrap_or_default();
        let settings = if settings.base_url.is_empty() {
            Settings::local()
        } else {
            settings
        };
        let mut key = TextEdit::default();
        key.set_secure(true);
        Self {
            url: TextEdit::new(&settings.base_url),
            model: TextEdit::new(&settings.model),
            key,
            key_saved: provider::key_available_for(&settings),
            settings,
            status,
            catalogue: provider::OPENROUTER_MODELS
                .iter()
                .map(|s| (*s).into())
                .collect(),
            catalogue_busy: false,
            metrics: HashMap::new(),
            labels: HashMap::new(),
            slow: HashSet::new(),
        }
    }
    fn refresh_key(&mut self) {
        self.key_saved = provider::key_available_for(&self.settings);
    }
    fn fill(&mut self, settings: Settings) {
        self.url = TextEdit::new(&settings.base_url);
        self.model = TextEdit::new(&settings.model);
        // Presets fill fields only. Saving and consent are separate user actions.
        self.status = Some("Preset filled in. Save the connection to use these values.".into());
    }
    fn draft(&self) -> Settings {
        let mut settings = self.settings.clone();
        settings.base_url = self.url.text.trim().into();
        settings.model = self.model.text.trim().into();
        if settings.base_url != self.settings.base_url || settings.model != self.settings.model {
            settings.consent = None;
        }
        settings
    }
}
impl State {
    pub fn save_ask_connection(&mut self) {
        let next = self.ask_settings.connection.draft();
        match next.save() {
            Ok(()) => {
                self.ask_settings.connection.settings = next;
                self.ask_settings.connection.refresh_key();
                self.ask_settings.connection.status = Some("Connection saved on this device. Cloud Ask still requires opt-in for this endpoint and model.".into());
            }
            Err(e) => self.ask_settings.connection.status = Some(e),
        }
    }
    fn stop_endpoint_attempt(&mut self) {
        let id = self.ask_key();
        if let Some(run) = self.asks.remove(&id) {
            run.cancel.stop();
            self.finish_ask(&id, run, Some(sidenote::core::ask::STOPPED.into()));
        }
    }
    pub(super) fn open_connection_settings(&mut self) {
        let question = self.question.clone();
        self.open_hex();
        self.question = question;
        super::hex::settings::navigate(self, super::hex::settings::Page::Helpers, 0);
        self.pending_scroll = Some(super::PendingScroll {
            key: "ask-connection".into(),
            center: true,
            left: 3,
        });
    }
    pub fn connection_recovery(&mut self, choice: usize) {
        self.stop_endpoint_attempt();
        match choice {
            0 => {
                let question = self.question.text.clone();
                self.send(&question);
            }
            1 => {
                self.ask_settings.connection.fill(Settings::local());
                self.open_connection_settings();
            }
            _ => self.open_connection_settings(),
        }
    }
}
fn action(label: &str, key: &str, p: Palette) -> El {
    widgets::button(label, key, ButtonRole::Secondary, p)
}
pub fn section(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let c = &s.ask_settings.connection;
    let presets = [
        "OpenRouter",
        "Local \u{b7} Ollama",
        "Local \u{b7} LM Studio",
        "Local \u{b7} llama.cpp",
        "Local \u{b7} vLLM",
    ];
    let preset = Select::new("ask-connection-preset", &presets.map(str::to_string))
        .label("Connection preset")
        .placeholder("Fill from a preset")
        .width(230.0)
        .appearance(SelectAppearance::Field)
        .open(s.ask_settings.select_open.as_deref() == Some("ask-connection-preset"))
        .on_toggle(|s: &mut State| {
            s.ask_settings.select_open = (s.ask_settings.select_open.as_deref()
                != Some("ask-connection-preset"))
            .then(|| "ask-connection-preset".into())
        })
        .on_close(|s: &mut State| s.ask_settings.select_open = None)
        .on_choose(|s: &mut State, value: &str, _| {
            let mut settings = if value == "OpenRouter" {
                Settings::openrouter()
            } else {
                Settings::local()
            };
            if value.contains("LM Studio") {
                settings.base_url = "http://127.0.0.1:1234/v1".into();
                settings.model = "local-model".into();
            }
            if value.contains("llama.cpp") || value.contains("vLLM") {
                settings.base_url = "http://127.0.0.1:8080/v1".into();
                settings.model = "local-model".into();
            }
            if value.contains("vLLM") {
                settings.base_url = "http://127.0.0.1:8000/v1".into();
            }
            s.ask_settings.connection.fill(settings);
            s.ask_settings.select_open = None;
        })
        .build(k, layer);
    let url = TextInput::new(&c.url)
        .label("Endpoint base URL")
        .placeholder("https://host/v1")
        .width(Some(270.0))
        .on_edit(|s: &mut State| &mut s.ask_settings.connection.url)
        .build(k)
        .key("ask-connection-url");
    let model = TextInput::new(&c.model)
        .label("Endpoint model")
        .placeholder("Exact model ID")
        .width(Some(270.0))
        .on_edit(|s: &mut State| &mut s.ask_settings.connection.model)
        .build(k)
        .key("ask-connection-model");
    let key = TextInput::new(&c.key)
        .secure()
        .label("Optional API key")
        .placeholder("Leave empty for no key")
        .width(Some(270.0))
        .on_edit(|s: &mut State| &mut s.ask_settings.connection.key)
        .build(k)
        .key("ask-connection-key");
    let mut models = col().gap(8.0);
    if c.url.text.trim() == provider::OPENROUTER_URL {
        let catalog = Select::new("ask-connection-catalogue", &c.catalogue)
            .label("OpenRouter models")
            .placeholder("Choose a model ID")
            .width(270.0)
            .appearance(SelectAppearance::Field)
            .open(s.ask_settings.select_open.as_deref() == Some("ask-connection-catalogue"))
            .on_toggle(|s: &mut State| {
                s.ask_settings.select_open = (s.ask_settings.select_open.as_deref()
                    != Some("ask-connection-catalogue"))
                .then(|| "ask-connection-catalogue".into())
            })
            .on_close(|s: &mut State| s.ask_settings.select_open = None)
            .on_choose(|s: &mut State, value: &str, _| {
                s.ask_settings.connection.model = TextEdit::new(value);
                s.ask_settings.select_open = None;
            })
            .build(k, layer);
        models = models.child(catalog).child(action(if c.catalogue_busy { "Loading models..." } else { "Load OpenRouter model list" }, "ask-connection-load-models", p)
            .disabled(c.catalogue_busy)
            .on_press(|s: &mut State| {
                if rune::tooling::headless_run().is_some() { return; }
                s.ask_settings.connection.catalogue_busy = true;
                let sender = s.events_tx.clone();
                let waker = s.waker.clone();
                std::thread::spawn(move || {
                    let _ = sender.send(super::Event::CloudModels(provider::openrouter_models()));
                    if let Some(waker) = waker { waker(); }
                });
            }))
            .child(roles::META.wrap("Load fetches the public model list without a key or meeting content. Until loaded, the list contains editable suggestions. Prices are not estimated.", p.faint));
    }
    let mut body = col().w_full().gap(10.0)
        .child(roles::SMALL.wrap("Stream direct Ask answers using an OpenAI-compatible server. Presets fill editable fields; they do not enable Ask or send content.", p.soft))
        .child(setting_row(s, "ask-connection-preset-row", "Preset", "Local servers: Ollama, LM Studio, llama.cpp and vLLM.", preset, true, p))
        .child(setting_row(s, "ask-connection-url-row", "Base URL", "Shown on every Ask. HTTPS is required except on localhost.", url, true, p))
        .child(setting_row(s, "ask-connection-model-row", "Model", "Exact server model ID. OpenRouter suggestions are editable; availability and cost depend on the provider.", model, true, p))
        .child(models)
        .child(action("Save connection", "ask-connection-save", p).on_press(|s: &mut State| s.save_ask_connection()))
        .child(setting_row(s, "ask-connection-key-row", "API key (optional)", if c.key_saved { "A protected key is saved for this endpoint. Delete removes saved keys for every endpoint." } else { if cfg!(windows) { "Stored using Windows current-user DPAPI, outside config and sync. Delete removes keys for every endpoint." } else { "API key storage is unavailable on Linux." } }, key, true, p))
        .child(row().gap(8.0).flex_wrap()
            .child(action("Save key", "ask-connection-key-save", p).on_press(|s: &mut State| {
                let result = if s.ask_settings.connection.draft() != s.ask_settings.connection.settings { Err("Save the endpoint and model before saving its key".into()) } else { provider::save_key(s.ask_settings.connection.key.text.as_bytes()) };
                s.ask_settings.connection.key = TextEdit::default();
                s.ask_settings.connection.key.set_secure(true);
                s.ask_settings.connection.refresh_key();
                s.ask_settings.connection.status = Some(result.map(|_| "Protected key saved for this endpoint".into()).unwrap_or_else(|e| e));
            }))
            .child(action("Delete all saved keys", "ask-connection-key-delete", p).on_press(|s: &mut State| {
                s.ask_settings.connection.status = Some(provider::delete_key().map(|_| "Protected Ask keys deleted for every endpoint".into()).unwrap_or_else(|e| e));
                s.ask_settings.connection.refresh_key();
            })))
        .child(roles::SMALL.wrap(format!("Ask recipient: {}. Sends the question, bounded conversation history and selected context: summaries, transcripts, typed notes, tasks and memory when included by the current Ask scope. Local gateways can forward content to another service. Other jobs keep their own settings and consent.", c.settings.label()), p.soft).key("ask-connection-consent-disclosure"))
        .child(row().gap(8.0).flex_wrap()
            .child(action(if c.settings.enabled { "Use installed CLI for Ask" } else { "Use this connection for Ask" }, "ask-connection-enable", p).on_press(|s: &mut State| {
                let mut next = s.ask_settings.connection.settings.clone(); next.enabled = !next.enabled;
                match next.save() {
                    Ok(()) => s.ask_settings.connection.settings = next,
                    Err(e) => s.ask_settings.connection.status = Some(e),
                }
            }))
            .child(action(if c.settings.consent.is_some() { "Revoke cloud Ask consent" } else { "Opt Ask in for this endpoint and model" }, "ask-connection-consent", p).on_press(|s: &mut State| {
                let mut next = s.ask_settings.connection.settings.clone();
                let result = if next.consent.is_some() { next.consent = None; Ok(()) }
                    else { next.consent_identity().map(|identity| next.consent = Some(identity)) };
                match result.and_then(|_| next.save()) {
                    Ok(()) => s.ask_settings.connection.settings = next,
                    Err(e) => s.ask_settings.connection.status = Some(e),
                }
            })));
    if let Some(status) = &c.status {
        body = body.child(
            roles::META
                .wrap(status, p.soft)
                .role("status")
                .key("ask-connection-status"),
        );
    }
    card("Ask connection", body, p).key("ask-connection")
}
pub fn recovery(s: &State, p: Palette) -> Option<El> {
    let id = s.ask_key();
    if !s.ask_settings.connection.labels.contains_key(&id)
        || (!s.ask_errors.contains_key(&id)
            && !(s.asks.contains_key(&id) && s.ask_settings.connection.slow.contains(&id)))
    {
        return None;
    }
    let mut choices = row().gap(8.0).flex_wrap();
    for (i, label) in provider::FALLBACK_CHOICES.iter().enumerate() {
        choices = choices.child(
            action(label, &format!("ask-connection-recovery-{i}"), p)
                .on_press(move |s: &mut State| s.connection_recovery(i)),
        );
    }
    Some(col().w_full().gap(6.0)
        .child(roles::META.wrap(if s.asks.contains_key(&id) { "Endpoint is taking longer than 15 seconds. You can keep waiting or start a new attempt." } else { "Choose how to retry. Changing endpoint or model requires saving the connection; remote changes require Ask opt-in." }, p.warn).role("alert"))
        .child(choices))
}
pub fn scene(s: &mut State, settings: bool) {
    if settings {
        s.ask_settings.connection.key = TextEdit::new("synthetic-render-only-key");
        s.ask_settings.connection.key.set_secure(true);
        super::hex::settings::scene(s, "settings-helpers");
        s.pending_scroll = Some(super::PendingScroll {
            key: "ask-connection".into(),
            center: true,
            left: 3,
        });
    } else {
        super::scene_meeting(s);
        s.saved.messages = vec![
            sidenote::core::Message {
                role: "user".into(),
                text: "What did we decide?".into(),
                ..Default::default()
            },
            sidenote::core::Message {
                role: "assistant".into(),
                text: "Test the shorter workshop with five participants next Monday. [00:42]"
                    .into(),
                ..Default::default()
            },
        ];
        s.ask_settings.connection.labels.insert(
            s.ask_key(),
            "Cloud \u{b7} https://openrouter.ai/api/v1 \u{b7} openai/gpt-4.1-mini".into(),
        );
        s.ask_settings.connection.metrics.insert(
            s.ask_key(),
            Metrics {
                first_token_ms: Some(430),
                total_ms: 1740,
                prompt_tokens: Some(320),
                completion_tokens: Some(28),
                total_tokens: Some(348),
            },
        );
    }
    s.ask_settings.connection.settings = Settings {
        enabled: true,
        ..Settings::openrouter()
    };
    s.ask_settings.connection.url = TextEdit::new(provider::OPENROUTER_URL);
    s.ask_settings.connection.model = TextEdit::new(provider::OPENROUTER_MODELS[0]);
}
