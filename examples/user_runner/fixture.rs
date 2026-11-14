use sidenote::platform::UserRunner;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

struct Parent(std::process::Child);
impl Drop for Parent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "fixture deadline exceeded");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn spawn(root: &Path) -> UserRunner {
    let mut command = Command::new("/bin/sh");
    command
        .arg(root.join("helper.sh"))
        .arg(root)
        .current_dir(root)
        .env("SIDENOTE_FIXTURE", "inherited");
    UserRunner::spawn(&mut command, true).unwrap()
}
fn ready(root: &Path) -> Vec<i32> {
    let names = ["helper", "child", "grandchild"];
    until(|| {
        names.iter().all(|name| {
            fs::read_to_string(root.join(name)).is_ok_and(|s| s.trim().parse::<i32>().is_ok())
        })
    });
    let ids: Vec<i32> = names
        .iter()
        .map(|name| {
            fs::read_to_string(root.join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap()
        })
        .collect();
    for pid in &ids {
        assert_eq!(unsafe { libc::getpgid(*pid) }, ids[0]);
        let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        let uid = status
            .lines()
            .find(|line| line.starts_with("Uid:"))
            .unwrap();
        assert!(
            uid.split_whitespace()
                .skip(1)
                .all(|v| v.parse::<u32>().unwrap() == unsafe { libc::getuid() })
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("context")).unwrap(),
        format!("inherited\n{}\n", root.display())
    );
    ids
}
fn gone(ids: &[i32]) {
    until(|| {
        ids.iter()
            .all(|pid| !Path::new(&format!("/proc/{pid}")).exists())
    });
}
fn tree(root: &Path) {
    fs::write(
        root.join("helper.sh"),
        r#"#!/bin/sh
printf '%s\n%s\n' "$SIDENOTE_FIXTURE" "$PWD" > "$1/context"
echo $$ > "$1/helper"
(sleep 60 & echo $! > "$1/grandchild"; wait) &
echo $! > "$1/child"
while [ ! -f "$1/release" ]; do sleep 0.02; done
printf 'synthetic\n'
exit 0
"#,
    )
    .unwrap();
}
pub fn run() {
    if std::env::args().nth(1).as_deref() == Some("--victim") {
        let root = std::path::PathBuf::from(std::env::args_os().nth(2).unwrap());
        let _runner = spawn(&root);
        loop {
            std::thread::park();
        }
    }
    // Reap the supervisor orphaned by the deliberately crashed fixture parent.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    let mut smoke = UserRunner::spawn(
        Command::new("/bin/sh").args(["-c", "printf synthetic"]),
        true,
    )
    .unwrap();
    let mut output = String::new();
    smoke
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(smoke.wait().unwrap().success());
    assert_eq!(output, "synthetic");
    println!("PASS 8a smoke");
    for mode in [
        "completion",
        "timeout",
        "cancel",
        "shutdown",
        "helper-crash",
        "parent-crash",
    ] {
        let root = tempfile::tempdir().unwrap();
        tree(root.path());
        if mode == "parent-crash" {
            let mut parent = Parent(
                Command::new(std::env::current_exe().unwrap())
                    .arg("--victim")
                    .arg(root.path())
                    .spawn()
                    .unwrap(),
            );
            let ids = ready(root.path());
            parent.0.kill().unwrap();
            assert!(!parent.0.wait().unwrap().success());
            gone(&ids);
            assert!(unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) } > 0);
        } else {
            let mut runner = spawn(root.path());
            let supervisor = runner.id();
            assert_eq!(
                unsafe { libc::getpgid(supervisor as i32) },
                supervisor as i32
            );
            let ids = ready(root.path());
            match mode {
                "completion" => {
                    fs::write(root.path().join("release"), "").unwrap();
                    assert!(runner.wait().unwrap().success());
                    let mut output = String::new();
                    runner
                        .stdout
                        .take()
                        .unwrap()
                        .read_to_string(&mut output)
                        .unwrap();
                    assert_eq!(output, "synthetic\n");
                }
                "timeout" => {
                    let deadline = Instant::now() + Duration::from_millis(100);
                    until(|| Instant::now() >= deadline);
                    assert!(runner.try_wait().unwrap().is_none());
                    runner.stop().unwrap();
                    assert!(!runner.wait().unwrap().success());
                }
                "helper-crash" => {
                    assert_eq!(unsafe { libc::kill(ids[0], libc::SIGKILL) }, 0);
                    assert!(!runner.wait().unwrap().success());
                }
                "cancel" => runner.stop().unwrap(),
                _ => {}
            }
            drop(runner);
            gone(&ids);
            assert!(!Path::new(&format!("/proc/{supervisor}")).exists());
        }
        println!("PASS {mode}: helper, child and grandchild killed and reaped");
    }
    // No CLI installation, credentials or account is involved in these errors.
    for (script, code) in [
        ("printf 'authentication required' >&2; exit 7", 7),
        ("printf invalid-output", 0),
    ] {
        let mut runner =
            UserRunner::spawn(Command::new("/bin/sh").args(["-c", script]), true).unwrap();
        assert_eq!(runner.wait().unwrap().code(), Some(code));
    }
    let mut missing =
        UserRunner::spawn(&mut Command::new("/nonexistent/synthetic-helper"), true).unwrap();
    assert_eq!(missing.wait().unwrap().code(), Some(125));
    let mut diagnostic = String::new();
    missing
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut diagnostic)
        .unwrap();
    assert!(diagnostic.is_empty());
    println!(
        "PASS synthetic missing/auth/invalid-output status and missing-helper redaction; protocol validation belongs to layer 9"
    );
    for piped in [true, false] {
        let script = if piped {
            "test -p /proc/self/fd/0 && cat"
        } else {
            "test -c /proc/self/fd/0 && cat"
        };
        let mut runner =
            UserRunner::spawn(Command::new("/bin/sh").args(["-c", script]), piped).unwrap();
        assert_eq!(runner.stdin.is_some(), piped);
        if let Some(input) = runner.stdin.as_mut() {
            input.write_all(b"synthetic EOF input").unwrap();
        }
        // wait must close the retained writer, allowing cat to observe EOF.
        assert!(runner.wait().unwrap().success());
        assert!(runner.stdin.is_none());
        let mut output = String::new();
        runner
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, if piped { "synthetic EOF input" } else { "" });
        println!("PASS EOF-reading helper: piped_stdin={piped}");
    }
}
