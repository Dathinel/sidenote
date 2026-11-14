use super::*;
use crate::core::{engine_settings::Validation, notes::Helper};

#[test]
fn preserved_helper_and_legacy_invalid_fields_do_not_block_unrelated_saves() {
    for engine in ["app", "scripts", "off"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut raw: Value = serde_json::from_slice(&super::tests::fixture(root)).unwrap();
        raw["notes"]["engine"] = engine.into();
        raw["notes"]["helper"] = "open_code".into();
        raw["notes"]["model"] = "preserved synthetic model\n".into();
        raw["engine_settings"]["transcript_pattern"] = "[preserved-invalid-pattern".into();
        raw["engine_settings"]["stages"] = serde_json::json!({"title_deadline_seconds":0});
        let notes = raw["notes"].clone();
        store_io::write(&root.join("config.json"), &raw).unwrap();
        let mut config = Config::load_existing(root).unwrap();
        config.engine_settings.worker_threads = 8;
        config.engine_settings.steps.translation = false;
        config.save(root).unwrap();
        let saved = Config::load_existing(root).unwrap();
        assert_eq!(saved.engine_settings.worker_threads, 8);
        assert!(!saved.engine_settings.steps.translation);
        assert_eq!(saved.notes.as_ref().unwrap().helper, Some(Helper::OpenCode));
        assert_eq!(
            saved.engine_field_status()["notes.helper"].validation,
            Validation::Unavailable
        );
        assert_eq!(
            saved.engine_field_status()["transcript_pattern"].validation,
            Validation::Invalid
        );
        let raw_saved: Value = serde_json::from_slice(&config::read(root).unwrap()).unwrap();
        assert_eq!(raw_saved["notes"], notes);
        assert_eq!(raw_saved["engine_settings"]["steps"]["render_html"], false);
        let prior = config::read(root).unwrap();
        config.engine_settings.worker_threads = 0;
        assert!(config.save(root).is_err());
        assert_eq!(config::read(root).unwrap(), prior);
    }
}

#[test]
fn resolving_authoritative_route_clears_only_its_migration_status() {
    for legacy in ["synthetic-conflicting-model", "invalid synthetic model"] {
        for action in ["model", "provider", "remove"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let mut raw: Value = serde_json::from_slice(&super::tests::fixture(root)).unwrap();
            raw["pipeline"] = serde_json::json!({"after_stop_languages":{
                "primary":"te", "supporting":[], "routes":{
                    "te":{"provider":"local", "model":"indicconformer-te-ctc"}
                }
            }});
            store_io::write(&root.join("config.json"), &raw).unwrap();
            let mut config = migrate_in(root, |name| {
                Ok(match name {
                    "HEX_TE_ASR_MODEL" => Some(legacy.into()),
                    "HEX_MEETING_PROJECT" => Some("synthetic-project".into()),
                    _ => None,
                })
            })
            .unwrap();
            let status = config.engine_field_status()["models.te"];
            assert!(status.conflict || status.invalid_import);
            let routes = &mut config
                .pipeline
                .after_stop_languages
                .as_mut()
                .unwrap()
                .routes;
            match action {
                "model" => routes.get_mut("te").unwrap().model = "synthetic-resolved-model".into(),
                "provider" => routes.get_mut("te").unwrap().provider = "hex".into(),
                _ => {
                    routes.remove("te");
                }
            }
            config.save(root).unwrap();
            let saved = Config::load_existing(root).unwrap();
            let status = saved.engine_field_status()["models.te"];
            assert!(!status.conflict && !status.invalid_import);
            assert!(
                saved
                    .engine_settings
                    .imported_fields
                    .contains("project_key")
            );
        }
    }
}
