use super::*;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::io::Read;

/// Identity of an installed catalogue model without re-reading it. `asr::model_path` only
/// returns a pack whose receipt matches the catalogue and whose files are not newer than the
/// receipt, and the receipt is written after each file's SHA-256 was verified at download. So
/// the catalogue hashes name the pack that was installed. This is an identity for cache keys,
/// not an integrity proof: a same-size change that keeps old timestamps, or bit rot, is not
/// caught here. Re-hashing 200 MB on every run cost several CPU seconds per model.
pub fn installed_identity(root: &Path, id: &str) -> Result<String> {
    asr::model_path(root, id)?;
    let model = crate::core::model_catalog::catalog()
        .iter()
        .find(|m| m.id == id)
        .ok_or("Unknown meeting model")?;
    Ok(model
        .files
        .iter()
        .map(|f| f.sha256)
        .collect::<Vec<_>>()
        .join(":"))
}

pub fn model_checksum(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}
pub fn key(samples: &[f32], provider: &str, checksum: &str, language: &str) -> String {
    let mut hash = Sha256::new();
    for item in [provider, checksum, language, "meeting-router-v1"] {
        hash.update((item.len() as u64).to_le_bytes());
        hash.update(item.as_bytes());
    }
    for sample in samples {
        hash.update(sample.to_le_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}
pub fn get<T: DeserializeOwned + Serialize>(root: &Path, step: &str, key: &str) -> Option<T> {
    let path = root.join(step).join(format!("{key}.json"));
    let value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
    // Replacing a hit advances its mtime, used for device-local least-recently-used eviction.
    let _ = store_io::write(&path, &value);
    Some(value)
}
pub fn put<T: Serialize>(root: &Path, step: &str, key: &str, value: &T) -> Result<()> {
    store_io::write(&root.join(step).join(format!("{key}.json")), value)?;
    let mut entries = Vec::new();
    for step in ["language_detect", "transcribe"] {
        if let Ok(dir) = fs::read_dir(root.join(step)) {
            for entry in dir {
                let entry = entry?;
                let meta = entry.metadata()?;
                if meta.is_file() && entry.path().extension().is_some_and(|s| s == "json") {
                    entries.push((meta.modified()?, meta.len(), entry.path()));
                }
            }
        }
    }
    entries.sort_by_key(|e| e.0);
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    for (_, size, path) in entries {
        if total <= 500 * 1024 * 1024 {
            break;
        }
        fs::remove_file(path)?;
        total -= size;
    }
    Ok(())
}
