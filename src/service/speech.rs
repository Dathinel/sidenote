//! Speech owns a separate service lifetime, never the ordinary routing supervisor.
use std::io;

pub(super) fn run() -> i32 {
    let result = (|| -> io::Result<()> {
        if super::role() != super::DeviceRole::Server || unsafe { libc::geteuid() } == 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        sidenote_sync::tree_worker::speech_command(std::ffi::OsStr::new("/bin/true"))?;
        unsafe { libc::umask(0o077) };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                _ = terminate.recv() => {},
                result = tokio::signal::ctrl_c() => { result?; },
            }
            Ok::<_, io::Error>(())
        })
    })();
    if result.is_err() {
        eprintln!("SPEECH_SERVICE_UNAVAILABLE");
        return 2;
    }
    0
}
