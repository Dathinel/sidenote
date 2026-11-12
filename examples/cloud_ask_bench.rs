//! Synthetic loopback benchmark; no real keys, cloud requests, installed state or UI.
//! cargo run --locked --example cloud_ask_bench -- 20
use sidenote::core::{
    self, AskProgress,
    provider::{self, Settings},
};
use std::{
    cell::RefCell,
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runs = std::env::args()
        .nth(1)
        .map(|n| n.parse::<usize>())
        .transpose()?
        .unwrap_or(20)
        .clamp(1, 100);
    let state = tempfile::tempdir()?;
    core::configure_paths(&["--data-dir".into(), state.path().as_os_str().into()])?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let settings = Settings {
        enabled: true,
        base_url: format!("http://{}/v1", listener.local_addr()?),
        model: "synthetic-mock".into(),
        consent: None,
    };
    settings.save()?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        for attempt in 0..runs {
            let (mut socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(10)))?;
            // Tiny synthetic fragments must not wait on Nagle/delayed ACKs.
            socket.set_nodelay(true)?;
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer)?;
                if n == 0 {
                    return Err(std::io::Error::other("Mock request ended early"));
                }
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..end]);
                    assert!(header.starts_with("POST /v1/chat/completions HTTP/1.1"));
                    if cfg!(windows) && attempt >= runs / 2 {
                        assert!(
                            header
                                .to_ascii_lowercase()
                                .contains("authorization: bearer synthetic-benchmark-only-key")
                        );
                    } else {
                        assert!(!header.to_ascii_lowercase().contains("authorization:"));
                    }
                    let size = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + size {
                        let body: serde_json::Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + size])?;
                        assert_eq!(body["model"], "synthetic-mock");
                        assert_eq!(body["stream"], true);
                        break;
                    }
                }
                assert!(request.len() < 64 * 1024);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
            std::thread::sleep(Duration::from_millis(25));
            for text in [
                "data: {\"choices\":[{\"delta\":{\"content\":\"Synthetic answer\"}}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"total_tokens\":14}}\n\n",
                "data: [DONE]\n\n",
            ] {
                // Split HTTP chunks inside SSE framing to exercise the real adapter.
                for chunk in text.as_bytes().chunks(7) {
                    write!(socket, "{:x}\r\n", chunk.len())?;
                    socket.write_all(chunk)?;
                    socket.write_all(b"\r\n")?;
                    socket.flush()?;
                }
            }
            socket.write_all(b"0\r\n\r\n")?;
        }
        Ok(())
    });
    let mut samples = Vec::new();
    for attempt in 0..runs {
        if cfg!(windows) && attempt == runs / 2 {
            provider::save_key(b"synthetic-benchmark-only-key")?;
            assert!(provider::key_available());
            let local = state.path().join("local/cloud-ask");
            let settings_bytes = std::fs::read(local.join("settings.json"))?;
            assert!(
                !String::from_utf8_lossy(&settings_bytes).contains("synthetic-benchmark-only-key")
            );
            for entry in std::fs::read_dir(local)? {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "dpapi") {
                    let encrypted = std::fs::read(path)?;
                    assert!(
                        !encrypted
                            .windows(b"synthetic-benchmark-only-key".len())
                            .any(|v| v == b"synthetic-benchmark-only-key")
                    );
                }
            }
        }
        let metrics = RefCell::new(None);
        let first = RefCell::new(None);
        let start = Instant::now();
        let answer = provider::chat(
            settings.clone(),
            "Synthetic transcript: [00:42] We chose paper cards.\nMe: What did we choose?".into(),
            &|event| match event {
                AskProgress::Text { text } if !text.is_empty() => {
                    first
                        .borrow_mut()
                        .get_or_insert(start.elapsed().as_millis() as u64);
                }
                AskProgress::Metrics(value) => *metrics.borrow_mut() = Some(value),
                _ => {}
            },
        )
        .inspect_err(|_| {
            eprintln!(
                "mock: failed attempt {attempt}, elapsed {}ms",
                start.elapsed().as_millis()
            );
        })?;
        assert_eq!(answer, "Synthetic answer");
        assert_eq!(metrics.borrow().as_ref().unwrap().total_tokens, Some(14));
        samples.push(serde_json::json!({"first_visible_ms":first.into_inner().unwrap(),"total_ms":start.elapsed().as_millis()}));
    }
    server.join().map_err(|_| "Mock server panicked")??;
    if cfg!(windows) {
        provider::delete_key()?;
        assert!(!provider::key_available());
    }
    let mut first: Vec<_> = samples
        .iter()
        .map(|s| s["first_visible_ms"].as_u64().unwrap())
        .collect();
    first.sort_unstable();
    let percentile = |p: usize| first[(first.len() * p).div_ceil(100).saturating_sub(1)];
    println!(
        "{}",
        serde_json::json!({"kind":"synthetic-loopback-only","runs":runs,"mock_delay_ms":25,"dpapi_synthetic_key_verified":cfg!(windows),"first_visible_p50_ms":percentile(50),"first_visible_p95_ms":percentile(95),"samples":samples})
    );
    Ok(())
}
