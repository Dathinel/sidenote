use super::*;
use std::{fs, io::Read, net::TcpListener, thread};

fn server(
    reply: impl AsRef<str> + Send + 'static,
) -> (tempfile::TempDir, std::thread::JoinHandle<()>) {
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
        socket.write_all(reply.as_ref().as_bytes()).unwrap();
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

#[test]
fn oversized_lines_and_silent_peers_disconnect() {
    let (root, task) = server(format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n{}",
        "x".repeat(1025)
    ));
    let mut connection = Connection::connect(&root.path().join("local-api.json")).unwrap();
    task.join().unwrap();
    assert_eq!(connection.poll(), Err(State::InvalidInput));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut connection = Connection {
        stream,
        pending: Vec::new(),
        last_read: Instant::now() - Duration::from_secs(46),
    };
    assert_eq!(connection.poll(), Err(State::Offline));
}

#[test]
fn opt_out_never_connects_and_control_is_explicit() {
    use super::super::{Session, folders::Discovery};
    let (root, meetings, cursor) = super::super::tests::fixture();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let discovery = root.path().join("local-api.json");
    fs::write(
        &discovery,
        serde_json::json!({"port":listener.local_addr().unwrap().port(),"token":"synthetic"})
            .to_string(),
    )
    .unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = Events::new(Discovery::new(meetings.clone()).unwrap(), discovery, false).unwrap();
    let mut session = Session::open(source, cursor).unwrap();
    session.drain(|_| panic!("opted-out delivery")).unwrap();
    assert_eq!(session.source().state(), State::OptedOut);
    assert_eq!(session.source().control_state(), ControlState::Unsupported);
    fs::rename(&meetings, root.path().join("hidden")).unwrap();
    session.drain(|_| panic!("opted-out folder read")).unwrap();
    assert_eq!(
        listener.accept().err().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    session.stop();
    assert_eq!(session.source().state(), State::Stopped);
    assert_eq!(
        session.drain(|_| panic!("stopped")),
        Err(super::super::Error::Stopped)
    );
}

#[test]
fn subscribe_then_catch_up_deduplicates_reconnect_and_restart() {
    use super::super::{Session, folders::Discovery};
    use std::sync::mpsc::channel;
    let (root, meetings, cursor) = super::super::tests::fixture();
    let arriving = meetings.join("99-1");
    fs::remove_dir_all(&arriving).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let discovery = root.path().join("local-api.json");
    fs::write(
        &discovery,
        serde_json::json!({"port":listener.local_addr().unwrap().port(),"token":"synthetic"})
            .to_string(),
    )
    .unwrap();
    let (subscribed, subscription) = channel();
    let (publish, publication) = channel();
    let (written, completion) = channel();
    let (close, closed) = channel();
    let folder = arriving.clone();
    let task = thread::spawn(move || {
        for generation in 0..2 {
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
            assert!(request.starts_with(b"GET /meetings/events HTTP/1.1\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                .unwrap();
            subscribed.send(generation).unwrap();
            if generation == 0 {
                publication.recv().unwrap();
                fs::create_dir(&folder).unwrap();
                fs::write(folder.join("manifest.json"), br#"{"status":"complete"}"#).unwrap();
                fs::write(
                    folder.join("transcript.ndjson"),
                    b"{\"text\":\"synthetic gap\"}\n",
                )
                .unwrap();
            }
            socket
                .write_all(b"data: {\"type\":\"meeting.finished\",\"id\":\"99-1\"}\n\n")
                .unwrap();
            written.send(()).unwrap();
            closed.recv().unwrap();
        }
    });
    let source = Events::new(
        Discovery::new(meetings.clone()).unwrap(),
        discovery.clone(),
        true,
    )
    .unwrap();
    let mut session = Session::open(source, cursor.clone()).unwrap();
    assert_eq!(session.source().state(), State::Connected);
    assert_eq!(
        subscription.recv_timeout(Duration::from_secs(5)).unwrap(),
        0
    );
    assert!(!arriving.exists());
    publish.send(()).unwrap();
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut queued = Vec::new();
    session
        .drain(|m| {
            queued.push(m.id().to_owned());
            Ok(())
        })
        .unwrap();
    assert_eq!(queued, ["100-1", "99-1"]);
    session.drain(|_| panic!("duplicate after gap")).unwrap();
    session.source.failed(State::Offline);
    close.send(()).unwrap();
    session.source.retry_at = Instant::now();
    session.drain(|_| panic!("duplicate at reconnect")).unwrap();
    assert_eq!(
        subscription.recv_timeout(Duration::from_secs(5)).unwrap(),
        1
    );
    completion.recv_timeout(Duration::from_secs(5)).unwrap();
    session.drain(|_| panic!("replayed event")).unwrap();
    assert_eq!(session.source().state(), State::Connected);
    assert_eq!(queued, ["100-1", "99-1"]);
    session.stop();
    close.send(()).unwrap();
    task.join().unwrap();
    assert_eq!(session.source().state(), State::Stopped);
    drop(session);
    let source = Events::new(Discovery::new(meetings).unwrap(), discovery, true).unwrap();
    Session::open(source, cursor)
        .unwrap()
        .drain(|_| panic!("duplicate after restart"))
        .unwrap();
}

#[test]
fn lab_disabled_or_busy_still_catches_up_and_offline_backoff_is_capped() {
    use super::super::{Session, Source, folders::Discovery};
    for (reply, state) in [
        ("HTTP/1.1 404 Not Found\r\n\r\nlabs-off", State::Disabled),
        ("HTTP/1.1 409 Conflict\r\n\r\nsubscriber-busy", State::Busy),
    ] {
        let (_root, meetings, cursor) = super::super::tests::fixture();
        let (api, task) = server(reply);
        let source = Events::new(
            Discovery::new(meetings).unwrap(),
            api.path().join("local-api.json"),
            true,
        )
        .unwrap();
        let mut session = Session::open(source, cursor).unwrap();
        assert_eq!(session.source().state(), state);
        task.join().unwrap();
        session.source.retry_at = Instant::now() + Duration::from_secs(3600);
        let mut count = 0;
        session
            .drain(|_| {
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(session.source().state(), state);
    }
    let (root, meetings, _) = super::super::tests::fixture();
    let mut source = Events::new(
        Discovery::new(meetings).unwrap(),
        root.path().join("missing.json"),
        true,
    )
    .unwrap();
    for seconds in [2, 4, 8, 16, 30, 30] {
        source.retry_at = Instant::now();
        assert_eq!(source.poll().unwrap().len(), 2);
        assert_eq!(source.state(), State::Offline);
        assert_eq!(source.backoff, Duration::from_secs(seconds));
        let retry = Instant::now() + Duration::from_secs(3600);
        source.retry_at = retry;
        source.poll().unwrap();
        assert_eq!(source.retry_at, retry);
    }
}
