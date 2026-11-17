use super::*;
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const LIVE: &str = r#"{"status":"settings","settings":[{"key":"meetings.audio_retention","value":null,"kind":"choice","choices":["day","week","month","forever"],"writable":true,"about":"How long finished meetings keep their audio while their transcripts, notes, summary, and Markdown stay; none follows meetings.retention","available":true}]}"#;

const WORKER_TIMEOUT: Duration = Duration::from_secs(5);
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const READY: &str = "audio-test-ready";

#[test]
fn keep_audio_contracts_through_hex_settings_api() {
    if std::env::var_os("SIDENOTE_AUDIO_TEST").is_none() {
        let root = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "hex_engine::audio_retention::tests::keep_audio_contracts_through_hex_settings_api",
                "--nocapture",
            ])
            .stdout(std::process::Stdio::piped())
            .env("SIDENOTE_AUDIO_TEST", "1")
            .env("SIDENOTE_DATA_DIR", root.path().join("data"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().expect("spawn isolated Keep audio test");
        let output = child.stdout.take().unwrap();
        let (ready, started) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let line = line.unwrap();
                eprintln!("{line}");
                if line == READY {
                    let _ = ready.send(());
                }
            }
        });
        let mut deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut fixture_ready = false;
        let status = loop {
            if let Some(status) = child.try_wait().expect("check isolated Keep audio child") {
                break status;
            }
            if Instant::now() >= deadline {
                child
                    .kill()
                    .expect("stalled Keep audio child could not be killed");
                child.wait().expect("reap stalled Keep audio child");
                reader.join().unwrap();
                panic!("isolated Keep audio child exceeded its startup or 30s body deadline");
            }
            match started.recv_timeout(Duration::from_millis(20)) {
                Ok(()) => {
                    if !fixture_ready {
                        fixture_ready = true;
                        deadline = Instant::now() + CHILD_TIMEOUT;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(Duration::from_millis(2));
                }
            }
        };
        reader.join().unwrap();
        assert!(
            status.success(),
            "isolated Keep audio test failed: {status}"
        );
        return;
    }
    let dir = hex_dir().unwrap();
    assert!(dir.starts_with(crate::test_home()));
    fs::create_dir_all(&dir).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    fs::write(
        dir.join("local-api.json"),
        json!({"port":address.port(),"token":"synthetic"}).to_string(),
    )
    .unwrap();
    let data = Arc::new(Mutex::new((
        serde_json::from_str::<Value>(LIVE).unwrap(),
        Vec::<Value>::new(),
        false,
    )));
    let shutdown = Arc::new(AtomicBool::new(false));
    let shared = data.clone();
    let stopping = shutdown.clone();
    let (closed, eof) = mpsc::channel::<()>();
    let (listening, ready) = mpsc::channel();
    let worker = thread::spawn(move || {
        listening.send(()).unwrap();
        'connections: while !stopping.load(Ordering::Relaxed) {
            let Ok((stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(2));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut length = 0;
            // Other local clients (e.g. a host port prober) are not Sidenote calls: 404 and move on.
            let mut request_line = String::new();
            loop {
                let mut line = String::new();
                let read = reader.read_line(&mut line);
                let ours = request_line.starts_with("POST /control ");
                if !ours && read.is_err() {
                    continue 'connections;
                }
                if read.unwrap() == 0 {
                    if ours {
                        let _ = closed.send(());
                    }
                    continue 'connections;
                }
                if request_line.is_empty() {
                    request_line = line.clone();
                }
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':')
                    && key.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            if request_line != "POST /control HTTP/1.1\r\n" {
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                continue;
            }
            assert!(length < 4096);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let mut data = shared.lock().unwrap();
            let mut reply = match request["command"].as_str().unwrap() {
                "status" => {
                    json!({"status":"status","app_version":"test","model":{},"meeting":{"state":"idle"},"capture":{"state":"idle"},"capabilities":{}})
                }
                "settings-get" => data.0.clone(),
                "settings-set" => {
                    data.1.push(request.clone());
                    if data.2 {
                        json!({"status":"error","code":"invalid-value"})
                    } else {
                        let setting = data.0["settings"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|s| s["key"] == request["key"])
                            .unwrap();
                        setting["value"] = match request["value"].as_str().unwrap() {
                            "none" => Value::Null,
                            "true" => json!(true),
                            "false" => json!(false),
                            value => json!(value),
                        };
                        json!({"status":"done"})
                    }
                }
                other => panic!("unexpected command {other}"),
            };
            reply["version"] = json!(PROTOCOL);
            let reply = reply.to_string();
            write!(
                reader.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            )
            .unwrap();
        }
    });
    ready
        .recv_timeout(STARTUP_TIMEOUT)
        .expect("fake Hex server did not start");
    println!("{READY}");
    let engine = HexEngine::new();
    {
        let mut cache = engine.cache.lock().unwrap();
        cache.version = "test".into();
        cache.capabilities = Some(Capabilities::from([
            (Feature::SettingsList, Support::Supported),
            (Feature::SettingsSet, Support::Supported),
        ]));
    }
    for (value, lab, expected, writes) in [
        (
            Value::Null,
            None,
            Some("Same as meetings"),
            vec![("meetings.audio_retention", "week")],
        ),
        (
            json!("day"),
            None,
            Some("24 hours"),
            vec![("meetings.audio_retention", "forever")],
        ),
        (
            json!("month"),
            Some(true),
            Some("1 month"),
            vec![
                ("meetings.audio_retention", "week"),
                ("labs.expire_meeting_audio", "true"),
            ],
        ),
        (
            json!("day"),
            Some(false),
            Some("Forever"),
            vec![
                ("meetings.audio_retention", "forever"),
                ("labs.expire_meeting_audio", "false"),
            ],
        ),
    ] {
        let mut reply: Value = serde_json::from_str(LIVE).unwrap();
        reply["settings"][0]["value"] = value;
        if let Some(lab) = lab {
            reply["settings"].as_array_mut().unwrap().push(json!({"key":"labs.expire_meeting_audio","value":lab,"kind":"bool","writable":true,"available":true}));
        }
        *data.lock().unwrap() = (reply, vec![], false);
        let audio = AudioRetention::from_settings(&engine.settings_list().unwrap());
        assert_eq!(audio.current, expected);
        assert!(audio.editable);
        assert_eq!(audio.choices().contains(&"Same as meetings"), lab.is_none());
        audio.set(&engine, writes[0].1).unwrap();
        let actual: Vec<_> = data
            .lock()
            .unwrap()
            .1
            .iter()
            .map(|r| {
                (
                    r["key"].as_str().unwrap().to_owned(),
                    r["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            actual,
            writes
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        );
        if lab.is_none() {
            audio.set(&engine, "none").unwrap();
            let refreshed = AudioRetention::from_settings(&engine.settings_list().unwrap());
            assert_eq!(refreshed.current, Some("Same as meetings"));
            assert!(refreshed.follows_meetings);
            assert!(
                data.lock()
                    .unwrap()
                    .1
                    .iter()
                    .all(|r| r["key"] == "meetings.audio_retention")
            );
        }
    }
    *data.lock().unwrap() = (json!({"status":"settings","settings":[]}), vec![], false);
    let unknown = AudioRetention::from_settings(&engine.settings_list().unwrap());
    assert_eq!(unknown.current, None);
    assert!(!unknown.editable);
    assert_eq!(unknown.set(&engine, "day"), Err(HexError::Unsupported));
    assert!(data.lock().unwrap().1.is_empty());
    *data.lock().unwrap() = (serde_json::from_str(LIVE).unwrap(), vec![], true);
    let audio = AudioRetention::from_settings(&engine.settings_list().unwrap());
    assert!(
        matches!(audio.set(&engine, "day"), Err(HexError::Failed { code: Some(code), .. }) if code == "invalid-value")
    );
    assert_eq!(data.lock().unwrap().1.len(), 1);
    let started = Instant::now();
    let deadline = started + WORKER_TIMEOUT;
    let mut probe = TcpStream::connect_timeout(&address, Duration::from_secs(1))
        .expect("connect forced-EOF probe");
    probe
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    probe
        .write_all(b"POST /control HTTP/1.1\r\nHost: forced-eof\r\n")
        .unwrap();
    probe.shutdown(std::net::Shutdown::Write).unwrap();
    eof.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("fake Hex header reader did not close EOF within 5s");
    drop(probe);
    shutdown.store(true, Ordering::Relaxed);
    let (joined, finished) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = joined.send(worker.join());
    });
    finished
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("fake Hex worker join exceeded the shared 5s deadline")
        .expect("fake Hex worker panicked");
    assert!(
        started.elapsed() < WORKER_TIMEOUT,
        "forced-EOF probe exceeded 5s"
    );
    println!(
        "forced EOF and worker shutdown: {:?}, bound {:?}",
        started.elapsed(),
        WORKER_TIMEOUT
    );
}
