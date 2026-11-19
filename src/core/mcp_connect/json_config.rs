//! Read-only look at an agent's own JSON config, only to show whether Sidenote is already
//! connected. Sidenote never writes another app's config file.
use super::Client;
use serde_json::Value;
use std::{fs, path::Path};
pub(super) fn key(client: Client) -> &'static str {
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
/// The agent's config folder and file must be real, so no read leaves that folder.
fn confine(path: &Path) -> Result<(), String> {
    let folder = path.parent().ok_or("Config has no parent folder.")?;
    for part in [folder, path] {
        match is_link(part) {
            Ok(false) => {}
            Ok(true) => return Err("The config folder or file is a link to another place. Sidenote did not read it. Use the copyable config.".into()),
            Err(_) => return Err("Could not check the config location.".into()),
        }
    }
    Ok(())
}
fn read(path: &Path) -> Result<Value, String> {
    #[cfg(test)]
    super::isolated_tests::check_path(path);
    confine(path)?;
    match fs::read(path) {
        Ok(bytes) => {
            // Strict JSON: a config with comments or errors is reported, never edited.
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                "Config contains comments or invalid JSON, so Sidenote cannot tell if it is connected.".to_string()
            })?;
            if !value.is_object() {
                return Err("Config must be a JSON object.".into());
            }
            Ok(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(_) => Err("Could not read config.".into()),
    }
}
pub(super) fn has_entry(client: Client, path: &Path) -> Result<bool, String> {
    match read(path)?.get(key(client)) {
        None => Ok(false),
        Some(section) => section
            .as_object()
            .map(|m| m.contains_key("sidenote"))
            .ok_or("MCP config must be an object.".into()),
    }
}
