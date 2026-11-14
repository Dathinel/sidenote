use super::*;
use std::collections::VecDeque;
const HELP: &str = "--model --language --json";
const TEXT: &str = r#"{"text":"hello"}"#;
const TIMED: &str = r#"{"transcript":"hello","segments":[{"start":0,"end":0.01,"text":"hello"}]}"#;
struct FakeHex(&'static str, VecDeque<Result<String>>);
impl HexCommand for FakeHex {
    fn call(&mut self, args: &[&str], timeout: Duration) -> Result<String> {
        let before = self.0;
        match args {
            ["transcribe", "--help"] | ["status", "--json"] => {}
            _ => {
                assert_eq!(args[0], "transcribe");
                assert!(Path::new(args[1]).is_file());
                assert_eq!(
                    args[2..].join(" "),
                    "--model parakeet_unified_en --language en --json"
                );
                assert_eq!(timeout, Duration::from_secs(120));
            }
        }
        let result = self.1.pop_front().expect("unexpected Hex call");
        assert_eq!(self.0, before);
        result
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
    for selected in ["whisper-small", "custom-user-dictation"] {
        let h = || ok(HELP);
        let s = || status(selected, 0, 4);
        let busy = || status(selected, 1, 4);
        let race = || ok(r#"{"code":"busy"}"#);
        let cases = [
            (vec![h(), s(), ok("hello")], Ok(None)),
            (vec![h(), s(), ok(TEXT)], Ok(Some(TEXT))),
            (vec![h(), s(), ok(TIMED)], Ok(Some(TIMED))),
            (vec![h(), busy(), s(), ok("hello")], Ok(None)),
            (vec![h(), s(), race(), s(), ok("hello")], Ok(None)),
            (vec![h(), s(), Err(Timeout)], Err(Timeout)),
            (vec![h(), status(selected, 0, 3)], Err(Unsupported)),
            (vec![ok("--model")], Err(Unsupported)),
            (vec![h(), busy()], Err(Busy)),
        ];
        for (replies, expected) in cases {
            let exhausted = expected == Err(Busy);
            let fake = FakeHex(selected, replies.into());
            let mut bridge = HexBridge::new(fake);
            bridge.busy_wait = Duration::from_millis(1000 * u64::from(!exhausted));
            let result = bridge.transcribe(&wav, "parakeet_unified_en", "en");
            let expected = expected.map(|json| HexReading {
                text: "hello".into(),
                json: json.map(|s| serde_json::from_str(s).unwrap()),
            });
            assert_eq!(result, expected);
            assert!(bridge.command.1.is_empty());
            assert_eq!(bridge.command.0, selected);
            assert_eq!(bridge.transcribe(&wav, "", "en"), Err(InvalidRange));
        }
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
