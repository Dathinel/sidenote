#![cfg(windows)]
mod common;
use sidenote::core::{
    self,
    notes::queue,
    recording_source::{Session, folders::Discovery},
};
use std::{fs, sync::Mutex};

static SERIAL: Mutex<()> = Mutex::new(());

fn reads() -> u64 {
    use windows_sys::Win32::System::Threading::*;
    unsafe {
        let mut counters = std::mem::zeroed();
        assert_ne!(GetProcessIoCounters(GetCurrentProcess(), &mut counters), 0);
        counters.ReadTransferCount
    }
}

fn fixture(prefix: &str) -> Session<Discovery> {
    common::isolate();
    let state = core::state_dir();
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("config.json"),
        br#"{"version":1,"features":{},"notes":{"engine":"app","helper":null}}"#,
    )
    .unwrap();
    for index in 1..=2 {
        meeting(&format!("{prefix}-{index}"), false);
    }
    let mut source = Session::open(
        Discovery::new(core::meetings_dir()).unwrap(),
        state.join("local/recording-source.json"),
    )
    .unwrap();
    source
        .drain(|meeting| queue::enqueue(meeting.id()))
        .unwrap();
    source
}

fn meeting(id: &str, short: bool) {
    let folder = core::meetings_dir().join(id);
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("manifest.json"),
        serde_json::json!({"status":"complete","duration_ms":if short { 1000 } else { 30000 }})
            .to_string(),
    )
    .unwrap();
    fs::write(folder.join("transcript.ndjson"), serde_json::json!({"text":if short { "Synthetic".into() } else { "Synthetic ".repeat(20_000) }}).to_string()).unwrap();
    if !short {
        fs::write(
            folder.join("english-summary.json"),
            b"{\"title\":\"Synthetic final output\"}",
        )
        .unwrap();
    }
}

#[test]
fn idle_cpu_completed_source_does_not_reread_acknowledged_transcripts() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut source = fixture("7100000000000");
    let before = reads();
    source.drain(|_| panic!("duplicate completion")).unwrap();
    let bytes = reads() - before;
    println!("acknowledged source idle read bytes={bytes}");
    assert!(
        bytes < 64 * 1024,
        "idle discovery reread historical transcript payloads: {bytes}"
    );
    meeting("7100000000000-3", false);
    let mut added = Vec::new();
    source
        .drain(|m| {
            added.push(m.id().to_owned());
            queue::enqueue(m.id())
        })
        .unwrap();
    assert_eq!(added, ["7100000000000-3"]);
}

#[test]
fn idle_cpu_notes_catch_up_skips_acknowledged_final_outputs_but_sees_new_meetings() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _source = fixture("7200000000000");
    let before = reads();
    queue::catch_up().unwrap();
    let bytes = reads() - before;
    println!("acknowledged notes idle read bytes={bytes}");
    assert!(
        bytes < 64 * 1024,
        "idle notes catch-up reread historical transcript payloads: {bytes}"
    );
    meeting("7200000000000-3", true);
    queue::catch_up().unwrap();
    assert_eq!(
        core::recording_guard::receipt(&core::state_dir(), "7200000000000-3"),
        core::recording_guard::Eligibility::Short
    );
    // Missing outputs must re-enter the original validation path even after acknowledgement.
    fs::remove_file(core::meetings_dir().join("7200000000000-1/english-summary.json")).unwrap();
    meeting("7200000000000-1", true);
    queue::catch_up().unwrap();
    assert_eq!(
        core::recording_guard::receipt(&core::state_dir(), "7200000000000-1"),
        core::recording_guard::Eligibility::Short
    );
    let cursor = core::state_dir().join("local/recording-source.json");
    let previous = fs::read(&cursor).unwrap();
    fs::write(&cursor, b"invalid synthetic cursor").unwrap();
    meeting("7200000000000-4", true);
    queue::catch_up().unwrap();
    fs::write(cursor, previous).unwrap();
    assert_eq!(
        core::recording_guard::receipt(&core::state_dir(), "7200000000000-4"),
        core::recording_guard::Eligibility::Short
    );
}
