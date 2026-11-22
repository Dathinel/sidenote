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
    pub role: crate::service::DeviceRole,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "this-device".into()),
            daily_check: true,
            role: Default::default(),
        }
    }
}

impl DeviceSettings {
    pub fn set_role(role: crate::service::DeviceRole) -> io::Result<()> {
        let root = state_dir();
        let path = root.join("device.json");
        let _lock = store_io::lock(&path.with_extension("lock"))?;
        let mut settings: Self = store_io::read(&path)?;
        if settings.id.is_empty() {
            settings.id = random_id()?;
        }
        if !valid_id(&settings.id) {
            return Err(store_io::invalid("Invalid local device id"));
        }
        settings.role = role;
        store_io::write(&path, &settings)
    }

    /// The name other devices show. Same rule as pairing: 1 to 80 characters, no control characters.
    pub fn valid_name(name: &str) -> Option<String> {
        let name = name.trim();
        (!name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control))
            .then(|| name.to_owned())
    }
    pub fn set_name(name: &str) -> io::Result<()> {
        Self::set_name_in(&state_dir(), name)
    }
    pub(crate) fn set_name_in(root: &Path, name: &str) -> io::Result<()> {
        let name = Self::valid_name(name).ok_or_else(|| {
            store_io::invalid("Use 1 to 80 characters, with no control characters")
        })?;
        let path = root.join("device.json");
        let _lock = store_io::lock(&path.with_extension("lock"))?;
        let mut settings: Self = store_io::read(&path)?;
        if settings.id.is_empty() {
            settings.id = random_id()?;
        }
        if !valid_id(&settings.id) {
            return Err(store_io::invalid("Invalid local device id"));
        }
        settings.name = name;
        store_io::write(&path, &settings)
    }

    /// No identity creation or lock-file writes for agent read requests.
    pub(crate) fn read_in(root: &Path) -> io::Result<Self> {
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
        // device.json is replaced atomically, so reading an existing id needs no lock. Only
        // creating the id takes it, and re-reads first in case another thread created it.
        let mut settings = Self::read_in(root)?;
        if settings.id.is_empty() {
            let path = root.join("device.json");
            let _lock = store_io::lock(&path.with_extension("lock"))?;
            settings = store_io::read(&path)?;
            if settings.id.is_empty() {
                settings.id = random_id()?;
                store_io::write(&path, &settings)?;
            }
        }
        if !valid_id(&settings.id) {
            return Err(store_io::invalid("Invalid local device id"));
        }
        super::area_version::record(root, &settings)?;
        Ok(settings)
    }
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(windows)]
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

#[cfg(not(windows))]
pub(super) fn random_id() -> io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_keeps_the_identity_and_rejects_bad_names() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let before = DeviceSettings::load_in(root.path())?;
        DeviceSettings::set_name_in(root.path(), "  Studio PC ")?;
        let after = DeviceSettings::read_in(root.path())?;
        assert_eq!(after.name, "Studio PC");
        assert_eq!(after.id, before.id);
        for bad in ["", "   ", "tab	here", &"x".repeat(81)] {
            assert!(
                DeviceSettings::set_name_in(root.path(), bad).is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(DeviceSettings::read_in(root.path())?.name, "Studio PC");
        assert!(DeviceSettings::valid_name(&"y".repeat(80)).is_some());
        Ok(())
    }
}
