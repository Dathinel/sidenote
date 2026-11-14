pub mod core;
#[cfg(feature = "desktop")]
pub mod desktop;
pub mod hex_engine;
#[cfg(windows)]
pub mod hex_install;
#[cfg(not(windows))]
#[path = "hex_install_unix.rs"]
pub mod hex_install;
mod hex_install_state;
pub mod hex_labs;
pub mod listener;
pub mod mcp;
#[cfg(all(windows, feature = "desktop"))]
pub mod meeting_icon;
#[cfg(target_os = "linux")]
pub mod operations;
pub mod platform;
pub mod response_time;
#[cfg(target_os = "linux")]
pub mod server_models;
pub mod service;
#[cfg(any(target_os = "linux", debug_assertions, feature = "relay-qualification"))]
pub mod sync_owner;
mod update_state;
pub mod updates;

#[cfg(test)]
pub(crate) fn test_home() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-storage");
        std::fs::create_dir_all(&parent).expect("create isolated test storage");
        tempfile::tempdir_in(parent).expect("create test home")
    })
    .path()
}

#[cfg(test)]
#[test]
fn test_paths_never_resolve_into_owner_appdata() {
    for path in [
        core::log_path(),
        core::state_dir().join("config.json"),
        core::meetings_dir(),
        core::peers_dir(),
        platform::data_home(),
        platform::local_home(),
        platform::config_home(),
        platform::home(),
        core::live::notes_pipeline(),
    ] {
        assert!(
            path.starts_with(test_home()),
            "test storage must stay in its isolated root"
        );
        for name in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(owner) = std::env::var_os(name).filter(|value| !value.is_empty()) {
                assert!(
                    !path.starts_with(std::path::Path::new(&owner)),
                    "test path resolved into owner app data"
                );
            }
        }
    }
}

/// Prefer the Sidenote name while accepting existing launchers and render tools.
pub fn env_var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let suffix = name.strip_prefix("SIDENOTE_")?;
            std::env::var_os(format!("HEX_MEETINGS_{suffix}")).filter(|value| !value.is_empty())
        })
}
