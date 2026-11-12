//! Standalone, opt-in-to-integration device synchronization. No app or global paths.
pub mod capabilities;
pub mod client;
#[cfg(target_os = "linux")]
pub mod containment;
mod crypto;
pub mod identity;
#[cfg(target_os = "linux")]
mod linux_secret;
pub mod model;
pub mod pairing;
mod preflight;
#[cfg(target_os = "linux")]
mod secure_store;
pub mod status;
pub mod store;
pub mod transport;
#[cfg(target_os = "linux")]
pub mod tree_worker;
mod wire;
pub mod worker;
/// Reuse the Linux StrictModes boundary for the owner's private local endpoint.
#[cfg(target_os = "linux")]
pub fn validate_private_directory(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let path = secure_store::ancestors(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "STATE_DIRECTORY_NOT_PRIVATE"
    );
    Ok(())
}
