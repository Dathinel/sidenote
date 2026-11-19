use super::{
    no_speech_tests::{Fixture, NativeFake},
    *,
};

struct CountedStatus {
    speech: HexSpeech,
    attempts: std::rc::Rc<std::cell::Cell<usize>>,
}
impl HexCommand for CountedStatus {
    fn call(
        &mut self,
        args: &[&str],
        timeout: Duration,
    ) -> std::result::Result<String, HexFailure> {
        if args == ["status", "--json"] {
            self.attempts.set(self.attempts.get() + 1);
        }
        self.speech.call(args, timeout)
    }
}

fn fixture(sequence: &str) -> Fixture {
    let mut f = Fixture::new(b"public English", 0, b"");
    f.real_controls();
    let mut ranges: Vec<Range> = std::mem::take(&mut f.plan.batches)
        .into_values()
        .flat_map(|b| b.into_values())
        .collect();
    for i in 0..3 {
        let mut extra: Range = serde_json::from_value(
            serde_json::to_value(
                ranges
                    .iter()
                    .find(|r| r.range.language.as_deref() == Some("en"))
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        extra.range.context = (20_000 + i * 4000, 21_600 + i * 4000);
        extra.range.members = vec![(0, extra.range.context)];
        ranges.push(extra);
    }
    f.plan = batches::spool(&f.work, ranges).unwrap();
    std::fs::write(f.work.join("status-sequence"), sequence).unwrap();
    f
}
fn count(f: &Fixture, name: &str) -> usize {
    std::fs::read_to_string(f.work.join(name))
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap()
}
fn log_before() -> String {
    assert!(crate::core::log_path().starts_with(crate::test_home()));
    std::fs::read_to_string(crate::core::log_path()).unwrap_or_default()
}
fn logged(before: &str, expected: usize) {
    let after = std::fs::read_to_string(crate::core::log_path()).unwrap();
    assert_eq!(
        after
            .strip_prefix(before)
            .unwrap()
            .matches("Hex status probe timed out")
            .count(),
        expected
    );
}
#[test]
fn status_probe_timeout_after_idle_never_authorizes_dispatch() {
    let f = fixture("ITIII");
    let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut bridge = English(HexBridge::new(CountedStatus {
        speech: HexSpeech {
            executable: f.work.join("fake-hex.exe"),
        },
        attempts: attempts.clone(),
    }));
    let error = batches::run(&f.work, &f.plan, &mut NativeFake::default(), &mut bridge)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("status unknown") && error.contains("1 consecutive"),
        "{error}"
    );
    assert_eq!(count(&f, "calls"), 1);
    assert_eq!(attempts.get(), 2);
    assert_eq!(count(&f, "help-calls"), 1);
}
#[test]
fn status_probe_without_known_idle_fails_on_first_timeout() {
    let mut f = fixture("TIIII");
    let before = log_before();
    let error = f.run(&mut NativeFake::default()).unwrap_err().to_string();
    assert!(
        error.contains("status unknown") && error.contains("last known idle=false"),
        "wrong cause: {error}"
    );
    assert_eq!(count(&f, "calls"), 0);
    logged(&before, 1);
}
#[test]
fn recognizer_busy_error_is_fatal_and_never_retried() {
    let mut f = Fixture::new(br#"{"code":"busy"}"#, 4, b"");
    let mut native = NativeFake::default();
    let error = f.run(&mut native).unwrap_err().to_string();
    assert!(
        error.contains("Hex is busy") && error.contains("exit=4"),
        "wrong cause: {error}"
    );
    assert_eq!(count(&f, "calls"), 1);
    assert!(native.0.is_empty());
}
