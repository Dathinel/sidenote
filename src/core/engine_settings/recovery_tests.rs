use super::*;

#[test]
fn selected_intact_backup_recovers_missing_corrupt_and_incompatible_primaries() {
    for damage in ["missing", "truncated", "unsupported", "bom-invalid"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let prior = super::tests::fixture(root);
        let migrated = migrate_in(root, |_| Ok(None)).unwrap();
        let expected = migrated.engine_settings.backup_file.unwrap();
        match damage {
            "missing" => std::fs::remove_file(root.join("config.json")).unwrap(),
            "truncated" => std::fs::write(root.join("config.json"), b"{\"version\":").unwrap(),
            "unsupported" => store_io::write(
                &root.join("config.json"),
                &serde_json::json!({
                    "version":99, "features":{}, "engine_settings":{"version":99}
                }),
            )
            .unwrap(),
            _ => std::fs::write(root.join("config.json"), b"\xEF\xBB\xBFsynthetic-damage").unwrap(),
        }
        // Selection succeeds without parsing or even having the primary file.
        let names = Config::engine_backup_names(root).unwrap();
        assert_eq!(names, [expected]);
        Config::restore_engine_backup(root, &names[0]).unwrap();
        assert_eq!(config::read(root).unwrap(), prior);
        assert_eq!(
            std::fs::read(root.join("local").join(&names[0])).unwrap(),
            prior
        );
        assert_eq!(Config::engine_backup_names(root).unwrap(), names);
    }
}

#[test]
fn failed_and_interrupted_publications_keep_old_or_new_config_and_recovery_points() {
    // Before/after each durable write: backup first, primary second.
    for interruption in 1..=4 {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let prior = super::tests::fixture(root);
        let mut boundary = 0;
        let result = migrate_with_writer(
            root,
            |name| Ok((name == "HEX_MEETING_PROJECT").then(|| "synthetic-import".into())),
            |path, bytes| {
                boundary += 1;
                if boundary == interruption {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                store_io::atomic_write(path, bytes)?;
                boundary += 1;
                if boundary == interruption {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        let current = config::read(root).unwrap();
        if interruption < 4 {
            assert_eq!(current, prior);
            assert!(
                !Config::load_existing(root)
                    .unwrap()
                    .engine_settings
                    .legacy_imported
            );
        } else {
            let published = Config::load_existing(root).unwrap();
            assert_eq!(published.engine_settings.project_key, "synthetic-import");
            assert!(published.engine_settings.legacy_imported);
        }
        let before_retry = Config::engine_backup_names(root).unwrap();
        assert_eq!(before_retry.len(), usize::from(interruption != 1));
        for name in &before_retry {
            assert_eq!(std::fs::read(root.join("local").join(name)).unwrap(), prior);
        }
        let retried = migrate_in(root, |name| {
            assert!(
                interruption < 4,
                "published migration must not reread environment"
            );
            Ok((name == "HEX_MEETING_PROJECT").then(|| "synthetic-import".into()))
        })
        .unwrap();
        assert!(retried.engine_settings.legacy_imported);
        if interruption == 4 {
            assert_eq!(config::read(root).unwrap(), current);
        }
        // Orphan snapshots are retained and selectable, not overwritten by a retry.
        let names = Config::engine_backup_names(root).unwrap();
        for name in &before_retry {
            assert!(names.contains(name));
        }
        for name in &names {
            assert_eq!(std::fs::read(root.join("local").join(name)).unwrap(), prior);
            Config::restore_engine_backup(root, name).unwrap();
            assert_eq!(config::read(root).unwrap(), prior);
        }
    }
}
