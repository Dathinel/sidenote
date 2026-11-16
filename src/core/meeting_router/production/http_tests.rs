//! Enter through the production bridge factory and batch executor, with real child HTTP I/O.
use super::no_speech_tests::{Fixture, NativeFake};
use super::*;
use crate::platform::CommandExt;
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
    let source = f.work.join("http.c");
    fs::write(&source, include_str!("http_fake.c")).unwrap();
    assert!(
        Command::new("gcc.exe")
            .creation_flags(0x08000000)
            .args(["-Os", "-static", "-s"])
            .arg(source)
            .arg("-o")
            .arg(f.work.join("fake-hex.exe"))
            .arg("-lws2_32")
            .status()
            .unwrap()
            .success()
    );
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
        timeout,
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
            assert!(batches::read_result(&f.work, &f.plan, id).unwrap().is_none());
        }
        let assembled = revision::assemble(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
        for unit in &f.baseline {
            assert!(super::super::baseline::source_covered(unit, &assembled.transcript.segments));
        }
        assert!(revision::assemble(&f.work, &f.plan, &f.manifest, &[], 1).is_err());
        reaped(&f);
    }
}
#[test]
fn production_http_timeout_reaps_request_and_handshake_children() {
    for mode in [3, 4] {
        let f = fixture(mode);
        let start = Instant::now();
        run_with_request_timeout(&f, Duration::from_secs(20), Duration::from_secs(2)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(8));
        assert_eq!(count(&f, "starts"), 1);
        assert_eq!(count(&f, "command-calls"), 0);
        reaped(&f);
        assert!(receipt(&f)["fallback_reason"].is_string());
    }
}
#[test]
fn production_http_request_failure_preserves_whole_run_budget() {
    let f = fixture(9);
    let start = Instant::now();
    run(&f, Duration::from_millis(900)).unwrap();
    assert!(start.elapsed() < Duration::from_secs(8));
    assert_eq!(count(&f, "starts"), 1);
    reaped(&f);
    assert!(receipt(&f)["fallback_reason"].is_string());
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
}

#[test]
fn production_http_owner_activity_cancels_startup_and_inference() {
    for mode in [10, 11] {
        let f = fixture(mode);
        let start = Instant::now();
        assert!(run(&f, Duration::from_secs(20)).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(count(&f, "starts"), 1);
        assert_eq!(count(&f, "command-calls"), 0);
        assert_eq!(receipt(&f)["fallback_reason"], "service-owner-active-or-unknown");
        for id in f.plan.batches["en"].keys() {
            assert!(batches::read_result(&f.work, &f.plan, id).unwrap().is_none());
        }
        reaped(&f);
    }
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
    let mut bridge = english_http::bridge(HexSpeech { executable: f.work.join("fake-hex.exe") }, &f.work, Duration::from_secs(10));
    pipeline::run_batches(&f.work, &f.plan, &mut Check(&f), &mut bridge,
        Duration::from_secs(20), &mut Default::default(), Instant::now()).unwrap();
}
