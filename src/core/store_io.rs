//! Small JSON stores share flushed replacements and an OS-released writer lock.
use serde::{Serialize, de::DeserializeOwned};
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::{fs, io, io::Write, path::Path};

#[derive(Debug)]
pub(crate) struct RelayFailure(pub &'static str, pub Option<i32>, pub Option<&'static str>);
impl std::fmt::Display for RelayFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for RelayFailure {}
pub(crate) fn relay_reason(error: &io::Error) -> Option<&'static str> {
    let reason = error.get_ref()?.downcast_ref::<RelayFailure>()?.0;
    match reason {
        "relay_lock" | "relay_database" | "relay_decode" | "relay_setup" | "relay_io" => {
            Some(reason)
        }
        _ => None,
    }
}
pub(crate) fn os_code(error: &io::Error) -> Option<i32> {
    error
        .raw_os_error()
        .or_else(|| error.get_ref()?.downcast_ref::<RelayFailure>()?.1)
}

#[derive(Debug)]
pub(crate) struct UserMessage(pub String);
impl std::fmt::Display for UserMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for UserMessage {}
pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, UserMessage(message.into()))
}
pub(crate) fn message(error: &io::Error) -> Option<&str> {
    error
        .get_ref()?
        .downcast_ref::<UserMessage>()
        .map(|m| m.0.as_str())
}

/// Contended settings are deferred without replacing the last known state.
pub(crate) fn defer_busy<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn read<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match read_bounded(path, super::file_reads::SUMMARY_MAX) {
        Ok(bytes) => serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map_err(io::Error::from),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

struct Parent {
    path: std::path::PathBuf,
    _pins: Vec<fs::File>,
}
#[cfg(windows)]
fn checked_parent(path: &Path) -> io::Result<Parent> {
    use std::os::windows::fs::MetadataExt;
    let parent = std::path::absolute(path.parent().ok_or_else(|| invalid("Missing parent"))?)?;
    let mut pins = Vec::new();
    for p in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if let Err(e) = fs::symlink_metadata(p) {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e);
            }
            match fs::create_dir(p) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(p)?;
        let m = file.metadata()?;
        if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unsafe target directory",
            ));
        }
        pins.push(file);
    }
    Ok(Parent {
        path: parent,
        _pins: pins,
    })
}
#[cfg(windows)]
pub(crate) fn lock(path: &Path) -> io::Result<fs::File> {
    let _parent = checked_parent(path)?;
    for attempt in 0..5 {
        match fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(0x00200000)
            .open(path)
        {
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) && attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Ok(file) => {
                if !super::file_reads::regular(&file.metadata()?) {
                    return Err(invalid("Unsafe lock file"));
                }
                return Ok(file);
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}
pub(crate) fn write(path: &Path, value: &impl Serialize) -> io::Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
struct Temp {
    path: std::path::PathBuf,
    file: Option<fs::File>,
    _parent: Parent,
}
impl Drop for Temp {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
#[cfg(windows)]
fn temporary(path: &Path, bytes: &[u8]) -> io::Result<Temp> {
    let parent = checked_parent(path)?;
    let name = parent.path.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        super::device::random_id()?
    ));
    let file = retry_sharing_violation(|| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&name)
    })?;
    let mut temp = Temp {
        path: name,
        file: Some(file),
        _parent: parent,
    };
    let file = temp.file.as_mut().unwrap();
    file.write_all(bytes)?;
    file.sync_all()?;
    temp.file.take();
    Ok(temp)
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temporary(path, bytes)?;
    #[cfg(windows)]
    let target = path.to_owned();
    #[cfg(unix)]
    let target = temp
        ._parent
        .path
        .join(path.file_name().ok_or_else(|| invalid("Missing name"))?);
    crate::platform::replace_file(&temp.path, &target)
}

/// Stable FNV-1a over length-delimited UTF-8 parts, independent of Rust's default hasher.
pub(super) fn fingerprint(parts: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        let part = part.as_ref();
        for byte in (part.len() as u64)
            .to_le_bytes()
            .into_iter()
            .chain(part.bytes())
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    format!("{hash:016x}")
}

/// Publish a flushed immutable file with a write-through rename that refuses replacement.
pub(super) fn publish(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = temporary(path, bytes)?;
    #[cfg(windows)]
    let target = path.to_owned();
    #[cfg(unix)]
    let target = temp
        ._parent
        .path
        .join(path.file_name().ok_or_else(|| invalid("Missing name"))?);
    crate::platform::publish_file(&temp.path, &target)
}

/// Windows scanners and sync clients can briefly deny sharing. Other errors fail immediately.
pub(crate) fn retry_sharing_violation<T>(
    mut action: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    for attempt in 0..5 {
        match action() {
            Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) && attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result => return result,
        }
    }
    unreachable!()
}
pub(super) fn reject_reparse(path: &Path) -> io::Result<()> {
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) if crate::platform::linked(&meta) => {
                return Err(invalid("Linked state paths are not supported"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
pub(crate) fn read_bounded(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let _read = super::idle_pass::Span::begin("store_reads");
    reject_reparse(path)?;
    let read = || super::file_reads::read(path, cap).map(|bytes| bytes.to_vec());
    if super::file_reads::refreshing() {
        read()
    } else {
        retry_sharing_violation(read)
    }
}

#[cfg(unix)]
fn checked_parent(path: &Path) -> io::Result<Parent> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let parent = std::path::absolute(path.parent().ok_or_else(|| invalid("Missing parent"))?)?;
    if path
        .components()
        .any(|part| part == std::path::Component::ParentDir)
    {
        return Err(invalid("INVALID_STATE_PATH"));
    }
    let mut pins = vec![fs::File::open("/")?];
    for component in parent.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let name = CString::new(name.as_bytes()).map_err(|_| invalid("Invalid path"))?;
        let fd = pins
            .last()
            .ok_or_else(|| invalid("Missing parent"))?
            .as_raw_fd();
        // Resolve each component relative to its pinned ancestor, refusing symlinks.
        let open = || unsafe {
            libc::openat(
                fd,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let mut child = open();
        if child < 0 && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            if unsafe { libc::mkdirat(fd, name.as_ptr(), 0o700) } < 0
                && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
            {
                return Err(io::Error::last_os_error());
            }
            child = open();
        }
        if child < 0 {
            return Err(io::Error::last_os_error());
        }
        pins.push(unsafe { fs::File::from_raw_fd(child) });
    }
    // /proc/self/fd keeps all subsequent operations anchored to the pinned directory.
    let pinned = Path::new("/proc/self/fd").join(
        pins.last()
            .ok_or_else(|| invalid("Missing parent"))?
            .as_raw_fd()
            .to_string(),
    );
    Ok(Parent {
        path: pinned,
        _pins: pins,
    })
}
#[cfg(unix)]
pub(crate) fn lock(path: &Path) -> io::Result<fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let parent = checked_parent(path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(
            parent
                .path
                .join(path.file_name().ok_or_else(|| invalid("Missing name"))?),
        )?;
    if !file.metadata()?.is_file() {
        return Err(invalid("Unsafe lock file"));
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}
#[cfg(unix)]
fn temporary(path: &Path, bytes: &[u8]) -> io::Result<Temp> {
    use std::os::unix::fs::OpenOptionsExt;
    let parent = checked_parent(path)?;
    let name = parent.path.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        super::device::random_id()?
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&name)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(Temp {
        path: name,
        file: None,
        _parent: parent,
    })
}

#[cfg(all(test, unix))]
mod unix_tests {
    #[test]
    fn parent_traversal_is_rejected_before_opening_directories() {
        let error = super::checked_parent(std::path::Path::new(
            "/tmp/../sidenote-traversal-proof/file.json",
        ))
        .err()
        .expect("parent traversal must be rejected");
        assert_eq!(super::message(&error), Some("INVALID_STATE_PATH"));
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn busy_defers_but_other_settings_errors_propagate() {
        assert_eq!(defer_busy(Ok(true)).unwrap(), Some(true));
        assert_eq!(
            defer_busy::<bool>(Err(io::ErrorKind::WouldBlock.into())).unwrap(),
            None
        );
        assert_eq!(
            defer_busy::<bool>(Err(io::ErrorKind::InvalidData.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[test]
    fn sharing_retry_attempts_are_bounded_and_other_errors_fail_immediately() {
        for code in [32, 33, 5, 2] {
            let mut attempts = 0;
            let error = retry_sharing_violation::<()>(|| {
                attempts += 1;
                Err(io::Error::from_raw_os_error(code))
            })
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(code));
            assert_eq!(attempts, if matches!(code, 32 | 33) { 5 } else { 1 });
        }
    }
    #[test]
    fn sharing_retry_returns_the_first_success_after_a_released_handle() {
        // Deterministic replacement for a thread that released a real handle after 75 ms,
        // which raced the 200 ms retry budget on a loaded machine.
        for code in [32, 33] {
            let mut attempts = 0;
            let value = retry_sharing_violation(|| {
                attempts += 1;
                if attempts < 3 {
                    Err(io::Error::from_raw_os_error(code))
                } else {
                    Ok(attempts)
                }
            })
            .unwrap();
            assert_eq!((value, attempts), (3, 3));
        }
    }
}
