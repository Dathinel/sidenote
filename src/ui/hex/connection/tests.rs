use super::*;
use crate::ui::{
    State,
    hex::{self, Capabilities, Feature, Msg, Support},
    theme,
};
use std::collections::VecDeque;

struct FakeControl {
    replies: VecDeque<Result<EngineState, HexError>>,
    now: Instant,
}
impl FakeControl {
    fn poll(&mut self, state: &mut State) {
        // Same status-apply entry as Msg::Status, with a deterministic clock and fake client.
        state.hex_status_at(self.replies.pop_front().unwrap(), self.now);
        self.now += Duration::from_secs(2);
    }
}
fn setup(
    errors: impl IntoIterator<Item = Result<EngineState, HexError>>,
    recording: bool,
) -> (State, FakeControl) {
    let mut state = State::new();
    state.hex.caps_version = Some("test".into());
    state.hex.caps = Some(Capabilities::from([
        (Feature::RecordStart, Support::Supported),
        (Feature::RecordStop, Support::Supported),
    ]));
    let good = EngineState {
        running: true,
        version: "test".into(),
        protocol: 4,
        recording: recording.then_some(Recording {
            id: Some("100-1".into()),
            title: None,
            since: None,
            elapsed_seconds: Some(1),
        }),
        meeting_state: "recording".into(),
        dictating: Some(false),
        capture_state: "idle".into(),
        model: hex::ModelState {
            id: None,
            name: None,
            language: None,
            installed: None,
            preparing: None,
            downloaded_bytes: None,
        },
        labs: Default::default(),
    };
    let replies = std::iter::once(Ok(good)).chain(errors).collect();
    (
        state,
        FakeControl {
            replies,
            // Keep real scheduling delays out of the fake clock.
            now: Instant::now() + Duration::from_secs(3600),
        },
    )
}

#[test]
fn timeout_and_transport_keep_recording_until_recovery() {
    for failure in [
        HexError::Timeout,
        HexError::Failed {
            code: None,
            stderr_summary: "synthetic",
        },
    ] {
        let (mut state, mut client) = setup([Err(failure)], true);
        client.poll(&mut state);
        let good = state.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
        assert!(state.hex_running());
        assert!(state.hex_recording());
        client.poll(&mut state);
        assert!(state.hex_running());
        assert!(state.hex_recording());
        assert_eq!(hex::stop_blocked(&state), None);
        assert_eq!(hex::start_blocked(&state), Some(SLOW));
        assert!(
            hex::indicator(&state, theme::palette(false))
                .1
                .starts_with(SLOW)
        );
        client.replies.push_back(Ok(good));
        client.poll(&mut state);
        assert!(state.hex_running());
        assert!(state.hex_recording());
        assert!(
            !hex::indicator(&state, theme::palette(false))
                .1
                .contains(SLOW)
        );
        let mut idle = state.hex.status.as_ref().unwrap().as_ref().unwrap().clone();
        idle.recording = None;
        client.replies.push_back(Ok(idle));
        client.poll(&mut state);
        assert!(
            !state.hex_recording(),
            "a fresh idle answer clears the recording"
        );
    }
}

#[test]
fn not_running_clears_immediately_and_initial_timeout_never_says_open_hex() {
    let (mut state, mut client) = setup([Err(HexError::HexNotRunning)], true);
    client.poll(&mut state);
    client.poll(&mut state);
    assert!(!state.hex_running());
    assert!(!state.hex_recording());
    // Exercise the ordinary installed case independently of this laptop's installation.
    state.hex.installation = sidenote::hex_install::Installation::Incomplete;
    assert_eq!(
        hex::start_blocked(&state),
        Some("Open Hex, then try New meeting again.")
    );
    state.hex_apply(Msg::Status(Err(HexError::Timeout)));
    assert_eq!(hex::start_blocked(&state), Some(SLOW));
    assert_eq!(hex::indicator(&state, theme::palette(false)).1, SLOW);
}

#[test]
fn stale_bound_expires_availability_but_keeps_stop() {
    let (mut state, mut client) = setup(
        [
            Err(HexError::Timeout),
            Err(HexError::Timeout),
            Err(HexError::HexNotRunning),
        ],
        true,
    );
    client.poll(&mut state);
    client.now += STALE_LIMIT - Duration::from_secs(2);
    client.poll(&mut state);
    assert!(state.hex_running(), "last good is valid at the bound");
    client.poll(&mut state);
    assert!(
        !state.hex_running(),
        "expired status must not claim availability"
    );
    assert_eq!(hex::start_blocked(&state), Some(SLOW));
    assert_eq!(hex::stop_blocked(&state), None);
    client.poll(&mut state);
    assert!(!state.hex_recording());
    assert!(hex::stop_blocked(&state).is_some());
}
