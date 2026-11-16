use super::super::*;
mod api;
use api::Fixture;

fn isolated_api_test(name: &str) -> bool {
    if std::env::var("SIDENOTE_HEX_API_TEST").as_deref() == Ok(name) {
        return false;
    }
    let root = tempfile::Builder::new()
        .prefix("live-replacement-")
        .tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts"))
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("ui::hex::connection::stop_tests::{name}"),
            "--nocapture",
        ])
        .env("SIDENOTE_HEX_API_TEST", name)
        .env("SIDENOTE_DATA_DIR", root.path().join("data"))
        .env("SIDENOTE_STATE_DIR", root.path().join("state"))
        .env("SIDENOTE_HEX_DIR", root.path().join("hex"))
        .env("SIDENOTE_PEERS_DIR", root.path().join("peers"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(
        command.status().unwrap().success(),
        "isolated Hex API test failed"
    );
    true
}

fn pending_notes(state: &mut State, api: &Fixture, text: &str) -> u64 {
    use crate::ui::live::Phase;
    state.live.as_mut().unwrap().phase = Phase::Making;
    api.data.lock().unwrap().id = None;
    let idle = state.hex.engine.status_for_command().unwrap();
    state.hex_status_at(Ok(idle), Instant::now() - Duration::from_secs(11));
    assert!(state.hex_status_unknown());
    state.live_notes = rune::TextEdit::new(text);
    state.live_tick();
    if !text.is_empty() {
        assert_eq!(
            state.live.as_ref().unwrap().saving,
            crate::ui::live::Saving::Pending
        );
    }
    api.data.lock().unwrap().id = Some("200-2");
    state.live.as_ref().unwrap().recording_id
}

#[test]
fn active_recheck_preserves_pending_notes() {
    if isolated_api_test("active_recheck_preserves_pending_notes") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "Synthetic notes inside autosave delay");
    state.new_meeting();
    Fixture::finish(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    let files: Vec<_> = std::fs::read_dir(recovery)
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        std::fs::read_to_string(files[0].path()).unwrap(),
        "Synthetic notes inside autosave delay"
    );
    assert_ne!(state.live.as_ref().unwrap().recording_id, old);
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Recording
    );
    assert!(
        state
            .hex
            .error
            .unwrap()
            .contains("No new meeting was started")
    );
}

#[test]
fn active_recheck_failed_save_keeps_pending_notes() {
    if isolated_api_test("active_recheck_failed_save_keeps_pending_notes") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "Synthetic notes on failed save");
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
    std::fs::write(&recovery, "synthetic blocker").unwrap();
    state.new_meeting();
    Fixture::finish(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    let live = state.live.as_ref().unwrap();
    assert_eq!(live.recording_id, old);
    assert_eq!(live.phase, crate::ui::live::Phase::Making);
    assert_eq!(state.live_notes.text, "Synthetic notes on failed save");
    assert!(live.close_failed);
    assert!(matches!(live.saving, crate::ui::live::Saving::Failed(_)));
    assert!(state.on_live());
}

#[test]
fn active_recheck_replaces_empty_page() {
    if isolated_api_test("active_recheck_replaces_empty_page") {
        return;
    }
    let (api, mut state, _) = Fixture::start();
    let old = pending_notes(&mut state, &api, "");
    state.new_meeting();
    Fixture::finish(&mut state);
    assert_eq!(api.data.lock().unwrap().starts, 0);
    assert_ne!(state.live.as_ref().unwrap().recording_id, old);
    assert_eq!(
        state.live.as_ref().unwrap().phase,
        crate::ui::live::Phase::Recording
    );
    assert!(!state.live.as_ref().unwrap().close_failed);
}

fn saved_removal_notes(text: &str) {
    let recovery = sidenote::core::state_dir().join("unsaved-notes");
    assert!(
        std::fs::read_dir(recovery)
            .unwrap()
            .any(|entry| { std::fs::read_to_string(entry.unwrap().path()).unwrap() == text }),
        "latest notes must be durable before removing or leaving the page"
    );
}

fn start_removal_cases(failed_start: bool) {
    use crate::ui::live::{Phase, Saving};
    for (text, fail_save) in [
        ("Synthetic pending Start notes", true),
        ("Synthetic pending Start notes", false),
        ("", false),
    ] {
        let (api, mut state, _) = Fixture::start();
        let (release, wait) = std::sync::mpsc::channel();
        {
            let mut data = api.data.lock().unwrap();
            data.id = None;
            data.start_failure = failed_start;
            data.start_records = false;
            data.start_release = Some(wait);
        }
        let idle = state.hex.engine.status_for_command().unwrap();
        state.hex_apply(Msg::Status(Ok(idle)));
        state.new_meeting();
        assert!(state.hex.record_busy);
        assert_eq!(state.live.as_ref().unwrap().phase, Phase::Starting);
        let page = state.live.as_ref().unwrap().recording_id;
        if !failed_start {
            release.send(()).unwrap();
            Fixture::finish(&mut state);
            assert!(state.hex.error.is_none());
            assert!(
                state.live.is_some(),
                "idle before START_WAIT must keep the page"
            );
            // Exercise the real elapsed-time condition without altering a production clock.
            thread::sleep(Duration::from_secs(26));
        }
        state.live_notes = rune::TextEdit::new(text);
        state.live_tick();
        if !text.is_empty() {
            assert_eq!(state.live.as_ref().unwrap().saving, Saving::Pending);
        }
        let recovery = sidenote::core::state_dir().join("unsaved-notes");
        if fail_save {
            std::fs::create_dir_all(recovery.parent().unwrap()).unwrap();
            std::fs::write(&recovery, "synthetic recovery blocker").unwrap();
        }
        if failed_start {
            release.send(()).unwrap();
            Fixture::finish(&mut state);
            assert!(
                state.hex.error.is_some(),
                "apply the real failed Start reply"
            );
        } else {
            let idle = state.hex.engine.status_for_command().unwrap();
            assert!(idle.recording.is_none());
            state.hex_apply(Msg::Status(Ok(idle)));
        }
        assert_eq!(api.data.lock().unwrap().starts, 1);
        if fail_save {
            let live = state.live.as_ref().expect("failed save must keep the page");
            assert_eq!(live.recording_id, page);
            assert_eq!(live.phase, Phase::Starting);
            assert_eq!(state.live_notes.text, text);
            assert!(live.close_failed);
            assert!(matches!(live.saving, Saving::Failed(_)));
            assert!(state.on_live());
            std::fs::remove_file(&recovery).unwrap();
            state.live_notes = rune::TextEdit::new("Synthetic notes edited after storage recovery");
            state.leave_live();
            assert!(!state.on_live(), "restored storage must permit navigation");
            saved_removal_notes("Synthetic notes edited after storage recovery");
        } else {
            assert!(state.live.is_none(), "saved or empty page may be removed");
            assert!(!state.on_live());
            if !text.is_empty() {
                saved_removal_notes(text);
            }
        }
    }
}

#[test]
fn start_timeout_preserves_pending_notes_before_removal() {
    if isolated_api_test("start_timeout_preserves_pending_notes_before_removal") {
        return;
    }
    start_removal_cases(false);
}

#[test]
fn failed_start_preserves_pending_notes_before_removal() {
    if isolated_api_test("failed_start_preserves_pending_notes_before_removal") {
        return;
    }
    start_removal_cases(true);
}

#[test]
fn stale_stop_refuses_replacement_recording_through_ui_and_engine() {
    if isolated_api_test("stale_stop_refuses_replacement_recording_through_ui_and_engine") {
        return;
    }
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
    if isolated_api_test("stale_stop_uses_command_budget_with_cached_capabilities") {
        return;
    }
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
    if isolated_api_test("stop_keeps_the_identity_captured_by_the_rendered_button") {
        return;
    }
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
    if isolated_api_test("expired_recording_is_unknown_and_new_meeting_rechecks_then_starts") {
        return;
    }
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
    if isolated_api_test("successful_snapshot_expires_without_another_poll") {
        return;
    }
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
