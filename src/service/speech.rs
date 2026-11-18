//! Speech owns a separate service lifetime, never the ordinary routing supervisor.
use std::io;
mod events;
mod listener;
mod routes;

pub(super) fn run() -> i32 {
    let result = (|| -> io::Result<()> {
        if super::role() != super::DeviceRole::Server || unsafe { libc::geteuid() } == 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        sidenote_sync::tree_worker::speech_command(std::ffi::OsStr::new("/bin/true"))?;
        unsafe { libc::umask(0o077) };
        let _lock = crate::core::store_io::lock(
            &crate::core::state_dir().join("local/speech/service.lock"),
        )?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let (socket, router) = listener::bind()?;
            let (stop, stopped) = std::sync::mpsc::channel::<()>();
            let mut jobs = tokio::task::spawn_blocking(move || {
                let store = crate::core::speech_sessions::Store(
                    crate::core::state_dir().join("local/speech"),
                );
                #[cfg(feature = "asr-runtime")]
                let mut config = crate::core::Config::default();
                #[cfg(feature = "asr-runtime")]
                {
                    config.pipeline.model_state_dir = crate::core::state_dir();
                    config.pipeline.runtime_dll = listener::runtime_library()?;
                }
                loop {
                    if crate::core::model_catalog::ModelStore::default()
                        .installed_paths("silero-vad")?
                        .is_none()
                    {
                        if stopped.recv_timeout(std::time::Duration::from_secs(1))
                            != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        {
                            break;
                        }
                        continue;
                    }
                    store
                        .tick(|id, piece, session| {
                            #[cfg(feature = "asr-runtime")]
                            {
                                crate::core::speech_jobs::process(
                                    &store, id, piece, session, &config,
                                )
                            }
                            #[cfg(not(feature = "asr-runtime"))]
                            {
                                let _ = (id, piece, session);
                                Err("ASR_UNAVAILABLE".into())
                            }
                        })
                        .map_err(|_| io::Error::other("SPEECH_STORAGE"))?;
                    if stopped.recv_timeout(std::time::Duration::from_secs(1))
                        != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
                Ok::<_, io::Error>(())
            });
            let result = tokio::select! {
                result = events::serve(socket, router) => result,
                _ = terminate.recv() => Ok(()),
                result = tokio::signal::ctrl_c() => result,
                result = &mut jobs => return result?,
            };
            crate::core::speech_jobs::STOP.store(true, std::sync::atomic::Ordering::Release);
            drop(stop);
            jobs.await??;
            result
        })
    })();
    if result.is_err() {
        eprintln!("SPEECH_SERVICE_UNAVAILABLE");
        return 2;
    }
    0
}
