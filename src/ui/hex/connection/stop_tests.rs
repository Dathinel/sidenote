use super::super::*;
mod api;
use api::Fixture;

#[test]
fn stale_stop_refuses_replacement_recording_through_ui_and_engine() {
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().id = Some("200-2");
    Fixture::stop(&mut state);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 0, "must not send generic Stop for replacement B");
    assert_eq!(id, Some("200-2"), "B must keep recording");
    assert!(state.hex.error.unwrap().contains("changed or ended"));
}

#[test]
fn stale_stop_uses_command_budget_with_cached_capabilities() {
    let (api, mut state, _) = Fixture::start();
    api.data.lock().unwrap().delay = Duration::from_millis(1000);
    Fixture::stop(&mut state);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 1, "Stop must survive a status slower than 750 ms");
    assert_eq!(id, None);
    assert!(state.hex.error.is_none());
}

#[test]
fn stop_keeps_the_identity_captured_by_the_rendered_button() {
    let (api, mut state, _) = Fixture::start();
    let displayed = recording_id(&state);
    api.data.lock().unwrap().id = Some("200-2");
    let newer = state.hex.engine.status_for_command().unwrap();
    state.hex_apply(Msg::Status(Ok(newer)));
    state.hex_record_stop(displayed);
    let (stops, id) = {
        let data = api.data.lock().unwrap();
        (data.stops, data.id)
    };
    assert_eq!(stops, 0);
    assert_eq!(id, Some("200-2"));
    assert!(state.hex.error.unwrap().contains("changed before Stop"));
}

#[test]
fn expired_recording_is_unknown_and_new_meeting_rechecks_then_starts() {
    let (api, mut state, now) = Fixture::start();
    state.hex_status_at(Err(HexError::Timeout), now + Duration::from_secs(11));
    assert_eq!(
        crate::ui::nav::recording_words(&state, false),
        "Recording status unknown"
    );
    assert!(
        indicator(&state, crate::ui::theme::palette(false))
            .1
            .contains("status unknown")
    );
    assert_eq!(
        start_blocked(&state),
        None,
        "stale observations must not permanently block New meeting"
    );
    assert_eq!(stop_blocked(&state), None);
    {
        let mut data = api.data.lock().unwrap();
        data.id = None;
        data.delay = Duration::from_millis(1000);
    }
    state.new_meeting();
    Fixture::finish(&mut state);
    let starts = api.data.lock().unwrap().starts;
    assert_eq!(
        starts, 1,
        "New meeting must recheck, then send Start after a confirmed idle reply"
    );
    assert!(state.hex.error.is_none());
}

#[test]
fn successful_snapshot_expires_without_another_poll() {
    let (_api, mut state, _) = Fixture::start();
    let good = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(good), Instant::now() - Duration::from_secs(11));
    assert!(state.hex_status_unknown());
    assert!(!state.hex_running());
    assert_eq!(
        crate::ui::nav::recording_words(&state, false),
        "Recording status unknown"
    );
    assert_eq!(start_blocked(&state), None);
    assert_eq!(stop_blocked(&state), None);
}
