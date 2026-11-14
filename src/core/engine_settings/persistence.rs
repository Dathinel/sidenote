use super::{field_pointer, migration};
use crate::core::{Config, config, store_io};
use serde_json::Value;
use std::{io, path::Path};

impl Config {
    /// Explicit adoption action, reading only this process's ten recognized values.
    /// Does not change the selected engine, schedule work, or refresh the KB.
    pub fn import_engine_environment(root: &Path) -> io::Result<Self> {
        migrate_in(root, |name| {
            std::env::var(name).map(Some).or_else(|error| {
                if matches!(error, std::env::VarError::NotPresent) {
                    Ok(None)
                } else {
                    Err(())
                }
            })
        })
    }
    /// Restore the exact pre-import config under the same writer lock. Backup is retained.
    /// Explicit action only; the caller must reload settings after restoring.
    pub fn restore_engine_backup(root: &Path) -> io::Result<()> {
        let _lock = config::lock(root)?;
        let config = required_config(root)?;
        let name = config
            .engine_settings
            .backup_file
            .as_deref()
            .ok_or_else(|| store_io::invalid("No engine settings backup"))?;
        let id = name
            .strip_prefix("engine-settings-v1-")
            .and_then(|s| s.strip_suffix(".json"))
            .filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| store_io::invalid("Invalid engine settings backup name"))?;
        let bytes = store_io::read_bounded(
            &root
                .join("local")
                .join(format!("engine-settings-v1-{id}.json")),
            super::super::file_reads::SUMMARY_MAX,
        )?;
        let prior: Config =
            serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                .map_err(|_| store_io::invalid("Invalid engine settings backup"))?;
        if prior.version != 1 || prior.engine_settings.version != 1 {
            return Err(store_io::invalid(
                "Unsupported engine settings backup version",
            ));
        }
        store_io::atomic_write(&root.join("config.json"), &bytes)
    }
}
fn required_config(root: &Path) -> io::Result<Config> {
    let config = Config::read_config(root)?
        .ok_or_else(|| store_io::invalid("Load config before engine migration"))?;
    if config.notes.is_none() {
        return Err(store_io::invalid(
            "Load notes settings before engine migration",
        ));
    }
    if config.engine_settings.version != 1 {
        return Err(store_io::invalid("Unsupported engine settings version"));
    }
    Ok(config)
}
fn migrate_in(
    root: &Path,
    read: impl FnMut(&str) -> Result<Option<String>, ()>,
) -> io::Result<Config> {
    {
        let _lock = config::lock(root)?;
        let mut config = required_config(root)?;
        if !config.engine_settings.legacy_imported {
            let prior = config::read(root)?;
            if prior.len() as u64 > super::super::file_reads::SUMMARY_MAX {
                return Err(store_io::invalid("Config exceeds restorable backup limit"));
            }
            migration::import(&mut config, read);
            let backup = format!(
                "engine-settings-v1-{}.json",
                super::super::device::random_id()?
            );
            // Flush the immutable backup before publishing the migration marker.
            store_io::atomic_write(&root.join("local").join(&backup), &prior)?;
            config.engine_settings.backup_file = Some(backup);
            let mut raw: Value =
                serde_json::from_slice(prior.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&prior))
                    .map_err(|_| store_io::invalid("Invalid prior engine config"))?;
            raw["engine_settings"] = serde_json::to_value(&config.engine_settings)?;
            for (key, imported) in [
                ("repo_path", &config.knowledge_base.repo_path),
                ("company_name", &config.knowledge_base.company_name),
            ] {
                if raw["knowledge_base"][key].as_str().unwrap_or("") != imported {
                    if !raw["knowledge_base"].is_object() {
                        raw["knowledge_base"] = serde_json::json!({});
                    }
                    raw["knowledge_base"][key] = Value::String(imported.clone());
                }
            }
            store_io::write(&root.join("config.json"), &raw)?;
        }
    }
    Config::load_existing(root)
}
pub(in crate::core) fn prepare_save(
    latest: &mut Value,
    baseline: &Value,
    edited: &Value,
) -> io::Result<()> {
    if baseline.get("engine_settings") != edited.get("engine_settings") {
        let merged: Config = serde_json::from_value(latest.clone())
            .map_err(|_| store_io::invalid("Invalid engine settings"))?;
        if merged.validate_engine_settings().into_iter().any(|field| {
            let pointer = field_pointer(field);
            baseline.pointer(&pointer) != edited.pointer(&pointer)
        }) {
            return Err(store_io::invalid(
                "Invalid engine settings; inspect field validation",
            ));
        }
    }
    clear_edited_sources(latest, baseline, edited);
    Ok(())
}
/// Clear only explicitly edited migration fields, preserving concurrent card changes.
pub(super) fn clear_edited_sources(latest: &mut Value, baseline: &Value, edited: &Value) {
    let mut changed: Vec<_> = migration::LEGACY
        .iter()
        .filter(|(_, _, p)| baseline.pointer(p) != edited.pointer(p))
        .map(|(_, field, _)| *field)
        .collect();
    let route = "/pipeline/after_stop_languages/routes/te";
    if baseline.pointer(route) != edited.pointer(route) {
        changed.push("models.te");
    }
    for key in ["imported_fields", "conflicts", "invalid_imports"] {
        if let Some(fields) = latest
            .get_mut("engine_settings")
            .and_then(|v| v.get_mut(key))
            .and_then(Value::as_array_mut)
        {
            fields.retain(|v| !changed.iter().any(|field| v.as_str() == Some(field)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine_settings::Source;
    pub(super) fn fixture(root: &Path) -> Vec<u8> {
        let value = serde_json::json!({
            "version":1, "features":{}, "future_field":{"keep":"synthetic"},
            "engine_settings":{"steps":{"render_html":false,"future":{"mode":"synthetic","enabled":false}}},
            "notes":{"engine":"scripts", "helper":"codex", "model":"synthetic-notes-model",
                "auto":false, "enabled_at_ms":123456, "script":"C:/synthetic/custom.ps1",
                "future_notes_field":"keep-synthetic"}
        });
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(root.join("config.json"), &bytes).unwrap();
        bytes
    }
    #[test]
    fn backup_restore_round_trip_and_noop_reimport() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let prior = fixture(root);
        let migrated = migrate_in(root, |name| {
            Ok((name == "HEX_MEETING_PROJECT").then(|| "synthetic-project".into()))
        })
        .unwrap();
        let backup = root
            .join("local")
            .join(migrated.engine_settings.backup_file.as_ref().unwrap());
        assert_eq!(std::fs::read(&backup).unwrap(), prior);
        let after = std::fs::read(root.join("config.json")).unwrap();
        let raw: Value = serde_json::from_slice(&after).unwrap();
        assert_eq!(
            raw["notes"],
            serde_json::from_slice::<Value>(&prior).unwrap()["notes"]
        );
        assert_eq!(raw["notes"]["future_notes_field"], "keep-synthetic");
        assert_eq!(raw["future_field"]["keep"], "synthetic");
        let original: Value = serde_json::from_slice(&prior).unwrap();
        for key in ["render_html", "future"] {
            assert_eq!(
                raw["engine_settings"]["steps"][key],
                original["engine_settings"]["steps"][key]
            );
        }
        let again = migrate_in(root, |_| panic!("must not read environment twice")).unwrap();
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), after);
        assert_eq!(std::fs::read_dir(root.join("local")).unwrap().count(), 2); // backup + writer lock
        assert!(!root.join("state").exists());
        let notes = again.notes.unwrap();
        assert_eq!(notes.engine, "scripts");
        assert!(!notes.auto);
        assert_eq!(notes.model, "synthetic-notes-model");
        Config::restore_engine_backup(root).unwrap();
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), prior);
        assert_eq!(std::fs::read(backup).unwrap(), prior);
    }
    #[test]
    fn edits_clear_only_their_provenance_and_reject_invalid_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fixture(root);
        let mut config = migrate_in(root, |name| {
            Ok(match name {
                "HEX_MEETING_PRODUCT_NAME" => Some("synthetic-product".into()),
                "HEX_MEETING_PROJECT" => Some("synthetic-project".into()),
                _ => None,
            })
        })
        .unwrap();
        let mut stale = Config::load_existing(root).unwrap();
        config.engine_settings.product = "edited-product".into();
        config.save(root).unwrap();
        stale.engine_settings.worker_threads = 8;
        stale.save(root).unwrap();
        let loaded = Config::load_existing(root).unwrap();
        assert_eq!(loaded.engine_settings.product, "edited-product");
        assert_eq!(
            loaded.engine_field_status()["product"].source,
            Source::Saved
        );
        assert_eq!(
            loaded.engine_field_status()["project_key"].source,
            Source::LegacyEnvironment
        );
        let prior = std::fs::read(root.join("config.json")).unwrap();
        stale.engine_settings.worker_threads = 0;
        assert!(stale.save(root).is_err());
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), prior);
    }
    #[test]
    fn unreadable_environment_and_unsafe_backup_fail_without_values_or_writes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fixture(root);
        let mut config = migrate_in(root, |_| Err(())).unwrap();
        assert_eq!(config.engine_settings.invalid_imports.len(), 10);
        config.engine_settings.backup_file = Some("../synthetic".into());
        store_io::write(&root.join("config.json"), &config).unwrap();
        let prior = std::fs::read(root.join("config.json")).unwrap();
        assert!(Config::restore_engine_backup(root).is_err());
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), prior);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    #[test]
    fn corrupt_backup_and_future_schema_do_not_replace_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        store_io::write(
            &root.join("config.json"),
            &serde_json::json!({
                "version":1, "features":{}, "notes":{}
            }),
        )
        .unwrap();
        let migrated = migrate_in(root, |_| Ok(None)).unwrap();
        let backup = root
            .join("local")
            .join(migrated.engine_settings.backup_file.unwrap());
        std::fs::write(backup, b"synthetic invalid config").unwrap();
        let prior = std::fs::read(root.join("config.json")).unwrap();
        assert!(Config::restore_engine_backup(root).is_err());
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), prior);
        let mut future: Value = serde_json::from_slice(&prior).unwrap();
        future["engine_settings"]["version"] = 2.into();
        store_io::write(&root.join("config.json"), &future).unwrap();
        let prior = std::fs::read(root.join("config.json")).unwrap();
        assert!(
            migrate_in(root, |_| panic!(
                "future schema must reject before environment read"
            ))
            .is_err()
        );
        assert_eq!(std::fs::read(root.join("config.json")).unwrap(), prior);
    }
}

#[cfg(test)]
#[path = "save_tests.rs"]
mod save_tests;
