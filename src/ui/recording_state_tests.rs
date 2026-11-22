//! The recording state of the meeting page, through the real page: the pause control, the live
//! dot and the line over the notes. Each test runs in its own process on an isolated data
//! folder, with invented meeting text only. Nothing here talks to Hex.
use super::*;
use rune::{Env, Runtime, text::TextSystem};
use sidenote::core::meeting_parts::{self, Part, Phase as SavedPhase};
use sidenote::hex_engine::{EngineState, ModelState, Recording};

const MEETING: &str = "1800000000000-1";
const OTHER: &str = "1800000000002-2";

/// Relaunches the named test in a child process; true in the parent, which has then checked it.
fn in_parent(name: &str) -> bool {
    const FLAG: &str = "SIDENOTE_RECORDING_STATE_TEST";
    if std::env::var(FLAG).ok().as_deref() == Some(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("ui::recording_state_tests::{name}"),
            "--nocapture",
        ])
        .env(FLAG, name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// The saved-meeting page over two invented meetings. The first is held in memory by a live scene
/// in `phase`, so no press reaches Hex. No parts file is written: a held meeting leaves the list.
fn page(phase: SavedPhase, size: (f32, f32)) -> Runtime<State> {
    crate::test_paths::isolate();
    for id in [MEETING, OTHER] {
        let folder = core::meetings_dir().join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let summary = serde_json::json!({
            "title": "Beta launch plan",
            "context": ["The team met to plan the beta launch."],
            "topics": [{"title": "Launch", "points": [{"type": "decided", "text": "Ship the beta on Friday."}]}],
        });
        std::fs::write(folder.join("english-summary.json"), summary.to_string()).unwrap();
    }
    let parts = meeting_parts::Meeting {
        id: MEETING.into(),
        parts: vec![Part {
            id: MEETING.into(),
            started_ms: 1800000000000,
            duration_ms: 724000,
        }],
        phase,
        ..Default::default()
    };
    let mut state = State::new();
    if phase == SavedPhase::Recording {
        let mut live = live::Live::scene(live::Phase::Recording, None);
        live.session = Some(parts);
        state.live = Some(live);
    }
    let mut rt = Runtime::with_text(
        state,
        view,
        size,
        Env::default(),
        TextSystem::new(theme::fonts(), (theme::SANS, "Inter")),
    );
    rt.state.selected = Some(MEETING.into());
    rt.state.load_saved();
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn element(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn tap(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.2);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: rune::Mods::default(),
    });
    rt.frame(0.2);
}

/// Hex says one meeting records, so the top right offers the way back and Stop.
fn hex_records(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        s.hex.status = Some(Ok(EngineState {
            running: true,
            version: "test".into(),
            protocol: 4,
            recording: Some(Recording {
                id: Some(MEETING.into()),
                title: None,
                since: None,
                elapsed_seconds: Some(724),
            }),
            meeting_state: "recording".into(),
            dictating: Some(false),
            capture_state: "idle".into(),
            model: ModelState {
                id: None,
                name: None,
                language: None,
                installed: None,
                preparing: None,
                downloaded_bytes: None,
            },
            labs: Default::default(),
        }));
    });
    rt.frame(0.2);
}

#[test]
fn pause_is_two_bars_apart_from_stop_and_works_by_mouse_and_keyboard() {
    if in_parent("pause_is_two_bars_apart_from_stop_and_works_by_mouse_and_keyboard") {
        return;
    }
    // The test environment draws the dark theme.
    let p = theme::palette(true);
    let mut rt = page(SavedPhase::Recording, DEFAULT_SIZE);
    hex_records(&mut rt);
    // The control is chip filled; the button is 28 round on the page colour, with two 3 x 9 bars.
    assert_eq!(element(&rt, "meeting-recording-control").fill, p.chip);
    let pause = element(&rt, "meeting-pause");
    assert_eq!(
        (pause.rect[2], pause.rect[3], pause.radius),
        (28.0, 28.0, 14.0)
    );
    assert_eq!(pause.fill, p.bg);
    assert_eq!(
        (pause.role, pause.label.as_deref()),
        ("button", Some("Pause recording"))
    );
    for key in ["meeting-pause-bar-0", "meeting-pause-bar-1"] {
        let bar = element(&rt, key);
        assert_eq!((bar.rect[2], bar.rect[3]), (3.0, 9.0), "{key}");
        assert!(
            bar.path.starts_with(&pause.path),
            "{key} sits in the button"
        );
    }
    assert!(rt.find("meeting-pause-square").is_none());
    // Stop is a different control with a different name, and it is on the other pages.
    assert!(rt.find("hex-stop").is_none());
    rt.update(|s, _| {
        s.selected = Some(OTHER.into());
        s.load_saved();
    });
    rt.frame(0.2);
    let stop = element(&rt, "hex-stop");
    let stop_name = stop.label.unwrap();
    assert!(stop_name.starts_with("Stop"), "{stop_name}");
    assert!(!stop_name.contains("Pause"), "{stop_name}");
    assert!(rt.find("meeting-pause").is_none() && rt.find("meeting-pause-bar-0").is_none());
    rt.update(|s, _| {
        s.selected = Some(MEETING.into());
        s.load_saved();
    });
    rt.frame(0.2);
    // Mouse: a press reaches the pause command, which asks Hex and is told it is not running.
    assert!(rt.state.hex.error.is_none());
    rt.update(|s, _| s.hex.status = None);
    tap(&mut rt, "meeting-pause");
    assert!(
        rt.state.hex.error.is_some(),
        "mouse press did not reach Pause"
    );
    // Keyboard: the button takes focus with a visible ring and Space presses it. Enter presses
    // only rows and links in the toolkit, so it is not part of the contract for a button.
    rt.update(|s, _| s.hex.error = None);
    rt.update(|_, tx| tx.focus("meeting-pause", true));
    rt.frame(0.0);
    let pause = element(&rt, "meeting-pause");
    assert!(pause.focusable && pause.focused && pause.focus_visible);
    press(&mut rt, Key::Text(" ".into()));
    assert!(rt.state.hex.error.is_some(), "Space did not reach Pause");
}
