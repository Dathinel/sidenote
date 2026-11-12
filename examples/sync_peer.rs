//! Full-role synthetic peer. No Velopack, UI, installed listener or task startup.
#[cfg(any(debug_assertions, feature = "relay-qualification"))]
fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::umask(0o077);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    sidenote::core::configure_paths(&args).map_err(anyhow::Error::msg)?;
    let root = sidenote::core::state_dir();
    if !sidenote::core::is_proof_path(&root, "sidenote-sync-step3") {
        anyhow::bail!("explicit synthetic proof path required");
    }
    sidenote::service::configure(&args).map_err(anyhow::Error::msg)?;
    sidenote::sync_owner::proof_peer()?;
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "relay-qualification")))]
fn main() {
    std::process::exit(2);
}
