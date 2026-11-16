//! A cheap invalidation key, never permission to read an input. Capture still validates paths.
use std::{fs, io, path::Path};

pub(crate) fn input(folder: &Path) -> io::Result<String> {
    let root = fs::metadata(folder.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    input_with_root(folder, &root)
}

pub(crate) fn input_with_root(folder: &Path, root: &fs::Metadata) -> io::Result<String> {
    input_with_metadata(folder, root, &fs::metadata(folder)?)
}

pub(crate) fn input_with_metadata(
    folder: &Path,
    root: &fs::Metadata,
    directory: &fs::Metadata,
) -> io::Result<String> {
    let mut hash = blake3::Hasher::new();
    for meta in [root, directory] {
        if !meta.is_dir() || crate::platform::linked(meta) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            hash.update(&meta.creation_time().to_le_bytes());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            hash.update(&meta.dev().to_le_bytes());
            hash.update(&meta.ino().to_le_bytes());
        }
    }
    match fs::symlink_metadata(folder.join("hex-original")) {
        Ok(meta) if meta.is_dir() && !crate::platform::linked(&meta) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
        _ => return Err(io::ErrorKind::InvalidInput.into()),
    }
    for name in [
        "manifest.json",
        "transcript.ndjson",
        "hex-original/transcript.ndjson",
    ] {
        hash.update(name.as_bytes());
        append_file(&mut hash, &folder.join(name))?;
    }
    Ok(hash.finalize().to_hex().to_string())
}

pub(crate) fn validation(folder: &Path, input: &str) -> io::Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(input.as_bytes());
    for name in ["audio-evidence.json", "system.wav", "microphone.wav"] {
        hash.update(name.as_bytes());
        append_file(&mut hash, &folder.join(name))?;
    }
    Ok(hash.finalize().to_hex().to_string())
}

pub(crate) fn file(path: &Path) -> io::Result<String> {
    let mut hash = blake3::Hasher::new();
    append_file(&mut hash, path)?;
    Ok(hash.finalize().to_hex().to_string())
}

fn append_file(hash: &mut blake3::Hasher, path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !crate::platform::linked(&meta) => {
            hash.update(&meta.len().to_le_bytes());
            hash.update(format!("{:?}", meta.modified()?).as_bytes());
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                hash.update(&meta.creation_time().to_le_bytes());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                hash.update(&meta.ino().to_le_bytes());
                hash.update(&meta.ctime().to_le_bytes());
                hash.update(&meta.ctime_nsec().to_le_bytes());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            hash.update(b"missing");
        }
        Err(e) => return Err(e),
        _ => return Err(io::ErrorKind::InvalidInput.into()),
    }
    Ok(())
}
