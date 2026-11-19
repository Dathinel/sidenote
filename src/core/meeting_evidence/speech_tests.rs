use super::*;
use crate::core::{model_catalog, speech_host::Host};
use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    thread,
};
const WAIT: Duration = Duration::from_secs(3);
const READY: &str = r#"{"api":1,"role":"server","speech_jobs":true,"asr":{"state":"ready","languages":["hi"],"models":[]}}"#;

enum Reply {
    Json(u16, String),
    Stream(String, bool),
    CutUpload,
}
use Reply::{CutUpload, Json, Stream};
fn response(body: Value, status: u16) -> Reply {
    Json(status, body.to_string())
}

struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    source: PathBuf,
    settings: PathBuf,
    collector: Collector,
    listener: TcpListener,
    session: String,
    piece: String,
    expected_bytes: Vec<u8>,
}

impl Fixture {
    fn new(consent: bool) -> Self {
        use crate::core::speech_host::upload::digest;
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let source = temp.path().join("hex");
        let meeting = source.join("100-1");
        let settings = data.join("host.dpapi");
        fs::create_dir_all(&meeting).unwrap();
        model_catalog::save_meeting_languages(&data, &["hi".to_owned()]).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let save = if consent {
            Host::save_with_upload_consent
        } else {
            Host::save_and_consent
        };
        save(&settings, &url, b"fixture-device").unwrap();
        super::tests::audio(&meeting.join("microphone.wav"), 3);
        store_io::write(
            &meeting.join("manifest.json"),
            &json!({
                "status":"complete","created_at_ms":chrono::Utc::now().timestamp_millis()+1,
                "microphone":{"first_arrival_us":1_000_000}
            }),
        )
        .unwrap();
        fs::write(meeting.join("transcript.ndjson"),
            concat!(r#"{"text":"English baseline","start_ms":1000,"end_ms":4000,"source":"microphone"}"#,
                "\n",r#"{"text":"English kept","start_ms":4000,"end_ms":5000,"source":"microphone"}"#,"\n")).unwrap();
        let collector = Collector::new(&data);
        collector.collect_with_limit(&meeting, true, true).unwrap();
        let (_, mut job) = jobs(&collector.root.join("100-1")).unwrap().swap_remove(0);
        let cut = collector.root.join("100-1/microphone-000000");
        // Set a deterministic clock relation without sleeping or relying on the host clock.
        job.queued_ms += 1;
        store_io::write(&cut.join("job.json"), &job).unwrap();
        let expected_bytes = fs::read(cut.join("audio.wav")).unwrap();
        let config = digest(&serde_json::to_vec(&(vec!["hi"], json!([]))).unwrap());
        let piece = digest(
            &serde_json::to_vec(&(
                "100-1",
                "microphone",
                0u32,
                0u64,
                48_000u64,
                digest(&expected_bytes),
                &config,
            ))
            .unwrap(),
        );
        Self {
            _temp: temp,
            data,
            source,
            settings,
            collector,
            listener,
            session: "a".repeat(64),
            piece,
            expected_bytes,
        }
    }

    fn run(&self, steps: Vec<(&str, Reply)>) -> io::Result<()> {
        thread::scope(|scope| {
            let (active, stopped) = std::sync::mpsc::channel::<()>();
            let server = scope.spawn(move || self.serve(steps, stopped));
            let host = Host::load(&self.settings).unwrap().unwrap();
            let result = upload_windows(
                &self.collector.root,
                &self.data,
                &self.settings,
                host,
                &self.source,
            );
            drop(active);
            server.join().unwrap();
            result
        })
    }

    fn serve(&self, steps: Vec<(&str, Reply)>, stopped: std::sync::mpsc::Receiver<()>) {
        for (expected, reply) in steps {
            let start = Instant::now();
            let (mut socket, _) = loop {
                match self.listener.accept() {
                    Ok(conn) => break conn,
                    Err(e)
                        if expected == "snapshot"
                            && e.kind() == io::ErrorKind::WouldBlock
                            && stopped.try_recv()
                                == Err(std::sync::mpsc::TryRecvError::Disconnected) =>
                    {
                        return;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock && start.elapsed() < WAIT => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    _ => panic!("Fake host did not receive {expected}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(WAIT)).unwrap();
            socket.set_write_timeout(Some(WAIT)).unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let (entry, _) = line.trim_end().rsplit_once(" HTTP/1.1").unwrap();
            match expected {
                "POST /v1/sessions" | "GET /v1/host" => assert_eq!(entry, expected),
                "PUT" => assert_eq!(
                    entry,
                    format!("PUT /v1/sessions/{}/pieces/{}", self.session, self.piece)
                ),
                "finish" => assert_eq!(entry, format!("POST /v1/sessions/{}/finish", self.session)),
                "events" => assert_eq!(entry, format!("GET /v1/sessions/{}/events", self.session)),
                "snapshot" => assert_eq!(
                    entry,
                    format!("GET /v1/sessions/{}/pieces/{}", self.session, self.piece)
                ),
                _ => unreachable!(),
            }
            let mut len = 0;
            let mut cursor = 0;
            let mut authorized = false;
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                let (header, value) = line.trim_end().split_once(": ").unwrap();
                if header.eq_ignore_ascii_case("content-length") {
                    len = value.parse().unwrap();
                } else if header.eq_ignore_ascii_case("last-event-id") {
                    cursor = value.parse().unwrap();
                } else if header.eq_ignore_ascii_case("authorization") {
                    assert!(
                        !authorized && value == "Bearer fixture-device",
                        "Unexpected authorization"
                    );
                    authorized = true;
                } else {
                    assert!(!header.eq_ignore_ascii_case("origin"));
                }
            }
            assert!(authorized);
            if matches!(reply, CutUpload) {
                assert_eq!(len, self.expected_bytes.len());
                reader.read_exact(&mut [0u8; 100]).unwrap();
                continue;
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            if expected == "PUT" {
                assert!(
                    body == self.expected_bytes,
                    "A retry changed collector WAV bytes"
                );
            }
            match reply {
                Json(status, body) => {
                    Self::reply(&mut socket, status, "application/json", body.len(), &body)
                }
                Stream(mut body, dropped) => {
                    // Resume replay is bounded by the link. Starting over loses the tail again.
                    if !dropped && cursor != 1 {
                        body = self.range_frame(1, 'b', 0, "first");
                        Self::reply(&mut socket, 200, "text/event-stream", 8192, &padded(body));
                    } else {
                        let len = if dropped { 8192 } else { body.len() };
                        if dropped {
                            body = padded(body);
                        }
                        Self::reply(&mut socket, 200, "text/event-stream", len, &body);
                    }
                }
                CutUpload => unreachable!(),
            }
        }
    }

    fn reply(socket: &mut impl Write, status: u16, kind: &str, len: usize, body: &str) {
        write!(socket, "HTTP/1.1 {status} Fake\r\nContent-Type: {kind}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}").unwrap();
    }

    fn create(&self, status: u16, next: u64) -> (&'static str, Reply) {
        (
            "POST /v1/sessions",
            response(json!({"session":self.session,"next_seq":next}), status),
        )
    }

    fn range(&self, id: char, start: u64, word: &str) -> Value {
        json!({"piece":self.piece,"range":id.to_string().repeat(64),"track":"microphone",
            "start_sample":start,"end_sample":start+16000,"language":"hi","route":"hi","lang_conf":null,
            "model_sha256":"f".repeat(64),"text":word,"timing":"range"})
    }

    fn range_frame(&self, seq: u64, id: char, start: u64, word: &str) -> String {
        format!(
            "id: {seq}\nevent: range.result\ndata: {}\n\n",
            self.range(id, start, word)
        )
    }

    fn start(&self, next: u64) -> Vec<(&'static str, Reply)> {
        vec![
            ("GET /v1/host", Json(200, READY.into())),
            self.create(200, next),
        ]
    }
    fn upload_and_interrupt(&self) {
        let mut first = self.start(0);
        first[1] = self.create(201, 0);
        first.extend([
            (
                "PUT",
                response(
                    json!({"piece":self.piece,"state":"accepted","queue_pos":1}),
                    201,
                ),
            ),
            ("finish", response(json!({"state":"accepted"}), 200)),
            ("events", Stream(self.range_frame(1, 'b', 0, "first"), true)),
        ]);
        self.run(first).unwrap();
    }

    fn replay(&self) {
        let mut replay = self.start(5);
        replay.push(("finish", response(json!({"state":"accepted"}), 200)));
        let done = json!({"piece":self.piece,"ranges_total":3,"ranges_ok":3,"failed":[]});
        let stream = self.range_frame(3, 'd', 32000, "third")
            + &self.range_frame(1, 'b', 0, "duplicate")
            + &self.range_frame(2, 'c', 16000, "second")
            + &self.range_frame(4, 'c', 16000, "duplicate")
            + &format!("id: 5\nevent: piece.done\ndata: {done}\n\n");
        replay.push(("events", Stream(stream, false)));
        self.run(replay).unwrap();
    }

    fn assert_golden(&self, generation: usize) {
        use crate::core::{
            meeting_router::revision::Revision,
            speech_host::{
                journal::{self, Journal},
                upload::digest,
            },
        };
        let root = self.data.join("local/speech-uploads/100-1");
        let config = folders(&root).swap_remove(0);
        let receipts = folders(&config).swap_remove(0);
        let journal = receipts.join("microphone-0.dpapi");
        let journal = Journal::load(journal).unwrap().unwrap();
        let revision = journal::assemble([&journal], "fake config", "fake source", &[], 0).unwrap();
        let text: Vec<_> = revision
            .transcript
            .segments
            .iter()
            .map(|s| s.text_original.as_str())
            .collect();
        assert_eq!(text, ["first", "second", "third"]);
        assert!(journal.acknowledged() && journal.done());
        assert_eq!(journal.session_generation(), generation);
        let source = fs::read(self.source.join("100-1/transcript.ndjson")).unwrap();
        let staged = receipts.join(format!("revision-{}.dpapi", digest(&source)));
        let bytes = fs::read(staged).unwrap();
        let staged: Revision =
            serde_json::from_slice(&sidenote_sync::identity::unprotect(&bytes).unwrap()).unwrap();
        let text: Vec<_> = staged
            .transcript
            .segments
            .iter()
            .map(|s| s.text_original.as_str())
            .collect();
        assert_eq!(text, ["first", "second", "third", "English kept"]);
        assert_eq!(staged.evidence.len(), 3);
        assert_eq!(staged.transcript.segments[2].end, 4.0);
    }
}

fn padded(mut body: String) -> String {
    body.push(':');
    body.push_str(&" ".repeat(4200 - body.len()));
    body.push_str("\n\nid: ");
    body
}

#[test]
fn speech_host_retries_put_and_replays_after_a_drop() {
    let fixture = Fixture::new(true);
    let mut dropped = fixture.start(0);
    dropped.push(("PUT", CutUpload));
    assert!(fixture.run(dropped).is_err());
    fixture.upload_and_interrupt();
    fixture.replay();
    fixture.assert_golden(0);
}

#[test]
fn speech_host_recovers_from_snapshot_when_replay_log_is_lost() {
    let fixture = Fixture::new(true);
    fixture.upload_and_interrupt();
    let mut resumed = fixture.start(0);
    resumed.push(("finish", response(json!({"state":"accepted"}), 200)));
    resumed.push((
        "snapshot",
        response(
            json!({"state":"done","failed":[],"ranges":[
                fixture.range('c', 16000, "second"), fixture.range('b', 0, "duplicate"),
                fixture.range('d', 32000, "third"), fixture.range('d', 32000, "duplicate"),
            ]}),
            200,
        ),
    ));
    fixture.run(resumed).unwrap();
    fixture.assert_golden(0);

    let erased = Fixture::new(true);
    erased.upload_and_interrupt();
    let mut resumed = erased.start(0);
    resumed.extend([
        ("finish", response(json!({"state":"accepted"}), 200)),
        (
            "snapshot",
            response(json!({"state":"queued","ranges":[],"failed":[]}), 200),
        ),
    ]);
    erased.run(resumed).unwrap();
    erased.upload_and_interrupt();
    erased.replay();
    erased.assert_golden(1);
}

#[test]
fn speech_host_off_and_unready_send_no_audio() {
    let off = Fixture::new(false);
    off.run(Vec::new()).unwrap();
    assert!(matches!(off.listener.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
    let unavailable = Fixture::new(true);
    unavailable
        .run(vec![(
            "GET /v1/host",
            Json(200, READY.replace(r#""ready""#, r#""unavailable""#)),
        )])
        .unwrap();
    assert!(
        matches!(unavailable.listener.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
    );
    assert!(!unavailable.data.join("local/speech-uploads").exists());
}
