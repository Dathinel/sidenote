//! Small JSON stores share flushed replacements and an OS-released writer lock.
use serde::{Serialize, de::DeserializeOwned};
use std::{fs, io, io::Write, os::windows::fs::OpenOptionsExt, path::Path};

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
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
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
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
