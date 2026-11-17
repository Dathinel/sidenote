//! Read-only diagnostic: exactly three production status requests, with no raw replies or errors.
use sidenote::hex_engine::{HexEngine, HexError};
use std::time::Instant;

fn main() {
    let engine = HexEngine::new();
    for attempt in 1..=3 {
        let start = Instant::now();
        let (budget_ms, result) = if attempt < 3 {
            (750, engine.status())
        } else {
            (12_000, engine.status_for_command())
        };
        match result {
            Ok(state) => println!(
                "attempt={attempt} budget_ms={budget_ms} elapsed_ms={} running={} version={} protocol={} meeting_state={} capture_state={}",
                start.elapsed().as_millis(), state.running, state.version, state.protocol,
                known(&state.meeting_state), known(&state.capture_state)
            ),
            Err(error) => println!("attempt={attempt} budget_ms={budget_ms} elapsed_ms={} error={}", start.elapsed().as_millis(), match error {
                HexError::HexNotRunning => "not-running",
                HexError::Unsupported => "unsupported",
                HexError::Timeout => "timeout",
                HexError::Failed { stderr_summary, .. } => stderr_summary,
            }),
        }
    }
}

fn known(value: &str) -> &str {
    match value {
        "idle" | "recording" | "starting" | "stopping" | "transcribing" | "paused" => value,
        _ => "other",
    }
}
