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
