use super::Client;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn key(client: Client) -> &'static str {
    if client == Client::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    }
}
fn is_link(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        #[cfg(windows)]
        Ok(m) => {
            use std::os::windows::fs::MetadataExt;
            // Any reparse point, which includes junctions that symlink checks miss.
            Ok(m.file_attributes() & 0x400 != 0)
        }
        #[cfg(not(windows))]
        Ok(m) => Ok(m.file_type().is_symlink()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
/// The agent's config folder and file must be real, so no read or write leaves that folder.
/// The backup and temporary file are created beside the file, inside the same checked folder.
fn confine(path: &Path) -> Result<(), String> {
    let folder = path.parent().ok_or("Config has no parent folder.")?;
    for part in [folder, path] {
        match is_link(part) {
            Ok(false) => {}
            Ok(true) => return Err("The config folder or file is a link to another place. Nothing was changed. Use the copyable config.".into()),
            Err(_) => return Err("Could not check the config location. Nothing was changed.".into()),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<(Option<Vec<u8>>, Value), String> {
    #[cfg(test)]
    super::isolated_tests::check_path(path);
    confine(path)?;
    match fs::read(path) {
        Ok(bytes) => {
            // Strict JSON refuses JSONC comments without stripping the user's text.
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Config contains comments or invalid JSON. Use the copyable config; nothing was changed.".to_string())?;
            if !value.is_object() {
                return Err("Config must be a JSON object. Nothing was changed.".into());
            }
            Ok((Some(bytes), value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((None, serde_json::json!({}))),
        Err(_) => Err("Could not read config. Nothing was changed.".into()),
    }
}
pub(super) fn has_entry(client: Client, path: &Path) -> Result<bool, String> {
    let (_, value) = read(path)?;
    match value.get(key(client)) {
        None => Ok(false),
        Some(section) => section
            .as_object()
            .map(|m| m.contains_key("sidenote"))
            .ok_or("MCP config must be an object. Nothing was changed.".into()),
    }
}
pub(super) fn add(client: Client, path: &Path, exe: &Path) -> Result<String, String> {
    use std::io::Write;
    let (original, mut value) = read(path)?;
    let section = value
        .as_object_mut()
        .unwrap()
        .entry(key(client))
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("MCP config must be an object. Nothing was changed.")?;
    if section.contains_key("sidenote") {
        return Err("A sidenote entry already exists. Nothing was replaced.".into());
    }
    let config: Value = serde_json::from_str(&client.config(exe)).unwrap();
    let expected = config[key(client)]["sidenote"].clone();
    section.insert("sidenote".into(), expected.clone());
    let parent = path.parent().ok_or("Config has no parent folder.")?;
    fs::create_dir_all(parent).map_err(|_| "Could not create config folder.")?;
    let backup = original.as_ref().map(|_| backup_path(path));
    let undo = backup
        .as_ref()
        .map(|p| format!("To undo, restore {}.", p.display()))
        .unwrap_or_else(|| {
            format!(
                "To undo, remove the sidenote entry from {} (new file; no previous config).",
                path.display()
            )
        });
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Could not create temporary config.")?;
    temp.write_all(&serde_json::to_vec_pretty(&value).map_err(|_| "Could not encode config.")?)
        .map_err(|_| "Could not write temporary config.")?;
    if original.is_some() {
        let permissions = fs::metadata(path)
            .map_err(|_| format!("Could not read config permissions. {undo}"))?
            .permissions();
        temp.as_file()
            .set_permissions(permissions)
            .map_err(|_| format!("Could not preserve config permissions. {undo}"))?;
    }
    temp.as_file()
        .sync_all()
        .map_err(|_| "Could not flush temporary config.")?;
    match (&original, &backup) {
        (Some(bytes), Some(backup)) => swap(path, bytes, backup, temp)?,
        _ => {
            temp.persist_noclobber(path)
                .map_err(|_| format!("Could not save config. {undo}"))?;
        }
    }
    let (_, verified) =
        read(path).map_err(|e| format!("Write could not be verified. {e} {undo}"))?;
    if verified[key(client)]["sidenote"] != expected {
        return Err(format!("Write could not be verified. {undo}"));
    }
    Ok(format!(
        "Added to {}. Restart or reconnect the app. {undo}",
        client.name()
    ))
}

fn backup_path(path: &Path) -> PathBuf {
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".sidenote-backup-{millis}-{n}"));
    PathBuf::from(name)
}
/// Replace `path` only if it still holds `original` at the instant it is moved away. The moved
/// file becomes the backup, so a save by the app is kept on disk and never overwritten.
fn swap(
    path: &Path,
    original: &[u8],
    backup: &Path,
    temp: tempfile::NamedTempFile,
) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Reserve the backup name so two Adds cannot pick the same one.
    options
        .open(backup)
        .map_err(|_| "Could not create backup. Nothing was changed.")?;
    #[cfg(test)]
    super::isolated_tests::race("before-swap", path);
    if fs::rename(path, backup).is_err() {
        let _ = fs::remove_file(backup);
        return Err("Could not move the config aside. Nothing was changed. Try again.".into());
    }
    if fs::read(backup).ok().as_deref() != Some(original) {
        return Err(put_back(path, backup));
    }
    #[cfg(test)]
    super::isolated_tests::race("after-move", path);
    temp.persist_noclobber(path)
        .map(|_| ())
        .map_err(|_| put_back(path, backup))
}
fn put_back(path: &Path, backup: &Path) -> String {
    let why = "The file changed, try again. The config changed while Sidenote was saving";
    let back = fs::hard_link(backup, path).is_ok() && fs::remove_file(backup).is_ok()
        || !path.exists() && fs::rename(backup, path).is_ok();
    if back {
        format!("{why}. Nothing was replaced.")
    } else {
        format!(
            "{why}. Both versions are kept: the newest is at {}, the earlier one at {}. Nothing was lost.",
            path.display(),
            backup.display()
        )
    }
}
