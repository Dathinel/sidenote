use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::Instant,
};

#[derive(Clone, Copy, Default, serde::Serialize)]
struct Counter {
    calls: u64,
    nanos: u64,
    bytes: u64,
}
static COUNTERS: Mutex<BTreeMap<&'static str, Counter>> = Mutex::new(BTreeMap::new());
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SIDENOTE_IDLE_PASS").is_some()
            && [
                super::state_dir(),
                super::meetings_dir(),
                super::peers_dir(),
            ]
            .iter()
            .all(|p| super::is_proof_path(p, "sidenote-idle-pass"))
    })
}
pub(crate) struct Span {
    stage: &'static str,
    start: Option<Instant>,
    bytes: u64,
}
impl Span {
    pub(crate) fn begin(stage: &'static str) -> Self {
        Self {
            stage,
            start: enabled().then(Instant::now),
            bytes: 0,
        }
    }
    pub(crate) fn bytes(&mut self, bytes: usize) {
        self.bytes += bytes as u64;
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let mut counters = COUNTERS.lock().unwrap();
            let counter = counters.entry(self.stage).or_default();
            counter.calls += 1;
            counter.nanos += start.elapsed().as_nanos() as u64;
            counter.bytes += self.bytes;
            if self.stage.ends_with("_pass") {
                eprintln!(
                    "idle-pass {}",
                    serde_json::json!({"pass": self.stage, "cumulative": *counters})
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_stage_counts_its_bytes_and_elapsed_time() {
        let mut span = Span {
            stage: "counter_test",
            start: Some(Instant::now()),
            bytes: 0,
        };
        span.bytes(123);
        drop(span);
        let counters = COUNTERS.lock().unwrap();
        let counter = counters.get("counter_test").unwrap();
        assert_eq!(counter.calls, 1);
        assert_eq!(counter.bytes, 123);
        assert!(counter.nanos > 0);
    }
}
