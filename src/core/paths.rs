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
        paths.data =
            env_path("HEX_MEETINGS_DATA_DIR").or_else(|| env_path("HEX_MEETINGS_STATE_DIR"));
    }
    paths.meetings = paths.meetings.or_else(|| env_path("HEX_MEETINGS_HEX_DIR"));
    paths.peers = paths.peers.or_else(|| env_path("HEX_MEETINGS_PEERS_DIR"));
    PATHS
        .set(paths)
        .map_err(|_| "Folder overrides were already configured".into())
}
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}
pub(super) fn read_only() -> bool {
    PATHS.get().is_some_and(|p| p.read_only)
}
pub(super) fn data() -> Option<PathBuf> {
    PATHS
        .get()
        .and_then(|p| p.data.clone())
        .or_else(|| env_path("HEX_MEETINGS_DATA_DIR"))
        .or_else(|| env_path("HEX_MEETINGS_STATE_DIR"))
}
pub(super) fn meetings() -> Option<PathBuf> {
    PATHS
        .get()
        .and_then(|p| p.meetings.clone())
        .or_else(|| env_path("HEX_MEETINGS_HEX_DIR"))
}
pub(super) fn peers() -> Option<PathBuf> {
    PATHS
        .get()
        .and_then(|p| p.peers.clone())
        .or_else(|| env_path("HEX_MEETINGS_PEERS_DIR"))
        // An isolated data directory must never accidentally read the owner's peer meetings.
        .or_else(|| data().map(|p| p.join("peers")))
}
