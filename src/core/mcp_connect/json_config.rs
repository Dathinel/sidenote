use super::Client;
use serde_json::Value;
use std::{fs, path::Path};
fn key(client: Client) -> &'static str {
    if client == Client::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    }
}
fn read(path: &Path) -> Result<(Option<Vec<u8>>, Value), String> {
    #[cfg(test)]
    super::isolated_tests::check_path(path);
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
    let backup = if let Some(bytes) = &original {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System clock unavailable.")?
            .as_millis();
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".sidenote-backup-{millis}"));
        let backup = std::path::PathBuf::from(name);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&backup)
            .map_err(|_| "Could not create backup. Nothing was changed.")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not finish backup. Nothing was changed.")?;
        Some(backup)
    } else {
        None
    };
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
    // Refuse an edit if the file changed while the backup was being prepared.
    if fs::read(path).ok() != original {
        return Err(format!(
            "Config changed during setup. Nothing was replaced. {undo}"
        ));
    }
    let saved = if original.is_some() {
        temp.persist(path).map(|_| ())
    } else {
        temp.persist_noclobber(path).map(|_| ())
    };
    saved.map_err(|_| format!("Could not save config. {undo}"))?;
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
