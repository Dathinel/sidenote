use super::{valid_model, valid_pattern, valid_text};
use crate::core::Config;
use serde_json::{Value, json};

// Field names are also the public report vocabulary. Never retain rejected values.
pub(super) const LEGACY: [(&str, &str, &str); 10] = [
    (
        "HEX_MEETING_PROJECT",
        "project_key",
        "/engine_settings/project_key",
    ),
    (
        "HEX_MEETING_PROJECT_PATTERN",
        "transcript_pattern",
        "/engine_settings/transcript_pattern",
    ),
    (
        "HEX_MEETING_PROJECT_CONTEXT_PATTERN",
        "context_pattern",
        "/engine_settings/context_pattern",
    ),
    (
        "HEX_MEETING_KB_REPO",
        "knowledge_base.repo_path",
        "/knowledge_base/repo_path",
    ),
    (
        "HEX_MEETING_OWNER_NAME",
        "owner_name",
        "/engine_settings/owner_name",
    ),
    (
        "HEX_MEETING_COMPANY_NAME",
        "company",
        "/knowledge_base/company_name",
    ),
    (
        "HEX_MEETING_PRODUCT_NAME",
        "product",
        "/engine_settings/product",
    ),
    (
        "HEX_TE_ASR_MODEL",
        "models.te",
        "/engine_settings/models/te",
    ),
    (
        "HEX_MEETING_OWNER_ALIASES",
        "owner_aliases",
        "/engine_settings/owner_aliases",
    ),
    (
        "HEX_TRANSCRIPTION_TIMEOUT_SECONDS",
        "stage_deadline_seconds",
        "/engine_settings/stage_deadline_seconds",
    ),
];
pub(super) fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|s| s.trim().is_empty())
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}
fn decode(field: &str, text: String) -> Option<Value> {
    match field {
        "owner_aliases" => {
            let aliases: Vec<_> = text.split(';').map(str::trim).collect();
            aliases
                .iter()
                .all(|s| !s.is_empty() && valid_text(s))
                .then(|| json!(aliases))
        }
        "stage_deadline_seconds" => text
            .parse::<u32>()
            .ok()
            .filter(|v| *v <= 3600)
            .map(Value::from),
        "models.te" => valid_model(&text).then(|| text.into()),
        "transcript_pattern" | "context_pattern" => valid_pattern(&text).then(|| text.into()),
        _ => (valid_text(&text) && !text.trim().is_empty()).then(|| text.into()),
    }
}
pub(super) fn import(
    config: &mut Config,
    mut read: impl FnMut(&str) -> Result<Option<String>, ()>,
) -> bool {
    if config.engine_settings.legacy_imported {
        return false;
    }
    // Snapshot the pre-existing local profile without writing the synced notes name.
    if config.engine_settings.owner_name.is_empty() {
        config.engine_settings.owner_name = config
            .notes
            .as_ref()
            .map_or(String::new(), |s| s.your_name.clone());
    }
    let mut value = serde_json::to_value(&*config).expect("serializable settings");
    let settings = &mut config.engine_settings;
    for (name, field, pointer) in LEGACY {
        let input = match read(name) {
            Ok(None) => continue,
            Ok(Some(input)) if input.is_empty() => continue,
            Ok(Some(input)) => decode(field, input),
            Err(()) => None,
        };
        let Some(input) = input else {
            settings.invalid_imports.insert(field.into());
            continue;
        };
        let current = if field == "models.te" {
            value
                .pointer("/pipeline/after_stop_languages/routes/te/model")
                .filter(|v| !empty(v))
                .or_else(|| value.pointer(pointer))
        } else {
            value.pointer(pointer)
        };
        if current.is_some_and(|v| !empty(v)) {
            if current != Some(&input) {
                settings.conflicts.insert(field.into());
            }
            continue;
        }
        // Only the model entry can be absent; all other leaves serialize with defaults.
        if field == "models.te" {
            value["engine_settings"]["models"]["te"] = input;
        } else {
            *value.pointer_mut(pointer).expect("known settings field") = input;
        }
        settings.imported_fields.insert(field.into());
    }
    value["engine_settings"]["legacy_imported"] = json!(true);
    for (key, fields) in [
        ("imported_fields", &settings.imported_fields),
        ("conflicts", &settings.conflicts),
        ("invalid_imports", &settings.invalid_imports),
    ] {
        value["engine_settings"][key] = json!(fields);
    }
    let imported: Config = serde_json::from_value(value).expect("validated settings shapes");
    config.engine_settings = imported.engine_settings;
    config.knowledge_base = imported.knowledge_base;
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    fn migrate(config: &mut Config, values: &[(&str, &str)]) -> bool {
        import(config, |name| {
            Ok(values
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.to_string()))
        })
    }
    #[test]
    fn all_ten_import_round_trip_and_idempotency() {
        let mut config = Config::default();
        let inputs = [
            "synthetic-project",
            "(?i)synthetic",
            "example",
            "C:/synthetic-kb",
            "Example Owner",
            "Example Company",
            "Example Product",
            "synthetic-te-model",
            "Example;You",
            "120",
        ];
        let values: Vec<_> = LEGACY
            .iter()
            .zip(inputs)
            .map(|((name, _, _), value)| (*name, value))
            .collect();
        assert!(migrate(&mut config, &values));
        let bytes = serde_json::to_vec(&config).unwrap();
        let mut loaded: Config = serde_json::from_slice(&bytes).unwrap();
        assert!(loaded.validate_engine_settings().is_empty());
        assert!(!migrate(
            &mut loaded,
            &[("HEX_MEETING_OWNER_NAME", "Replacement")]
        ));
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), bytes);
        assert_eq!(loaded.engine_settings.imported_fields.len(), 10);
        for (_, field, pointer) in LEGACY {
            assert!(!empty(
                serde_json::to_value(&loaded)
                    .unwrap()
                    .pointer(pointer)
                    .unwrap()
            ));
            assert!(loaded.engine_settings.imported_fields.contains(field));
        }
        assert_eq!(loaded.engine_settings.owner_aliases, ["Example", "You"]);
        assert_eq!(loaded.engine_settings.stage_deadline_seconds, Some(120));
        assert_eq!(loaded.engine_settings.owner_name, "Example Owner");
        assert_eq!(loaded.engine_settings.product, "Example Product");
        assert_eq!(loaded.engine_settings.project_key, "synthetic-project");
        assert_eq!(loaded.engine_settings.transcript_pattern, "(?i)synthetic");
        assert_eq!(loaded.engine_settings.context_pattern, "example");
        assert_eq!(loaded.knowledge_base.repo_path, "C:/synthetic-kb");
        assert_eq!(loaded.knowledge_base.company_name, "Example Company");
        assert_eq!(loaded.engine_model("te"), Some("synthetic-te-model"));
    }
    #[test]
    fn conflicts_invalid_and_missing_values_are_value_free() {
        let mut config = Config::default();
        config.engine_settings.product = "Existing synthetic product".into();
        config.notes = Some(crate::core::notes::NotesSettings {
            your_name: "Existing owner".into(),
            ..Default::default()
        });
        migrate(
            &mut config,
            &[
                ("HEX_MEETING_PRODUCT_NAME", "Rejected product"),
                ("HEX_MEETING_OWNER_NAME", "Rejected owner"),
                ("HEX_MEETING_PROJECT_PATTERN", "[Rejected"),
                ("HEX_TRANSCRIPTION_TIMEOUT_SECONDS", "3601"),
                ("HEX_MEETING_OWNER_ALIASES", "one;;two"),
            ],
        );
        let statuses = &config.engine_settings;
        assert!(
            statuses.conflicts.contains("product") && statuses.conflicts.contains("owner_name")
        );
        for field in [
            "transcript_pattern",
            "stage_deadline_seconds",
            "owner_aliases",
        ] {
            assert!(statuses.invalid_imports.contains(field));
        }
        assert_eq!(config.engine_settings.product, "Existing synthetic product");
        assert_eq!(config.engine_settings.owner_name, "Existing owner");
        assert!(
            !format!("{:?}{:?}", statuses.conflicts, statuses.invalid_imports).contains("Rejected")
        );
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    #[test]
    fn equal_values_are_not_conflicts_and_existing_route_wins() {
        let mut config = Config::default();
        config.engine_settings.product = "same-synthetic".into();
        config.pipeline.after_stop_languages = Some(crate::core::meeting_router::LanguageRoutes {
            primary: "te".into(),
            supporting: vec!["en".into()],
            routes: [(
                "te".into(),
                crate::core::meeting_router::ProviderRoute {
                    provider: "local".into(),
                    model: "indicconformer-te-ctc".into(),
                },
            )]
            .into(),
        });
        import(&mut config, |name| {
            Ok(match name {
                "HEX_MEETING_PRODUCT_NAME" => Some("same-synthetic".into()),
                "HEX_TE_ASR_MODEL" => Some("synthetic-other-model".into()),
                _ => None,
            })
        });
        assert_eq!(
            config
                .engine_settings
                .conflicts
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["models.te"]
        );
        assert!(config.engine_settings.imported_fields.is_empty());
        assert_eq!(config.engine_model("te"), Some("indicconformer-te-ctc"));
        assert!(config.engine_settings.models.is_empty());
    }
}
