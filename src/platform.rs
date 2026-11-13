//! OS boundary. Unsupported capabilities fail closed with a content-free code.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::*;

pub const UNAVAILABLE: &str = "UNAVAILABLE_ON_PLATFORM";
pub fn worker_command(
    program: impl AsRef<std::ffi::OsStr>,
) -> std::io::Result<std::process::Command> {
    #[cfg(target_os = "linux")]
    {
        sidenote_sync::tree_worker::command(program.as_ref())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(std::process::Command::new(program))
    }
}
/// Call before configuring stdio; retain the original argv, cwd and explicit environment.
pub fn contain_command(command: &mut std::process::Command) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let args: Vec<_> = command.get_args().map(std::ffi::OsStr::to_owned).collect();
        let env: Vec<_> = command
            .get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(std::ffi::OsStr::to_owned)))
            .collect();
        let cwd = command.get_current_dir().map(std::path::Path::to_owned);
        let mut replacement = worker_command(command.get_program())?;
        replacement.args(args);
        if let Some(cwd) = cwd {
            replacement.current_dir(cwd);
        }
        for (key, value) in env {
            if let Some(value) = value {
                replacement.env(key, value);
            } else {
                replacement.env_remove(key);
            }
        }
        *command = replacement;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
    Ok(())
}
pub fn unavailable() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, UNAVAILABLE)
}
pub fn require_process_support() -> std::io::Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(unavailable())
    }
}
pub fn config_home() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        data_home()
    }
    #[cfg(not(windows))]
    {
        xdg("XDG_CONFIG_HOME", ".config")
    }
}
pub fn asr_library_name() -> &'static str {
    #[cfg(windows)]
    {
        "onnxruntime.dll"
    }
    #[cfg(not(windows))]
    {
        "libonnxruntime.so"
    }
}

#[cfg(windows)]
pub use std::os::windows::process::CommandExt;
#[cfg(not(windows))]
pub trait CommandExt {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}
#[cfg(not(windows))]
impl CommandExt for std::process::Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        #[cfg(target_os = "linux")]
        sidenote_sync::containment::configure(self);
        self
    }
}

pub fn data_home() -> std::path::PathBuf {
    #[cfg(test)]
    {
        crate::test_home().join("appdata")
    }
    #[cfg(all(windows, not(test)))]
    {
        std::env::var_os("APPDATA")
            .map(Into::into)
            .unwrap_or_default()
    }
    #[cfg(all(not(windows), not(test)))]
    {
        xdg("XDG_DATA_HOME", ".local/share")
    }
}
pub fn local_home() -> std::path::PathBuf {
    #[cfg(test)]
    {
        crate::test_home().join("localappdata")
    }
    #[cfg(all(windows, not(test)))]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(Into::into)
            .unwrap_or_default()
    }
    #[cfg(all(not(windows), not(test)))]
    {
        xdg("XDG_STATE_HOME", ".local/state")
    }
}
pub fn home() -> std::path::PathBuf {
    #[cfg(test)]
    {
        crate::test_home().join("home")
    }
    #[cfg(not(test))]
    {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(Into::into)
            .unwrap_or_default()
    }
}
#[cfg(not(windows))]
pub fn xdg(variable: &str, fallback: &str) -> std::path::PathBuf {
    std::env::var_os(variable)
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = home();
            if home.is_absolute() {
                home.join(fallback)
            } else {
                std::path::PathBuf::from("/nonexistent")
            }
        })
}
pub fn linked(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
