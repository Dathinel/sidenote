use super::{no_speech_tests::Fixture, *};
use std::{cell::RefCell, rc::Rc};

type Events = Rc<RefCell<Vec<String>>>;
struct NativeFault {
    events: Events,
    empty: &'static str,
    fail: bool,
}
struct NativeSession {
    language: String,
    calls: usize,
    fault: NativeFault,
}
impl NativeLoader for NativeFault {
    fn load(&mut self, _: NativeLanguage, r: &Range) -> Result<Box<dyn Recognizer>> {
        Ok(Box::new(NativeSession {
            language: r.range.language.clone().unwrap(),
            calls: 0,
            fault: NativeFault {
                events: self.events.clone(),
                empty: self.empty,
                fail: self.fail,
            },
        }))
    }
}
impl Recognizer for NativeSession {
    fn recognize(&mut self, _: &Range) -> Result<Value> {
        self.calls += 1;
        self.fault.events.borrow_mut().push(self.language.clone());
        if self.language == "hi" && self.calls == 1 {
            return Ok(json!({"text":self.fault.empty}));
        }
        if self.language == "hi" && self.fault.fail {
            return Err("native model load failed".into());
        }
        Ok(json!({"text":"public native"}))
    }
}
fn native_case(empty: &'static str, fail: bool) -> (Fixture, NativeFault) {
    let mut f = Fixture::new(b"public English", 0, b"");
    let mut ranges: Vec<Range> = std::mem::take(&mut f.plan.batches)
        .into_values()
        .flat_map(|b| b.into_values())
        .collect();
    let mut extra: Range = serde_json::from_value(
        serde_json::to_value(
            ranges
                .iter()
                .find(|r| r.range.language.as_deref() == Some("hi"))
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    extra.range.context = (72_000, 80_000);
    extra.range.members = vec![(0, extra.range.context)];
    ranges
        .iter_mut()
        .find(|r| r.range.language.as_deref() == Some("hi"))
        .unwrap()
        .range
        .context
        .1 = 72_000;
    let first = ranges
        .iter_mut()
        .find(|r| r.range.language.as_deref() == Some("hi"))
        .unwrap();
    first.range.members = vec![(0, first.range.context)];
    ranges.push(extra);
    f.plan = batches::spool(&f.work, ranges).unwrap();
    let native = NativeFault {
        events: Events::default(),
        empty,
        fail,
    };
    (f, native)
}
#[test]
fn native_empty_becomes_no_speech_and_remaining_languages_continue() {
    for empty in ["", " \n\t"] {
        let (mut f, mut native) = native_case(empty, false);
        f.run(&mut native)
            .expect("native empty text must use the common NoSpeech path");
        assert_eq!(*native.events.borrow(), ["hi", "hi", "te"]);
        let missing: Vec<_> = f.plan.batches["hi"]
            .keys()
            .filter(|id| {
                batches::read_result(&f.work, &f.plan, id)
                    .unwrap()
                    .is_none()
            })
            .collect();
        assert_eq!(missing.len(), 1);
        assert!(
            !f.work
                .join(format!("{}-{}.receipt", f.plan.revision, missing[0]))
                .exists()
        );
        let r = revision::assemble(&f.work, &f.plan, &f.manifest, &f.baseline, 1).unwrap();
        assert_eq!(r.provenance["no_speech"][missing[0]]["reason"], "no-speech");
        assert!(
            r.transcript
                .segments
                .iter()
                .any(|s| s.text_original == "public baseline 4")
        );
        assert!(f.plan.batches["te"].keys().all(|id| {
            batches::read_result(&f.work, &f.plan, id)
                .unwrap()
                .is_some()
        }));
        assert!(revision::assemble(&f.work, &f.plan, &f.manifest, &[], 1).is_err());
    }
}
#[test]
fn native_error_after_no_speech_still_fails_the_pass() {
    let (mut f, mut native) = native_case("", true);
    let error = f.run(&mut native).unwrap_err().to_string();
    assert!(
        error.contains("native model load failed"),
        "wrong failure: {error}"
    );
    assert_eq!(*native.events.borrow(), ["hi", "hi"]);
    assert!(f.plan.batches["te"].keys().all(|id| {
        batches::read_result(&f.work, &f.plan, id)
            .unwrap()
            .is_none()
    }));
}
