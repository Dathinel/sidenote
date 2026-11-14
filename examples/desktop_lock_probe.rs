//! Synthetic box qualification: acquire the real desktop lock without starting a window.
#[cfg(target_os = "linux")]
fn main() {
    sidenote::core::configure_paths(&[]).unwrap();
    match sidenote::platform::single_instance() {
        Ok(Some(_lock)) => {
            println!("LOCKED");
            if std::env::args().any(|a| a == "--hold") {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
            }
        }
        Ok(None) => std::process::exit(3),
        Err(_) => std::process::exit(2),
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {}
