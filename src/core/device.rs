use super::{state_dir, store_io};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

/// Machine-local settings. Syncthing must ignore device.json.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceSettings {
    pub name: String,
    /// Random installation identity, independent of the editable display name.
    pub id: String,
    pub daily_check: bool,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "this-device".into()),
            daily_check: true,
        }
    }
}

impl DeviceSettings {
    /// No identity creation or lock-file writes for agent read requests.
    pub(super) fn read_in(root: &Path) -> io::Result<Self> {
        let settings: Self = store_io::read(&root.join("device.json"))?;
        if !settings.id.is_empty() && !valid_id(&settings.id) {
            return Err(store_io::invalid("Invalid local device id"));
        }
        Ok(settings)
    }
    pub fn load() -> io::Result<Self> {
        Self::load_in(&state_dir())
    }
    pub(super) fn load_in(root: &Path) -> io::Result<Self> {
        if super::file_reads::refreshing() {
            return Self::read_in(root);
        }
        let path = root.join("device.json");
        let _lock = store_io::lock(&path.with_extension("lock"))?;
        let mut settings: Self = store_io::read(&path)?;
        if settings.id.is_empty() {
            settings.id = random_id()?;
            store_io::write(&path, &settings)?;
        }
        if !valid_id(&settings.id) {
            return Err(store_io::invalid("Invalid local device id"));
        }
        Ok(settings)
    }
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub(super) fn random_id() -> io::Result<String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut bytes = [0u8; 16];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err(io::Error::other("Could not create a random state identity"));
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
