use super::*;
use std::{fs, io::Read, net::TcpListener, thread};

fn server(reply: &'static str) -> (tempfile::TempDir, std::thread::JoinHandle<()>) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    fs::write(
        root.path().join("local-api.json"),
        serde_json::json!({
            "port": listener.local_addr().unwrap().port(), "token": "synthetic-fixture",
            "apiVersion": "1", "pid": 1
        })
        .to_string(),
    )
    .unwrap();
    let task = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        // Assertions intentionally never print request/discovery bytes.
        assert!(request.starts_with(b"GET /meetings/events HTTP/1.1\r\n"));
        assert!(
            request
                .windows(41)
                .any(|s| s == b"Authorization: Bearer synthetic-fixture\r\n")
        );
        socket.write_all(reply.as_bytes()).unwrap();
    });
    (root, task)
}

#[test]
fn authenticated_statuses_are_typed_without_response_details() {
    for (reply, state) in [
        ("HTTP/1.1 404 Not Found\r\n\r\nlabs-off", State::Disabled),
        ("HTTP/1.1 409 Conflict\r\n\r\nsubscriber-busy", State::Busy),
        (
            "HTTP/1.1 401 Unauthorized\r\n\r\nprivate",
            State::Unauthorized,
        ),
        (
            "HTTP/1.1 302 Found\r\nLocation: https://example.com\r\n\r\n",
            State::InvalidInput,
        ),
    ] {
        let (root, task) = server(reply);
        assert!(Connection::connect(&root.path().join("local-api.json")).err() == Some(state));
        task.join().unwrap();
    }
}

#[test]
fn fragmented_stream_accepts_only_finished_ids_and_keepalives() {
    for valid in [
        b": keep-alive\n".as_slice(),
        b"\n",
        b"data: {\"type\":\"meeting.finished\",\"id\":\"100-1\"}\n",
    ] {
        assert_eq!(Connection::line(valid), Ok(()));
    }
    for invalid in [
        b"data: {\"type\":\"meeting.started\",\"id\":\"100-1\"}\n".as_slice(),
        b"data: {\"type\":\"meeting.finished\",\"id\":\"../1\"}\n",
        b"data: {\"type\":\"meeting.finished\",\"id\":\"100-1\",\"title\":\"private\"}\n",
    ] {
        assert_eq!(Connection::line(invalid), Err(State::InvalidInput));
    }
    let (root, task) = server(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {\"type\":\"meeting.finished\",\"id\":\"100-1\"}\n\n",
    );
    let mut connection = Connection::connect(&root.path().join("local-api.json")).unwrap();
    task.join().unwrap();
    assert_eq!(connection.poll(), Err(State::Offline));
    assert!(connection.pending.is_empty());
}

#[test]
fn missing_or_invalid_discovery_is_redacted() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("local-api.json");
    assert!(Connection::connect(&path).err() == Some(State::Offline));
    for document in [
        r#"{"port":1,"token":"bad\r\nheader"}"#.to_owned(),
        "x".repeat(4097),
    ] {
        fs::write(&path, document).unwrap();
        assert!(Connection::connect(&path).err() == Some(State::InvalidInput));
    }
}
