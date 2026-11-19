use super::*;
use std::cell::Cell;

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }

pub(crate) fn active() -> bool {
    ACTIVE.get()
}

pub(crate) fn with_synthetic_models<T>(f: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.set(self.0);
        }
    }
    let _reset = Reset(ACTIVE.replace(true));
    f()
}

pub(super) struct SyntheticRecognizer;

impl Recognizer for SyntheticRecognizer {
    fn recognize(&mut self, range: &Range) -> Result<Value> {
        let samples = asr::read_wav(&range.wav)?;
        assert_eq!(
            samples.len(),
            16000,
            "Recognizer must receive only the uncovered second"
        );
        assert!(
            samples.iter().all(|sample| (sample - 0.25).abs() < 0.001),
            "Recognizer received audio outside the uncovered span"
        );
        Ok(json!({"text":"synthetic","timing":"range"}))
    }
}
