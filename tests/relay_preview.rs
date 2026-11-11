//! Real headless app processes, isolated stores, real local Worker and encrypted sessions.
use serde_json::{Value, json};
use sidenote::core::{ManualTask, TaskStore, relay_preview};

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().is_ok_and(|status| status.is_some()) {
            return;
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .creation_flags(0x08000000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct App {
    process: Process,
    input: ChildStdin,
    output: Receiver<String>,
}
impl App {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sidenote"))
            .arg("--relay-preview-headless")
            .arg("--data-dir")
            .arg(root)
            .arg("--meetings-dir")
            .arg(root.join("meetings"))
            .arg("--peers-dir")
            .arg(root.join("peers"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start isolated headless app");
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let app = Self {
            process: Process(child),
            input,
            output,
        };
        assert_eq!(app.receive(), json!({"ready":true}), "app startup");
        app
    }
    fn receive(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("app protocol timeout or exit");
        serde_json::from_str(&line).expect("app protocol response")
    }
    fn request(&mut self, request: Value) -> Value {
        serde_json::to_writer(&mut self.input, &request).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        let result = self.receive();
        assert_eq!(result["ok"], true, "app operation rejected");
        result["value"].clone()
    }
    fn stop(mut self) {
        self.request(json!({"op":"quit"}));
        assert!(self.process.0.wait().unwrap().success());
    }
}

fn wait_for(app: &mut App, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let tasks = app.request(json!({"op":"list"}));
        if predicate(&tasks) {
            return tasks;
        }
        assert!(Instant::now() < deadline, "replicas did not converge");
        thread::sleep(Duration::from_millis(50));
    }
}

fn available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_relay(state: &Path) -> (Process, String) {
    let port = available_port();
    let inspector = available_port();
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("relay/node_modules/.bin/wrangler.exe");
    let mut command = if local.is_file() {
        Command::new(local)
    } else {
        let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join("wrangler.exe"))
            .find(|p| p.is_file())
            .expect("installed Wrangler required for local relay test");
        Command::new(executable)
    };
    command
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("relay"))
        .args([
            "dev",
            "--config",
            "harness/expiry.toml",
            "--local",
            "--ip",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--inspector-ip",
            "127.0.0.1",
            "--inspector-port",
            &inspector.to_string(),
            "--show-interactive-dev-session",
            "false",
            "--log-level",
            "none",
            "--persist-to",
        ])
        .arg(state)
        .env("WRANGLER_SEND_METRICS", "false")
        .env("WRANGLER_CHECK_UPDATE", "false")
        .env("CI", "true")
        .env("BROWSER", "none")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut process = Process(command.spawn().expect("start local Wrangler"));
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "local Wrangler exited before health check"
        );
        if let Ok(mut socket) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(250),
        ) {
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let _ = socket
                .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
            let mut response = String::new();
            if socket.read_to_string(&mut response).is_ok()
                && response.contains("sidenote-relay-v1")
            {
                break;
            }
        }
        assert!(Instant::now() < deadline, "local Wrangler startup timeout");
        thread::sleep(Duration::from_millis(100));
    }
    (process, format!("ws://127.0.0.1:{port}"))
}

fn journals(root: &Path) -> BTreeFiles {
    fn collect(root: &Path, output: &mut BTreeFiles) {
        if !root.exists() {
            return;
        }
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, output);
            } else {
                output.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    let mut output = BTreeFiles::new();
    collect(&root.join("state"), &mut output);
    output
}
type BTreeFiles = std::collections::BTreeMap<PathBuf, Vec<u8>>;

fn status(app: &mut App, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let value = app.request(json!({"op":"status"}));
        if value["phase"] == phase {
            return value;
        }
        assert!(Instant::now() < deadline, "pairing phase did not arrive");
        thread::sleep(Duration::from_millis(30));
    }
}
fn confirmation(root: &Path) -> Value {
    let bytes = std::fs::read(root.join("local/relay/test-confirm.dpapi")).unwrap();
    let plain = sidenote_sync::identity::unprotect(&bytes).expect("protected confirmation");
    serde_json::from_slice(&plain).expect("confirmation shape")
}
fn expire_invitation(root: &Path, address: &str) {
    let encrypted = std::fs::read(root.join("local/relay/test-code.dpapi")).unwrap();
    let code = sidenote_sync::identity::unprotect(&encrypted).expect("protected invitation");
    let locator: String = sidenote_sync::identity::hash(
        format!(
            "sidenote-pair-v1:{}",
            std::str::from_utf8(&code[..3]).unwrap()
        )
        .as_bytes(),
    )
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect();
    let mut socket = TcpStream::connect(address.strip_prefix("ws://").unwrap()).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket, "POST /__test/expire/{locator} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 204"),
        "test invitation expiry failed"
    );
}
fn find<'a>(tasks: &'a Value, id: &Value) -> Option<&'a Value> {
    tasks.as_array()?.iter().find(|t| &t["id"] == id)
}

#[test]
fn two_apps_real_pairing_migration_rollback_conflict_resolution_and_removal() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    let a = fixture.path().join("a");
    let b = fixture.path().join("b");
    for (root, name, id) in [(&a, "Studio PC", "a"), (&b, "Travel laptop", "b")] {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("device.json"),
            serde_json::to_vec(&json!({"name":name,"id":id.repeat(32),"daily_check":false}))
                .unwrap(),
        )
        .unwrap();
    }
    let folder = TaskStore { root: a.clone() };
    let (_, folder_id) = folder
        .add_manual(ManualTask {
            task: "Synthetic shared folder task".into(),
            ..Default::default()
        })
        .unwrap();
    let shared = Value::String(folder_id.clone());
    for (path, bytes) in journals(&a) {
        let dest = b.join(path.strip_prefix(&a).unwrap());
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(dest, bytes).unwrap();
    }
    TaskStore { root: b.clone() }
        .update_overrides(|_| {})
        .unwrap();
    let original_a = journals(&a);
    let original_b = journals(&b);
    let (_relay, address) = start_relay(&fixture.path().join("relay"));
    for root in [&a, &b] {
        relay_preview::Settings {
            enabled: true,
            address: address.clone(),
            ..Default::default()
        }
        .save(root)
        .unwrap();
    }
    let mut app_a = App::start(&a);
    let mut app_b = App::start(&b);
    assert_eq!(
        app_a
            .request(json!({"op":"list"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        app_b
            .request(json!({"op":"list"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let unique_a = app_a.request(json!({"op":"add","task":{"task":"Synthetic private A"}}));
    let unique_b = app_b.request(json!({"op":"add","task":{"task":"Synthetic private B"}}));
    app_a.request(json!({"op":"host"}));
    status(&mut app_a, "code");
    expire_invitation(&a, &address);
    app_b.request(json!({"op":"join","from":a}));
    status(&mut app_b, "failed");
    assert!(
        find(&app_b.request(json!({"op":"list"})), &unique_a).is_none(),
        "expired code leaked tasks"
    );
    assert_eq!(
        app_b.request(json!({"op":"status"}))["devices"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app_a.request(json!({"op":"host"}));
    status(&mut app_a, "code");
    app_b.request(json!({"op":"join","from":a,"wrong":true}));
    status(&mut app_b, "failed");
    assert!(
        find(&app_b.request(json!({"op":"list"})), &unique_a).is_none(),
        "wrong code leaked tasks"
    );
    assert_eq!(
        app_b.request(json!({"op":"status"}))["devices"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app_a.request(json!({"op":"host"}));
    status(&mut app_a, "code");
    app_b.request(json!({"op":"join","from":a}));
    status(&mut app_a, "confirm");
    status(&mut app_b, "confirm");
    let ac = confirmation(&a);
    let bc = confirmation(&b);
    assert!(ac["phrase"] == bc["phrase"], "safety phrases differ");
    assert!(ac["local"] == "Studio PC" && ac["peer"] == "Travel laptop");
    assert!(bc["local"] == "Travel laptop" && bc["peer"] == "Studio PC");
    app_a.request(json!({"op":"confirm","accept":true}));
    thread::sleep(Duration::from_millis(300));
    assert!(
        find(&app_b.request(json!({"op":"list"})), &unique_a).is_none(),
        "data flowed before both confirmations"
    );
    app_b.request(json!({"op":"confirm","accept":true}));
    status(&mut app_a, "paired");
    status(&mut app_b, "paired");
    for app in [&mut app_a, &mut app_b] {
        let tasks = wait_for(app, |t| {
            find(t, &unique_a).is_some() && find(t, &unique_b).is_some()
        });
        assert_eq!(
            tasks.as_array().unwrap().len(),
            3,
            "migration duplicated tasks"
        );
        assert_eq!(
            tasks
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["id"] == shared)
                .count(),
            1
        );
    }
    assert_eq!(
        app_a.request(json!({"op":"conflicts"})),
        json!([]),
        "identical migration values are not conflicts"
    );
    app_b.request(json!({"op":"join","from":a}));
    status(&mut app_b, "failed");
    assert_eq!(
        app_b.request(json!({"op":"status"}))["devices"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "consumed code changed membership"
    );
    app_a.request(json!({"op":"offline"}));
    app_b.request(json!({"op":"offline"}));
    app_a.request(json!({"op":"edit","id":shared,"fields":{"owner":"Synthetic owner"}}));
    app_b.request(json!({"op":"edit","id":shared,"fields":{"due":"2030-02-03"}}));
    app_a.stop();
    app_b.stop();
    let mut app_a = App::start(&a);
    let mut app_b = App::start(&b);
    for app in [&mut app_a, &mut app_b] {
        wait_for(app, |t| {
            find(t, &shared)
                .is_some_and(|t| t["owner"] == "Synthetic owner" && t["due"] == "2030-02-03")
        });
    }
    app_a.request(json!({"op":"offline"}));
    app_b.request(json!({"op":"offline"}));
    app_a.request(json!({"op":"edit","id":shared,"fields":{"owner":"Owner A"}}));
    app_b.request(json!({"op":"edit","id":shared,"fields":{"owner":"Owner B"}}));
    app_a.request(json!({"op":"online"}));
    app_b.request(json!({"op":"online"}));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let a_versions = app_a.request(json!({"op":"conflicts"}));
        let b_versions = app_b.request(json!({"op":"conflicts"}));
        if a_versions[0]["choices"]
            .as_array()
            .is_some_and(|v| v.len() == 2)
            && b_versions[0]["choices"]
                .as_array()
                .is_some_and(|v| v.len() == 2)
        {
            assert!(
                a_versions[0]["choices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["at"].as_str().is_some_and(|t| !t.is_empty()))
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "same-field conflict did not arrive"
        );
        thread::sleep(Duration::from_millis(30));
    }
    app_a.request(json!({"op":"resolve","id":shared,"field":"owner","choice":0}));
    for app in [&mut app_a, &mut app_b] {
        wait_for(app, |t| {
            find(t, &shared).is_some_and(|t| t["owner"] == "Owner A")
        });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let records = app_b.request(json!({"op":"resolutions"}));
        if records.as_array().is_some_and(|v| v.len() == 1) {
            assert_eq!(records[0]["alternatives"].as_array().unwrap().len(), 2);
            break;
        }
        assert!(Instant::now() < deadline, "resolution log did not sync");
        thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(app_a.request(json!({"op":"conflicts"})), json!([]));
    app_a.request(json!({"op":"remove","name":"Travel laptop"}));
    assert_eq!(
        app_a.request(json!({"op":"status"}))["devices"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let after_remove = app_a.request(json!({"op":"add","task":{"task":"Synthetic after removal"}}));
    thread::sleep(Duration::from_millis(300));
    assert!(
        find(&app_b.request(json!({"op":"list"})), &after_remove).is_none(),
        "removed peer received a new task"
    );
    assert_eq!(journals(&a), original_a);
    assert_eq!(journals(&b), original_b);
    app_a.request(json!({"op":"disable"}));
    let folder_tasks = app_a.request(json!({"op":"list"}));
    assert_eq!(folder_tasks.as_array().unwrap().len(), 1);
    assert_eq!(folder_tasks[0]["id"], shared);
    assert_eq!(journals(&a), original_a, "rollback changed folder bytes");
    app_a.request(json!({"op":"enable"}));
    assert!(
        find(&app_a.request(json!({"op":"list"})), &after_remove).is_some(),
        "rollback discarded relay data"
    );
    app_a.stop();
    app_b.stop();
}
