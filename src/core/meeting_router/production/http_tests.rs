//! Enter through the production bridge factory and batch executor, with real child HTTP I/O.
use super::super::hex_http::HTTP_PHASES;
use super::no_speech_tests::{Fixture, NativeFake};
use super::*;
use std::io::Write;
fn captured() -> Value {
    serde_json::from_str(include_str!("http_reply_fixture.json")).unwrap()
}
fn expected_text(top: bool) -> String {
    let c = captured();
    if top {
        c["top_level"]["text"].as_str().unwrap().into()
    } else {
        c["transcription"]["transcript"].as_str().unwrap().into()
    }
}

fn fixture(mode: u8) -> Fixture {
    let f = Fixture::new(b"public", 0, b"");
    fs::copy(
        super::fixture_tools::executable(true),
        f.work.join("fake-hex.exe"),
    )
    .unwrap();
    fs::write(f.work.join("mode"), mode.to_string()).unwrap();
    let c = captured();
    let mut body = c["transcription"].clone();
    body["durationMs"] = 200.into();
    if mode == 6 {
        body["transcript"] = "".into();
    }
    for (name, value) in [
        ("http-reply", body),
        ("top-reply", c["top_level"].clone()),
        ("status-reply", c["status_fields"].clone()),
        ("http-error", c["http_error"]["reply"].clone()),
    ] {
        fs::write(f.work.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    for (name, value) in [
        ("api-version", c["ready"]["apiVersion"].as_str().unwrap()),
        ("ready-type", c["ready"]["type"].as_str().unwrap()),
        ("help-reply", c["help"].as_str().unwrap()),
    ] {
        fs::write(f.work.join(name), value).unwrap();
    }
    f
}
fn run(f: &Fixture, timeout: Duration) -> Result<()> {
    run_with_request_timeout(f, timeout, Duration::from_secs(10))
}
fn run_with_request_timeout(
    f: &Fixture,
    timeout: Duration,
    request_timeout: Duration,
) -> Result<()> {
    let mut bridge = english_http::bridge(
        HexSpeech {
            executable: f.work.join("fake-hex.exe"),
        },
        &f.work,
        request_timeout,
    );
    let result = pipeline::run_batches(
        &f.work,
        &f.plan,
        &mut NativeFake::default(),
        &mut bridge,
        Instant::now() + timeout,
        &mut Default::default(),
        Instant::now(),
    );
    drop(bridge);
    result
}
fn count(f: &Fixture, name: &str) -> u32 {
    fs::read_to_string(f.work.join(name))
        .unwrap_or_default()
        .parse()
        .unwrap_or(0)
}
fn receipt(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.work.join("english-transport.json")).unwrap()).unwrap()
}
fn run_timed(f: &Fixture, mode: u8, timeout: Duration, request_timeout: Duration) {
    HTTP_PHASES.take();
    let result = run_with_request_timeout(f, timeout, request_timeout);
    let finished = Instant::now();
    let phases = HTTP_PHASES.take();
    let (failed_at, request) = if mode == 4 {
        assert!(phases.request_io.is_none());
        (phases.handshake_failed.expect("handshake failure"), None)
    } else {
        let (started, failed) = phases.request_io.expect("stalled HTTP exchange");
        let (sample, deadline) = phases.request_budget.expect("request budget clock");
        let allowance = deadline.duration_since(sample) / 2;
        (failed, Some((failed.duration_since(started), allowance)))
    };
    let reap = phases
        .aborted
        .expect("successful service abort")
        .duration_since(failed_at);
    let tail = finished.duration_since(phases.batches_done.expect("batch executor returned"));
    // Write directly so hosted logs retain timing samples from passing runs.
    writeln!(
        std::io::stderr().lock(),
        "HTTP fixture mode {mode}: request (elapsed, allowance)={request:?}, reap={reap:?}, tail={tail:?}"
    )
    .unwrap();
    if let Some((elapsed, allowance)) = request {
        let slack = Duration::from_millis(750);
        assert!(
            elapsed <= allowance + slack,
            "stalled HTTP request exceeded its share"
        );
    }
    assert!(reap < Duration::from_secs(8), "service was reaped late");
    assert!(
        tail < Duration::from_secs(8),
        "batch teardown finished late"
    );
    result.unwrap();
}

fn reaped(f: &Fixture) {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::*};
    let pid: u32 = fs::read_to_string(f.work.join("service-pid"))
        .unwrap()
        .parse()
        .unwrap();
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !handle.is_null() {
            let mut code = 0;
            assert_ne!(GetExitCodeProcess(handle, &mut code), 0);
            CloseHandle(handle);
            assert_ne!(code, 259, "owned service must have exited");
        } else {
            assert_eq!(windows_sys::Win32::Foundation::GetLastError(), 87);
        }
    }
}
#[test]
fn production_http_reuses_service_across_english_pieces() {
    let f = fixture(0);
    run(&f, Duration::from_secs(20)).unwrap();
    assert_eq!(count(&f, "starts"), 1);
    assert_eq!(count(&f, "http-calls"), 2);
    assert_eq!(count(&f, "command-calls"), 0);
    let r = receipt(&f);
    assert_eq!(r["http_pieces"], 2);
    assert_eq!(r["service_reaped"], true);
    assert!(r["fallback_reason"].is_null());
    for id in f.plan.batches["en"].keys() {
        assert_eq!(
            batches::read_result(&f.work, &f.plan, id).unwrap().unwrap()["text"],
            expected_text(false)
        );
    }
    reaped(&f);
}
#[test]
fn production_http_failure_retains_baseline_without_owner_command() {
    for mode in [1, 2, 5] {
        let f = fixture(mode);
        run(&f, Duration::from_secs(20)).unwrap();
        assert_eq!(count(&f, "starts"), 1);
        assert_eq!(count(&f, "http-calls"), if mode == 1 { 1 } else { 0 });
        assert_eq!(count(&f, "command-calls"), 0);
        let r = receipt(&f);
        assert!(r["fallback_reason"].is_string());
        if mode == 1 {
            assert_eq!(
                r["fallback_reason"],
                format!("service-http-status-{}", captured()["http_error"]["status"])
            );
        }
        assert_eq!(r["command_pieces"], 0);
        for id in f.plan.batches["en"].keys() {
            assert!(
                batches::read_result(&f.work, &f.plan, id)
                    .unwrap()
                    .is_none()
            );
        }
        let assembled = revision::assemble(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
        for unit in &f.baseline {
            assert!(super::super::baseline::source_covered(
                unit,
                &assembled.transcript.segments
            ));
        }
        assert!(revision::assemble(&f.work, &f.plan, &f.manifest, &[], 1).is_err());
        reaped(&f);
    }
}
#[test]
fn production_http_timeout_reaps_request_and_handshake_children() {
    for mode in [3, 4] {
        let f = fixture(mode);
        run_timed(&f, mode, Duration::from_secs(20), Duration::from_secs(10));
        assert_eq!(count(&f, "starts"), 1);
        assert_eq!(count(&f, "command-calls"), 0);
        reaped(&f);
        let r = receipt(&f);
        if mode == 3 {
            assert_eq!(count(&f, "http-calls"), 1);
            assert_eq!(r["fallback_reason"], "service-request");
        } else {
            assert_eq!(count(&f, "http-calls"), 0);
            assert_eq!(r["fallback_reason"], "service-handshake");
        }
    }
}
#[test]
fn production_http_request_failure_preserves_whole_run_budget() {
    let f = fixture(9);
    // The caller's deadline, rather than the bridge's configured cap, limits this run.
    run_timed(&f, 9, Duration::from_secs(10), Duration::from_secs(20));
    assert_eq!(count(&f, "starts"), 1);
    assert_eq!(count(&f, "http-calls"), 1);
    assert_eq!(count(&f, "command-calls"), 0);
    reaped(&f);
    let r = receipt(&f);
    assert_eq!(r["fallback_reason"], "service-request");
    assert!(f.work.join("hi.clock.json").exists());
    assert!(f.work.join("te.clock.json").exists());
}

#[test]
fn production_http_expired_whole_run_fails_without_dispatch() {
    let f = fixture(0);
    assert!(run(&f, Duration::ZERO).is_err());
    assert_eq!(count(&f, "starts"), 0);
    assert_eq!(count(&f, "command-calls"), 0);
}
#[test]
fn production_http_empty_success_keeps_no_speech_and_later_languages() {
    let f = fixture(6);
    run(&f, Duration::from_secs(20)).unwrap();
    assert_eq!(count(&f, "http-calls"), 2);
    assert_eq!(count(&f, "command-calls"), 0);
    assert!(f.work.join("hi.clock.json").exists());
    assert!(f.work.join("te.clock.json").exists());
    for id in f.plan.batches["en"].keys() {
        assert!(
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_none()
        );
        assert!(
            f.work
                .join(format!("{}-{id}.fallback", f.plan.revision))
                .exists()
        );
    }
    reaped(&f);
}

#[test]
fn production_http_cancel_drop_reaps_service() {
    let f = fixture(0);
    let mut bridge = english_http::bridge(
        HexSpeech {
            executable: f.work.join("fake-hex.exe"),
        },
        &f.work,
        Duration::from_secs(10),
    );
    bridge
        .transcribe(f.plan.batches["en"].values().next().unwrap())
        .unwrap();
    // Cancellation/unwinding drops the same owner even without the normal finish hook.
    drop(bridge);
    reaped(&f);
    assert_eq!(receipt(&f)["service_reaped"], true);
    for range in receipt(&f)["ranges"].as_object().unwrap().values() {
        assert_eq!(range["cleanup_reason"], "helper-aborted");
        assert_eq!(range["service_reaped"], true);
    }
}

fn owner_activity(mode: u8) {
    let f = fixture(mode);
    let start = Instant::now();
    assert!(run(&f, Duration::from_secs(20)).is_err());
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(count(&f, "starts"), 1);
    assert_eq!(count(&f, "command-calls"), 0);
    assert_eq!(
        receipt(&f)["fallback_reason"],
        "service-owner-active-or-unknown"
    );
    for id in f.plan.batches["en"].keys() {
        assert!(
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_none()
        );
    }
    reaped(&f);
}
#[test]
fn production_http_owner_activity_cancels_startup() {
    owner_activity(10);
}
#[test]
fn production_http_owner_activity_cancels_inference() {
    owner_activity(11);
}

#[test]
fn production_http_releases_model_before_native_load() {
    struct Check<'a>(&'a Fixture);
    impl NativeLoader for Check<'_> {
        fn load(&mut self, language: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>> {
            assert_eq!(receipt(self.0)["service_reaped"], true);
            reaped(self.0);
            NativeFake::default().load(language, range)
        }
    }
    let f = fixture(0);
    let mut bridge = english_http::bridge(
        HexSpeech {
            executable: f.work.join("fake-hex.exe"),
        },
        &f.work,
        Duration::from_secs(10),
    );
    pipeline::run_batches(
        &f.work,
        &f.plan,
        &mut Check(&f),
        &mut bridge,
        Instant::now() + Duration::from_secs(20),
        &mut Default::default(),
        Instant::now(),
    )
    .unwrap();
}

#[test]
fn production_http_mixed_transport_survives_parent_commit_and_load() {
    let f = fixture(12);
    run(&f, Duration::from_secs(20)).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    let mut coverage = f.baseline.clone();
    for c in &mut coverage {
        c.english.clear();
    }
    store_io::write(&f.work.join("coverage.json"), &coverage).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let saved = publish(&f, &assembled);
    let transport = &saved.provenance["english_transport"];
    assert_eq!(transport, &assembled.provenance["english_transport"]);
    let ranges = transport["ranges"].as_object().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        ranges.values().filter(|r| r["transport"] == "http").count(),
        1
    );
    assert_eq!(
        ranges
            .values()
            .filter(|r| r["transport"] == "failed")
            .count(),
        1
    );
    for (id, entry) in ranges {
        assert!(f.plan.batches["en"].contains_key(id));
        assert_eq!(entry["cleanup_reason"], "service-http-status-409");
        assert_eq!(entry["service_reaped"], true);
        if entry["transport"] == "http" {
            assert_eq!(entry["reply"]["body_sha256"].as_str().unwrap().len(), 64);
            assert_eq!(entry["reply"]["api_version"], "2");
        } else {
            assert_eq!(entry["fallback"], "captured-hex-baseline");
            assert_eq!(entry["failure_reason"], "service-http-status-409");
        }
    }
    assert!(!serde_json::to_string(transport).unwrap().contains("token"));
}

fn publish(f: &Fixture, assembled: &revision::Revision) -> revision::Revision {
    let output = jobs::BatchOutput {
        generation: "1".into(),
        source_revision: "public".into(),
        plan: serde_json::from_value(serde_json::to_value(&f.plan).unwrap()).unwrap(),
        manifest: serde_json::from_value(serde_json::to_value(&f.manifest).unwrap()).unwrap(),
        fallbacks: vec![],
        english_transport: assembled.provenance["english_transport"].clone(),
    };
    store_io::write(&f.work.join("output.json"), &output).unwrap();
    store_io::write(
        &f.root.path().join("local/router-queue/123-4.json"),
        &json!({"id":"123-4","generation":1}),
    )
    .unwrap();
    let published = f.root.path().join("local/transcripts/123-4");
    revision::begin(&published, 1).unwrap();
    let id = jobs::commit_batches(f.root.path(), "123-4", 1, "public", &f.baseline).unwrap();
    revision::load(&published, Some(&id)).unwrap().unwrap().1
}

#[test]
fn production_http_failure_preserves_baseline_crossing_native_reading() {
    for padded in [false, true] {
        crossing_native_reading(padded);
    }
}
fn crossing_native_reading(padded: bool) {
    let mut f = fixture(1);
    let mut ranges = Vec::new();
    for (lang, context) in [("en", (0, 1600)), ("hi", (1600, 4800))] {
        let mut r: Range = serde_json::from_value(
            serde_json::to_value(f.plan.batches[lang].values().next().unwrap()).unwrap(),
        )
        .unwrap();
        r.range.context = context;
        r.range.members = vec![(0, context)];
        if padded && lang == "en" {
            r.range.context.1 = 4000;
        }
        r.wav = f.work.join(format!("crossing-{lang}.wav"));
        asr::write_wav(
            &r.wav,
            &vec![0.0; (r.range.context.1 - r.range.context.0) as usize],
        )
        .unwrap();
        ranges.push(r);
    }
    f.plan = batches::spool(&f.work, ranges).unwrap();
    f.baseline.truncate(1);
    f.baseline[0].end = 0.2;
    run(&f, Duration::from_secs(20)).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    let coverage: Vec<_> = [(0.0, 0.1), (0.1, 0.3)]
        .into_iter()
        .map(|(start, end)| {
            let mut c = f.baseline[0].clone();
            c.start = start;
            c.end = end;
            c.english.clear();
            c
        })
        .collect();
    store_io::write(&f.work.join("coverage.json"), &coverage).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let saved = publish(&f, &assembled);
    assert!(
        saved
            .transcript
            .segments
            .iter()
            .any(|s| s.text_original == f.baseline[0].english)
    );
    assert!(
        saved
            .transcript
            .segments
            .iter()
            .any(|s| s.text_original == "public native")
    );
    assert_eq!(
        saved.provenance["assembly_coverage"]["fallback_required"][0]["reason"],
        "helper-failed"
    );
    let mut invalid = saved.clone();
    invalid.provenance["english_transport"]["ranges"] = json!({});
    assert!(revision::commit(&f.root.path().join("local/transcripts/123-4"), &invalid).is_err());
    assert_eq!(count(&f, "command-calls"), 0);
}

#[test]
fn production_http_cached_batch_retains_reply_identity_and_cleanup() {
    let f = fixture(0);
    run(&f, Duration::from_secs(20)).unwrap();
    let first = receipt(&f)["ranges"].clone();
    run(&f, Duration::from_secs(20)).unwrap();
    assert_eq!(count(&f, "starts"), 1);
    assert_eq!(count(&f, "http-calls"), 2);
    assert_eq!(receipt(&f)["ranges"], first);
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    let mut coverage = f.baseline.clone();
    for c in &mut coverage {
        c.english.clear();
    }
    store_io::write(&f.work.join("coverage.json"), &coverage).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    assert_eq!(
        publish(&f, &assembled).provenance["english_transport"]["ranges"],
        first
    );
}

#[test]
fn production_http_legacy_cache_is_reread_before_publication() {
    let f = fixture(0);
    run(&f, Duration::from_secs(20)).unwrap();
    store_io::write(
        &f.work.join("english-transport.json"),
        &json!({"service_starts":1,"http_pieces":1,"command_pieces":1,
            "fallback_reason":"service-http-status-409","service_reaped":true}),
    )
    .unwrap();
    run(&f, Duration::from_secs(20)).unwrap();
    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    let mut coverage = f.baseline.clone();
    for c in &mut coverage {
        c.english.clear();
    }
    store_io::write(&f.work.join("coverage.json"), &coverage).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let saved = publish(&f, &assembled);
    let ranges = saved.provenance["english_transport"]["ranges"]
        .as_object()
        .unwrap();
    assert_eq!(
        ranges.len(),
        f.plan.batches["en"].len(),
        "cached English must have per-range identity"
    );
    assert_eq!(count(&f, "http-calls"), 4);
    assert_eq!(count(&f, "command-calls"), 0);
    for id in f.plan.batches["en"].keys() {
        let r = &ranges[id];
        assert_eq!(r["transport"], "http");
        assert_eq!(r["model"], f.plan.batches["en"][id].model_id);
        assert_eq!(r["language"], "en");
        assert_eq!(r["reply"]["http_status"], 200);
        assert_eq!(r["reply"]["body_sha256"].as_str().unwrap().len(), 64);
        assert!(saved.evidence.contains_key(id));
    }
}

#[test]
fn production_http_unverified_cache_cannot_survive_failed_reread() {
    for prior in [
        None,
        Some(b"not json".as_slice()),
        Some(b"{\"ranges\":{}}".as_slice()),
    ] {
        let f = fixture(0);
        run(&f, Duration::from_secs(20)).unwrap();
        let transport = f.work.join("english-transport.json");
        if let Some(bytes) = prior {
            fs::write(&transport, bytes).unwrap();
        } else {
            fs::remove_file(&transport).unwrap();
        }
        fs::write(f.work.join("mode"), "1").unwrap();
        run(&f, Duration::from_secs(20)).unwrap();
        for id in f.plan.batches["en"].keys() {
            assert!(
                batches::read_result(&f.work, &f.plan, id)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(receipt(&f)["ranges"][id]["transport"], "failed");
        }
        assert_eq!(count(&f, "command-calls"), 0);
    }
}

#[test]
fn saved_fallback_crossing_two_baseline_sentences_publishes_exact_bounds() {
    let mut f = fixture(0);
    let mut ranges = Vec::new();
    for (lang, context) in [("en", (0, 8000)), ("hi", (24000, 40000))] {
        let mut r: Range = serde_json::from_value(
            serde_json::to_value(f.plan.batches[lang].values().next().unwrap()).unwrap(),
        )
        .unwrap();
        r.range.context = context;
        r.range.members = vec![(0, context)];
        r.wav = f.work.join(format!("boundary-{lang}.wav"));
        asr::write_wav(&r.wav, &vec![0.0; (context.1 - context.0) as usize]).unwrap();
        ranges.push(r);
    }
    f.plan = batches::spool(&f.work, ranges).unwrap();
    f.baseline.truncate(2);
    run(&f, Duration::from_secs(20)).unwrap();
    let record = json!({"reason":"guard_rejected",
        "interval":{"track":"system","start":0.5,"end":1.5}});
    store_io::write(&f.work.join("uncertainty.json"), &json!([record])).unwrap();
    let coverage: Vec<_> = [(0.0, 0.5), (0.5, 1.5), (1.5, 2.5)]
        .into_iter()
        .map(|(start, end)| {
            let mut c = f.baseline[0].clone();
            c.start = start;
            c.end = end;
            c.english.clear();
            c
        })
        .collect();
    store_io::write(&f.work.join("coverage.json"), &coverage).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let saved = publish(&f, &assembled);
    for unit in &f.baseline {
        let s = saved
            .transcript
            .segments
            .iter()
            .find(|s| s.text_original == unit.english)
            .unwrap();
        assert_eq!((s.start, s.end), (unit.start, unit.end));
    }
    let native = saved
        .transcript
        .segments
        .iter()
        .find(|s| s.route == "hi")
        .unwrap();
    assert_eq!((native.start, native.end), (1.5, 2.5));
    let receipt = &saved.provenance["assembly_coverage"]["overlaps"][0];
    assert_eq!(receipt["fallback_required"][0]["record"], record);
    assert_eq!(
        receipt["fallback_required"][0]["interval"],
        record["interval"]
    );
    let dest = f.root.path().join("local/transcripts/123-4");
    let mut invalid = saved.clone();
    invalid.transcript.segments.push(native.clone());
    assert!(revision::commit(&dest, &invalid).is_err());
    let mut invalid = saved.clone();
    let expanded = invalid
        .transcript
        .segments
        .iter_mut()
        .find(|s| s.text_original == f.baseline[1].english)
        .unwrap();
    expanded.end += 1.0;
    expanded.sources[0].chunk.end += 1.0;
    assert!(revision::commit(&dest, &invalid).is_err());

    let mut stale = saved.clone();
    stale.provenance["routing_fallbacks"] = json!([]);
    stale.provenance["assembly_coverage"]["fallback_required"] = json!([]);
    assert_eq!(
        revision::commit(&dest, &stale).unwrap_err().to_string(),
        "Competing selected readings"
    );

    store_io::write(&f.work.join("uncertainty.json"), &json!([])).unwrap();
    let assembled =
        pipeline::assemble_saved(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
    let saved = publish(&f, &assembled);
    for unit in &f.baseline {
        assert!(saved.transcript.segments.iter().any(|s| {
            s.text_original == unit.english && (s.start, s.end) == (unit.start, unit.end)
        }));
    }
    assert_eq!(
        saved.provenance["assembly_coverage"]["overlaps"][0]["fallback_required"],
        json!([])
    );
}
