use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    os::windows::process::CommandExt,
    process::Command,
    thread,
};

const TOKEN: &[u8] = b"synthetic-speech-host-test-value";
const REPLY: &str = r#"{"api":1,"asr":{"languages":[],"models":[],"reason":null,"runtime_version":"1.28.2","state":"ready"},"limits":{"max_piece_bytes":10000000},"role":"server","speech_jobs":false}"#;
const WAIT: Duration = Duration::from_secs(3);

fn contains_token(bytes: &[u8]) -> bool {
    bytes.windows(TOKEN.len()).any(|window| window == TOKEN)
}

fn fake_probe(host: &Host, status: u16, body: &str) -> Option<Asr> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let start = Instant::now();
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock && start.elapsed() < WAIT => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    _ => panic!("Fake host did not accept a probe"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(WAIT)).unwrap();
            socket.set_write_timeout(Some(WAIT)).unwrap();
            let mut input = BufReader::new(&mut socket);
            let mut line = String::new();
            input.read_line(&mut line).unwrap();
            assert!(line.starts_with("GET /v1/host HTTP/"));
            let mut authorization = 0;
            let mut right_host = false;
            loop {
                line.clear();
                assert!(input.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.trim_end().split_once(": ").unwrap();
                if name.eq_ignore_ascii_case("authorization") {
                    assert!(
                        value
                            .strip_prefix("Bearer ")
                            .is_some_and(|v| v.as_bytes() == TOKEN)
                    );
                    authorization += 1;
                }
                if name.eq_ignore_ascii_case("host") {
                    right_host = value == address.to_string();
                }
                assert!(!name.eq_ignore_ascii_case("origin"));
            }
            assert!(right_host && authorization == 1);
            write!(
                socket,
                "HTTP/1.1 {status} Fake\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let loopback = Host {
            endpoint: url::Url::parse(&format!("http://{address}/v1/host")).unwrap(),
            token: host.token.clone(),
        };
        let result = loopback.probe();
        server.join().unwrap();
        result
    })
}

fn consented(path: &Path) -> Host {
    Host::save_and_consent(path, "https://host.example", TOKEN).unwrap()
}

#[test]
fn rejects_http_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.dpapi");
    let result = Host::save_and_consent(&path, "http://localhost", TOKEN);
    assert_eq!(
        result.err().as_deref(),
        Some("The speech host URL must use HTTPS.")
    );
    assert!(!path.exists());
}

#[test]
fn token_stays_out_of_written_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    store_io::write(&config, &serde_json::json!({"version": 1})).unwrap();
    let before = fs::read(&config).unwrap();
    let path = dir.path().join("host.dpapi");
    consented(&path);
    let after = fs::read(&config).unwrap();
    assert!(!contains_token(&after), "config contained a token");
    assert!(before == after, "speech host changed config.json");
    assert!(
        !contains_token(&fs::read(&path).unwrap()),
        "protected file contained plaintext"
    );
    let loaded = Host::load(&path).unwrap().unwrap();
    assert_eq!(loaded.recipient(), "https://host.example");
    assert!(loaded.token.as_slice() == TOKEN);
}

#[test]
fn token_stays_out_of_captured_logs() {
    const CHILD: &str = "SIDENOTE_SPEECH_TEST_CAPTURE";
    if std::env::var_os(CHILD).is_some() {
        let dir = tempfile::tempdir().unwrap();
        let host = consented(&dir.path().join("host.dpapi"));
        let reflected = std::str::from_utf8(TOKEN).unwrap();
        assert!(fake_probe(&host, 401, reflected).is_none());
        assert!(fake_probe(&host, 200, reflected).is_none());
        eprintln!("speech host capture verified");
        return;
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "core::speech_host::tests::token_stays_out_of_captured_logs",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        !contains_token(&child.stdout) && !contains_token(&child.stderr),
        "probe logs contained a token"
    );
    assert!(child.status.success(), "probe log capture child failed");
    assert!(String::from_utf8_lossy(&child.stderr).contains("speech host capture verified"));
}

fn observe(responses: Vec<(u16, &'static str)>, expected: &[Status]) {
    let dir = tempfile::tempdir().unwrap();
    let host = consented(&dir.path().join("host.dpapi"));
    let mut responses = responses.into_iter();
    let monitor = Monitor::start(
        Duration::from_millis(10),
        move || {
            responses
                .next()
                .and_then(|(status, reply)| fake_probe(&host, status, reply))
        },
        || {},
    )
    .unwrap();
    for state in expected {
        assert_eq!(monitor.updates.recv_timeout(WAIT), Ok(*state));
    }
}

#[test]
fn first_miss_is_amber() {
    observe(vec![(503, "")], &[Status::Amber]);
}

#[test]
fn second_miss_is_away() {
    observe(vec![(503, ""), (200, "{}")], &[Status::Amber, Status::Away]);
}

#[test]
fn ready_after_recovery() {
    observe(
        vec![(503, ""), (403, ""), (200, REPLY)],
        &[Status::Amber, Status::Away, Status::Ready],
    );
}

#[test]
fn clearing_stops_probes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.dpapi");
    let host = consented(&path);
    let (alive, stopped) = mpsc::channel::<()>();
    let mut monitor = Some(
        Monitor::start(
            INTERVAL,
            move || fake_probe(&host, 200, REPLY),
            move || {
                let _keep_until_worker_exits = &alive;
            },
        )
        .unwrap(),
    );
    assert_eq!(
        monitor.as_ref().unwrap().updates.recv_timeout(WAIT),
        Ok(Status::Ready)
    );
    clear(&path, &mut monitor).unwrap();
    assert_eq!(
        stopped.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    assert!(Host::load(&path).unwrap().is_none());
}
