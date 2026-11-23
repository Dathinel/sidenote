//! The recording check on Download, Resume and the 4-pack queue. Hex is a fake status that the
//! test changes between clicks; nothing here reaches the real Hex or the network.
use super::*;
use sidenote::hex_engine::Recording;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

type Live = Arc<Mutex<Result<EngineState, HexError>>>;

pub(super) fn engine(meeting_state: &str, recording: bool) -> EngineState {
    EngineState {
        labs: Default::default(),
        running: true,
        version: "test".into(),
        protocol: 4,
        recording: recording.then(|| Recording {
            id: Some("fixture".into()),
            title: None,
            since: None,
            elapsed_seconds: Some(5),
        }),
        meeting_state: meeting_state.into(),
        dictating: Some(false),
        capture_state: "idle".into(),
        model: serde_json::from_str("{}").unwrap(),
    }
}

pub(super) fn always_idle() -> Arc<dyn Fn() -> Result<EngineState, HexError> + Send + Sync> {
    Arc::new(|| Ok(engine("idle", false)))
}

fn live(status: Result<EngineState, HexError>) -> Live {
    Arc::new(Mutex::new(status))
}
fn set(live: &Live, status: Result<EngineState, HexError>) {
    *live.lock().unwrap() = status;
}
fn recording() -> Result<EngineState, HexError> {
    Ok(engine("recording", true))
}

const WAIT: Duration = Duration::from_secs(5);

struct Fixture {
    runtime: rune::Runtime<State>,
    installs: mpsc::Receiver<&'static str>,
    live: Live,
    root: tempfile::TempDir,
}
fn fixture(status: Result<EngineState, HexError>, prepare: impl FnOnce(&Path)) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    prepare(root.path());
    let mut state = State::new();
    state.config = sidenote::core::Config::default();
    state.config.features.indic_engine = true;
    // This fixture also exercises the download guard on Linux.
    state.config.labs.after_stop_language_routing = true;
    state.config.pipeline.profile = "experimental".into();
    state.models = Models::load(root.path(), &state.config);
    state.models.languages = vec!["en".into(), "hi".into()];
    let (tx, installs) = mpsc::channel();
    state.models.install_fixture = Some(tx);
    let live = live(status);
    let read = live.clone();
    state.models.status_fixture = Some(Arc::new(move || read.lock().unwrap().clone()));
    let runtime = settings_models_runtime(state);
    Fixture {
        runtime,
        installs,
        live,
        root,
    }
}

/// Applies UI events until the worker of `id` reports back, and returns its result.
fn finish(state: &mut State, id: &str) -> std::io::Result<()> {
    loop {
        let event = state.events_rx.recv_timeout(WAIT).unwrap();
        let done = match &event {
            Event::Models(Msg::Finished(done, result)) if *done == id => Some(match result {
                Ok(()) => Ok(()),
                Err(error) => Err(std::io::Error::new(error.kind(), error.to_string())),
            }),
            _ => None,
        };
        state.apply_event(event);
        if let Some(result) = done {
            return result;
        }
    }
}

const HINDI: &str = "indicconformer-hi-ctc";

fn click_download(fixture: &mut Fixture) {
    // Redraw first so the button reflects the last event, as it does on screen.
    fixture.runtime.invalidate();
    fixture.runtime.frame(0.01);
    assert!(click_model_control(
        &mut fixture.runtime,
        &format!("model-download-{HINDI}")
    ));
}

#[test]
fn recording_at_click_refuses_download_and_resume_and_leaves_the_partial() {
    let part = |root: &Path| {
        let dir = root.join("models").join(HINDI);
        std::fs::create_dir_all(&dir).unwrap();
        let name = model_catalog::model(HINDI).unwrap().files[0].name;
        std::fs::write(dir.join(format!("{name}.part")), vec![7u8; 1000]).unwrap();
    };
    let mut f = fixture(recording(), part);
    assert!(
        f.runtime.state.models.partial.contains(HINDI),
        "Resume is offered"
    );
    click_download(&mut f);
    assert_eq!(
        finish(&mut f.runtime.state, HINDI).unwrap_err().to_string(),
        RECORDING_LINE
    );
    assert_eq!(f.runtime.state.models.errors[HINDI], RECORDING_LINE);
    assert!(f.installs.try_recv().is_err(), "nothing started");
    assert!(f.runtime.state.models.jobs.is_empty());
    assert_eq!(
        f.runtime.state.models.store.stored_bytes(HINDI).unwrap(),
        1000
    );
    // A search that matches nothing must not hide the card that says why.
    assert_card_listed_under_unmatched_search(&mut f, HINDI);

    // Nothing retries by itself and nothing stays blocked: the next click starts it.
    f.runtime.state.hex.shell.model_query.text.clear();
    set(&f.live, Ok(engine("idle", false)));
    click_download(&mut f);
    assert_eq!(f.installs.recv_timeout(WAIT).unwrap(), HINDI);
    finish(&mut f.runtime.state, HINDI).unwrap();
    assert!(!f.runtime.state.models.errors.contains_key(HINDI));
}

#[test]
fn unreadable_hex_state_refuses_with_its_own_line() {
    for status in [
        Err(HexError::HexNotRunning),
        Err(HexError::Timeout),
        Ok(engine("not-a-state", false)),
    ] {
        let mut f = fixture(status, |_| {});
        click_download(&mut f);
        assert_eq!(
            finish(&mut f.runtime.state, HINDI).unwrap_err().to_string(),
            UNKNOWN_LINE
        );
        assert_eq!(f.runtime.state.models.errors[HINDI], UNKNOWN_LINE);
        assert!(f.installs.try_recv().is_err(), "nothing started");
    }
}

fn assert_card_listed_under_unmatched_search(f: &mut Fixture, id: &str) {
    f.runtime.state.hex.shell.model_query.text = "unmatched search".into();
    f.runtime.invalidate();
    f.runtime.frame(0.01);
    assert!(
        f.runtime.find(&format!("speech-model-{id}")).is_some(),
        "{id}"
    );
}

#[test]
fn busy_hex_states_refuse_with_the_recording_line() {
    for state in ["paused", "stopping", "finishing"] {
        let mut f = fixture(Ok(engine(state, false)), |_| {});
        click_download(&mut f);
        let line = finish(&mut f.runtime.state, HINDI).unwrap_err().to_string();
        assert_eq!(line, RECORDING_LINE, "{state}");
        assert!(f.installs.try_recv().is_err(), "{state} started");
    }
}

#[test]
fn every_finished_hex_state_starts_the_download() {
    let mut f = fixture(recording(), |_| {});
    f.runtime.state.models.status_fixture = Some(always_idle());
    click_download(&mut f);
    assert_eq!(f.installs.recv_timeout(WAIT).unwrap(), HINDI);
    finish(&mut f.runtime.state, HINDI).unwrap();
    assert!(f.runtime.state.models.errors.is_empty());
    for state in ["idle", "stopped", "complete", "finished", "transcribing"] {
        let mut f = fixture(Ok(engine(state, false)), |_| {});
        click_download(&mut f);
        assert_eq!(f.installs.recv_timeout(WAIT).unwrap(), HINDI, "{state}");
        finish(&mut f.runtime.state, HINDI).unwrap();
        assert!(f.runtime.state.models.errors.is_empty(), "{state}");
    }
}

/// Writes what a finished install leaves: every file at its size, then the receipt last.
fn plant_receipt(root: &Path, id: &str) {
    let m = model_catalog::model(id).unwrap();
    let dir = root.join("models").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    for file in m.files {
        std::fs::File::create(dir.join(file.name))
            .unwrap()
            .set_len(file.bytes)
            .unwrap();
    }
    let expected: Vec<_> = m
        .files
        .iter()
        .map(|f| (f.name, f.sha256, f.bytes))
        .collect();
    std::fs::write(
        dir.join("installed.json"),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();
}

#[test]
fn recording_between_pack_one_and_two_stops_the_queue_and_keeps_pack_one() {
    let mut f = fixture(Ok(engine("idle", false)), |_| {});
    f.runtime.state.download_cpu_packs();
    let [first, second, ..] = model_catalog::CPU_LANGUAGE_PACKS;
    assert_eq!(f.installs.recv_timeout(WAIT).unwrap(), first);

    // Pack 1 finishes with its receipt, then a recording starts before pack 2 begins.
    plant_receipt(f.root.path(), first);
    set(&f.live, recording());
    finish(&mut f.runtime.state, first).unwrap();
    assert_eq!(
        finish(&mut f.runtime.state, second)
            .unwrap_err()
            .to_string(),
        RECORDING_LINE
    );

    assert!(f.installs.try_recv().is_err(), "pack 2 never started");
    assert!(
        f.runtime.state.models.pack_queue.is_empty(),
        "the queue stopped"
    );
    assert!(f.runtime.state.models.jobs.is_empty());
    assert_eq!(f.runtime.state.models.errors[second], RECORDING_LINE);
    assert!(
        f.runtime.state.models.installed.contains(first),
        "pack 1 receipt kept"
    );
    assert!(
        f.runtime
            .state
            .models
            .store
            .installed_paths(first)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        f.runtime.state.models.store.stored_bytes(second).unwrap(),
        0
    );
}
