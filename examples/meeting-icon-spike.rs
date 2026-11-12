//! Developer-only floating icon probe: cargo run --example meeting-icon-spike.
#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]
#[cfg(all(windows, feature = "desktop"))]
#[path = "meeting-icon-spike/windows.rs"]
mod windows;

fn main() {
    #[cfg(all(windows, feature = "desktop"))]
    windows::run();
    #[cfg(not(all(windows, feature = "desktop")))]
    {
        eprintln!("UNAVAILABLE_ON_PLATFORM");
        std::process::exit(2);
    }
}
