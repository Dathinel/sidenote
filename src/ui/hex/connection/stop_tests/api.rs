use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

pub(super) struct Data {
    pub id: Option<&'static str>,
    pub delay: Duration,
    pub stops: usize,
}
pub(super) struct Fixture {
    pub data: Arc<Mutex<Data>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Fixture {
    pub fn start() -> (Self, State, Instant) {
        let mut state = State::new();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let data = Arc::new(Mutex::new(Data {
            id: Some("100-1"),
            delay: Duration::ZERO,
            stops: 0,
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let shared = data.clone();
        let stopping = shutdown.clone();
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopping.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let shared = shared.clone();
                clients.push(thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                if length == 0 { return; }
                assert!(length < 4096);
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let request: Value = serde_json::from_slice(&body).unwrap();
                let reply = match request["command"].as_str().unwrap() {
                    "status" => {
                        let delay = shared.lock().unwrap().delay;
                        thread::sleep(delay);
                        let id = shared.lock().unwrap().id;
                        serde_json::json!({"version":4,"status":"status","app_version":"test",
                            "model":{},"capture":{"state":"idle"},"capabilities":{"meetings":true},
                            "features":{"meetings":true},
                            "meeting":{"state":if id.is_some() {"recording"} else {"idle"},"id":id,"elapsed_seconds":1}})
                    }
                    "meeting-stop" => {
                        let mut data = shared.lock().unwrap();
                        data.stops += 1;
                        data.id = None;
                        serde_json::json!({"version":4,"status":"done","changed":true})
                    }
                    other => panic!("unexpected fake API command: {other}"),
                }.to_string();
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
                }));
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        let dir = sidenote::core::meetings_dir()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(dir.starts_with(env!("CARGO_MANIFEST_DIR")));
        std::fs::create_dir_all(&dir).unwrap();
        // Synthetic discovery only, inside the test's isolated meetings parent.
        std::fs::write(
            dir.join("local-api.json"),
            serde_json::json!({"port":port,"token":"fixture"}).to_string(),
        )
        .unwrap();
        state.hex_apply(Msg::Caps(
            "test".into(),
            Ok(Capabilities::from([
                (Feature::RecordStart, Support::Supported),
                (Feature::RecordStop, Support::Supported),
            ])),
        ));
        let now = Instant::now() + Duration::from_secs(3600);
        let good = state.hex.engine.status_for_command().unwrap();
        state.hex_status_at(Ok(good), now);
        state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(2));
        (
            Self {
                data,
                shutdown,
                worker: Some(worker),
            },
            state,
            now,
        )
    }
    pub fn stop(state: &mut State) {
        state.hex_record_stop(recording_id(state));
        while state.hex.record_busy {
            let event = state
                .events_rx
                .recv_timeout(Duration::from_secs(15))
                .expect("Stop completion");
            if let Event::Hex(msg @ Msg::Done(_, _)) = event {
                state.hex_apply(msg);
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
