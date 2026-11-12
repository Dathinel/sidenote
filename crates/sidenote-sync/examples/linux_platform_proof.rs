//! Synthetic qualification only; never prints secret key material.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use std::{
        io::Write,
        path::Path,
        process::{Command, Stdio},
    };
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("identity") => {
            let device = sidenote_sync::client::Device::open(Path::new(&args[2]))?;
            println!(
                "public_fingerprint={}",
                hex::encode(sidenote_sync::identity::hash(&device.identity.public()))
            );
        }
        Some("parent") => {
            let mut command = Command::new(&args[2]);
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            sidenote_sync::containment::configure(&mut command);
            let mut child = command.spawn()?;
            child.stdin.as_mut().unwrap().write_all(&[1])?;
            println!("worker_pid={}", child.id());
            std::io::stdout().flush()?;
            std::thread::sleep(std::time::Duration::from_secs(60));
            sidenote_sync::containment::kill_group(child.id());
            child.wait()?;
        }
        Some("verify") => sidenote_sync::containment::verify()?,
        _ => anyhow::bail!("expected identity, parent or verify"),
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {}
