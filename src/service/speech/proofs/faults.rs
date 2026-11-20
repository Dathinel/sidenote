use super::*;
use crate::core::{
    lid::{self, anuvani::Detector},
    speech_sessions::Store,
};
pub(crate) struct Logits {
    calls: usize,
    partial: bool,
}
impl Logits {
    pub(crate) fn new() -> Self {
        Self {
            calls: 0,
            partial: fs::read_to_string(crate::core::state_dir().join("gaps")).unwrap()
                == "partial",
        }
    }
}
impl Detector for Logits {
    fn scores(&mut self, _: &[f32]) -> asr::Result<Vec<f32>> {
        Err("raw logits only".into())
    }
    fn logits(&mut self, _: &[f32]) -> asr::Result<Vec<f32>> {
        self.calls += 1;
        let mut scores = vec![0.0; lid::LANGUAGES.len()];
        if self.partial && self.calls > 1 {
            scores[if self.calls.is_multiple_of(3) { 4 } else { 6 }] = 20.0;
        }
        Ok(scores)
    }
}
pub(crate) fn crash(path: &Path, stage: &str) {
    if active()
        && path.components().any(|p| p.as_os_str() == "jobs")
        && fs::read_to_string(crate::core::state_dir().join("crash")).is_ok_and(|s| s == stage)
    {
        std::process::exit(72);
    }
}
#[test]
fn rejected_speech_survives_production_planning_and_http_completion() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    let model = crate::core::model_catalog::model("anuvani-lid-int8").unwrap();
    let dir = root.join("models/anuvani-lid-int8");
    fs::create_dir_all(&dir).unwrap();
    for file in model.files {
        let bytes = crate::core::asr::fixtures::fetch(file.name, file.url, file.sha256);
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
    for mode in ["all", "partial"] {
        fs::write(root.join("gaps"), mode).unwrap();
        let service = Service::start(root);
        let mut request = request();
        request.client_session = mode.into();
        request.languages = vec!["en".into(), "hi".into()];
        let (id, piece) = service.upload(&request, 0);
        wait(|| {
            let snapshot = service.snapshot(&id, &piece);
            assert_ne!(
                snapshot["state"],
                "failed",
                "{}",
                fs::read_to_string(root.join("service.log")).unwrap()
            );
            snapshot["state"] == "done"
        });
        let snapshot = service.snapshot(&id, &piece);
        let gaps: Vec<crate::core::meeting_router::fallback::RoutingFallback> =
            serde_json::from_slice(
                &fs::read(root.join(format!("local/speech/{id}/jobs/{piece}/uncertainty.json")))
                    .unwrap(),
            )
            .unwrap();
        assert!(!gaps.is_empty());
        let failed = snapshot["failed"].as_array().unwrap();
        assert_eq!(failed.len(), gaps.len());
        for (actual, gap) in failed.iter().zip(gaps) {
            let interval = gap.interval.unwrap();
            assert_eq!(actual["code"], "LID_UNCERTAIN");
            assert_eq!(actual["reason"], serde_json::to_value(gap.reason).unwrap());
            assert_eq!(
                actual["start_sample"],
                80000 + (interval.start * 16000.0).round() as u64
            );
            assert_eq!(
                actual["end_sample"],
                80000 + (interval.end * 16000.0).round() as u64
            );
        }
        let saved = events(root, &id);
        assert_eq!(
            saved
                .iter()
                .filter(|e| e["event"] == "range.failed")
                .map(|e| e["data"].clone())
                .collect::<Vec<_>>(),
            *failed
        );
        assert!(
            saved
                .iter()
                .position(|e| e["event"] == "range.failed")
                .unwrap()
                < saved
                    .iter()
                    .position(|e| e["event"] == "piece.done")
                    .unwrap()
        );
        let ranges = snapshot["ranges"].as_array().unwrap();
        if mode == "all" {
            assert!(ranges.is_empty());
        } else {
            assert!(ranges.iter().any(|r| r["route"] == "hi"));
            assert!(
                ranges
                    .iter()
                    .any(|r| r["route"] == "en" && r["text"].is_null())
            );
        }
    }
}
#[test]
fn nested_publication_interruptions_recover_without_losing_receipts() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    fs::write(root.join("hold"), b"pause").unwrap();
    let mut service = Service::start(root);
    let (id, piece) = service.upload(&request(), 0);
    wait(|| root.join("blocked").exists());
    service.stop();
    let job = root.join(format!("local/speech/{id}/jobs/{piece}"));
    let receipts = || -> Vec<String> {
        let mut v = fs::read_dir(&job)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|s| s.ends_with(".receipt"))
            .collect::<Vec<_>>();
        v.sort();
        v
    };
    let saved = receipts();
    assert_eq!(saved.len(), 1);
    let retained =
        || fs::read_dir(&job).unwrap().count() + fs::read_dir(job.join("input")).unwrap().count();
    let baseline = retained();
    for stage in ["input", "cut", "input", "cut"] {
        fs::write(root.join("crash"), stage).unwrap();
        let mut service = Service::start(root);
        wait(|| service.child.try_wait().unwrap().is_some());
        assert_eq!(service.child.wait().unwrap().code(), Some(72));
        fs::remove_file(root.join("crash")).unwrap();
        let store = Store(root.join("local/speech"));
        let token = zeroize::Zeroizing::new(fs::read_to_string(root.join("token")).unwrap());
        let grant = crate::mcp::access::authenticate_speech(root, &token).unwrap();
        let owner = digest(&serde_json::to_vec(&grant).unwrap());
        store.events(&owner, &id, 0).unwrap();
        assert_eq!(receipts(), saved);
        assert_eq!(
            retained(),
            baseline,
            "interrupted artifacts must not accumulate"
        );
        assert_eq!(
            fs::read(root.join(format!("local/speech/{id}/{piece}.wav"))).unwrap(),
            fs::read(root.join("audio.wav")).unwrap()
        );
        for path in [&job, &job.join("input")] {
            assert!(fs::read_dir(path).unwrap().all(|e| {
                let n = e.unwrap().file_name();
                let n = n.to_string_lossy();
                !n.starts_with(".tmp") && !n.ends_with(".tmp")
            }));
        }
    }
    fs::remove_file(root.join("hold")).unwrap();
    let service = Service::start(root);
    wait(|| service.snapshot(&id, &piece)["state"] == "done");
}
#[test]
fn retained_status_is_bounded_and_cannot_kill_the_listener() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    let mut service = Service::start(root);
    let body = serde_json::to_vec(&request()).unwrap();
    let (_, v) = service.send("POST", "/v1/sessions", &body, "");
    let id = v["session"].as_str().unwrap().to_owned();
    assert_eq!(
        service
            .send(
                "POST",
                &format!("/v1/sessions/{id}/finish"),
                br#"{"pieces_total":0}"#,
                ""
            )
            .0,
        200
    );
    wait(|| {
        root.join(format!("local/speech/{id}/events.ndjson"))
            .exists()
            && events(root, &id)
                .iter()
                .any(|e| e["event"] == "session.done")
    });
    service.stop();
    let store = Store(root.join("local/speech"));
    for n in 0..100 {
        store.emit(&id, "host.status", json!({"queue":n})).unwrap();
    }
    let saved = events(root, &id);
    assert_eq!(
        saved.iter().filter(|e| e["event"] == "host.status").count(),
        64
    );
    let path = root.join(format!("local/speech/{id}/events.ndjson"));
    let mut bytes = fs::read(&path).unwrap();
    let mut filler =
        json!({"seq":saved.len()+1,"event":"synthetic.retained","data":{"padding":""}});
    let n = (16 << 20) - bytes.len() - serde_json::to_vec(&filler).unwrap().len() - 1;
    filler["data"]["padding"] = "x".repeat(n).into();
    bytes.extend(serde_json::to_vec(&filler).unwrap());
    bytes.push(b'\n');
    assert_eq!(bytes.len(), 16 << 20);
    fs::write(&path, &bytes).unwrap();
    let mut service = Service::start(root);
    let mut other = request();
    other.client_session = "other".into();
    let (other_id, piece) = service.upload(&other, 0);
    wait(|| service.snapshot(&other_id, &piece)["state"] == "done");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    service.stop();
    let service = Service::start(root);
    assert_eq!(service.send("GET", "/v1/host", &[], "").0, 200);
    assert_eq!(service.snapshot(&other_id, &piece)["state"], "done");
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
#[test]
fn one_unavailable_status_log_does_not_stop_other_sessions() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    let mut service = Service::start(root);
    let (id, queued) = service.upload(&request(), 0);
    service.stop();
    let ledger: Value =
        serde_json::from_slice(&fs::read(root.join("local/speech/sessions.json")).unwrap())
            .unwrap();
    assert_eq!(ledger[&id]["pieces"][&queued]["done"], false);
    store_io::write(
        &root.join(format!("local/speech/{id}/events.ndjson")),
        &"unavailable log",
    )
    .unwrap();
    let service = Service::start(root);
    let mut other = request();
    other.client_session = "healthy".into();
    let (id, piece) = service.upload(&other, 0);
    wait(|| service.snapshot(&id, &piece)["state"] == "done");
    assert_eq!(service.send("GET", "/v1/host", &[], "").0, 200);
}
#[test]
fn http_stream_disconnect_releases_slots_and_revocation_ends_replay() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = setup();
    let root = root.path();
    let service = Service::start(root);
    let (_, v) = service.send(
        "POST",
        "/v1/sessions",
        &serde_json::to_vec(&request()).unwrap(),
        "",
    );
    let path = format!("/v1/sessions/{}/events", v["session"].as_str().unwrap());
    let mut streams = Vec::new();
    for _ in 0..32 {
        let stream = service.stream("GET", &path, &[], "");
        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("200"));
        streams.push(reader);
    }
    drop(streams);
    wait(|| {
        let mut stream = service.stream("GET", "/v1/host", &[], "");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).is_ok() && reply.starts_with("HTTP/1.1 200")
    });
    let mut reader = std::io::BufReader::new(service.stream(
        "GET",
        &path,
        &[],
        "Last-Event-ID: 18446744073709551615\r\n",
    ));
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("200"));
    assert!(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", FIXTURE, "--nocapture"])
            .env("SIDENOTE_L7_ROOT", root)
            .env("SIDENOTE_L7_REVOKE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let mut end = String::new();
    reader.read_to_string(&mut end).unwrap();
    assert_eq!(service.send("GET", "/v1/host", &[], "").0, 401);
}
