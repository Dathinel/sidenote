use super::*;
use std::{cell::RefCell, rc::Rc};
type Log = Rc<RefCell<Vec<String>>>;
struct Native(String, Log, bool);
impl Drop for Native {
    fn drop(&mut self) {
        self.1.borrow_mut().push(format!("unload {}", self.0));
    }
}
impl Recognizer for Native {
    fn recognize(&mut self, r: &Range) -> Result<Value> {
        self.1
            .borrow_mut()
            .push(format!("read {} {}", self.0, r.piece));
        assert!(!(self.2 && r.piece == "second"), "fake crash");
        Ok(serde_json::json!({"text":self.0}))
    }
}
struct Bridge(Log);
impl Recognizer for Bridge {
    fn recognize(&mut self, _: &Range) -> Result<Value> {
        self.0.borrow_mut().push("hex".into());
        Ok(serde_json::json!({"text":"English", "timing":"segment"}))
    }
}
fn fake(root: &Path, plan: &Plan, log: &Log, crash: bool) -> Result<()> {
    run(root, plan, |lang| {
        if lang == "en" {
            return Ok(Box::new(Bridge(log.clone())));
        }
        let mut events = log.borrow_mut();
        assert_eq!(
            events.iter().filter(|e| e.starts_with("load")).count(),
            events.iter().filter(|e| e.starts_with("unload")).count()
        );
        events.push(format!("load {lang}"));
        Ok(Box::new(Native(lang.into(), log.clone(), crash)))
    })
}
fn ranges() -> Vec<Range> {
    [
        ("hi", "first"),
        ("en", "first"),
        ("te", "first"),
        ("hi", "second"),
    ]
    .into_iter()
    .map(|(lang, piece)| {
        let start = if piece == "first" { 0 } else { 4_800_000 };
        Range {
            track: "system".into(),
            piece: piece.into(),
            wav: PathBuf::from("synthetic.wav"),
            range: AsrRange {
                context: (start, start + 160),
                members: vec![(0, (start, start + 160))],
                language: Some(lang.into()),
            },
        }
    })
    .collect()
}
#[test]
fn sequential_batches_resume_and_reject_stale_or_partial() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let plan = spool(root, "source+config+models-v1".into(), ranges()).unwrap();
    assert!(spool(root, plan.revision.clone(), ranges()).is_ok());
    let log = Log::default();
    fake(root, &plan, &log, false).unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "hex",
            "load hi",
            "read hi first",
            "read hi second",
            "unload hi",
            "load te",
            "read te first",
            "unload te"
        ]
    );
    log.borrow_mut().clear();
    fake(root, &plan, &log, false).unwrap();
    assert!(log.borrow().is_empty());
    let id = key(&(&plan, &plan.batches["hi"][0])).unwrap();
    store_io::atomic_write(&root.join(&id), b"[\"partial").unwrap();
    fake(root, &plan, &log, false).unwrap();
    assert_eq!(*log.borrow(), ["load hi", "read hi first", "unload hi"]);
    assert!(spool(root, "v2".into(), ranges()).is_err());
    let stale = Plan {
        revision: "v2".into(),
        batches: plan.batches,
    };
    assert!(run(root, &stale, |_| panic!("stale loader")).is_err());
    assert!(spool(&root.join("new"), "".into(), ranges()).is_err());
}
#[test]
fn crash_mid_batch_keeps_complete_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let plan = spool(root, "v1".into(), ranges()).unwrap();
    let log = Log::default();
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fake(root, &plan, &log, true)
    }));
    assert!(crash.is_err());
    assert_eq!(
        *log.borrow(),
        [
            "hex",
            "load hi",
            "read hi first",
            "read hi second",
            "unload hi"
        ]
    );
    log.borrow_mut().clear();
    let resumed: Plan =
        serde_json::from_slice(&store_io::read_bounded(&root.join("plan.json"), 16 << 20).unwrap())
            .unwrap();
    fake(root, &resumed, &log, false).unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "load hi",
            "read hi second",
            "unload hi",
            "load te",
            "read te first",
            "unload te"
        ]
    );
    let new = spool(&root.join("v2"), "v2".into(), ranges()).unwrap();
    let old_id = key(&(&plan, &plan.batches["hi"][0])).unwrap();
    let new_id = key(&(&new, &new.batches["hi"][0])).unwrap();
    assert_ne!(old_id, new_id);
    let old = store_io::read_bounded(&root.join(&old_id), 1 << 20).unwrap();
    store_io::atomic_write(&root.join("v2").join(&new_id), &old).unwrap();
    assert!(!cached(&root.join("v2"), &new_id));
}
