pub mod core;
pub mod hex_engine;
#[cfg(windows)]
pub mod hex_install;
pub mod hex_labs;
pub mod listener;
pub mod mcp;
#[cfg(all(windows, feature = "desktop"))]
pub mod meeting_icon;
#[cfg(target_os = "linux")]
pub mod operations;
pub mod platform;
#[cfg(target_os = "linux")]
pub mod server_models;
pub mod service;
#[cfg(any(target_os = "linux", debug_assertions, feature = "relay-qualification"))]
pub mod sync_owner;
pub mod updates;

/// Prefer the Sidenote name while accepting existing launchers and render tools.
pub fn env_var(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let suffix = name.strip_prefix("SIDENOTE_")?;
            std::env::var_os(format!("HEX_MEETINGS_{suffix}")).filter(|value| !value.is_empty())
        })
}
