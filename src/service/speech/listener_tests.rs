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
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET /v1/host HTTP/1.1\r\nHost: {host}\r\n{headers}Connection: close\r\n\r\n"
    )
    .unwrap();
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
    cli("revoke", true);
    assert_eq!(request(address, address, auth).0, 401);
    assert!(sidenote::mcp::access::authenticate(root.path(), &mcp).is_ok());
    drop(server);
    assert!(TcpStream::connect(address).is_err());
}
