use super::*;

#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    version: u32,
    files: BTreeMap<String, String>,
}

fn manifest(files: &BTreeMap<String, Vec<u8>>) -> Manifest {
    Manifest {
        version: 1,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), blake3::hash(bytes).to_hex().to_string()))
            .collect(),
    }
}

pub(super) fn enable(root: &Path, settings: &mut Settings) -> io::Result<()> {
    let (tasks, deleted) = super::super::sync_state::folder_manual(root)?;
    let files = super::super::sync_state::manual_journal_files(root)?;
    let manifest = manifest(&files);
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
    let device = open_device(root, settings)?;
    let name = super::super::DeviceSettings::read_in(root)?.name;
    device
        .store(|s| {
            s.edit("tasks", |doc| {
                let marker = serde_json::to_string(&["$migration", &receipt])?;
                if doc.get(ROOT, &marker)?.is_some() {
                    return Ok(());
                }
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
                let mut tx = doc.transaction();
                tx.put(ROOT, marker, "true")?;
                tx.commit();
                Ok(())
            })
        })
        .map_err(failed)?;
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
    if manifest(&current) != saved {
        return Err(store_io::invalid(
            "Folder task journals changed outside preview. Restore the verified backup before turning preview off.",
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
