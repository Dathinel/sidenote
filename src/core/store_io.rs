//! Small JSON stores share flushed replacements and an OS-released writer lock.
use serde::{Serialize, de::DeserializeOwned};
use std::{fs, io, io::Write, os::windows::fs::OpenOptionsExt, path::Path};

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

pub(super) fn read<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
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
pub(super) fn lock(path: &Path) -> io::Result<fs::File> {
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
pub(super) fn write(path: &Path, value: &impl Serialize) -> io::Result<()> {
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
    crate::platform::replace_file(&temp.path, path)
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
    crate::platform::publish_file(&temp.path, path)
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
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) if meta.file_attributes() & 0x400 != 0 => {
                return Err(invalid("Linked state paths are not supported"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
pub(super) fn read_bounded(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    reject_reparse(path)?;
    let read = || super::file_reads::read(path, cap).map(|bytes| bytes.to_vec());
    if super::file_reads::refreshing() {
        read()
    } else {
        retry_sharing_violation(read)
    }
}
