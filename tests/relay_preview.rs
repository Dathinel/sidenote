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
        Self::start_binary(root, Path::new(env!("CARGO_BIN_EXE_sidenote")))
    }
    fn start_binary(root: &Path, binary: &Path) -> Self {
        let app = Self::spawn(root, binary);
        assert_eq!(app.receive(), json!({"ready":true}), "app startup");
        app
    }
    fn spawn(root: &Path, binary: &Path) -> Self {
        let mut child = Command::new(binary)
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
        Self {
            process: Process(child),
            input,
            output,
        }
    }
    fn receive(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("app protocol timeout or exit");
        serde_json::from_str(&line).expect("app protocol response")
    }
    #[track_caller]
    fn request(&mut self, request: Value) -> Value {
        serde_json::to_writer(&mut self.input, &request).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        let result = self.receive();
        assert!(
            result["ok"] == true,
            "app operation {} rejected",
            request["op"].as_str().unwrap_or("unknown")
        );
        result["value"].clone()
    }
    fn rejected(&mut self, request: Value) {
        serde_json::to_writer(&mut self.input, &request).unwrap();
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        assert!(
            self.receive()["ok"] == false,
            "unsafe operation was accepted"
        );
    }
    fn stop(mut self) {
        self.request(json!({"op":"quit"}));
        assert!(self.process.0.wait().unwrap().success());
    }
}

#[track_caller]
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

#[track_caller]
fn wait_shared(app: &mut App, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let value = app.request(json!({"op":"shared-state"}));
        if predicate(&value) {
            return value;
        }
        assert!(Instant::now() < deadline, "shared domain did not converge");
        thread::sleep(Duration::from_millis(60));
    }
}
fn meeting(value: &Value) -> Option<&Value> {
    value["overrides"]["tasks"]["100-2"]
        .as_object()?
        .values()
        .find(|v| v["task"] == "Synthetic generated task")
}
fn available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

static RELAY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct LocalRelay {
    _process: Process,
    _lease: std::sync::MutexGuard<'static, ()>,
}

fn start_relay(state: &Path) -> (LocalRelay, String) {
    // Each case still runs isolated peers; avoid four competing Workerd startups.
    let lease = RELAY_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    (
        LocalRelay {
            _process: process,
            _lease: lease,
        },
        format!("ws://127.0.0.1:{port}"),
    )
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

#[track_caller]
fn status(app: &mut App, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let value = app.request(json!({"op":"status"}));
        if value["phase"] == phase {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "pairing phase {phase} did not arrive; current phase: {}",
            value["phase"].as_str().unwrap_or("unknown"),
        );
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
            subtasks: vec![sidenote::core::Subtask {
                id: "synthetic-subtask".into(),
                title: "Initial subtask".into(),
                done: false,
            }],
            ..Default::default()
        })
        .unwrap();
    let shared = Value::String(folder_id.clone());
    folder.add_project("Research").unwrap();
    folder.add_project("Fieldwork").unwrap();
    for (path, bytes) in journals(&a) {
        let dest = b.join(path.strip_prefix(&a).unwrap());
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(dest, bytes).unwrap();
    }
    TaskStore { root: b.clone() }
        .update_overrides(|_| {})
        .unwrap();
    for (root, model, sidebar) in [(&a, "local-model-a", false), (&b, "local-model-b", true)] {
        let store = TaskStore::new(root.clone());
        store
            .save_teams(&sidenote::core::Teams {
                teams: vec![sidenote::core::Team {
                    id: String::new(),
                    name: "Research team".into(),
                    members: vec!["Alex".into()],
                    aliases: Default::default(),
                }],
            })
            .unwrap();
        let mut config = serde_json::to_value(sidenote::core::Config::default()).unwrap();
        config["notes"] = json!({"your_name":"Synthetic owner","model":model});
        let config: sidenote::core::Config = serde_json::from_value(config).unwrap();
        config.save(root).unwrap();
        let mut prefs = sidenote::core::UiPrefs {
            sidebar_collapsed: Some(sidebar),
            ..Default::default()
        };
        prefs
            .people_sides
            .insert("alex".into(), sidenote::core::people::Side::OurTeam);
        prefs.save(root).unwrap();
        let memory = json!({"schema_version":1,"records":[{"id":"synthetic-memory","type":"story","text":"Synthetic initial memory","project":null,"people":[],"sources":[],"first_seen":null,"last_confirmed":null,"status":"active","basis":"owner_confirmed","supersedes":null,"updated_at":"2030-01-01T00:00:00Z","source_fingerprint":"synthetic","source_fingerprints":[]}],"proposals":[],"events":[],"migration":{"version":1,"source_path":"memory/company.md","fingerprint":"synthetic","accepted_at":"2030-01-01T00:00:00Z"}});
        std::fs::create_dir_all(root.join("memory")).unwrap();
        std::fs::write(
            root.join("memory/records.json"),
            serde_json::to_vec(&memory).unwrap(),
        )
        .unwrap();
    }
    let original_teams = std::fs::read(a.join("teams.json")).unwrap();
    let original_memory = std::fs::read(a.join("memory/records.json")).unwrap();
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
    app_a.request(json!({"op":"meeting-field","field":"owner","value":"Synthetic meeting owner"}));
    app_b.request(json!({"op":"meeting-field","field":"due","value":"2030-04-05"}));
    app_a.request(json!({"op":"meeting-check","task":"task-a","checked":true}));
    app_b.request(json!({"op":"meeting-check","task":"task-b","checked":true}));
    app_a.request(json!({"op":"project-add","name":"Offline A"}));
    app_b.request(json!({"op":"project-add","name":"Offline B"}));
    app_a.request(json!({"op":"area-create","name":"Synthetic area A"}));
    app_b.request(json!({"op":"area-create","name":"Synthetic area B"}));
    app_a.request(json!({"op":"team-member","name":"Bailey"}));
    app_b.request(json!({"op":"team-member","name":"Casey"}));
    app_b.request(json!({"op":"team-alias"}));
    app_a.request(json!({"op":"people-side","name":"alex","side":"external"}));
    app_b.request(json!({"op":"people-side","name":"casey","side":"our_team"}));
    app_a.request(json!({"op":"profile-name","name":"Synthetic shared owner"}));
    app_a.request(json!({"op":"memory-edit","text":"Synthetic offline memory edit"}));
    app_b.request(json!({"op":"memory-confirm"}));
    app_b.request(json!({"op":"proposal-stage"}));
    app_a.request(json!({"op":"edit","id":shared,"fields":{"subtasks":[{"id":"synthetic-subtask","title":"Offline subtask title","done":false}]}}));
    app_b.request(json!({"op":"edit","id":shared,"fields":{"subtasks":[{"id":"synthetic-subtask","title":"Initial subtask","done":true}]}}));
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
    for (app, model, sidebar) in [
        (&mut app_a, "local-model-a", false),
        (&mut app_b, "local-model-b", true),
    ] {
        let value = wait_shared(app, |v| {
            meeting(v).is_some_and(|t| {
                t["owner"] == "Synthetic meeting owner" && t["due"] == "2030-04-05"
            }) && v["teams"]["teams"][0]["members"]
                .as_array()
                .is_some_and(|a| a.len() == 3)
                && v["teams"]["teams"][0]["aliases"]["A."] == "Alex"
                && v["people"]["alex"] == "external"
                && v["people"]["casey"] == "our_team"
                && v["name"] == "Synthetic shared owner"
                && v["memory"]["records"][0]["text"] == "Synthetic offline memory edit"
                && v["memory"]["records"][0]["last_confirmed"].is_string()
                && v["memory"]["proposals"][0]["status"] == "pending"
                && v["checked"].as_array().is_some_and(|a| a.len() == 2)
                && v["projects"]["projects"]
                    .as_array()
                    .is_some_and(|projects| {
                        projects.contains(&json!("Offline A"))
                            && projects.contains(&json!("Offline B"))
                    })
                && v["projects"].as_object().is_some_and(|projects| {
                    projects
                        .keys()
                        .filter(|key| key.starts_with("area:"))
                        .count()
                        == 2
                })
        });
        assert!(
            value["model"] == model && value["sidebar"] == sidebar,
            "device-local choices crossed the relay"
        );
        assert!(
            value["projects"]["projects"]
                .as_array()
                .unwrap()
                .contains(&json!("Offline A"))
                && value["projects"]["projects"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("Offline B"))
        );
        assert_eq!(
            value["projects"]
                .as_object()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with("area:"))
                .count(),
            2
        );
        let tasks = wait_for(app, |t| {
            find(t, &shared).is_some_and(|t| {
                t["subtasks"][0]["title"] == "Offline subtask title"
                    && t["subtasks"][0]["done"] == true
            })
        });
        assert_eq!(
            find(&tasks, &shared).unwrap()["subtasks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            app.request(json!({"op":"conflicts"})),
            json!([]),
            "different fields produced conflicts"
        );
    }
    app_a.request(json!({"op":"offline"}));
    app_b.request(json!({"op":"offline"}));
    app_a.request(json!({"op":"edit","id":shared,"fields":{"owner":"Owner A"}}));
    app_b.request(json!({"op":"edit","id":shared,"fields":{"owner":"Owner B"}}));
    app_a.request(json!({"op":"memory-edit","text":"Synthetic Memory A"}));
    app_b.request(json!({"op":"memory-edit","text":"Synthetic Memory B"}));
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
    let conflicts = app_a.request(json!({"op":"conflicts"}));
    let memory = conflicts
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["domain"] == "memory" && c["field"].as_str().is_some_and(|p| p.contains("o:text"))
        })
        .expect("Memory text versions");
    let choice = memory["choices"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["value"] == "Synthetic Memory A")
        .unwrap();
    app_a.request(
        json!({"op":"resolve","id":memory["task_id"],"field":memory["field"],"choice":choice}),
    );
    for app in [&mut app_a, &mut app_b] {
        wait_shared(app, |v| {
            v["memory"]["records"][0]["text"] == "Synthetic Memory A"
        });
    }
    assert_eq!(app_a.request(json!({"op":"conflicts"})), json!([]));
    app_a.request(json!({"op":"offline"}));
    app_b.request(json!({"op":"offline"}));
    app_a.request(json!({"op":"proposal-decision","accept":true}));
    app_b.request(json!({"op":"proposal-decision","accept":false}));
    app_a.request(json!({"op":"online"}));
    app_b.request(json!({"op":"online"}));
    let deadline = Instant::now() + Duration::from_secs(30);
    let decision = loop {
        let a_versions = app_a.request(json!({"op":"conflicts"}));
        let b_versions = app_b.request(json!({"op":"conflicts"}));
        let find_decision = |v: &Value| {
            v.as_array()
                .unwrap()
                .iter()
                .find(|c| {
                    c["domain"] == "memory"
                        && c["field"]
                            .as_str()
                            .is_some_and(|p| p.contains("o:proposals") && p.contains("o:status"))
                })
                .cloned()
        };
        if let (Some(a), Some(_)) = (find_decision(&a_versions), find_decision(&b_versions)) {
            break a;
        }
        assert!(
            Instant::now() < deadline,
            "Memory decision conflict did not arrive"
        );
        thread::sleep(Duration::from_millis(50));
    };
    assert!(
        app_a.request(json!({"op":"memory-readable"})) == false
            && app_b.request(json!({"op":"memory-readable"})) == false,
        "conflicting decisions became usable Memory"
    );
    let rejected = decision["choices"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["value"] == "rejected")
        .unwrap();
    app_a.request(json!({"op":"resolve","id":decision["task_id"],"field":decision["field"],"choice":rejected}));
    for app in [&mut app_a, &mut app_b] {
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.request(json!({"op":"memory-readable"})) != true {
            assert!(Instant::now() < deadline, "decision choice did not arrive");
            thread::sleep(Duration::from_millis(50));
        }
        wait_shared(app, |v| {
            v["memory"]["proposals"][0]["status"] == "rejected"
                && v["memory"]["records"]
                    .as_array()
                    .is_some_and(|r| r.len() == 1)
        });
    }
    app_a.request(json!({"op":"merge-open"}));
    app_b.request(json!({"op":"meeting-field","field":"project","value":"Research"}));
    wait_shared(&mut app_a, |v| {
        meeting(v).is_some_and(|t| t["project"] == "Research")
    });
    app_a.rejected(json!({"op":"merge-commit"}));
    app_a.request(json!({"op":"merge-open"}));
    let receipt = app_a.request(json!({"op":"merge-commit"}));
    wait_shared(&mut app_b, |v| {
        v["projects"]["merges"]["Research"]["id"] == receipt
    });
    app_b.rejected(json!({"op":"merge-undo","id":receipt}));
    app_a.request(json!({"op":"merge-undo","id":receipt}));
    wait_shared(&mut app_b, |v| {
        v["projects"]["merges"].get("Research").is_none()
            && v["projects"]["aliases"].get("Research").is_none()
    });
    app_a.rejected(json!({"op":"remove","name":"Travel laptop"}));
    let after_remove =
        app_a.request(json!({"op":"add","task":{"task":"Synthetic retained relay collection"}}));
    wait_for(&mut app_b, |tasks| find(tasks, &after_remove).is_some());
    assert_eq!(journals(&a), original_a);
    assert_eq!(journals(&b), original_b);
    app_a.request(json!({"op":"disable"}));
    let folder_tasks = app_a.request(json!({"op":"list"}));
    assert_eq!(folder_tasks.as_array().unwrap().len(), 1);
    assert_eq!(folder_tasks[0]["id"], shared);
    assert_eq!(journals(&a), original_a, "rollback changed folder bytes");
    assert_eq!(std::fs::read(a.join("teams.json")).unwrap(), original_teams);
    assert_eq!(
        std::fs::read(a.join("memory/records.json")).unwrap(),
        original_memory
    );
    let rolled = app_a.request(json!({"op":"shared-state"}));
    assert!(rolled["name"] == "Synthetic owner" && rolled["people"]["alex"] == "our_team");
    assert!(rolled["memory"]["records"][0]["text"] == "Synthetic initial memory");
    app_a.request(json!({"op":"enable"}));
    assert!(
        find(&app_a.request(json!({"op":"list"})), &after_remove).is_some(),
        "rollback discarded relay data"
    );
    let resumed = app_a.request(json!({"op":"shared-state"}));
    assert!(
        resumed["name"] == "Synthetic shared owner"
            && resumed["memory"]["records"][0]["text"] == "Synthetic Memory A"
    );
    assert_eq!(
        resumed["teams"]["teams"][0]["members"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    app_a.stop();
    app_b.stop();
}

#[test]
fn newer_group_refuses_older_writer_across_offline_restart_and_downgrade() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    let a = fixture.path().join("older");
    let b = fixture.path().join("newer");
    let (_relay, address) = start_relay(&fixture.path().join("relay"));
    for (root, name, id) in [
        (&a, "Synthetic older PC", "c"),
        (&b, "Synthetic newer PC", "d"),
    ] {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("device.json"),
            serde_json::to_vec(&json!({"name":name,"id":id.repeat(32),"daily_check":false}))
                .unwrap(),
        )
        .unwrap();
        relay_preview::Settings {
            enabled: true,
            address: address.clone(),
            ..Default::default()
        }
        .save(root)
        .unwrap();
    }
    let caps = b.join("local/relay/test-capabilities.json");
    let release = std::env::var_os("SIDENOTE_RELEASE_PROOF_V1")
        .zip(std::env::var_os("SIDENOTE_RELEASE_PROOF_V2"));
    if release.is_none() {
        std::fs::write(&caps, br#"{"schema":2,"features":2}"#).unwrap();
    }
    let old_binary = release
        .as_ref()
        .map(|(a, _)| PathBuf::from(a))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_sidenote")));
    let new_binary = release
        .as_ref()
        .map(|(_, b)| PathBuf::from(b))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_sidenote")));
    let mut older = App::start_binary(&a, &old_binary);
    let mut newer = App::start_binary(&b, &new_binary);
    if release.is_some() {
        let av = older.request(json!({"op":"binary-version"}));
        let bv = newer.request(json!({"op":"binary-version"}));
        assert_eq!(av["debug"], false);
        assert_eq!(bv["debug"], false);
        assert_eq!(av["capabilities"]["features"], 1);
        assert_eq!(bv["capabilities"]["features"], 2);
        assert_eq!(av["version"], bv["version"]);
    }
    let health = older.request(json!({"op":"check-relay"}));
    assert!(
        health["message"]
            .as_str()
            .unwrap()
            .starts_with("Relay reachable.")
    );
    assert!(health["last_contact"].is_string());
    older.request(json!({"op":"host"}));
    status(&mut older, "code");
    newer.request(json!({"op":"join","from":a}));
    status(&mut older, "confirm");
    status(&mut newer, "confirm");
    older.request(json!({"op":"confirm","accept":true}));
    newer.request(json!({"op":"confirm","accept":true}));
    status(&mut older, "paired");
    status(&mut newer, "paired");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = older.request(json!({"op":"relay-state"}));
        if state["read_only"] == true {
            assert_eq!(state["message"], relay_preview::operations::UPDATE_REQUIRED);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "older instance did not refuse newer capabilities"
        );
        thread::sleep(Duration::from_millis(70));
    }
    older.rejected(json!({"op":"host"}));
    let authority = relay_preview::Settings::load(&a).unwrap().room;
    older.rejected(json!({"op":"remove","name":"Synthetic newer PC"}));
    assert!(
        relay_preview::Settings::load(&a).unwrap().room == authority,
        "incompatible removal reset the group"
    );
    older.rejected(json!({"op":"add","task":{"task":"Must not commit"}}));
    older.request(json!({"op":"offline"}));
    older.rejected(json!({"op":"add","task":{"task":"Must not commit offline"}}));
    let before = journals(&a);
    older.request(json!({"op":"folder-note"}));
    assert_ne!(journals(&a), before, "folder-only writes stopped");
    assert_eq!(
        older.request(json!({"op":"local-settings"})),
        json!({"sidebar":true,"model":"synthetic-local-model"})
    );
    older.stop();
    let mut older = App::start_binary(&a, &old_binary);
    assert_eq!(
        older.request(json!({"op":"relay-state"}))["read_only"],
        true
    );
    assert_eq!(
        older.request(json!({"op":"local-settings"})),
        json!({"sidebar":true,"model":"synthetic-local-model"})
    );
    older.rejected(json!({"op":"add","task":{"task":"Must not commit after restart"}}));
    assert!(
        older
            .request(json!({"op":"list"}))
            .as_array()
            .unwrap()
            .is_empty()
    );
    let future = newer.request(json!({"op":"add","task":{"task":"Future group keeps working"}}));
    newer.stop();
    if release.is_none() {
        std::fs::write(&caps, br#"{"schema":1,"features":1}"#).unwrap();
    }
    let mut downgraded = App::start_binary(&b, &old_binary);
    assert_eq!(
        downgraded.request(json!({"op":"relay-state"}))["read_only"],
        true
    );
    downgraded.rejected(json!({"op":"add","task":{"task":"Downgrade must not commit"}}));
    downgraded.stop();
    older.stop();
    if release.is_some() {
        let mut upgraded = App::start_binary(&a, &new_binary);
        let mut peer = App::start_binary(&b, &new_binary);
        wait_for(&mut upgraded, |t| find(t, &future).is_some());
        assert_eq!(
            upgraded.request(json!({"op":"relay-state"}))["read_only"],
            false
        );
        let id = upgraded
            .request(json!({"op":"add","task":{"task":"Synthetic after real binary upgrade"}}));
        wait_for(&mut peer, |tasks| find(tasks, &id).is_some());
        upgraded.stop();
        peer.stop();
    }
}

fn synthetic_root(root: &Path, name: &str, id: &str, address: &str) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("device.json"),
        serde_json::to_vec(&json!({"name":name,"id":id.repeat(32),"daily_check":false})).unwrap(),
    )
    .unwrap();
    relay_preview::Settings {
        enabled: true,
        address: address.into(),
        ..Default::default()
    }
    .save(root)
    .unwrap();
}
fn pair_apps(owner: &mut App, guest: &mut App, owner_root: &Path) {
    owner.request(json!({"op":"host"}));
    status(owner, "code");
    guest.request(json!({"op":"join","from":owner_root}));
    status(owner, "confirm");
    status(guest, "confirm");
    owner.request(json!({"op":"confirm","accept":true}));
    guest.request(json!({"op":"confirm","accept":true}));
    status(owner, "paired");
    status(guest, "paired");
}
#[test]
fn migration_recovers_after_process_kill_at_durable_boundaries() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    for point in ["migration-backup", "migration-import", "shared-import"] {
        let root = fixture.path().join(point);
        std::fs::create_dir_all(root.join("local/relay")).unwrap();
        std::fs::write(
            root.join("device.json"),
            serde_json::to_vec(
                &json!({"name":"Synthetic migration PC","id":"e".repeat(32),"daily_check":false}),
            )
            .unwrap(),
        )
        .unwrap();
        let original = serde_json::to_vec(
            &json!({"manual":[{"id":"synthetic-migrated-id","task":"Synthetic migration source"}]}),
        )
        .unwrap();
        std::fs::write(root.join("task-overrides.json"), &original).unwrap();
        std::fs::write(
            root.join("local/relay/settings.json"),
            br#"{"enabled":true}"#,
        )
        .unwrap();
        let control = root.join("local/relay/qualification-control.json");
        std::fs::write(
            &control,
            serde_json::to_vec(&json!({"pause":point})).unwrap(),
        )
        .unwrap();
        let crashed = App::spawn(&root, Path::new(env!("CARGO_BIN_EXE_sidenote")));
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("local/relay/qualification-state.json").exists() {
            assert!(
                Instant::now() < deadline,
                "migration did not reach crash boundary"
            );
            thread::sleep(Duration::from_millis(30));
        }
        drop(crashed);
        std::fs::remove_file(control).unwrap();
        let mut recovered = App::start(&root);
        let tasks = recovered.request(json!({"op":"list"}));
        assert_eq!(tasks.as_array().unwrap().len(), 1);
        assert_eq!(tasks[0]["id"], "synthetic-migrated-id");
        recovered.request(json!({"op":"disable"}));
        assert_eq!(
            std::fs::read(root.join("task-overrides.json")).unwrap(),
            original
        );
        recovered.request(json!({"op":"enable","enabled":true}));
        assert_eq!(
            recovered
                .request(json!({"op":"list"}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        recovered.stop();
    }
}
#[test]
fn three_peers_refuse_unavailable_removal_without_claiming_revocation() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    let a = fixture.path().join("a");
    let b = fixture.path().join("b");
    let c = fixture.path().join("c");
    let (_relay, address) = start_relay(&fixture.path().join("relay"));
    for (root, name, id) in [
        (&a, "Synthetic A", "a"),
        (&b, "Synthetic B", "b"),
        (&c, "Synthetic C", "c"),
    ] {
        synthetic_root(root, name, id, &address);
    }
    let (mut aa, mut bb, mut cc) = (App::start(&a), App::start(&b), App::start(&c));
    pair_apps(&mut aa, &mut bb, &a);
    pair_apps(&mut aa, &mut cc, &a);
    let initial =
        aa.request(json!({"op":"add","task":{"task":"Synthetic shared before rotation"}}));
    for app in [&mut bb, &mut cc] {
        wait_for(app, |t| find(t, &initial).is_some());
    }
    let old_room = relay_preview::Settings::load(&a).unwrap().room;
    aa.rejected(json!({"op":"remove","name":"Synthetic C"}));
    assert_eq!(relay_preview::Settings::load(&a).unwrap().room, old_room);
    let after = cc.request(
        json!({"op":"add","task":{"task":"Synthetic existing group remains explicitly shared"}}),
    );
    for app in [&mut aa, &mut bb] {
        wait_for(app, |tasks| find(tasks, &after).is_some());
    }
    aa.stop();
    bb.stop();
    cc.stop();
}

fn sync_control(address: &str, root: &Path, action: &str) -> Value {
    let room = relay_preview::Settings::load(root).unwrap().room;
    let url = format!(
        "{}/__test/sync-{action}/{room}",
        address.replacen("ws://", "http://", 1)
    );
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(3)))
        .build()
        .new_agent();
    let response = agent
        .post(&url)
        .send_empty()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    serde_json::from_str(&response).unwrap()
}
#[test]
fn killed_sender_recovers_after_relay_observed_an_interrupted_exchange() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    let a = fixture.path().join("a");
    let b = fixture.path().join("b");
    let (_relay, address) = start_relay(&fixture.path().join("relay"));
    synthetic_root(&a, "Synthetic crash sender", "a", &address);
    synthetic_root(&b, "Synthetic crash receiver", "b", &address);
    let (mut aa, mut bb) = (App::start(&a), App::start(&b));
    pair_apps(&mut aa, &mut bb, &a);
    let first = aa.request(json!({"op":"add","task":{"task":"Synthetic before crash"}}));
    wait_for(&mut bb, |t| find(t, &first).is_some());
    sync_control(&address, &a, "pause");
    let mut committed = Vec::new();
    for i in 0..12 {
        let mut rng = (i as u32 + 1).wrapping_mul(0x9e3779b9);
        committed.push(aa.request(json!({"op":"add","task":{"task":format!("Synthetic durable edit {i}: {}",(0..8000).map(|_|{rng^=rng<<13;rng^=rng>>17;rng^=rng<<5;(b'a'+(rng%26) as u8) as char}).collect::<String>())}})));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while sync_control(&address, &a, "status")["observed"] != 1 {
        assert!(
            Instant::now() < deadline,
            "no active exchange reached the relay"
        );
        thread::sleep(Duration::from_millis(30));
    }
    let pending = bb.request(json!({"op":"list"}));
    assert!(
        committed.iter().any(|id| find(&pending, id).is_none()),
        "exchange was already complete before kill"
    );
    drop(aa);
    sync_control(&address, &a, "resume");
    let mut restarted = App::start(&a);
    let local = restarted.request(json!({"op":"list"}));
    for id in &committed {
        assert!(
            find(&local, id).is_some(),
            "acknowledged local commit lost after kill"
        );
    }
    wait_for(&mut bb, |t| {
        committed.iter().all(|id| find(t, id).is_some())
    });
    let after =
        bb.request(json!({"op":"add","task":{"task":"Synthetic honest edit after recovery"}}));
    wait_for(&mut restarted, |t| find(t, &after).is_some());
    restarted.stop();
    bb.stop();
}
