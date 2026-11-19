use super::*;
use crate::core::{
    meeting_router::{batches::Range, ranges::AsrRange, revision::Revision},
    model_catalog,
    speech_host::{self, Host, WorkMode, journal::Piece, upload::digest},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};
const WAIT: Duration = Duration::from_secs(3);
const MEETING: &str = "100-1";
const TOKEN: &[u8] = b"synthetic-device";
const READY: &str = r#"{"api":1,"role":"server","speech_jobs":true,"session_retirement":true,"asr":{"state":"ready","languages":["hi"],"models":[]}}"#;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Normal,
    CutPut,
    Partial,
    Replay,
    Lost(u64),
    Idle,
    Malformed,
    Truncated,
    Empty,
    Failed,
    Unready,
    Stall,
    Dead,
}

#[derive(Default)]
struct Server {
    sessions: BTreeMap<String, Option<(String, Piece)>>,
    retired: usize,
    peak: usize,
    full: usize,
    uploads: Vec<(String, String)>,
    connections: usize,
    snapshots: usize,
    replayed: bool,
    raw_keys: BTreeSet<String>,
}

struct Fixture {
    _temp: tempfile::TempDir,
    data: PathBuf,
    source: PathBuf,
    settings: PathBuf,
    collector: Collector,
    listener: TcpListener,
    server: Server,
    forward_ms: i64,
}

impl Fixture {
    fn new(consent: bool, seconds: u64) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let source = temp.path().join("hex");
        let meeting = source.join(MEETING);
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
        save(&settings, &url, TOKEN).unwrap();
        super::tests::audio(&meeting.join("microphone.wav"), seconds);
        let collector = Collector::new(&data);
        Self {
            _temp: temp,
            data,
            source,
            settings,
            collector,
            listener,
            server: Server::default(),
            forward_ms: 0,
        }
    }

    fn finished() -> Self {
        let mut fixture = Self::new(true, 3);
        fixture.capture(false);
        fixture
    }

    fn capture(&mut self, recording: bool) {
        let meeting = self.source.join(MEETING);
        store_io::write(
            &meeting.join("manifest.json"),
            &json!({
                "status":if recording { "recording" } else { "complete" },
                "created_at_ms":chrono::Utc::now().timestamp_millis()+1,
                "microphone":{"first_arrival_us":(!recording).then_some(1_000_000u64)}
            }),
        )
        .unwrap();
        if recording {
            self.collector
                .capture(
                    &json!({"meeting_state":"recording","id":MEETING}),
                    &self.source,
                    true,
                )
                .unwrap();
        } else {
            if self.collector.active.is_some() {
                for _ in 0..3 {
                    self.collector
                        .capture(&json!({"meeting_state":"stopped"}), &self.source, true)
                        .unwrap();
                }
            } else {
                self.collector
                    .collect_with_limit(&meeting, true, true)
                    .unwrap();
            }
            fs::write(meeting.join("transcript.ndjson"), concat!(
                r#"{"text":"English baseline","start_ms":1000,"end_ms":4000,"source":"microphone"}"#, "\n",
                r#"{"text":"English kept","start_ms":4000,"end_ms":5000,"source":"microphone"}"#, "\n")).unwrap();
        }
        for (dir, mut job) in jobs(&self.collector.root.join(MEETING)).unwrap() {
            job.queued_ms += 1;
            store_io::write(&dir.join("job.json"), &job).unwrap();
        }
    }

    fn run(&mut self, mode: Mode) -> io::Result<()> {
        self.run_with(mode, |_, _| Err("Fake local failure".into()))
    }

    fn run_with(
        &mut self,
        mode: Mode,
        mut local: impl FnMut(&Piece, (u64, u64)) -> super::super::asr::Result<Vec<Value>>,
    ) -> io::Result<()> {
        let (active, stopped) = mpsc::channel::<()>();
        let listener = &self.listener;
        let evidence = &self.collector.root;
        let state = &mut self.server;
        thread::scope(|scope| {
            let server = scope.spawn(move || state.serve(listener, evidence, mode, stopped));
            let host = Host::load(&self.settings).unwrap();
            let result = upload_windows(
                evidence,
                &self.data,
                &self.settings,
                host,
                &self.source,
                || chrono::Utc::now().timestamp_millis() + self.forward_ms,
                |_, piece, span, _| local(piece, span),
            );
            drop(active);
            server.join().unwrap();
            result
        })
    }

    fn revision(&self) -> Revision {
        let root = self.data.join("local/speech-uploads").join(MEETING);
        let config = folders(&root).swap_remove(0);
        let receipts = folders(&config).swap_remove(0);
        let source = fs::read(self.source.join(MEETING).join("transcript.ndjson")).unwrap();
        let staged = receipts.join(format!("revision-{}.dpapi", digest(&source)));
        assert!(staged.is_file(), "Completed ranges must stage a revision");
        let bytes = fs::read(staged).unwrap();
        serde_json::from_slice(&sidenote_sync::identity::unprotect(&bytes).unwrap()).unwrap()
    }

    fn golden(&self) -> Vec<String> {
        let revision = self.revision();
        let words: Vec<_> = revision
            .transcript
            .segments
            .iter()
            .map(|s| s.text_original.as_str())
            .collect();
        assert_eq!(words, ["first", "second", "third", "English kept"]);
        assert_eq!(revision.transcript.segments[2].end, 4.0);
        assert_eq!(revision.evidence.len(), 3);
        revision.evidence.into_keys().collect()
    }
}

fn reply(socket: &mut TcpStream, status: u16, body: Value) {
    let body = if status == 204 {
        String::new()
    } else {
        body.to_string()
    };
    write!(socket, "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

fn frame(seq: usize, event: &str, data: &Value) -> String {
    format!("id: {seq}\nevent: {event}\ndata: {data}\n\n")
}

fn ranges(session: &str, piece: &str, metadata: &Piece) -> Vec<Value> {
    ["first", "second", "third"].into_iter().enumerate().map(|(n, text)| {
        let start = metadata.start_sample + 16000 * n as u64;
        let local = start - metadata.context_start_sample;
        let r = Range {
            track: metadata.track.clone(), piece: piece.into(),
            wav: PathBuf::from(session).join("jobs").join(piece).join(format!("{n}.wav")),
            range: AsrRange { context: (local, local + 16000), members: vec![(0, (local, local + 16000))], language: Some("hi".into()) },
            model_id: "indicconformer-hi-ctc".into(), model_sha256: "f".repeat(64),
        };
        let key = digest(&serde_json::to_vec(&r).unwrap());
        json!({"piece":piece,"range":key,"track":r.track,"start_sample":start,"end_sample":start+16000,
            "language":"hi","route":"hi","model_sha256":r.model_sha256,"text":text,"timing":"range"})
    }).collect()
}

impl Server {
    fn serve(
        &mut self,
        listener: &TcpListener,
        evidence: &Path,
        mode: Mode,
        stopped: mpsc::Receiver<()>,
    ) {
        let mut idle = None;
        while stopped.try_recv() != Err(mpsc::TryRecvError::Disconnected) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    socket.set_read_timeout(Some(WAIT)).unwrap();
                    socket.set_write_timeout(Some(WAIT)).unwrap();
                    self.connections += 1;
                    if self.request(&mut socket, evidence, mode) {
                        idle = Some(socket);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                _ => panic!("Fake host listener failed"),
            }
        }
        drop(idle);
    }

    fn request(&mut self, socket: &mut TcpStream, evidence: &Path, mode: Mode) -> bool {
        let mut reader = BufReader::new(&mut *socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut request = line.split_whitespace();
        let method = request.next().unwrap().to_owned();
        let path = request.next().unwrap().to_owned();
        let mut headers = BTreeMap::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim_end().split_once(": ").unwrap();
            assert!(
                headers
                    .insert(name.to_ascii_lowercase(), value.to_owned())
                    .is_none()
            );
        }
        assert!(
            headers
                .get("authorization")
                .and_then(|s| s.strip_prefix("Bearer "))
                .is_some_and(|s| s.as_bytes() == TOKEN),
            "Unexpected authorization"
        );
        assert!(!headers.contains_key("origin"));
        if method == "PUT" && matches!(mode, Mode::CutPut) {
            reader.read_exact(&mut [0u8; 100]).unwrap();
            return false;
        }
        let mut body = vec![
            0;
            headers
                .get("content-length")
                .map_or(0, |s| s.parse().unwrap())
        ];
        reader.read_exact(&mut body).unwrap();
        if path == "/v1/host" {
            if matches!(mode, Mode::Dead) {
                return false;
            }
            let mut status: Value = serde_json::from_str(READY).unwrap();
            if matches!(mode, Mode::Unready) {
                status["asr"]["state"] = "unavailable".into();
            }
            reply(socket, 200, status);
            return false;
        }
        if method == "POST" {
            assert_eq!(path, "/v1/sessions");
            let body: Value = serde_json::from_slice(&body).unwrap();
            let id = digest(&serde_json::to_vec(&("owner", &body["client_session"])).unwrap());
            let fresh = !self.sessions.contains_key(&id);
            if fresh && self.sessions.len() == 1 {
                self.full += 1;
                reply(socket, 429, Value::Null);
                return false;
            }
            self.sessions.entry(id.clone()).or_default();
            self.peak = self.peak.max(self.sessions.len());
            let next = match mode {
                Mode::Lost(next) => next,
                Mode::Replay => 5,
                _ => 0,
            };
            reply(
                socket,
                if fresh { 201 } else { 200 },
                json!({"session":id,"next_seq":next}),
            );
            return false;
        }
        let parts: Vec<_> = path.split('/').collect();
        let session = parts[3];
        if method == "DELETE" {
            self.retired += usize::from(self.sessions.remove(session).is_some());
            reply(socket, 204, Value::Null);
            return false;
        }
        if method == "PUT" {
            let piece: Piece = serde_json::from_str(&headers["x-piece"]).unwrap();
            assert_eq!(piece.origin_sample, 0);
            assert_eq!(piece.sha256, digest(&body));
            let wav = evidence
                .join(MEETING)
                .join(format!("{}-{:06}", piece.track, piece.index))
                .join("audio.wav");
            assert!(
                fs::read(wav).unwrap() == body,
                "Transfer changed the collector WAV"
            );
            self.uploads.push((session.into(), parts[5].into()));
            self.sessions
                .insert(session.into(), Some((parts[5].into(), piece)));
            reply(socket, 201, json!({"piece":parts[5],"state":"accepted"}));
            return false;
        }
        let (piece, metadata) = self.sessions[session].as_ref().unwrap();
        let mut readings = ranges(session, piece, metadata);
        if self.uploads.iter().filter(|(_, id)| id == piece).count() > 1 {
            readings[0]["text"] = "later reading".into();
        }
        if parts[4] == "pieces" {
            assert_eq!(parts[5], piece);
            self.snapshots += 1;
            let (state, rows, failed) = match mode {
                Mode::Lost(_) => ("lost", &readings[..0], json!([])),
                Mode::Partial => ("running", &readings[..1], json!([])),
                Mode::Stall => ("running", &readings[1..2], json!([])),
                Mode::Replay if !self.replayed => ("running", &readings[..1], json!([])),
                Mode::Failed => (
                    "failed",
                    &readings[2..],
                    json!([{"piece":piece,"code":"ASR_UNAVAILABLE","retryable":false}]),
                ),
                _ => {
                    readings.push(readings[0].clone());
                    ("done", readings.as_slice(), json!([]))
                }
            };
            reply(
                socket,
                200,
                json!({"state":state,"ranges":rows,"failed":failed}),
            );
            return false;
        }
        assert_eq!(parts[4], "events");
        self.raw_keys.extend(
            readings
                .iter()
                .map(|v| v["range"].as_str().unwrap().to_owned()),
        );
        let cursor = headers
            .get("last-event-id")
            .map_or(0, |v| v.parse::<usize>().unwrap());
        let done = json!({"piece":piece,"ranges_total":3,"ranges_ok":3,"failed":[]});
        let result = |seq, n| frame(seq, "range.result", &readings[n]);
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        if matches!(mode, Mode::Idle | Mode::Lost(1)) {
            return true;
        }
        let mut payload = match mode {
            Mode::Normal => {
                result(1, 0) + &result(2, 1) + &result(3, 2) + &frame(4, "piece.done", &done)
            }
            Mode::Replay if cursor == 1 => {
                self.replayed = true;
                result(3, 2)
                    + &result(1, 0)
                    + &result(2, 1)
                    + &result(4, 1)
                    + &frame(5, "piece.done", &done)
            }
            Mode::Partial | Mode::Replay | Mode::Truncated => {
                result(1, 0) + ":" + &" ".repeat(4200) + "\n\nid: "
            }
            Mode::Malformed => "id: broken\n\n:".to_owned() + &" ".repeat(4200),
            Mode::Lost(2) => frame(2, "host.status", &json!({})),
            Mode::Lost(_) | Mode::Empty | Mode::Failed | Mode::Stall => return false,
            _ => unreachable!(),
        };
        let terminal = matches!(mode, Mode::Normal) || matches!(mode, Mode::Replay) && cursor == 1;
        if terminal {
            // Let the client read terminal frames while the HTTP stream remains open.
            payload.push(':');
            payload.push_str(&" ".repeat(4200));
            payload.push_str("\n\n");
        }
        let chunk = format!("{:x}\r\n{payload}\r\n", payload.len());
        socket.write_all(chunk.as_bytes()).unwrap();
        terminal
    }
}

#[test]
fn speech_host_uploads_before_stop_and_releases_at_the_cap() {
    let mut fixture = Fixture::new(true, 302);
    fixture.capture(true);
    fixture.run(Mode::Normal).unwrap();
    assert_eq!(
        fixture.server.uploads.len(),
        1,
        "First window must upload while recording"
    );
    fixture.capture(false);
    let result = fixture.run(Mode::Normal);
    assert_eq!(
        fixture.server.full, 0,
        "Retained session cap blocked a later window"
    );
    result.unwrap();
    assert_eq!(fixture.server.uploads.len(), 2);
    assert_eq!(fixture.server.retired, 2);
    assert_eq!(fixture.server.peak, 1);
    assert_eq!(fixture.revision().transcript.segments[0].start, 1.0);
}

#[test]
fn speech_host_retries_put_and_replays_after_a_drop() {
    let mut fixture = Fixture::finished();
    assert!(fixture.run(Mode::CutPut).is_err());
    fixture.run(Mode::Partial).unwrap();
    fixture.run(Mode::Replay).unwrap();
    fixture.golden();
}

#[test]
fn speech_host_recovers_lost_log_with_stable_range_keys() {
    let mut reference = Fixture::finished();
    reference.run(Mode::Normal).unwrap();
    let golden = reference.golden();
    for next in [0, 1, 2] {
        let mut fixture = Fixture::finished();
        fixture.run(Mode::Partial).unwrap();
        fixture.run(Mode::Lost(next)).unwrap();
        fixture.run(Mode::Normal).unwrap();
        assert_eq!(fixture.server.uploads.len(), 2);
        let [(old, piece), (new, same_piece)] = fixture.server.uploads.as_slice() else {
            unreachable!()
        };
        assert_ne!(old, new);
        assert_eq!(piece, same_piece);
        assert_eq!(fixture.server.raw_keys.len(), 6);
        assert_eq!(fixture.golden(), golden);
    }
}

#[test]
fn speech_host_stages_missing_range_after_replacement_fails() {
    let mut fixture = Fixture::finished();
    for mode in [Mode::Partial, Mode::Lost(0), Mode::Failed] {
        fixture.run(mode).unwrap();
    }
    let revision = fixture.revision();
    assert_eq!(
        revision.provenance["missing_ranges"],
        json!([{"track":"microphone","start_sample":16000,"end_sample":32000,"origin_sample":16000}])
    );
    assert_eq!(
        revision
            .evidence
            .values()
            .map(|v| v["text"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["first", "third"])
    );
    fixture.run(Mode::Normal).unwrap();
    assert_eq!(fixture.server.uploads.len(), 2);
    assert_eq!(fixture.server.retired, 2);
}

#[test]
fn speech_host_snapshots_after_interrupted_streams() {
    for mode in [Mode::Idle, Mode::Empty, Mode::Truncated, Mode::Malformed] {
        let mut fixture = Fixture::finished();
        fixture.run(mode).unwrap();
        assert_eq!(fixture.server.snapshots, 1, "{mode:?}");
        fixture.golden();
    }
}

#[test]
fn speech_host_off_and_unready_send_no_audio() {
    for (consent, mode, connections) in [(false, Mode::Normal, 0), (true, Mode::Unready, 1)] {
        let mut fixture = Fixture::new(consent, 3);
        fixture.capture(false);
        fixture.run(mode).unwrap();
        assert_eq!(fixture.server.connections, connections);
        assert!(!fixture.data.join("local/speech-uploads").exists());
    }
}

fn local_reading(piece: &Piece, (start, end): (u64, u64)) -> Value {
    json!({"piece":"a".repeat(64),"range":"b".repeat(64),"track":piece.track,"start_sample":start,
        "end_sample":end,"language":"hi","route":"hi","model_sha256":"c".repeat(64),"text":"local","timing":"range"})
}

#[test]
fn speech_host_stall_and_death_fall_back_once_and_ignore_late_ranges() {
    let mut fixture = Fixture::finished();
    fixture.run(Mode::Stall).unwrap();
    assert_eq!(fixture.server.uploads.len(), 1);
    assert!(speech_host::progress(&fixture.settings).contains("Sent 1 of 1"));
    fixture.forward_ms = 31_000;
    let settings = fixture.settings.clone();
    let mut calls = Vec::new();
    for mode in [Mode::Dead, Mode::Dead, Mode::Normal] {
        fixture
            .run_with(mode, |piece, span| {
                assert!(speech_host::progress(&settings).contains("Running 1 here"));
                calls.push(span);
                Ok(vec![local_reading(piece, span)])
            })
            .unwrap();
    }
    assert_eq!(
        calls,
        [(0, 16000), (32000, 48000)],
        "Queued spans must run once"
    );
    let revision = fixture.revision();
    assert!(revision.provenance["missing_ranges"].is_null());
    assert_eq!(revision.evidence.len(), 3);
    assert_eq!(
        revision
            .transcript
            .segments
            .iter()
            .take(3)
            .map(|s| s.text_original.as_str())
            .collect::<Vec<_>>(),
        ["local", "second", "local"]
    );
    assert_eq!(fixture.server.uploads.len(), 1);
    assert_eq!(
        fixture.server.snapshots, 1,
        "Late ranges must stay on the host"
    );
    assert_eq!(fixture.server.retired, 1);
}

#[test]
fn speech_host_failed_replacement_fills_its_gap_before_the_deadline() {
    let mut fixture = Fixture::finished();
    fixture.run(Mode::Partial).unwrap();
    fixture.run(Mode::Lost(0)).unwrap();
    let mut count = 0;
    fixture
        .run_with(Mode::Failed, |piece, span| {
            assert_eq!(span, (16000, 32000));
            count += 1;
            Ok(vec![local_reading(piece, span)])
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(fixture.revision().provenance["missing_ranges"].is_null());
}

#[test]
fn speech_host_only_reports_gaps_at_the_pipeline_budget() {
    let mut fixture = Fixture::finished();
    WorkMode::HostOnly.save(&fixture.settings).unwrap();
    let mut config = super::super::Config::load_in(&fixture.data).unwrap();
    config.engine_settings.stage_deadline_seconds = Some(120);
    config.save(&fixture.data).unwrap();
    fixture.run(Mode::Stall).unwrap();
    fixture.forward_ms = 31_000;
    fixture
        .run_with(Mode::Dead, |_, _| panic!("Host only started local work"))
        .unwrap();
    assert!(speech_host::progress(&fixture.settings).contains("Gaps 0"));
    fixture.forward_ms = 121_000;
    fixture
        .run_with(Mode::Dead, |_, _| panic!("Host only started local work"))
        .unwrap();
    assert!(speech_host::progress(&fixture.settings).contains("Gaps 2"));
    assert_eq!(
        fixture.revision().provenance["missing_ranges"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn speech_host_auto_restages_a_host_only_gap() {
    let mut fixture = Fixture::finished();
    WorkMode::HostOnly.save(&fixture.settings).unwrap();
    fixture.run(Mode::Stall).unwrap();
    fixture.forward_ms = 31_000;
    fixture.run(Mode::Dead).unwrap();
    let before = fixture.revision();
    assert_eq!(before.evidence.len(), 1);
    assert_eq!(
        before.provenance["missing_ranges"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    WorkMode::Auto.save(&fixture.settings).unwrap();
    fixture
        .run_with(Mode::Dead, |piece, span| {
            Ok(vec![local_reading(piece, span)])
        })
        .unwrap();
    let after = fixture.revision();
    assert_eq!(
        after.evidence.len(),
        3,
        "Accepted local receipts must restage the revision"
    );
    assert!(after.provenance["missing_ranges"].is_null());
    for (id, value) in before.evidence {
        assert_eq!(after.evidence[&id], value);
    }
    fixture
        .run_with(Mode::Dead, |_, _| panic!("An accepted span ran twice"))
        .unwrap();
}

#[test]
fn speech_host_this_device_uses_no_connection() {
    let mut fixture = Fixture::new(true, 3);
    WorkMode::ThisDevice.save(&fixture.settings).unwrap();
    fixture.capture(false);
    fixture
        .run_with(Mode::Normal, |piece, span| {
            assert_eq!(span, (0, 48000));
            Ok(vec![local_reading(piece, span)])
        })
        .unwrap();
    assert_eq!(fixture.server.connections, 0);
    assert_eq!(fixture.revision().evidence.len(), 1);
    assert!(speech_host::progress(&fixture.settings).contains("Finished here 1"));
}
