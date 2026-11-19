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
    store.install_files(id, cancel, !cache_only, progress)
}
