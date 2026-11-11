#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    if sidenote_sync::worker::run_stdio().is_err() {
        std::process::exit(1);
    }
}
