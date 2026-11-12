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
        Some("tree") => {
            let mut command = sidenote_sync::tree_worker::command(std::ffi::OsStr::new(&args[2]))?;
            command.args(&args[3..]);
            sidenote_sync::containment::configure(&mut command);
            let mut child = command.spawn()?;
            println!("launcher_pid={}", child.id());
            std::io::stdout().flush()?;
            std::process::exit(child.wait()?.code().unwrap_or(2));
        }
        Some("probe") => {
            let started = std::time::Instant::now();
            let result = sidenote_sync::tree_worker::bounded_output(
                Command::new(&args[2]).args(&args[3..]),
                std::time::Duration::from_secs(2),
            );
            assert!(result.is_err());
            assert!(started.elapsed().as_secs_f64() < 3.0);
            println!(
                "probe timed out and reaped in {} ms",
                started.elapsed().as_millis()
            );
        }
        Some("verify") => sidenote_sync::containment::verify()?,
        _ => anyhow::bail!("expected identity, parent or verify"),
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {}
