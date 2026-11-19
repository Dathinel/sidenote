use super::*;
use serde_json::{Value, json};
use sidenote::core::{
    model_catalog,
    speech_sessions::{PieceRequest, SessionRequest, digest},
};
pub(super) fn native_setup(root: &std::path::Path) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (id, source) in [
        ("silero-vad", "target/l1-fixtures"),
        ("indicconformer-hi-ctc", "target/l4-fixtures"),
    ] {
        let model = model_catalog::model(id).unwrap();
        let dir = root.join("models").join(id);
        fs::create_dir_all(&dir).unwrap();
        for file in model.files {
            let name = if id == "silero-vad" {
                "silero_vad_catalogue.onnx"
            } else {
                file.name
            };
            let bytes = fs::read(repo.join(source).join(name)).unwrap();
            assert_eq!(digest(&bytes), file.sha256);
            assert_eq!(bytes.len() as u64, file.bytes);
            fs::write(dir.join(file.name), bytes).unwrap();
        }
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
    }
    let mp3 = repo.join("target/l4-fixtures/common_voice_hi_26988540.mp3");
    assert_eq!(
        digest(&fs::read(&mp3).unwrap()),
        "5f461e5f6ad6e5d7dfd9fd735e1a9f48b6365fc6bed0037d671ff970451d9dcc"
    );
    let mut command =
        sidenote_sync::tree_worker::speech_command(std::ffi::OsStr::new("/usr/bin/ffmpeg"))
            .unwrap();
    command
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(mp3)
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(root.join("public.wav"))
        .env_clear();
    let mut child = sidenote_sync::tree_worker::spawn_speech(&mut command).unwrap();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}
fn json_body(response: &str) -> Value {
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}
pub(super) fn native_piece(address: &str, auth: &str, root: &std::path::Path) {
    let request = SessionRequest {
        client_session: "native".into(),
        meeting_id: "public".into(),
        languages: vec!["hi".into()],
        language_mode: "auto".into(),
        config_sha256: "a".repeat(64),
    };
    let body = serde_json::to_vec(&request).unwrap();
    let (status, response) = send(
        address,
        address,
        "POST",
        "/v1/sessions",
        &format!("{auth}Content-Length: {}\r\n", body.len()),
        &body,
    );
    assert_eq!(status, 201);
    let session = json_body(&response)["session"].as_str().unwrap().to_owned();
    let wav = fs::read(root.join("public.wav")).unwrap();
    let samples = hound::WavReader::new(std::io::Cursor::new(&wav))
        .unwrap()
        .duration();
    let metadata = PieceRequest {
        track: "system".into(),
        index: 0,
        origin_sample: -12,
        start_sample: 0,
        end_sample: samples.into(),
        context_start_sample: 0,
        sha256: digest(&wav),
    };
    let piece = metadata.id(&request);
    let path = format!("/v1/sessions/{session}/pieces/{piece}");
    assert_eq!(
        send(
            address,
            address,
            "PUT",
            &path,
            &format!(
                "{auth}Content-Type: audio/wav\r\nX-Piece: {}\r\nContent-Length: {}\r\n",
                serde_json::to_string(&metadata).unwrap(),
                wav.len()
            ),
            &wav
        )
        .0,
        201
    );
    let started = Instant::now();
    let snapshot = loop {
        let (status, response) = send(address, address, "GET", &path, auth, &[]);
        assert_eq!(status, 200);
        let v = json_body(&response);
        if v["state"] == "done" {
            break v;
        }
        assert_ne!(v["state"], "failed");
        assert!(started.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    };
    let ranges = snapshot["ranges"].as_array().unwrap();
    assert!(!ranges.is_empty());
    assert!(ranges.iter().all(|r|r["text"].as_str().is_some_and(|s|!s.trim().is_empty()) && r["route"]=="hi"));
    let bytes = fs::read(
        root.join("local/speech")
            .join(&session)
            .join("events.ndjson"),
    )
    .unwrap();
    let events: Vec<Value> = bytes
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert_eq!(
        ranges,
        &events
            .iter()
            .filter(|e| e["event"] == "range.result")
            .map(|e| e["data"].clone())
            .collect::<Vec<_>>()
    );
    for kind in [
        "piece.accepted",
        "host.status",
        "piece.progress",
        "range.result",
        "piece.done",
    ] {
        assert!(events.iter().any(|e| e["event"] == kind));
    }
    for after in [0, events.len() as u64 / 2] {
        let expected: Vec<_> = events
            .iter()
            .filter(|e| e["seq"].as_u64().unwrap() > after)
            .cloned()
            .collect();
        assert_eq!(
            sse(address, auth, &session, after, expected.len()),
            expected
        );
    }
    assert_eq!(send(address, address, "GET", &path, "", &[]).0, 401);
    assert_eq!(
        send(
            address,
            address,
            "GET",
            &format!("/v1/sessions/{session}/events"),
            &format!("{auth}Last-Event-ID: bad\r\n"),
            &[]
        )
        .0,
        422
    );
    println!("native ranges={}", ranges.len());
}
fn sse(address: &str, auth: &str, session: &str, after: u64, count: usize) -> Vec<Value> {
    use std::io::BufRead;
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(stream,"GET /v1/sessions/{session}/events HTTP/1.1\r\nHost: {address}\r\n{auth}Last-Event-ID: {after}\r\nConnection: close\r\n\r\n").unwrap();
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("200"));
    let mut events = Vec::new();
    let mut seq = 0;
    let mut event = String::new();
    while events.len() < count {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        let line = line.trim();
        if let Some(v) = line.strip_prefix("id: ") {
            seq = v.parse::<u64>().unwrap();
        }
        if let Some(v) = line.strip_prefix("event: ") {
            event = v.into();
        }
        if let Some(v) = line.strip_prefix("data: ") {
            events.push(
                json!({"seq":seq,"event":event,"data":serde_json::from_str::<Value>(v).unwrap()}),
            );
        }
    }
    events
}
