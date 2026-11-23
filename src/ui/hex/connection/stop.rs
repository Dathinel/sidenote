use super::*;
use crate::ui::live::Phase;

const WAIT: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_millis(1500);

pub(super) struct Completion {
    recording: u64,
    previous: Option<Phase>,
    started: Instant,
    deadline: Option<Instant>,
    next_check: Instant,
}

impl State {
    pub(in crate::ui) fn hex_stop_pending(&self) -> bool {
        self.hex.stop.is_some()
    }

    pub(super) fn hex_stop_unconfirmed(&self) -> bool {
        self.hex
            .stop
            .as_ref()
            .is_some_and(|stop| stop.deadline.is_none())
    }

    pub(super) fn hex_begin_stop(&mut self, recording: u64, now: Instant) {
        let previous = self
            .live
            .as_mut()
            .map(|live| live.stop_phase(Phase::Stopping));
        self.hex.stop = Some(Completion {
            recording,
            previous,
            started: now,
            deadline: Some(now + WAIT),
            next_check: now,
        });
    }

    pub(super) fn hex_clear_stop(&mut self) -> bool {
        let Some(stop) = self.hex.stop.take() else {
            return false;
        };
        if let Some(live) = self
            .live
            .as_mut()
            .filter(|l| l.recording_id == stop.recording)
            && let Some(previous) = stop.previous
        {
            live.stop_phase(previous);
            return true;
        }
        false
    }

    pub(super) fn hex_stop_failed(&mut self, recording: u64, error: &HexError) {
        if let Some(live) = &mut self.live {
            super::super::response_time::failed(live.take_notes_timing(recording));
            live.generate_after_pause = false;
        }
        let refused = matches!(error, HexError::Unsupported)
            || matches!(error, HexError::Failed { code: Some(code), .. }
                if matches!(code.as_str(), "recording-changed" | "not-recording"));
        let transcribing =
            matches!(&self.hex.status, Some(Ok(s)) if s.meeting_state == "transcribing");
        if refused && !transcribing {
            self.hex_clear_stop();
        } else {
            self.hex_check_stop();
        }
    }

    pub(super) fn hex_check_stop(&mut self) {
        let Some(stop) = &mut self.hex.stop else {
            return;
        };
        stop.next_check = Instant::now() + CHECK_EVERY;
        let started = stop.started;
        // The watcher can deliver a sample taken before Stop replied.
        self.hex_spawn(move |engine| Msg::StopStatus(started, engine.status()));
    }

    pub(in crate::ui) fn hex_recheck_stop(&mut self) {
        if self.hex.record_busy || !self.hex_stop_unconfirmed() {
            return;
        }
        let stop = self.hex.stop.as_mut().unwrap();
        stop.started = Instant::now();
        stop.deadline = Some(stop.started + WAIT);
        if let Some(live) = &mut self.live {
            live.stop_phase(Phase::Stopping);
        }
        self.hex_check_stop();
    }

    pub(super) fn hex_stop_status(
        &mut self,
        started: Instant,
        result: Result<EngineState, HexError>,
    ) {
        if self.hex.stop.as_ref().is_none_or(|s| s.started != started) {
            return;
        }
        let terminal = result.as_ref().is_ok_and(|s| {
            s.recording.is_none()
                && matches!(
                    s.meeting_state.as_str(),
                    "idle" | "stopped" | "complete" | "finished"
                )
        });
        if terminal {
            self.hex_status(result);
            self.hex.error = None;
            if self.hex_clear_stop() {
                self.live_stopped();
                self.live_sync();
            } else {
                self.request_refresh();
            }
        }
    }

    pub(super) fn hex_stop_tick(&mut self, now: Instant, tx: &mut Tx) -> bool {
        let Some(stop) = &mut self.hex.stop else {
            return false;
        };
        let Some(deadline) = stop.deadline else {
            return false;
        };
        if now >= deadline {
            stop.deadline = None;
            if let Some(live) = &mut self.live {
                live.stop_phase(Phase::StopUnconfirmed);
            }
            return true;
        }
        let wake_at = if self.hex.record_busy {
            deadline
        } else if now >= stop.next_check {
            self.hex_check_stop();
            (now + CHECK_EVERY).min(deadline)
        } else {
            stop.next_check.min(deadline)
        };
        tx.wake_after(wake_at - now);
        false
    }
}
