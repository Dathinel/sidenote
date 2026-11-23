#![cfg(any(not(feature = "nightly"), target_os = "linux"))]

use std::process::{Command, Stdio};

fn app(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sidenote"));
    command
        .arg("--data-dir")
        .arg(root)
        .arg("--meetings-dir")
        .arg(root.join("meetings"))
        .arg("--peers-dir")
        .arg(root.join("peers"))
        .stdin(Stdio::null());
    #[cfg(windows)]
    command.env("LOCALAPPDATA", root.join("localapp"));
    command
}

#[cfg(not(feature = "nightly"))]
#[test]
fn stable_refuses_excluded_process_modes_before_initializing_state() {
    let root = tempfile::tempdir().unwrap();
    let cases: &[&[&str]] = &[
        &["--server"],
        &["--server", "--listen"],
        &["--device-role", "server"],
        &["--device-role", "hub"],
        &["--speech-worker"],
        &["--speech-port", "0"],
        &["--sync-control"],
        &["--ops", "status"],
        &["--cpu-models", "status"],
        &["--mcp"],
        &["--mcp-access", "status"],
        &["--mcp-port", "0"],
        &["--relay-preview-headless"],
        #[cfg(not(windows))]
        &["--listen"],
    ];
    for &args in cases {
        // Give an unguarded process a reason to stop before touching any state.
        let result = app(root.path())
            .args(args)
            .arg("--data-dir")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("unavailable in Sidenote Stable"),
            "{args:?}: {stderr}"
        );
        assert!(result.stdout.is_empty(), "{args:?}");
    }
    assert!(root.path().read_dir().unwrap().next().is_none());
}

#[cfg(all(not(feature = "desktop"), not(feature = "nightly")))]
#[test]
fn stable_ordinary_start_ignores_saved_server_role_without_writing_it() {
    let root = tempfile::tempdir().unwrap();
    let device = root.path().join("device.json");
    // A regressed service launch fails its lock instead of starting a server.
    std::fs::create_dir_all(root.path().join("local/service.lock")).unwrap();
    for role in ["server", "hub"] {
        let contents = format!(r#"{{"role":"{role}"}}"#);
        std::fs::write(&device, &contents).unwrap();
        let result = app(root.path()).output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(2));
        assert!(stderr.contains("Starting in full mode."), "{stderr}");
        assert!(stderr.contains("UNAVAILABLE_ON_PLATFORM"), "{stderr}");
        assert_eq!(std::fs::read_to_string(&device).unwrap(), contents);
        assert_eq!(root.path().read_dir().unwrap().count(), 2);
        assert_eq!(root.path().join("local").read_dir().unwrap().count(), 1);
    }
}

#[cfg(all(windows, not(feature = "desktop"), not(feature = "nightly")))]
#[test]
fn stable_windows_scheduled_listener_starts_its_recorder() {
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();
    let mut child = app(root.path())
        .args(["--listen", "--listener-isolated", "--listener-localappdata"])
        .arg(root.path().join("localapp"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let heartbeat = root.path().join("local/recorder-heartbeat.json");
    let deadline = Instant::now() + Duration::from_secs(2);
    let started = loop {
        if heartbeat.is_file() {
            break true;
        }
        if Instant::now() >= deadline || !matches!(child.try_wait(), Ok(None)) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let _ = child.kill();
    child.wait().unwrap();
    assert!(
        started,
        "scheduled listener did not publish its isolated heartbeat"
    );
}

#[cfg(not(feature = "nightly"))]
#[test]
fn stable_relay_entry_points_preserve_existing_files_and_local_reads() {
    use sidenote::core::relay_preview::{self, Settings, operations, pairing};

    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("local/relay");
    let existing: [(&str, &[u8]); 3] = [
        (
            "settings.json",
            br#"{"enabled":true,"address":"ws://127.0.0.1:0","backup":"fixture"}"#,
        ),
        ("backups/fixture/manifest.json", b"backup"),
        ("replica.sqlite", b"replica"),
    ];
    for (name, bytes) in existing {
        let path = data.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::write(
        root.path().join("config.json"),
        br#"{"notes":{"your_name":"Demo"}}"#,
    )
    .unwrap();

    assert!(!Settings::load(root.path()).unwrap().enabled);
    assert!(relay_preview::get(root.path()).unwrap().is_none());
    assert_eq!(
        relay_preview::profile(root.path()).unwrap().your_name,
        "Demo"
    );
    assert!(operations::status(root.path()).unwrap().read_only);
    for enabled in [true, false] {
        let settings = Settings {
            enabled,
            ..Default::default()
        };
        assert_eq!(
            settings.save(root.path()).unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
    }
    assert_eq!(
        operations::check_now(root.path()).err().unwrap().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(
        pairing::start(root.path(), None).err().unwrap().kind(),
        std::io::ErrorKind::Unsupported
    );
    for (name, bytes) in existing {
        assert_eq!(std::fs::read(data.join(name)).unwrap(), bytes);
    }
    assert_eq!(data.read_dir().unwrap().count(), existing.len());
    assert!(!root.path().join(".stignore").exists());
}

#[cfg(all(target_os = "linux", feature = "nightly"))]
#[test]
fn nightly_sync_control_still_accepts_an_empty_input_stream() {
    let root = tempfile::tempdir().unwrap();
    let result = app(root.path()).arg("--sync-control").output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
