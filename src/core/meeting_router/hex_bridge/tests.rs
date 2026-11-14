use super::*;
use std::collections::VecDeque;
const HELP: &str = "--model --language --json";
const TEXT: &str = r#"{"text":"hello","model":"parakeet_unified_en"}"#;
const TIMED: &str = r#"{"transcript":"hello","segments":[{"start":0,"end":0.01}]}"#;
struct FakeHex(String, VecDeque<Result<String>>, u8);
impl HexCommand for FakeHex {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String> {
        let before = (self.0.clone(), self.2);
        match args {
            // Pinned Hex f5681bc control/cli.rs:245-256,436-440: these select.
            ["model", "use" | "install", id] => {
                self.0 = (*id).into();
                return ok("");
            }
            ["model", "transcribe", "--help"] | ["status", "--json"] => {}
            [
                "model",
                "transcribe",
                wav,
                "--model",
                "parakeet_unified_en",
                "--language",
                "en",
                tail @ ..,
            ] => {
                assert!(tail.is_empty() || tail == ["--json"]);
                assert!(Path::new(wav).is_file());
                assert_eq!(timeout, Duration::from_secs(120));
            }
            _ => return Err(Unsupported),
        }
        let reply = self.1.pop_front().expect("unexpected Hex call");
        if let Ok(out) = &reply {
            let v: Value = serde_json::from_str(out).unwrap_or_default();
            if let Some(ms) = v["delay_ms"].as_u64() {
                thread::sleep(Duration::from_millis(ms));
            }
        }
        assert_eq!((self.0.clone(), self.2), before);
        reply
    }
}
fn ok(text: &str) -> Result<String> {
    Ok(text.into())
}
fn status(id: &str, pending: u32, version: u32) -> Result<String> {
    Ok(
        serde_json::json!({"version":version,"model":{"id":id,"preparing":null},
        "capture":{"state":"idle","pending_jobs":pending},"meeting":{"state":"idle"}})
        .to_string(),
    )
}
#[test]
fn fake_hex_contract() {
    let temp = tempfile::tempdir().unwrap();
    let wav = temp.path().join("range.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let request = HexRequest {
        range_id: "range-7".into(),
        model_id: "parakeet_unified_en".into(),
        language: "en".into(),
    };
    for selected in ["whisper-small", "custom-user-dictation"] {
        let h = || ok(HELP);
        let s = || status(selected, 0, 4);
        let busy = || status(selected, 1, 4);
        let race = || ok(r#"{"code":"busy"}"#);
        let mut cases = vec![
            (vec![ok("--model --language"), s(), ok("hello")], Ok(None)),
            (vec![h(), s(), ok(TEXT)], Ok(Some(TEXT))),
            (vec![h(), s(), ok(TIMED)], Ok(Some(TIMED))),
            (vec![h(), busy(), s(), ok(TEXT)], Ok(Some(TEXT))),
            (vec![h(), s(), race(), s(), ok(TEXT)], Ok(Some(TEXT))),
            (vec![h(), s(), Err(Timeout)], Err(Timeout)),
            (vec![h(), status(selected, 0, 3)], Err(Unsupported)),
            (vec![ok("--model")], Err(Unsupported)),
        ];
        for pending in [0, 1] {
            let mut delayed: Value =
                serde_json::from_str(&status(selected, pending, 4).unwrap()).unwrap();
            delayed["delay_ms"] = 100.into();
            cases.push((vec![h(), ok(&delayed.to_string())], Err(Busy)));
        }
        for bad in [
            "{bad",
            r#"{"text":"hello","model":"wrong"}"#,
            r#"{"text":"hello","model_id":"wrong"}"#,
        ] {
            cases.push((vec![h(), s(), ok(bad)], Err(Failed)));
        }
        for segments in [
            r#"[{"start":-1,"end":0}]"#,
            r#"[{"start":0.01,"end":0}]"#,
            r#"[{"start":0,"end":1}]"#,
            r#"[{"start":NaN,"end":0}]"#,
            r#"[{"start":0,"end":1e309}]"#,
            r#"[{"start":0.006,"end":0.009},{"start":0,"end":0.001}]"#,
        ] {
            let out = format!(r#"{{"text":"hello","segments":{segments}}}"#);
            cases.push((vec![h(), s(), ok(&out)], Err(Failed)));
        }
        for (replies, expected) in cases {
            let fake = FakeHex(selected.into(), replies.into(), 17);
            let mut bridge = HexBridge::new(fake);
            bridge.busy_wait = Duration::from_millis(if expected == Err(Busy) { 50 } else { 1000 });
            let result = bridge.transcribe(&wav, &request);
            let expected = expected.map(|json| HexReading {
                request: request.clone(),
                text: "hello".into(),
                json: json.map(|s| serde_json::from_str(s).unwrap()),
            });
            assert_eq!(result, expected);
            assert!(bridge.command.1.is_empty());
            assert_eq!(
                (bridge.command.0.as_str(), bridge.command.2),
                (selected, 17)
            );
        }
    }
    let mut fake = FakeHex("user-choice".into(), VecDeque::new(), 17);
    for command in ["use", "install"] {
        let before = fake.0.clone();
        fake.call(&["model", command, command], Duration::from_secs(2))
            .unwrap();
        assert_ne!(
            fake.0, before,
            "unchanged-selection assertion must catch {command}"
        );
    }
}
#[cfg(windows)]
#[test]
fn fake_process_timeout() {
    let fake = HexSpeech {
        executable: "powershell.exe".into(),
    };
    let args = ["-NoProfile", "-Command", "Start-Sleep -Seconds 5"];
    let error = fake
        .run_for(&args, None, Duration::from_millis(1))
        .unwrap_err();
    assert!(matches!(error.downcast_ref::<HexFailure>(), Some(Timeout)));
}
