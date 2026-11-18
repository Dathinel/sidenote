use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn request(address: &str, host: &str, headers: &str) -> (u16, String) {
    send(address, host, "GET", "/v1/host", headers, &[])
}
fn send(
    address: &str,
    host: &str,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{headers}Connection: close\r\n\r\n"
    )
    .unwrap();
    stream.write_all(body).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    (
        response.split_whitespace().nth(1).unwrap().parse().unwrap(),
        response,
    )
}
#[test]
fn server_role_exposes_only_scoped_host_probe() {
    let root = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_sidenote");
    let token_path = root.path().join("token");
    let cli = |action: &str, scope: bool| {
        let mut cmd = Command::new(binary);
        cmd.arg("--data-dir")
            .arg(root.path())
            .args(["--mcp-access", action, "synthetic"]);
        if action == "grant" {
            cmd.arg(&token_path);
        }
        if scope {
            cmd.arg("speech");
        }
        assert!(cmd.stdout(Stdio::null()).status().unwrap().success());
    };
    cli("grant", false);
    let mcp = zeroize::Zeroizing::new(fs::read_to_string(&token_path).unwrap());
    fs::remove_file(&token_path).unwrap();
    cli("grant", true);
    let token = zeroize::Zeroizing::new(fs::read_to_string(&token_path).unwrap());
    assert!(sidenote::mcp::access::authenticate(root.path(), &token).is_err());
    let mut command = Command::new(binary);
    command
        .arg("--data-dir")
        .arg(root.path())
        .arg("--meetings-dir")
        .arg(root.path().join("meetings"))
        .arg("--peers-dir")
        .arg(root.path().join("peers"))
        .args([
            "--server",
            "--device-role",
            "server",
            "--speech-port",
            "0",
            "--speech-host",
            "synthetic.tail-test.ts.net",
        ]);
    #[cfg(feature = "asr-runtime")]
    let library = std::env::var_os("SIDENOTE_L4_RUNTIME").expect("public pinned runtime fixture");
    #[cfg(feature = "asr-runtime")]
    command.arg("--runtime-library").arg(&library);
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let endpoint = root.path().join("local/speech/endpoint.json");
    let start = Instant::now();
    while !endpoint.is_file() {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "Server exited before speech bind"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "speech listener connection refused"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let endpoint: serde_json::Value = serde_json::from_slice(&fs::read(endpoint).unwrap()).unwrap();
    let address = endpoint["address"].as_str().unwrap();
    let auth = zeroize::Zeroizing::new(format!("Authorization: Bearer {}\r\n", token.as_str()));
    let auth = auth.as_str();
    let check = |host: &str, headers: &str, status| {
        assert_eq!(request(address, host, headers).0, status);
    };
    check(address, "", 401);
    check("wrong.ts.net", auth, 403);
    check("127.0.0.1:1", auth, 403);
    let mcp_auth = format!("Authorization: Bearer {}\r\n", mcp.as_str());
    check(address, &mcp_auth, 401);
    check(address, &format!("{auth}Origin: null\r\n"), 403);
    check(address, &format!("{auth}{auth}"), 401);
    check(address, &format!("{auth}Content-Length: 10000001\r\n"), 413);
    let (status, response) = request(address, "synthetic.tail-test.ts.net", auth);
    assert_eq!(status, 200);
    let host: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    println!("host={host}");
    #[cfg(feature = "asr-runtime")]
    {
        sidenote::core::asr::initialize(std::path::Path::new(&library)).unwrap();
        assert_eq!(host["asr"]["state"], "ready");
        assert_eq!(
            host["asr"]["runtime_version"],
            sidenote::core::asr::runtime_version().unwrap()
        );
    }
    #[cfg(not(feature = "asr-runtime"))]
    assert_eq!(host["asr"]["state"], "unavailable");
    assert_eq!(host["speech_jobs"], false);
    let (session, piece, metadata, wav) = exercise_sessions(address, auth, root.path(), binary);
    drop(server);
    fs::remove_file(root.path().join("local/speech/endpoint.json")).unwrap();
    server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    let endpoint = root.path().join("local/speech/endpoint.json");
    while !endpoint.is_file() {
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    let endpoint: serde_json::Value = serde_json::from_slice(&fs::read(endpoint).unwrap()).unwrap();
    let address = endpoint["address"].as_str().unwrap();
    let headers = format!(
        "{auth}Content-Type: audio/wav\r\nX-Piece: {metadata}\r\nContent-Length: {}\r\n",
        wav.len()
    );
    assert_eq!(
        send(
            address,
            address,
            "PUT",
            &format!("/v1/sessions/{session}/pieces/{piece}"),
            &headers,
            &wav
        )
        .0,
        200
    );
    assert_eq!(
        fs::read(
            root.path()
                .join(format!("local/speech/{session}/{piece}.wav"))
        )
        .unwrap(),
        wav
    );
    let finish = format!("/v1/sessions/{session}/finish");
    let finish_headers = format!("{auth}Content-Type: application/json\r\nContent-Length: 18\r\n");
    assert_eq!(
        send(
            address,
            address,
            "POST",
            &finish,
            &finish_headers,
            br#"{"pieces_total":2}"#
        )
        .0,
        200
    );
    cli("revoke", true);
    assert_eq!(request(address, address, auth).0, 401);
    assert!(sidenote::mcp::access::authenticate(root.path(), &mcp).is_ok());
    drop(server);
    assert!(TcpStream::connect(address).is_err());
}

fn exercise_sessions(
    address: &str,
    auth: &str,
    root: &std::path::Path,
    binary: &str,
) -> (String, String, String, Vec<u8>) {
    use sidenote::core::speech_sessions::{PieceRequest, SessionRequest, digest};
    let request = SessionRequest {
        client_session: "synthetic-l6".into(),
        meeting_id: "public-wav".into(),
        languages: vec!["hi".into()],
        language_mode: "auto".into(),
        config_sha256: "a".repeat(64),
    };
    let body = serde_json::to_vec(&request).unwrap();
    let create = |token: &str, expected| {
        let headers = format!(
            "{token}Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        let (status, response) = send(address, address, "POST", "/v1/sessions", &headers, &body);
        assert_eq!(status, expected);
        serde_json::from_str::<serde_json::Value>(response.split_once("\r\n\r\n").unwrap().1)
            .unwrap()["session"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let session = create(auth, 201);
    assert_eq!(create(auth, 200), session);
    assert_eq!(
        send(address, "wrong.ts.net", "POST", "/v1/sessions", "", &[]).0,
        403
    );
    assert_eq!(
        send(address, address, "POST", "/v1/sessions", "", &[]).0,
        401
    );
    let mut wav = b"RIFF".to_vec();
    wav.extend(38u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend([1, 0, 1, 0]);
    wav.extend(16_000u32.to_le_bytes());
    wav.extend(32_000u32.to_le_bytes());
    wav.extend([2, 0, 16, 0]);
    wav.extend(b"data");
    wav.extend(2u32.to_le_bytes());
    wav.extend([0, 0]);
    let metadata = PieceRequest {
        track: "system".into(),
        index: 0,
        origin_sample: 0,
        start_sample: 0,
        end_sample: 1,
        context_start_sample: 0,
        sha256: digest(&wav),
    };
    let piece = metadata.id(&request);
    let upload = |token: &str, session: &str, m: &PieceRequest, audio: &[u8], expected| {
        let path = format!("/v1/sessions/{session}/pieces/{}", m.id(&request));
        let headers = format!(
            "{token}Content-Type: audio/wav\r\nX-Piece: {}\r\nContent-Length: {}\r\n",
            serde_json::to_string(m).unwrap(),
            audio.len()
        );
        let result = send(address, address, "PUT", &path, &headers, audio);
        assert_eq!(result.0, expected);
        result.1
    };
    upload(auth, &session, &metadata, &wav, 201);
    upload(auth, &session, &metadata, &wav, 200);
    let mut bad = wav.clone();
    bad[44] ^= 1;
    upload(auth, &session, &metadata, &bad, 422);
    let mut bad_header = wav.clone();
    bad_header[22] = 2;
    let mut changed = metadata.clone();
    changed.sha256 = digest(&bad_header);
    upload(auth, &session, &changed, &bad_header, 422);
    let path = format!("/v1/sessions/{session}/pieces/{piece}");
    assert_eq!(
        send(
            address,
            address,
            "PUT",
            &path,
            &format!("{auth}Content-Length: 11000000\r\n"),
            &[]
        )
        .0,
        413
    );
    let mut large = TcpStream::connect(address).unwrap();
    large
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(large, "PUT {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", 11_000_000).unwrap();
    let _ = large.write_all(&vec![0; 11_000_000]);
    let _ = large.write_all(b"\r\n0\r\n\r\n");
    let mut rejected = String::new();
    let _ = large.read_to_string(&mut rejected);
    assert_eq!(rejected.split_whitespace().nth(1), Some("413"));
    let mut next = metadata.clone();
    next.index = 1;
    upload(auth, &session, &next, &wav, 201);
    next.index = 2;
    let full = upload(auth, &session, &next, &wav, 429);
    assert!(full.to_ascii_lowercase().contains("retry-after: 1"));
    assert!(full.contains("\"queue_pos\":3"));
    for label in ["second", "third"] {
        let token_path = root.join(format!("{label}-token"));
        assert!(
            Command::new(binary)
                .arg("--data-dir")
                .arg(root)
                .args(["--mcp-access", "grant", label])
                .arg(&token_path)
                .arg("speech")
                .stdout(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let token = zeroize::Zeroizing::new(fs::read_to_string(&token_path).unwrap());
        let auth2 =
            zeroize::Zeroizing::new(format!("Authorization: Bearer {}\r\n", token.as_str()));
        let session2 = create(&auth2, 201);
        upload(&auth2, &session, &metadata, &wav, 404);
        if label == "second" {
            upload(&auth2, &session2, &metadata, &wav, 201);
            next.index = 1;
            upload(&auth2, &session2, &next, &wav, 201);
        } else {
            let full = upload(&auth2, &session2, &metadata, &wav, 429);
            assert!(full.contains("\"queue_pos\":5"));
        }
    }
    upload(auth, &session, &metadata, &wav, 200);
    (
        session,
        piece,
        serde_json::to_string(&metadata).unwrap(),
        wav,
    )
}
