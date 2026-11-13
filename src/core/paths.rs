//! Process-wide folder overrides, installed before any worker or UI starts.
use std::{ffi::OsString, path::PathBuf, sync::OnceLock};

#[derive(Default)]
struct Paths {
    data: Option<PathBuf>,
    meetings: Option<PathBuf>,
    peers: Option<PathBuf>,
    read_only: bool,
}
static PATHS: OnceLock<Paths> = OnceLock::new();
thread_local! {static READ_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}

pub(crate) struct ReadOnlyScope(bool, std::marker::PhantomData<std::rc::Rc<()>>);
impl ReadOnlyScope {
    pub(crate) fn begin(enabled: bool) -> Self {
        Self(
            READ_ONLY.with(|mode| mode.replace(mode.get() || enabled)),
            Default::default(),
        )
    }
}
impl Drop for ReadOnlyScope {
    fn drop(&mut self) {
        READ_ONLY.with(|mode| mode.set(self.0));
    }
}

/// Confine synthetic proof paths, including fresh descendants, to a named OS temp folder.
#[doc(hidden)]
pub fn is_proof_path(path: &std::path::Path, name: &str) -> bool {
    let Ok(temp) = std::env::temp_dir().canonicalize() else {
        return false;
    };
    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return false;
    }
    let mut missing = Vec::new();
    let mut ancestor = path;
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for part in missing.into_iter().rev() {
                    resolved.push(part);
                }
                return resolved.starts_with(temp.join(name));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(part), Some(parent)) = (ancestor.file_name(), ancestor.parent()) else {
                    return false;
                };
                missing.push(part);
                ancestor = parent;
            }
            Err(_) => return false,
        }
    }
}

pub fn configure_paths(args: &[OsString]) -> Result<(), String> {
    let mut paths = Paths::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let slot = match arg.to_str() {
            Some("--data-dir") => &mut paths.data,
            Some("--meetings-dir") => &mut paths.meetings,
            Some("--peers-dir") => &mut paths.peers,
            Some("--mcp") => {
                paths.read_only = true;
                continue;
            }
            _ => continue,
        };
        let value = args
            .next()
            .filter(|a| !a.is_empty() && !a.to_string_lossy().starts_with("--"))
            .ok_or("Folder flags require a path argument")?;
        *slot = Some(PathBuf::from(value));
    }
    if paths.data.is_none() {
        paths.data = env_path("SIDENOTE_DATA_DIR").or_else(|| env_path("SIDENOTE_STATE_DIR"));
    }
    paths.meetings = paths.meetings.or_else(|| env_path("SIDENOTE_HEX_DIR"));
    paths.peers = paths.peers.or_else(|| env_path("SIDENOTE_PEERS_DIR"));
    PATHS
        .set(paths)
        .map_err(|_| "Folder overrides were already configured".into())
}
fn env_path(name: &str) -> Option<PathBuf> {
    crate::env_var(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}
pub(super) fn read_only() -> bool {
    READ_ONLY.with(|mode| mode.get())
        || super::file_reads::refreshing()
        || PATHS.get().is_some_and(|p| p.read_only)
}
pub(super) fn data() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::test_home().join("state"))
    }
    #[cfg(not(test))]
    {
        PATHS
            .get()
            .and_then(|p| p.data.clone())
            .or_else(|| env_path("SIDENOTE_DATA_DIR"))
            .or_else(|| env_path("SIDENOTE_STATE_DIR"))
    }
}
pub(super) fn meetings() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::test_home().join("meetings"))
    }
    #[cfg(not(test))]
    {
        let meetings = PATHS
            .get()
            .and_then(|p| p.meetings.clone())
            .or_else(|| env_path("SIDENOTE_HEX_DIR"));
        #[cfg(not(windows))]
        let meetings = meetings.or_else(|| data().map(|p| p.join("hex/meetings")));
        meetings
    }
}
pub(super) fn peers() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::test_home().join("peers"))
    }
    #[cfg(not(test))]
    {
        PATHS
            .get()
            .and_then(|p| p.peers.clone())
            .or_else(|| env_path("SIDENOTE_PEERS_DIR"))
            // An isolated data directory must never accidentally read the owner's peer meetings.
            .or_else(|| data().map(|p| p.join("peers")))
    }
}

pub(super) fn appdata_dir(name: &str, legacy: &str) -> PathBuf {
    let root = crate::platform::data_home();
    prefer_legacy_path(root.join(name), root.join(legacy))
}

/// Keep legacy paths in place: Syncthing shares and scheduled jobs may point at them.
pub(super) fn prefer_legacy_path(current: PathBuf, old: PathBuf) -> PathBuf {
    if !current.exists() && old.exists() {
        old
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::is_proof_path;

    #[test]
    fn proof_paths_resolve_temp_aliases_and_confine_fresh_descendants() {
        let temp = std::env::temp_dir();
        for base in [temp.clone(), temp.canonicalize().unwrap()] {
            assert!(is_proof_path(
                &base.join("areas-proof/fresh/data"),
                "areas-proof"
            ));
            assert!(!is_proof_path(
                &base.join("areas-proof-other/data"),
                "areas-proof"
            ));
            assert!(!is_proof_path(
                &base.join("areas-proof/../outside"),
                "areas-proof"
            ));
        }
    }
}
