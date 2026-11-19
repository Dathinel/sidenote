//! Startup installs use the same receipts and pinned downloader as Settings.
use super::*;
use crate::core::Config;

pub const PACKS: [&str; 4] = [
    "silero-vad",
    "anuvani-lid-int8",
    "indicconformer-hi-ctc",
    "indicconformer-te-ctc",
];

pub fn enabled(config: &Config) -> bool {
    config.labs.after_stop_language_routing
        && config.pipeline.enabled()
        && config.pipeline.language_mode == "auto"
}

fn opt_out_path(store: &ModelStore, id: &str) -> PathBuf {
    store.root.join(format!("{id}.automatic-download-off"))
}

pub fn opt_out(store: &ModelStore, id: &str, off: bool) -> io::Result<()> {
    if !PACKS.contains(&id) {
        return Ok(());
    }
    reject_link(&store.root)?;
    let path = opt_out_path(store, id);
    if off {
        store_io::atomic_write(&path, b"off\n")
    } else {
        match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

pub fn next_pack(
    store: &ModelStore,
    config: &Config,
    failed: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    PACKS.into_iter().find(|id| {
        enabled(config)
            && !failed(id)
            && fs::symlink_metadata(opt_out_path(store, id))
                .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
            && !matches!(store.installed_paths(id), Ok(Some(_)))
    })
}

fn lock_path(store: &ModelStore) -> PathBuf {
    store.root.join("automatic-downloads.lock")
}

pub fn reserve(store: &ModelStore) -> io::Result<File> {
    store_io::lock(&lock_path(store))
}

pub fn recording_idle(state: &Path) -> bool {
    use chrono::{DateTime, Utc};
    use serde_json::Value;
    use std::io::{Seek, SeekFrom};
    let status = (|| {
        let mut journal = File::open(state.join("local/status.ndjson")).ok()?;
        let start = journal.metadata().ok()?.len().saturating_sub(2048);
        journal.seek(SeekFrom::Start(start)).ok()?;
        let mut tail = Vec::new();
        journal.take(4096).read_to_end(&mut tail).ok()?;
        let last = tail.trim_ascii_end().rsplit(|b| *b == b'\n').next()?;
        serde_json::from_slice::<Value>(last).ok()
    })();
    status.is_some_and(|entry| {
        let timestamp = entry["timestamp"]
            .as_str()
            .and_then(|text| text.parse::<DateTime<Utc>>().ok());
        timestamp.is_some_and(|at| (0..=5000).contains(&(Utc::now() - at).num_milliseconds()))
            && entry["protocol"] == 4
            && entry.get("status_error").is_some_and(Value::is_null)
            && matches!(
                entry["meeting_state"].as_str(),
                Some("idle" | "stopped" | "complete" | "finished" | "transcribing")
            )
    })
}

fn check_download(state: &Path, cancel: &AtomicBool, cache_only: bool) -> io::Result<()> {
    check_cancel(cancel)?;
    if !cache_only && !recording_idle(state) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Model setup paused until recording is idle",
        ));
    }
    Ok(())
}

pub(crate) fn ready_while_provisioning(store: &ModelStore) -> Option<u8> {
    if !File::open(lock_path(store))
        .is_err_and(|error| matches!(error.raw_os_error(), Some(32 | 33)))
    {
        return None;
    }
    let ready = PACKS
        .iter()
        .filter(|id| store.installed_paths(id).is_ok_and(|paths| paths.is_some()))
        .count();
    (ready < PACKS.len()).then_some(ready as u8)
}

pub fn install(
    store: &ModelStore,
    state: &Path,
    id: &str,
    cancel: &AtomicBool,
    cache_only: bool,
    progress: impl FnMut(Progress),
) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) } == 0 {
        return Err(io::Error::last_os_error());
    }
    store.install_files(
        id,
        &|| check_download(state, cancel, cache_only),
        !cache_only,
        progress,
    )
}

#[cfg(test)]
#[test]
fn provisioning_review_recording_during_read_pauses_the_hidden_transfer() {
    struct RecordingStarts<'a>(&'a Path);
    impl Read for RecordingStarts<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            write_status(self.0, "recording");
            b"new bytes".as_slice().read(buffer)
        }
    }
    fn write_status(root: &Path, state: &str) {
        let record = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(), "protocol": 4,
            "meeting_state": state, "status_error": null, "window_visible": false
        });
        fs::write(root.join("local/status.ndjson"), format!("{record}\n")).unwrap();
    }
    let state = tempfile::tempdir().unwrap();
    fs::create_dir(state.path().join("local")).unwrap();
    write_status(state.path(), "idle");
    let cancel = AtomicBool::new(false);
    let check = || check_download(state.path(), &cancel, false);
    let mut chunk = [0; 64];
    assert_eq!(
        read_download_chunk(&mut RecordingStarts(state.path()), &mut chunk, &check)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    write_status(state.path(), "idle");
    assert_eq!(
        read_download_chunk(&mut b"resume".as_slice(), &mut chunk, &check).unwrap(),
        6
    );
}
