use super::*;
use std::{cell::RefCell, rc::Rc};
type Log = Rc<RefCell<Vec<String>>>;
pub(super) struct Fakes(pub(super) Log);
struct Session(Log);
impl Drop for Session {
    fn drop(&mut self) {
        self.0.borrow_mut().push("unload".into());
    }
}
impl Recognizer for Session {
    fn recognize(&mut self, r: &Range) -> Result<Value> {
        self.0.borrow_mut().push(format!("read {}", r.piece));
        Ok(serde_json::json!({"text":r.piece}))
    }
}
impl NativeLoader for Fakes {
    fn load(&mut self, lang: NativeLanguage, r: &Range) -> Result<Box<dyn Recognizer>> {
        let expected = match lang {
            NativeLanguage::Hindi => "hi",
            NativeLanguage::Telugu => "te",
        };
        assert_eq!(r.range.language.as_deref(), Some(expected));
        let mut events = self.0.borrow_mut();
        assert_eq!(
            events.iter().filter(|s| s.starts_with("load")).count(),
            events.iter().filter(|s| *s == "unload").count()
        );
        events.push(format!("load {lang:?}"));
        Ok(Box::new(Session(self.0.clone())))
    }
}
impl EnglishBridge for Fakes {
    fn transcribe(&mut self, r: &Range) -> Result<Value> {
        assert_eq!(r.range.language.as_deref(), Some("en"));
        self.0.borrow_mut().push("English".into());
        Ok(serde_json::json!({"text":"English"}))
    }
}
pub(super) fn fake(root: &Path, plan: &Plan, log: &Log) -> Result<()> {
    run(root, plan, &mut Fakes(log.clone()), &mut Fakes(log.clone()))
}
pub(super) fn ranges(wav: &Path) -> Vec<Range> {
    [
        ("en", "en"),
        ("hi", "a"),
        ("hi", "b"),
        ("hi", "c"),
        ("te", "te"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (lang, piece))| Range {
        track: "system".into(),
        piece: piece.into(),
        wav: wav.into(),
        range: AsrRange {
            context: (i as u64 * 4_800_000, i as u64 * 4_800_000 + 160),
            members: vec![(0, (i as u64 * 4_800_000, i as u64 * 4_800_000 + 160))],
            language: Some(lang.into()),
        },
        model_id: format!("fake-{lang}"),
        model_sha256: hash(lang.as_bytes()),
    })
    .collect()
}
pub(super) fn id(plan: &Plan, piece: &str) -> String {
    plan.batches
        .values()
        .flat_map(|b| b.iter())
        .find(|(_, r)| r.piece == piece)
        .unwrap()
        .0
        .clone()
}
#[test]
fn identities_integrity_and_sequential_native_batches() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let wav = root.join("audio.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(root, ranges(&wav)).unwrap();
    let mut reversed = ranges(&wav);
    reversed.reverse();
    assert_eq!(spool(root, reversed).unwrap().revision, plan.revision);
    let log = Log::default();
    fake(root, &plan, &log).unwrap();
    for name in ["load Hindi", "load Telugu"] {
        assert_eq!(
            log.borrow().iter().filter(|e| e.as_str() == name).count(),
            1
        );
    }
    assert_eq!(log.borrow().iter().filter(|e| *e == "English").count(), 1);
    log.borrow_mut().clear();
    fake(root, &plan, &log).unwrap();
    assert!(log.borrow().is_empty());
    for bytes in [
        b"{".as_slice(),
        br#"{"text":"edited valid JSON, same identity"}"#,
    ] {
        store_io::atomic_write(&path(root, &plan, &id(&plan, "a"), "result"), bytes).unwrap();
        fake(root, &plan, &log).unwrap();
        assert_eq!(*log.borrow(), ["load Hindi", "read a", "unload"]);
        log.borrow_mut().clear();
    }
    crate::core::asr::write_wav(&wav, &[0.5; 160]).unwrap();
    assert!(fake(root, &plan, &log).is_err());
    assert!(log.borrow().is_empty());
    let changed = spool(root, ranges(&wav)).unwrap();
    assert_ne!(changed.revision, plan.revision);
    fake(root, &changed, &log).unwrap();
    assert_eq!(
        log.borrow()
            .iter()
            .filter(|s| s.starts_with("read") || *s == "English")
            .count(),
        5
    );
    let mut pins = ranges(&wav);
    pins[4].model_sha256 = hash(b"new pinned model");
    assert_ne!(spool(root, pins).unwrap().revision, changed.revision);
}
#[test]
fn abrupt_death_artifacts_redo_only_incomplete_ranges() {
    // Simulate on-disk artifacts without unwinding; real process kill/reap belongs to 8b.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let wav = root.join("audio.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(root, ranges(&wav)).unwrap();
    fake(root, &plan, &Log::default()).unwrap();
    for (piece, kind) in [
        ("a", "result"),
        ("a", "receipt"),
        ("b", "receipt"),
        ("c", "result"),
    ] {
        std::fs::remove_file(path(root, &plan, &id(&plan, piece), kind)).unwrap();
    }
    std::fs::write(
        path(root, &plan, &id(&plan, "a"), "result.tmp"),
        b"unrenamed partial",
    )
    .unwrap();
    let fresh = spool(root, ranges(&wav)).unwrap();
    let log = Log::default();
    fake(root, &fresh, &log).unwrap();
    log.borrow_mut().sort();
    assert_eq!(
        *log.borrow(),
        ["load Hindi", "read a", "read b", "read c", "unload"]
    );
}

#[test]
fn failed_native_shutdown_never_starts_the_next_language() {
    struct FailFinish;
    impl Recognizer for FailFinish {
        fn recognize(&mut self, _: &Range) -> Result<Value> {
            Ok(serde_json::json!({"text":"public"}))
        }
        fn finish(&mut self) -> Result<()> {
            Err("worker cleanup failed".into())
        }
    }
    struct Loader(Vec<String>);
    impl NativeLoader for Loader {
        fn load(&mut self, _: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>> {
            self.0.push(range.range.language.clone().unwrap());
            Ok(Box::new(FailFinish))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("public.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(dir.path(), ranges(&wav)).unwrap();
    let mut loader = Loader(Vec::new());
    assert!(run(dir.path(), &plan, &mut loader, &mut Fakes(Log::default())).is_err());
    assert_eq!(loader.0, vec!["hi"]);
}

#[test]
fn expired_budget_never_loads_or_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("public.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(dir.path(), ranges(&wav)).unwrap();
    let log = Log::default();
    let error = run_until(
        dir.path(),
        &plan,
        &mut Fakes(log.clone()),
        &mut Fakes(log.clone()),
        std::time::Duration::ZERO,
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<super::super::hex_bridge::HexFailure>(),
        Some(&super::super::hex_bridge::HexFailure::Timeout)
    );
    assert!(log.borrow().is_empty());
    for id in plan.batches.values().flat_map(|b| b.keys()) {
        assert!(read_result(dir.path(), &plan, id).unwrap().is_none());
    }
}

#[test]
fn stopped_run_resumes_from_valid_receipts() {
    struct StopAfterOne(Box<dyn Recognizer>, bool);
    impl Recognizer for StopAfterOne {
        fn recognize(&mut self, range: &Range) -> Result<Value> {
            if self.1 {
                return Err("stopped between ranges".into());
            }
            self.1 = true;
            self.0.recognize(range)
        }
    }

    struct Interrupt(Fakes);
    impl NativeLoader for Interrupt {
        fn load(&mut self, lang: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>> {
            Ok(Box::new(StopAfterOne(self.0.load(lang, range)?, false)))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let wav = root.join("audio.wav");
    crate::core::asr::write_wav(&wav, &[0.0; 160]).unwrap();
    let plan = spool(root, ranges(&wav)).unwrap();
    let log = Log::default();
    let error = run_until(
        root,
        &plan,
        &mut Interrupt(Fakes(log.clone())),
        &mut Fakes(log.clone()),
        std::time::Duration::from_secs(30),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "stopped between ranges");

    let mut hindi = plan.batches["hi"].iter();
    let (first_id, first) = hindi.next().unwrap();
    let (second_id, _) = hindi.next().unwrap();
    assert_eq!(
        read_result(root, &plan, first_id).unwrap(),
        Some(serde_json::json!({"text":first.piece}))
    );
    assert!(read_result(root, &plan, second_id).unwrap().is_none());
    assert!(
        read_result(root, &plan, &id(&plan, "en"))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        *log.borrow(),
        [
            "English",
            "load Hindi",
            &format!("read {}", first.piece),
            "unload"
        ]
    );

    log.borrow_mut().clear();
    let reopened = spool(root, ranges(&wav)).unwrap();
    fake(root, &reopened, &log).unwrap();
    let mut expected = vec!["load Hindi".to_owned()];
    expected.extend(
        reopened.batches["hi"]
            .values()
            .skip(1)
            .map(|r| format!("read {}", r.piece)),
    );
    expected.extend(["unload", "load Telugu", "read te", "unload"].map(str::to_owned));
    assert_eq!(*log.borrow(), expected);
    for (id, range) in reopened.batches.values().flat_map(|batch| batch.iter()) {
        let expected = if range.piece == "en" {
            "English"
        } else {
            &range.piece
        };
        assert_eq!(
            read_result(root, &reopened, id).unwrap(),
            Some(serde_json::json!({"text":expected}))
        );
    }
}
