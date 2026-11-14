//! Device-local preparation only. No stage consumes these choices yet.
mod migration;
mod persistence;
pub(super) use persistence::prepare_save;
mod stages;
mod status;
pub use stages::StageSettings;
pub use status::{FieldStatus, Source, Validation};

use super::{
    Config,
    agent_instructions::{self, Agent},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    pub version: u32,
    // The existing notes name and agent instruction store can sync; these must not.
    pub owner_name: String,
    pub owner_aliases: Vec<String>,
    pub product: String,
    pub project_key: String,
    pub transcript_pattern: String,
    pub context_pattern: String,
    /// Legacy Hex model identifiers, with existing explicit routes taking precedence.
    pub models: BTreeMap<String, String>,
    pub worker_threads: u32,
    /// Transcription override only; None and Some(0) use the computed ASR deadline.
    pub stage_deadline_seconds: Option<u32>,
    pub steps: Steps,
    pub stages: StageSettings,
    pub notes_prompt: Option<String>,
    pub translation_prompt: Option<String>,
    #[serde(default)]
    pub(super) legacy_imported: bool,
    pub(super) backup_file: Option<String>,
    #[serde(default)]
    pub(super) imported_fields: BTreeSet<String>,
    #[serde(default)]
    pub(super) conflicts: BTreeSet<String>,
    #[serde(default)]
    pub(super) invalid_imports: BTreeSet<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            version: 1,
            owner_name: String::new(),
            owner_aliases: Vec::new(),
            product: String::new(),
            project_key: String::new(),
            transcript_pattern: String::new(),
            context_pattern: String::new(),
            models: BTreeMap::new(),
            worker_threads: 4,
            stage_deadline_seconds: None,
            steps: Steps::default(),
            stages: StageSettings::default(),
            notes_prompt: None,
            translation_prompt: None,
            backup_file: None,
            legacy_imported: false,
            imported_fields: BTreeSet::new(),
            conflicts: BTreeSet::new(),
            invalid_imports: BTreeSet::new(),
            extra: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Steps {
    pub translation: bool,
    pub quick_title: bool,
    pub tasks: bool,
    pub publish_to_hex: bool,
    pub external_comparison: bool,
}
impl Default for Steps {
    fn default() -> Self {
        Self {
            translation: true,
            quick_title: true,
            tasks: true,
            publish_to_hex: true,
            external_comparison: false,
        }
    }
}
#[derive(Clone, Copy)]
pub enum Prompt {
    Notes,
    Translation,
}
impl EngineSettings {
    pub fn prompt(&self, prompt: Prompt) -> &str {
        match prompt {
            Prompt::Notes => self
                .notes_prompt
                .as_deref()
                .unwrap_or_else(|| agent_instructions::default_base(Agent::Notes)),
            Prompt::Translation => self.translation_prompt.as_deref().unwrap_or(include_str!(
                "../../scripts/meeting-pipeline/meeting-translate-prompt.md"
            )),
        }
    }
    pub fn reset_prompt(&mut self, prompt: Prompt) {
        match prompt {
            Prompt::Notes => self.notes_prompt = None,
            Prompt::Translation => self.translation_prompt = None,
        }
    }
}
impl Config {
    /// Existing declared provider routes remain authoritative, including extra languages.
    pub fn engine_model(&self, language: &str) -> Option<&str> {
        self.pipeline
            .after_stop_languages
            .as_ref()
            .and_then(|routes| routes.routes.get(language))
            .filter(|route| !route.model.trim().is_empty())
            .map(|route| route.model.as_str())
            .or_else(|| {
                self.engine_settings
                    .models
                    .get(language)
                    .map(String::as_str)
            })
    }
    /// Static field names only; neither errors nor statuses contain user values.
    pub fn validate_engine_settings(&self) -> Vec<&'static str> {
        let settings = &self.engine_settings;
        let mut errors = Vec::new();
        if settings.version != 1 {
            errors.push("engine_settings.version");
        }
        if !(1..=256).contains(&settings.worker_threads) {
            errors.push("worker_threads");
        }
        if settings.stage_deadline_seconds.is_some_and(|v| v > 3600) {
            errors.push("stage_deadline_seconds");
        }
        for (field, value) in [
            ("owner_name", settings.owner_name.as_str()),
            ("product", &settings.product),
            ("project_key", &settings.project_key),
            ("company", &self.knowledge_base.company_name),
            ("knowledge_base.repo_path", &self.knowledge_base.repo_path),
            (
                "notes.model",
                self.notes.as_ref().map_or("", |s| s.model.as_str()),
            ),
        ] {
            if !valid_text(value) {
                errors.push(field);
            }
        }
        if settings
            .owner_aliases
            .iter()
            .any(|v| v.trim().is_empty() || !valid_text(v))
        {
            errors.push("owner_aliases");
        }
        for (field, pattern) in [
            ("transcript_pattern", &settings.transcript_pattern),
            ("context_pattern", &settings.context_pattern),
        ] {
            if !valid_pattern(pattern) {
                errors.push(field);
            }
        }
        if !valid_revision(&self.knowledge_base.revision) {
            errors.push("knowledge_base.revision");
        }
        if settings.models.iter().any(|(language, model)| {
            !super::lid::LANGUAGES.contains(&language.as_str()) || !valid_model(model)
        }) {
            errors.push("models");
        }
        for (field, text) in [
            ("notes_prompt", &settings.notes_prompt),
            ("translation_prompt", &settings.translation_prompt),
        ] {
            if text.as_ref().is_some_and(|s| !valid_prompt(s)) {
                errors.push(field);
            }
        }
        if self
            .notes
            .as_ref()
            .is_some_and(|s| s.helper == Some(super::notes::Helper::OpenCode))
        {
            errors.push("notes.helper");
        }
        errors.extend(settings.stages.errors());
        errors
    }
}
pub(super) fn valid_text(value: &str) -> bool {
    value.len() <= 4096 && !value.chars().any(char::is_control)
}
pub(super) fn valid_pattern(value: &str) -> bool {
    valid_text(value) && regex::Regex::new(value).is_ok()
}
pub(super) fn valid_model(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_/.:".contains(&c))
}
fn valid_revision(value: &str) -> bool {
    value.is_empty()
        || (value.starts_with("origin/")
            && value.len() > 7
            && valid_text(value)
            && !value.chars().any(char::is_whitespace)
            && !value.contains("..")
            && !value.contains("@{")
            && !value.chars().any(|c| "~^:?*[\\".contains(c)))
}
fn valid_prompt(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= agent_instructions::MAX_CHARS
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
}
/// Canonical paths are shared by validation, field status, and edit-scoped saves.
pub(super) fn field_pointer(field: &str) -> String {
    match field {
        "company" => "/knowledge_base/company_name".into(),
        f if f.starts_with("notes.")
            || f.starts_with("knowledge_base.")
            || f.starts_with("engine_settings.") =>
        {
            format!("/{}", f.replace('.', "/"))
        }
        f => format!("/engine_settings/{}", f.replace('.', "/")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_round_trip_and_prompt_reset() {
        let mut config = Config::default();
        assert_eq!(config.engine_settings.worker_threads, 4);
        assert!(!config.engine_settings.steps.external_comparison);
        config.engine_settings.notes_prompt = Some("Synthetic notes instructions".into());
        config.engine_settings.translation_prompt =
            Some("Synthetic translation instructions".into());
        config
            .engine_settings
            .models
            .insert("ur".into(), "example-model".into());
        config.notes = Some(super::super::notes::NotesSettings {
            helper: Some(super::super::notes::Helper::Codex),
            model: "synthetic-model".into(),
            ..Default::default()
        });
        let bytes = serde_json::to_vec(&config).unwrap();
        let mut loaded: Config = serde_json::from_slice(&bytes).unwrap();
        assert!(loaded.validate_engine_settings().is_empty());
        assert_eq!(loaded.engine_model("ur"), Some("example-model"));
        for prompt in [Prompt::Notes, Prompt::Translation] {
            loaded.engine_settings.reset_prompt(prompt);
            assert_eq!(
                loaded.engine_settings.prompt(prompt),
                EngineSettings::default().prompt(prompt)
            );
        }
        assert_eq!(serde_json::to_vec(&config).unwrap(), bytes);
    }
    #[test]
    fn validation_reports_only_field_names() {
        let mut config = Config::default();
        config.engine_settings.worker_threads = 0;
        config.engine_settings.stage_deadline_seconds = Some(3601);
        config.engine_settings.transcript_pattern = "[synthetic".into();
        config.engine_settings.owner_aliases.push("\n".into());
        config
            .engine_settings
            .models
            .insert("invented".into(), "bad model".into());
        config.engine_settings.notes_prompt = Some("\0".into());
        config.knowledge_base.revision = "origin/../synthetic".into();
        assert_eq!(
            config.validate_engine_settings(),
            [
                "worker_threads",
                "stage_deadline_seconds",
                "owner_aliases",
                "transcript_pattern",
                "knowledge_base.revision",
                "models",
                "notes_prompt"
            ]
        );
    }
}
