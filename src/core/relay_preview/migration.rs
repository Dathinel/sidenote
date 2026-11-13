use super::*;

#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    version: u32,
    files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline: Option<Value>,
}

fn manifest(files: &BTreeMap<String, Vec<u8>>) -> Manifest {
    Manifest {
        version: 1,
        baseline: None,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), blake3::hash(bytes).to_hex().to_string()))
            .collect(),
    }
}

fn previous_baseline(
    root: &Path,
    directory: &str,
    receipt: &str,
    current: &Manifest,
) -> io::Result<Option<Value>> {
    if receipt.is_empty() {
        return Ok(None);
    }
    if receipt.len() != 64 || !receipt.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(store_io::invalid("Invalid backup receipt"));
    }
    let backup = root.join("local/relay").join(directory).join(receipt);
    let saved: Manifest = store_io::read(&backup.join("manifest.json"))?;
    if saved.version != 1 || blake3::hash(&serde_json::to_vec(&saved)?).to_hex().as_str() != receipt
    {
        return Err(store_io::invalid(
            "Backup manifest changed; folder data was not changed",
        ));
    }
    for (name, hash) in &saved.files {
        let bytes =
            store_io::read_bounded(&backup.join(name), super::super::file_reads::STORE_MAX)?;
        if blake3::hash(&bytes).to_hex().as_str() != hash {
            return Err(store_io::invalid(
                "Backup integrity check failed; folder data was not changed",
            ));
        }
    }
    if let Some(base) = saved.baseline {
        return Ok(Some(base));
    }
    if saved.files != current.files {
        return Err(store_io::invalid(
            "Folder edits were found after an earlier preview. Keep preview off and keep the newer folder data. An explicit migration is needed; do not restore an older backup.",
        ));
    }
    Ok(current.baseline.clone())
}

pub(super) fn enable(root: &Path, settings: &mut Settings) -> io::Result<()> {
    let (tasks, deleted) = super::super::sync_state::folder_manual(root)?;
    let files = super::super::sync_state::manual_journal_files(root)?;
    let mut manifest = manifest(&files);
    manifest.baseline = Some(serde_json::json!({"tasks":tasks,"deleted":deleted}));
    let previous = previous_baseline(root, "backups", &settings.backup, &manifest)?;
    let bytes = serde_json::to_vec(&manifest)?;
    let receipt = blake3::hash(&bytes).to_hex().to_string();
    let backup = root.join("local/relay/backups").join(&receipt);
    for (relative, original) in &files {
        let destination = backup.join(relative);
        if !destination.try_exists()? {
            store_io::publish(&destination, original)?;
        }
        if store_io::read_bounded(&destination, super::super::file_reads::JOURNAL_MAX)? != *original
        {
            return Err(store_io::invalid(
                "Task backup verification failed. Folder data was not changed.",
            ));
        }
    }
    store_io::write(&backup.join("manifest.json"), &manifest)?;
    let verified: Manifest = store_io::read(&backup.join("manifest.json"))?;
    if verified != manifest {
        return Err(store_io::invalid(
            "Task backup manifest verification failed",
        ));
    }
    qualification_point(root, "migration-backup")?;
    let device = open_device(root, settings)?;
    let name = super::super::DeviceSettings::read_in(root)?.name;
    device
        .store(|s| {
            s.edit("tasks", |doc| {
                let marker = serde_json::to_string(&["$migration", &receipt])?;
                if doc.get(ROOT, &marker)?.is_some() {
                    return Ok(());
                }
                if let Some(base) = &previous {
                    let before: Vec<ManualTask> = serde_json::from_value(base["tasks"].clone())?;
                    versions::apply(doc, &before, &tasks, &name, "folder mode changes")?;
                } else {
                    let new_tasks: Vec<_> = tasks
                        .iter()
                        .filter(|task| {
                            let key = serde_json::to_string(&[task.id.as_str(), "$present"])
                                .expect("string keys");
                            doc.get(ROOT, key).ok().flatten().is_none()
                        })
                        .cloned()
                        .collect();
                    versions::apply(doc, &[], &new_tasks, &name, "folder import")?;
                    for id in &deleted {
                        let key = serde_json::to_string(&[id.as_str(), "$present"])?;
                        if doc.get(ROOT, key)?.is_none() {
                            versions::apply(
                                doc,
                                &[ManualTask {
                                    id: id.clone(),
                                    ..Default::default()
                                }],
                                &[],
                                &name,
                                "folder import",
                            )?;
                        }
                    }
                }
                let mut tx = doc.transaction();
                tx.put(ROOT, marker, "true")?;
                tx.commit();
                Ok(())
            })
        })
        .map_err(failed)?;
    qualification_point(root, "migration-import")?;
    settings.backup = receipt;
    Ok(())
}

pub(super) fn verify(root: &Path, receipt: &str) -> io::Result<()> {
    if receipt.is_empty() {
        return Ok(());
    }
    if receipt.len() != 64 || !receipt.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(store_io::invalid("Invalid task backup receipt"));
    }
    let backup = root.join("local/relay/backups").join(receipt);
    let saved: Manifest = store_io::read(&backup.join("manifest.json"))?;
    if saved.version != 1 || blake3::hash(&serde_json::to_vec(&saved)?).to_hex().as_str() != receipt
    {
        return Err(store_io::invalid(
            "Task backup manifest changed; preview remains on",
        ));
    }
    let current = super::super::sync_state::manual_journal_files(root)?;
    if manifest(&current).files != saved.files {
        return Err(store_io::invalid(
            "Newer folder task edits were found. Preview stays on to protect them. Keep the newer folder data; do not restore an older backup.",
        ));
    }
    for (relative, original) in current {
        if store_io::read_bounded(
            &backup.join(relative),
            super::super::file_reads::JOURNAL_MAX,
        )? != original
        {
            return Err(store_io::invalid(
                "Task backup bytes changed; preview remains on",
            ));
        }
    }
    Ok(())
}

fn shared_sources(root: &Path) -> io::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = super::super::sync_state::shared_files(root)?;
    files.insert(
        "allowlist/profile.json".into(),
        serde_json::to_vec(&shared::local_profile(root)?)?,
    );
    files.insert(
        "allowlist/people.json".into(),
        serde_json::to_vec(&shared::local_people(root)?)?,
    );
    Ok(files)
}
pub(super) fn enable_shared(root: &Path, settings: &mut Settings) -> io::Result<()> {
    let files = shared_sources(root)?;
    let own = super::super::DeviceSettings::read_in(root)?;
    let mut stores = super::super::sync_state::stores(root)?;
    stores.extend([
        "projects.json".into(),
        "memory/records.json".into(),
        "task-overrides.json".into(),
    ]);
    stores.sort();
    stores.dedup();
    let mut values = Vec::new();
    for store in stores {
        if shared::domain(&store).is_some() {
            values.push((
                store.clone(),
                super::super::sync_state::folder_shared(root, &store)?,
            ));
        }
    }
    let mut teams = super::super::TaskStore { root: root.into() }.folder_teams()?;
    shared::team_ids(&mut teams);
    values.push(("teams.json".into(), serde_json::to_value(teams)?));
    values.push(("people.json".into(), shared::local_people(root)?));
    values.push(("profile.json".into(), shared::local_profile(root)?));
    let mut manifest = manifest(&files);
    manifest.baseline = Some(serde_json::to_value(
        values.iter().cloned().collect::<BTreeMap<_, _>>(),
    )?);
    let previous = previous_baseline(root, "shared-backups", &settings.shared_backup, &manifest)?;
    let receipt = blake3::hash(&serde_json::to_vec(&manifest)?)
        .to_hex()
        .to_string();
    let backup = root.join("local/relay/shared-backups").join(&receipt);
    for (name, bytes) in &files {
        let path = backup.join(name);
        if !path.try_exists()? {
            store_io::publish(&path, bytes)?;
        }
        if store_io::read_bounded(&path, super::super::file_reads::STORE_MAX)? != *bytes {
            return Err(store_io::invalid("Shared backup verification failed"));
        }
    }
    store_io::write(&backup.join("manifest.json"), &manifest)?;
    let checked: Manifest = store_io::read(&backup.join("manifest.json"))?;
    if checked != manifest {
        return Err(store_io::invalid("Shared backup manifest changed"));
    }
    let device = open_device(root, settings)?;
    for (store, value) in values {
        let domain = shared::domain(&store).ok_or_else(|| failed("migration domain"))?;
        device
            .store(|s| {
                s.edit(&domain, |doc| {
                    let marker = serde_json::to_string(&[
                        "$shared-import".to_string(),
                        format!("{store}:{receipt}"),
                    ])?;
                    if doc.get(ROOT, &marker)?.is_some() {
                        return Ok(());
                    }
                    let before = previous
                        .as_ref()
                        .and_then(|v| v.get(&store))
                        .unwrap_or(&Value::Null);
                    shared::apply(
                        doc,
                        &store,
                        before,
                        &value,
                        &own.name,
                        &own.id,
                        previous.is_none(),
                    )?;
                    let mut tx = doc.transaction();
                    tx.put(ROOT, marker, true)?;
                    tx.commit();
                    Ok(())
                })
            })
            .map_err(failed)?;
        qualification_point(root, "shared-import")?;
    }
    settings.shared_backup = receipt;
    Ok(())
}
pub(super) fn verify_shared(root: &Path, receipt: &str) -> io::Result<()> {
    if receipt.is_empty() {
        return Ok(());
    }
    if receipt.len() != 64 || !receipt.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(store_io::invalid("Invalid shared backup receipt"));
    }
    let backup = root.join("local/relay/shared-backups").join(receipt);
    let saved: Manifest = store_io::read(&backup.join("manifest.json"))?;
    if saved.version != 1 || blake3::hash(&serde_json::to_vec(&saved)?).to_hex().as_str() != receipt
    {
        return Err(store_io::invalid(
            "Shared backup manifest changed; preview remains on",
        ));
    }
    let files = shared_sources(root)?;
    if manifest(&files).files != saved.files {
        return Err(store_io::invalid(
            "Newer shared folder edits were found. Preview stays on to protect them. Keep the newer folder data; do not restore an older backup.",
        ));
    }
    for (name, bytes) in files {
        if store_io::read_bounded(&backup.join(name), super::super::file_reads::STORE_MAX)? != bytes
        {
            return Err(store_io::invalid(
                "Shared backup bytes changed; preview remains on",
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod security_tests {
    use super::*;
    fn folder(root: &Path) -> super::super::super::TaskStore {
        super::super::super::TaskStore { root: root.into() }
    }
    fn mode(root: &Path, enabled: bool) -> io::Result<()> {
        let mut settings = Settings::load(root)?;
        settings.enabled = enabled;
        settings.save(root)
    }
    #[test]
    fn preview_excludes_local_replica_before_existing_ignore_rules() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let root = d.path();
        let original = b"!/local/**\n# synthetic existing rules\ncache\n";
        std::fs::write(root.join(".stignore"), original)?;
        mode(root, true)?;
        let bytes = std::fs::read(root.join(".stignore"))?;
        assert!(bytes.starts_with(b"/local\n"));
        assert_eq!(&bytes[7..], original);
        mode(root, false)?;
        mode(root, true)?;
        assert_eq!(std::fs::read(root.join(".stignore"))?, bytes);
        disconnect(root)?;
        Ok(())
    }
    fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()))?;
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name()))?;
            }
        }
        Ok(())
    }
    #[test]
    fn peer_folder_edits_refuse_disable_without_overwriting_or_restore_advice() -> anyhow::Result<()>
    {
        let a = tempfile::tempdir()?;
        let b = tempfile::tempdir()?;
        let (a, b) = (a.path(), b.path());
        folder(a).add_manual(ManualTask {
            task: "Synthetic A".into(),
            ..Default::default()
        })?;
        mode(a, true)?;
        folder(b).add_manual(ManualTask {
            task: "Synthetic newer B".into(),
            ..Default::default()
        })?;
        copy_tree(&b.join("state"), &a.join("state"))?;
        let newer = super::super::super::sync_state::manual_journal_files(a)?;
        let error = mode(a, false).unwrap_err().to_string();
        assert!(error.contains("Keep the newer folder data"));
        assert!(!error.contains("Restore the verified backup"));
        assert!(Settings::load(a)?.enabled);
        assert_eq!(
            super::super::super::sync_state::manual_journal_files(a)?,
            newer
        );
        disconnect(a)?;
        Ok(())
    }
    #[test]
    fn reenable_imports_existing_task_field_deltas_and_preserves_relay_fields() -> anyhow::Result<()>
    {
        let d = tempfile::tempdir()?;
        let root = d.path();
        let (_, id) = folder(root).add_manual(ManualTask {
            task: "Synthetic task".into(),
            ..Default::default()
        })?;
        mode(root, true)?;
        folder(root).update_manual(&id, |task| task.owner = Some("Relay owner".into()))?;
        mode(root, false)?;
        folder(root).update_manual(&id, |task| task.due = Some("2030-01-02".into()))?;
        let bytes = super::super::super::sync_state::manual_journal_files(root)?;
        mode(root, true)?;
        let tasks = get(root)?.unwrap().tasks()?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].owner.as_deref(), Some("Relay owner"));
        assert_eq!(tasks[0].due.as_deref(), Some("2030-01-02"));
        assert_eq!(
            super::super::super::sync_state::manual_journal_files(root)?,
            bytes
        );
        mode(root, false)?;
        mode(root, true)?;
        assert_eq!(get(root)?.unwrap().tasks()?.len(), 1);
        disconnect(root)?;
        Ok(())
    }
    #[test]
    fn unchanged_folder_deletion_does_not_delete_a_relay_restore() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let root = d.path();
        let (tasks, id) = folder(root).add_manual(ManualTask {
            task: "Synthetic restored task".into(),
            ..Default::default()
        })?;
        let task = tasks.manual.into_iter().find(|t| t.id == id).unwrap();
        folder(root).delete_manual(&id)?;
        mode(root, true)?;
        get(root)?.unwrap().edit(&[], &[task])?;
        mode(root, false)?;
        mode(root, true)?;
        assert_eq!(get(root)?.unwrap().tasks()?.len(), 1);
        disconnect(root)?;
        Ok(())
    }
    #[test]
    fn a_new_folder_deletion_is_imported_once_on_reenable() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let root = d.path();
        let (_, id) = folder(root).add_manual(ManualTask {
            task: "Synthetic deleted task".into(),
            ..Default::default()
        })?;
        mode(root, true)?;
        mode(root, false)?;
        folder(root).delete_manual(&id)?;
        mode(root, true)?;
        assert!(get(root)?.unwrap().tasks()?.is_empty());
        mode(root, false)?;
        mode(root, true)?;
        assert!(get(root)?.unwrap().tasks()?.is_empty());
        disconnect(root)?;
        Ok(())
    }
}
