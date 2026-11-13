#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    #[cfg(target_os = "linux")]
    if cfg!(target_os = "linux") {
        eprintln!("{}", sidenote_sync::RELAY_UNAVAILABLE);
        std::process::exit(2);
    }
    if sidenote_sync::worker::run_stdio().is_err() {
        std::process::exit(1);
    }
}
