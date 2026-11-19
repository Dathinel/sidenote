//! CI-only process fixture. Never selected by the production worker path.
#![cfg_attr(windows, windows_subsystem = "windows")]
use std::io::{Read, Write};
fn main() {
    sidenote_sync::worker::suppress_crash_dialogs();
    let mut mode = [0; 1];
    if std::io::stdin().read_exact(&mut mode).is_err() {
        return;
    }
    if mode[0] == 4 {
        std::io::stdout()
            .write_all(b"ready")
            .expect("fixture ready");
        std::io::stdout().flush().expect("fixture ready flush");
        if std::io::stdin().read_exact(&mut mode).is_err() {
            return;
        }
    }
    match mode[0] {
        0 => std::process::abort(),
        1 => std::thread::sleep(std::time::Duration::from_secs(60)),
        2 => {
            let mut allocations = Vec::new();
            for _ in 0..128 {
                allocations.push(vec![1u8; 8 * 1024 * 1024]);
                std::hint::black_box(&allocations);
            }
            std::io::stdout()
                .write_all(b"allocated")
                .expect("fixture output");
            std::io::stdout().flush().expect("fixture flush");
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        3 => {
            let allocation = vec![1u8; 300 * 1024 * 1024];
            std::hint::black_box(&allocation);
            std::io::stdout()
                .write_all(b"allocated")
                .expect("fixture output");
            std::io::stdout().flush().expect("fixture flush");
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        _ => {}
    }
}
