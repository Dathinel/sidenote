use super::{
    StageSettings, field_pointer,
    migration::{LEGACY, empty},
    valid_model,
};
use crate::core::Config;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    Saved,
    LegacyEnvironment,
    ExistingRoute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validation {
    Empty,
    Valid,
    Invalid,
    Unavailable,
    ModelQualificationRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldStatus {
    pub source: Source,
    pub validation: Validation,
    pub conflict: bool,
    pub invalid_import: bool,
}
impl Config {
    /// Value-free status for the later settings screen. Model validation is syntactic;
    /// runtime/catalog qualification belongs to the later transcription integration.
    pub fn engine_field_status(&self) -> BTreeMap<String, FieldStatus> {
        let value = serde_json::to_value(self).expect("serializable settings");
        let defaults = serde_json::to_value(Config::default()).expect("serializable defaults");
        let errors = self.validate_engine_settings();
        let mut fields: Vec<_> = LEGACY
            .iter()
            .map(|(_, field, pointer)| (*field, *pointer))
            .collect();
        fields.extend([
            ("models", "/engine_settings/models"),
            ("knowledge_base.revision", "/knowledge_base/revision"),
            ("worker_threads", "/engine_settings/worker_threads"),
            ("notes.helper", "/notes/helper"),
            ("notes.model", "/notes/model"),
            ("notes_prompt", "/engine_settings/notes_prompt"),
            ("translation_prompt", "/engine_settings/translation_prompt"),
            ("translation", "/engine_settings/steps/translation"),
            ("quick_title", "/engine_settings/steps/quick_title"),
            ("tasks", "/engine_settings/steps/tasks"),
            ("publish_to_hex", "/engine_settings/steps/publish_to_hex"),
            (
                "external_comparison",
                "/engine_settings/steps/external_comparison",
            ),
        ]);
        let settings = &self.engine_settings;
        let mut statuses: BTreeMap<_, _> = fields
            .into_iter()
            .map(|(field, pointer)| {
                let current = value.pointer(pointer).unwrap_or(&Value::Null);
                let source = if settings.imported_fields.contains(field) {
                    Source::LegacyEnvironment
                } else if empty(current) || defaults.pointer(pointer) == Some(current) {
                    Source::Default
                } else {
                    Source::Saved
                };
                (
                    field.into(),
                    FieldStatus {
                        source,
                        validation: if field == "notes.helper"
                            && self
                                .notes
                                .as_ref()
                                .is_some_and(|s| s.helper_reason().is_some())
                        {
                            Validation::Unavailable
                        } else if errors.contains(&field) {
                            Validation::Invalid
                        } else if empty(current)
                            && !["notes_prompt", "translation_prompt"].contains(&field)
                        {
                            Validation::Empty
                        } else {
                            Validation::Valid
                        },
                        conflict: settings.conflicts.contains(field),
                        invalid_import: settings.invalid_imports.contains(field),
                    },
                )
            })
            .collect();
        for field in StageSettings::FIELDS {
            let pointer = field_pointer(field);
            let current = value.pointer(&pointer).unwrap_or(&Value::Null);
            statuses.insert(
                (*field).into(),
                FieldStatus {
                    source: if defaults.pointer(&pointer) == Some(current) {
                        Source::Default
                    } else {
                        Source::Saved
                    },
                    validation: if errors.contains(field) {
                        Validation::Invalid
                    } else if field.ends_with("_model") {
                        Validation::ModelQualificationRequired
                    } else if empty(current) {
                        Validation::Empty
                    } else {
                        Validation::Valid
                    },
                    conflict: false,
                    invalid_import: false,
                },
            );
        }
        // Always expose English, Hindi and Telugu; include every additional declared language.
        for language in crate::core::lid::LANGUAGES {
            let route = self
                .pipeline
                .after_stop_languages
                .as_ref()
                .and_then(|r| r.routes.get(language))
                .filter(|r| !r.model.trim().is_empty());
            let model = self.engine_model(language);
            let declared = self
                .pipeline
                .after_stop_languages
                .as_ref()
                .is_some_and(|r| {
                    r.primary == language || r.supporting.iter().any(|v| v == language)
                });
            if model.is_none() && !declared && !["en", "hi", "te"].contains(&language) {
                continue;
            }
            let field = format!("models.{language}");
            statuses.insert(
                field.clone(),
                FieldStatus {
                    source: if route.is_some() {
                        Source::ExistingRoute
                    } else if settings.imported_fields.contains(&field) {
                        Source::LegacyEnvironment
                    } else if model.is_some() {
                        Source::Saved
                    } else {
                        Source::Default
                    },
                    validation: match model {
                        None | Some("") => Validation::Empty,
                        Some(model) if valid_model(model) => Validation::ModelQualificationRequired,
                        _ => Validation::Invalid,
                    },
                    conflict: settings.conflicts.contains(&field),
                    invalid_import: settings.invalid_imports.contains(&field),
                },
            );
        }
        statuses
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_covers_steps_prompts_routes_and_synthetic_import_without_values() {
        let mut config = Config::default();
        super::super::migration::import(&mut config, |name| {
            Ok(match name {
                "HEX_MEETING_PRODUCT_NAME" => Some("private synthetic text".into()),
                "HEX_TE_ASR_MODEL" => Some("synthetic-te-model".into()),
                _ => None,
            })
        });
        let statuses = config.engine_field_status();
        assert_eq!(statuses.len(), 24 + StageSettings::FIELDS.len());
        for field in [
            "models.en",
            "models.hi",
            "models.te",
            "translation",
            "quick_title",
            "tasks",
            "publish_to_hex",
            "external_comparison",
            "notes_prompt",
            "translation_prompt",
        ] {
            assert!(statuses.contains_key(field));
        }
        assert_eq!(statuses["product"].source, Source::LegacyEnvironment);
        assert_eq!(
            statuses["models.te"].validation,
            Validation::ModelQualificationRequired
        );
        assert!(!format!("{statuses:?}").contains("private synthetic text"));
        config.pipeline.after_stop_languages = Some(crate::core::meeting_router::LanguageRoutes {
            primary: "ur".into(),
            supporting: vec!["en".into()],
            routes: [(
                "ur".into(),
                crate::core::meeting_router::ProviderRoute {
                    provider: "local".into(),
                    model: "synthetic-ur-model".into(),
                },
            )]
            .into(),
        });
        assert_eq!(config.engine_model("ur"), Some("synthetic-ur-model"));
        assert_eq!(
            config.engine_field_status()["models.ur"].source,
            Source::ExistingRoute
        );
    }
}
