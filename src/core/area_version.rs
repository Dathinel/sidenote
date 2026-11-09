//! Area edits require all recently seen installations to understand assignment intent.
use super::{DeviceSettings, device, store_io};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};
const AREA_FORMAT: u32 = 1;
// device.json* is excluded by existing Syncthing shares; format markers must use another name.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Marker {
    device: String,
    name: String,
    app_version: String,
    area_format: u32,
    seen_at: i64,
}
pub(super) fn record(root: &Path, own: &DeviceSettings) -> io::Result<()> {
    let path = root.join("state").join(&own.id).join("area-format.json");
    store_io::reject_reparse(&path)?;
    let now = chrono::Utc::now().timestamp();
    if let Ok(bytes) = store_io::read_bounded(&path, 4096)
        && let Ok(marker) = serde_json::from_slice::<Marker>(&bytes)
        && marker.app_version == env!("CARGO_PKG_VERSION")
        && marker.area_format == AREA_FORMAT
        && marker.name == own.name
        && marker.seen_at > now - 3600
    {
        return Ok(());
    }
    store_io::write(
        &path,
        &Marker {
            device: own.id.clone(),
            name: own.name.chars().take(80).collect(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            area_format: AREA_FORMAT,
            seen_at: now,
        },
    )
}
pub fn ensure(root: &Path) -> io::Result<()> {
    let own = DeviceSettings::load_in(root)?;
    let now = chrono::Utc::now().timestamp();
    let entries = fs::read_dir(root.join("state"))?;
    for entry in entries {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !device::valid_id(&id) || id == own.id || !entry.file_type()?.is_dir() {
            continue;
        }
        store_io::reject_reparse(&entry.path())?;
        let marker = store_io::read_bounded(&entry.path().join("area-format.json"), 4096)
            .ok()
            .and_then(|b| serde_json::from_slice::<Marker>(&b).ok())
            .filter(|m| m.device == id);
        let folder_seen = entry
            .metadata()?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let mut seen = marker
            .as_ref()
            .map_or(folder_seen, |m| m.seen_at.max(folder_seen));
        // Old installations have no marker. Recent journal arrival still makes them visible.
        for folder in fs::read_dir(entry.path())? {
            let folder = folder?;
            if !folder.file_type()?.is_dir() {
                continue;
            }
            store_io::reject_reparse(&folder.path())?;
            for file in fs::read_dir(folder.path())? {
                let file = file?;
                if file.path().extension().is_some_and(|e| e == "json")
                    && file.file_type()?.is_file()
                {
                    store_io::reject_reparse(&file.path())?;
                    seen = seen.max(
                        file.metadata()?
                            .modified()?
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64,
                    );
                }
            }
        }
        if seen < now - 30 * 86400 {
            continue;
        }
        if marker.as_ref().is_none_or(|m| m.area_format < AREA_FORMAT) {
            let name = marker
                .as_ref()
                .map(|m| m.name.as_str())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(&id);
            return Err(store_io::invalid(&format!(
                "Update Sidenote on {name} first"
            )));
        }
    }
    Ok(())
}
pub(super) fn guard_changes(
    root: &Path,
    store: &str,
    changes: &[(
        Vec<String>,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    )],
) -> io::Result<()> {
    let area_edit = changes.iter().any(|(key, before, after)| {
        (store == "projects.json"
            && (key.first().is_some_and(|k| {
                k.starts_with("area")
                    || k.starts_with("assignment:")
                    || k == "merges"
                    || k == "aliases"
            })))
            || before.as_ref().and_then(|v| v.get("assignment"))
                != after.as_ref().and_then(|v| v.get("assignment"))
    });
    if area_edit {
        ensure(root)?;
    }
    Ok(())
}

pub fn device_name(root: &Path, id: &str) -> String {
    if !device::valid_id(id) {
        return "another device".into();
    }
    store_io::read_bounded(&root.join("state").join(id).join("area-format.json"), 4096)
        .ok()
        .and_then(|b| serde_json::from_slice::<Marker>(&b).ok())
        .filter(|m| m.device == id)
        .map(|m| m.name)
        .unwrap_or_else(|| id.into())
}
