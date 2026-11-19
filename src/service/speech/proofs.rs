//! Disposable real-service proofs. Only the model's recognizer is substituted.
use crate::core::{
    asr,
    meeting_router::batches::{self, Range, Recognizer},
    speech_sessions::{PieceRequest, SessionRequest, digest},
    store_io,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
const FIXTURE: &str = "service::speech::proofs::speech_service_fixture";
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(crate) fn active() -> bool {
    std::env::var_os("SIDENOTE_L7_ROOT").is_some()
}
pub(crate) fn recognizer() -> asr::Result<Box<dyn Recognizer>> {
    let mut command =
        sidenote_sync::tree_worker::speech_command(std::env::current_exe()?.as_os_str())?;
    command.args(["--exact", FIXTURE, "--nocapture"]);
    let worker = batches::worker::Worker::spawn(
        command,
        crate::core::speech_jobs::STOP.clone(),
        Duration::from_secs(30),
    )?;
    let mut pids = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::core::state_dir().join("workers"))?;
    writeln!(pids, "{}", worker.pid())?;
    Ok(Box::new(worker))
}
#[test]
fn speech_service_fixture() {
    if let Some(output) = std::env::var_os("SIDENOTE_BATCH_OUTPUT") {
        let output = PathBuf::from(output);
        for line in std::io::stdin().lock().lines() {
            match line.unwrap().as_str() {
                "start" => continue,
                "range" => {}
                _ => panic!("bad worker request"),
            }
            let range: Range =
                serde_json::from_slice(&fs::read(output.join("range.json")).unwrap()).unwrap();
            let root = range.wav.ancestors().nth(6).unwrap();
            let mut calls = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("calls"))
                .unwrap();
            writeln!(calls, "{}", digest(&serde_json::to_vec(&range).unwrap())).unwrap();
            if fs::read_to_string(root.join("calls"))
                .unwrap()
                .lines()
                .count()
                == 2
            {
                fs::write(root.join("blocked"), b"ready").unwrap();
                while root.join("hold").exists() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            store_io::write(
                &output.join("result.json"),
                &json!({"text":"synthetic","timing":"range"}),
            )
            .unwrap();
        }
        return;
    }
    let Some(root) = std::env::var_os("SIDENOTE_L7_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let args = vec![
        "--data-dir".into(),
        root.clone().into_os_string(),
        "--server".into(),
        "--device-role".into(),
        "server".into(),
    ];
    crate::core::configure_paths(&args).unwrap();
    crate::service::configure(&args).unwrap();
    if std::env::var_os("SIDENOTE_L7_REVOKE").is_some() {
        assert_eq!(
            crate::mcp::access::cli(&[
                "--mcp-access".into(),
                "revoke".into(),
                "proof".into(),
                "speech".into()
            ]),
            0
        );
        return;
    }
    if !root.join("token").exists() {
        assert_eq!(
            crate::mcp::access::cli(&[
                "--mcp-access".into(),
                "grant".into(),
                "proof".into(),
                root.join("token").into_os_string(),
                "speech".into()
            ]),
            0
        );
    }
    fs::write(root.join("pid"), std::process::id().to_string()).unwrap();
    assert_eq!(super::run(), 0);
}
struct Service {
    child: Child,
    root: PathBuf,
    address: String,
    auth: zeroize::Zeroizing<String>,
}
impl Service {
    fn start(root: &Path) -> Self {
        let endpoint = root.join("local/speech/endpoint.json");
        let _ = fs::remove_file(&endpoint);
        let log = fs::File::create(root.join("service.log")).unwrap();
        let child = Command::new("systemd-run")
            .args([
                "--user",
                "--wait",
                "--pipe",
                "--collect",
                "--unit=sidenote-speech-worker",
                "--property=MemoryMax=1G",
                "--property=TasksMax=64",
                "--property=CPUQuota=100%",
            ])
            .arg("/usr/bin/env")
            .arg(format!("SIDENOTE_L7_ROOT={}", root.display()))
            .arg(format!(
                "SIDENOTE_L4_RUNTIME={}",
                std::env::var("SIDENOTE_L4_RUNTIME").unwrap()
            ))
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--nocapture"])
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut service = Self {
            child,
            root: root.into(),
            address: String::new(),
            auth: zeroize::Zeroizing::new(String::new()),
        };
        wait(|| endpoint.is_file());
        let endpoint: Value = serde_json::from_slice(&fs::read(endpoint).unwrap()).unwrap();
        service.address = endpoint["address"].as_str().unwrap().into();
        service.auth = zeroize::Zeroizing::new(format!(
            "Authorization: Bearer {}\r\n",
            fs::read_to_string(root.join("token")).unwrap()
        ));
        service
    }
    fn stop(&mut self) {
        if self.child.try_wait().unwrap().is_some() {
            return;
        }
        let pid: i32 = fs::read_to_string(self.root.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
        wait(|| self.child.try_wait().unwrap().is_some());
        assert!(self.child.wait().unwrap().success());
    }
    fn send(&self, method: &str, path: &str, body: &[u8], extra: &str) -> (u16, Value) {
        let mut stream = self.stream(method, path, body, extra);
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        (
            status,
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap_or(Value::Null),
        )
    }
    fn stream(&self, method: &str, path: &str, body: &[u8], extra: &str) -> TcpStream {
        let mut stream = TcpStream::connect(&self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {}\r\n{}Content-Length: {}\r\n{extra}Connection: close\r\n\r\n",self.address,self.auth.as_str(),body.len()).unwrap();
        stream.write_all(body).unwrap();
        stream
    }
    fn upload(&self, request: &SessionRequest, index: u32) -> (String, String) {
        let (status, v) = self.send(
            "POST",
            "/v1/sessions",
            &serde_json::to_vec(request).unwrap(),
            "",
        );
        assert!(matches!(status, 200 | 201));
        let id = v["session"].as_str().unwrap().to_owned();
        let wav = fs::read(self.root.join("audio.wav")).unwrap();
        let m = PieceRequest {
            track: "system".into(),
            index,
            origin_sample: -12000,
            start_sample: 96000,
            end_sample: 256000,
            context_start_sample: 80000,
            sha256: digest(&wav),
        };
        let piece = m.id(request);
        assert_eq!(
            self.send(
                "PUT",
                &format!("/v1/sessions/{id}/pieces/{piece}"),
                &wav,
                &format!(
                    "Content-Type: audio/wav\r\nX-Piece: {}\r\n",
                    serde_json::to_string(&m).unwrap()
                )
            )
            .0,
            201
        );
        (id, piece)
    }
    #[track_caller]
    fn snapshot(&self, id: &str, piece: &str) -> Value {
        let (s, v) = self.send("GET", &format!("/v1/sessions/{id}/pieces/{piece}"), &[], "");
        assert_eq!(
            s,
            200,
            "{}",
            fs::read_to_string(self.root.join("service.log")).unwrap()
        );
        v
    }
    fn replay(&self, id: &str, after: u64, count: usize) -> Vec<Value> {
        let mut reader = std::io::BufReader::new(self.stream(
            "GET",
            &format!("/v1/sessions/{id}/events"),
            &[],
            &format!("Last-Event-ID: {after}\r\n"),
        ));
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("200"));
        let mut result = Vec::new();
        let mut seq = 0;
        let mut event = String::new();
        while result.len() < count {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if let Some(s) = line.trim().strip_prefix("id: ") {
                seq = s.parse::<u64>().unwrap();
            }
            if let Some(s) = line.trim().strip_prefix("event: ") {
                event = s.into();
            }
            if let Some(s) = line.trim().strip_prefix("data: ") {
                result.push(json!({"seq":seq,"event":event,"data":serde_json::from_str::<Value>(s).unwrap()}));
            }
        }
        result
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_some() {
            return;
        }
        if let Ok(pid) = fs::read_to_string(self.root.join("pid"))
            .and_then(|s| s.parse::<i32>().map_err(std::io::Error::other))
        {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            let start = Instant::now();
            while self.child.try_wait().unwrap().is_none()
                && start.elapsed() < Duration::from_secs(5)
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            if self.child.try_wait().unwrap().is_none() {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        let _ = self.child.wait();
    }
}
#[track_caller]
fn wait(mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < Duration::from_secs(5), "service deadline");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn setup() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let model = crate::core::model_catalog::model("silero-vad").unwrap();
    let dir = root.path().join("models/silero-vad");
    fs::create_dir_all(&dir).unwrap();
    let bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/l1-fixtures/silero_vad_catalogue.onnx"),
    )
    .unwrap();
    assert_eq!(digest(&bytes), model.files[0].sha256);
    fs::write(dir.join(model.files[0].name), bytes).unwrap();
    fs::write(
        dir.join("installed.json"),
        serde_json::to_vec(
            &model
                .files
                .iter()
                .map(|f| (f.name, f.sha256, f.bytes))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    let audio = crate::core::asr::fixtures::test_wav();
    asr::write_wav(&root.path().join("audio.wav"), &audio[..176000]).unwrap();
    root
}
fn request() -> SessionRequest {
    SessionRequest {
        client_session: "production-proof".into(),
        meeting_id: "public".into(),
        languages: vec!["hi".into()],
        language_mode: "auto".into(),
        config_sha256: "a".repeat(64),
    }
}
fn events(root: &Path, id: &str) -> Vec<Value> {
    fs::read_to_string(root.join(format!("local/speech/{id}/events.ndjson")))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
mod faults;
pub(crate) use faults::{Logits, crash};
mod locking;
pub(crate) use locking::{IntakeTrace, before_job_lock};
#[test]
fn service_shutdown_replans_and_replays_over_http() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    fs::write(root.join("hold"), b"pause").unwrap();
    let mut service = Service::start(root);
    let (id, piece) = service.upload(&request(), 0);
    wait(|| root.join("blocked").exists());
    let (_, second) = service.upload(&request(), 1);
    let before = service.snapshot(&id, &piece);
    assert_eq!(before["ranges"].as_array().unwrap().len(), 1);
    service.stop();
    for pid in fs::read_to_string(root.join("workers")).unwrap().lines() {
        assert!(
            !Path::new("/proc").join(pid).exists(),
            "owned worker was not reaped"
        );
    }
    let sessions: Value =
        serde_json::from_slice(&fs::read(root.join("local/speech/sessions.json")).unwrap())
            .unwrap();
    assert_eq!(sessions[&id]["pieces"][&piece]["done"], false);
    assert!(
        !root
            .join(format!("local/speech/{id}/jobs/{second}"))
            .exists()
    );
    assert!(
        !events(root, &id)
            .iter()
            .any(|e| e["event"] == "piece.failed")
    );
    let calls = fs::read_to_string(root.join("calls")).unwrap();
    let first = calls.lines().next().unwrap().to_owned();
    let job = root.join(format!("local/speech/{id}/jobs/{piece}"));
    fs::write(job.join("input/.input-crash.tmp"), b"interrupted").unwrap();
    fs::write(job.join(".tmp-derived-cut"), b"interrupted").unwrap();
    fs::remove_file(root.join("hold")).unwrap();
    let mut service = Service::start(root);
    wait(|| service.snapshot(&id, &second)["state"] == "done");
    let resumed = service.snapshot(&id, &piece);
    assert_eq!(resumed["state"], "done");
    assert!(!job.join("input/.input-crash.tmp").exists());
    assert!(!job.join(".tmp-derived-cut").exists());
    assert_eq!(
        fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .filter(|s| *s == first)
            .count(),
        1
    );
    let saved = events(root, &id);
    let after = saved.len() as u64 / 2;
    let later: Vec<_> = saved
        .iter()
        .filter(|e| e["seq"].as_u64().unwrap() > after)
        .cloned()
        .collect();
    assert_eq!(service.replay(&id, after, later.len()), later);
    assert_eq!(
        resumed["ranges"],
        json!(
            saved
                .iter()
                .filter(|e| e["event"] == "range.result" && e["data"]["piece"] == piece)
                .map(|e| e["data"].clone())
                .collect::<Vec<_>>()
        )
    );
    assert!(
        resumed["ranges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["start_sample"].as_u64().unwrap() >= 96000
                && r["end_sample"].as_u64().unwrap() <= 256000)
    );
    service.stop();
    fs::remove_dir_all(root.join("local/speech")).unwrap();
    fs::remove_file(root.join("calls")).unwrap();
    let service = Service::start(root);
    let (same_id, same_piece) = service.upload(&request(), 0);
    assert_eq!((&same_id, &same_piece), (&id, &piece));
    wait(|| service.snapshot(&id, &piece)["state"] == "done");
    assert_eq!(
        service.snapshot(&id, &piece),
        resumed,
        "uninterrupted production plan and snapshot"
    );
}
