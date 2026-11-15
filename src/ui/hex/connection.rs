use super::{EngineState, HexError, Instant, Recording};
use std::{cell::Cell, time::Duration};

// 10 s covers the lead's 2.694 s CLI delay and several 1.5 s + 750 ms status polls.
const STALE_LIMIT: Duration = Duration::from_secs(10);
pub(super) const SLOW: &str = "Hex is slow to answer, trying again";

#[derive(Default)]
pub(super) struct Connection {
    good: Option<EngineState>,
    good_at: Option<Instant>,
    sampled_at: Option<Instant>,
    phase: &'static str,
    blocked: Cell<Option<&'static str>>,
}

impl Connection {
    fn age(&self) -> Duration {
        self.good_at.map_or(Duration::ZERO, |at| {
            at.elapsed()
                .max(self.sampled_at.unwrap_or(at).saturating_duration_since(at))
        })
    }

    pub(super) fn state(&self) -> Option<&EngineState> {
        (self.phase == "stale" && self.age() <= STALE_LIMIT)
            .then_some(self.good.as_ref())
            .flatten()
    }

    // An expired observation cannot confirm availability, but must leave Stop accessible.
    pub(super) fn recording(&self) -> Option<&Recording> {
        self.good
            .as_ref()
            .and_then(|state| state.recording.as_ref())
    }

    pub(super) fn slow(&self, status: &Option<Result<EngineState, HexError>>) -> bool {
        matches!(
            status,
            Some(Err(HexError::Timeout | HexError::Failed { .. }))
        )
    }

    pub(super) fn message(&self) -> String {
        match self.good_at {
            Some(_) => format!("{SLOW} (last answer {} s ago)", self.age().as_secs()),
            None => SLOW.into(),
        }
    }

    pub(super) fn apply(
        &mut self,
        previous: &Option<Result<EngineState, HexError>>,
        result: &Result<EngineState, HexError>,
        now: Instant,
    ) {
        let interval = self
            .sampled_at
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.sampled_at = Some(now);
        let (phase, reason) = match result {
            Ok(_) => ("ok", "answer"),
            Err(HexError::HexNotRunning) => ("not_running", "not_running"),
            Err(HexError::Unsupported) => ("unavailable", "unsupported"),
            Err(error) => {
                if let Some(Ok(state)) = previous {
                    self.good = Some(state.clone());
                }
                (
                    if self.good_at.is_some() && self.age() > STALE_LIMIT {
                        "expired"
                    } else {
                        "stale"
                    },
                    if matches!(error, HexError::Timeout) {
                        "timeout"
                    } else {
                        "transport"
                    },
                )
            }
        };
        if phase != self.phase {
            sidenote::listener::log(&format!(
                "hex connection {} -> {phase} reason={reason} last_ok_age_ms={} sample_interval_ms={}",
                self.phase,
                self.age().as_millis(),
                interval.as_millis()
            ));
        }
        self.phase = phase;
        match result {
            Ok(_) => {
                self.good = None;
                self.good_at = Some(now);
            }
            Err(HexError::HexNotRunning | HexError::Unsupported) => {
                self.good = None;
                self.good_at = None;
            }
            Err(_) => {}
        }
    }

    pub(super) fn log_blocked(&self, reason: Option<&'static str>) {
        if self.blocked.replace(reason) != reason
            && let Some(reason) = reason
        {
            sidenote::listener::log(&format!(
                "hex new_meeting blocked reason={reason} last_ok_age_ms={}",
                self.age().as_millis()
            ));
        }
    }
}

#[cfg(test)]
mod tests;
