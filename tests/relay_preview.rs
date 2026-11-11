//! Real headless app processes, isolated stores, real local Worker and encrypted sessions.
use serde_json::{Value, json};
use sidenote::core::{ManualTask, TaskStore, relay_preview};
use sidenote_sync::{client::Device, identity::Group};
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

fn provision(a: &Path, b: &Path, address: &str) {
    for root in [a, b] {
        std::fs::create_dir_all(root.join("local/relay")).unwrap();
    }
    let a_device = Device::open(&a.join("local/relay/replica.sqlite")).unwrap();
    let b_device = Device::open(&b.join("local/relay/replica.sqlite")).unwrap();
    let mut group = Group::new(&a_device.identity);
    group.enroll(&a_device.identity, b_device.identity.public());
    a_device.merge_group(&group).unwrap();
    b_device.merge_group(&group).unwrap();
    for root in [a, b] {
        relay_preview::Settings {
            enabled: true,
            address: address.into(),
            room: group.room(),
        }
        .save(root)
        .unwrap();
    }
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

#[test]
fn two_isolated_apps_relay_offline_fields_restart_and_folder_rollback() {
    let parent = std::env::temp_dir().join("sidenote-relay-preview");
    std::fs::create_dir_all(&parent).unwrap();
    let fixture = tempfile::tempdir_in(parent).unwrap();
    let a = fixture.path().join("a");
    let b = fixture.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    let folder = TaskStore { root: a.clone() };
    let (_, folder_id) = folder
        .add_manual(ManualTask {
            task: "Synthetic folder task".into(),
            ..Default::default()
        })
        .unwrap();
    let original_journals = journals(&a);
    assert!(!relay_preview::Settings::load(&a).unwrap().enabled);
    let (_relay, address) = start_relay(&fixture.path().join("relay"));
    provision(&a, &b, &address);
    let mut app_a = App::start(&a);
    let mut app_b = App::start(&b);
    assert_eq!(app_a.request(json!({"op":"list"})), json!([]));
    let id = app_a
        .request(json!({"op":"add","task":{"task":"Synthetic relay task","added":"2030-01-01"}}));
    wait_for(&mut app_b, |tasks| tasks[0]["id"] == id);
    app_a.request(json!({"op":"offline"}));
    app_b.request(json!({"op":"offline"}));
    app_a.request(json!({"op":"edit","id":id,"fields":{"owner":"Synthetic owner"}}));
    app_b.request(json!({"op":"edit","id":id,"fields":{"due":"2030-02-03"}}));
    let a_offline = app_a.request(json!({"op":"list"}));
    let b_offline = app_b.request(json!({"op":"list"}));
    assert_eq!(a_offline[0]["owner"], "Synthetic owner");
    assert!(a_offline[0]["due"].is_null());
    assert!(b_offline[0]["owner"].is_null());
    assert_eq!(b_offline[0]["due"], "2030-02-03");
    app_a.stop();
    app_b.stop();
    let mut app_a = App::start(&a);
    let mut app_b = App::start(&b);
    for app in [&mut app_a, &mut app_b] {
        wait_for(app, |tasks| {
            tasks[0]["owner"] == "Synthetic owner" && tasks[0]["due"] == "2030-02-03"
        });
    }
    app_a.request(json!({"op":"delete","id":id}));
    wait_for(&mut app_b, |tasks| tasks == &json!([]));
    app_a.stop();
    app_b.stop();
    assert_eq!(
        journals(&a),
        original_journals,
        "preview must not publish folder journals"
    );
    let mut settings = relay_preview::Settings::load(&a).unwrap();
    settings.enabled = false;
    settings.save(&a).unwrap();
    assert_eq!(folder.load_overrides().unwrap().manual[0].id, folder_id);
    assert_eq!(
        journals(&a),
        original_journals,
        "rollback must preserve folder journals"
    );
}
