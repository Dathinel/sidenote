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
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map_err(io::Error::from),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

pub(super) fn lock(path: &Path) -> io::Result<fs::File> {
    fs::create_dir_all(path.parent().ok_or_else(|| invalid("Missing parent"))?)?;
    retry_sharing_violation(|| {
        fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
    })
}

pub(super) fn write(path: &Path, value: &impl Serialize) -> io::Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| invalid("Missing parent"))?)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::platform::replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
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
    let pending = path.with_extension("pending.tmp");
    atomic_write(&pending, bytes)?;
    let result = crate::platform::publish_file(&pending, path);
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
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
    use std::io::Read;
    reject_reparse(path)?;
    let file = retry_sharing_violation(|| fs::File::open(path))?;
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > cap {
        return Err(invalid("State item is too large"));
    }
    Ok(bytes)
}
