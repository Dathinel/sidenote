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
pub fn unavailable() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, UNAVAILABLE)
}
pub fn require_process_support() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        Ok(())
    }
    #[cfg(not(windows))]
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
        use std::os::unix::process::CommandExt;
        self.process_group(0)
    }
}

pub fn data_home() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(Into::into)
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        xdg("XDG_DATA_HOME", ".local/share")
    }
}
pub fn local_home() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(Into::into)
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        xdg("XDG_STATE_HOME", ".local/state")
    }
}
pub fn home() -> std::path::PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(Into::into)
        .unwrap_or_default()
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
