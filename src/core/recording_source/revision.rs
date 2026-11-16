//! A cheap invalidation key, never permission to read an input. Capture still validates paths.
use std::{fs, io, path::Path};

pub(crate) fn input(folder: &Path) -> io::Result<String> {
    let mut hash = blake3::Hasher::new();
    for directory in [folder.parent().ok_or(io::ErrorKind::InvalidInput)?, folder] {
        let meta = fs::metadata(directory)?;
        if !meta.is_dir() {
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
    for name in [
        "manifest.json",
        "transcript.ndjson",
        "hex-original/transcript.ndjson",
    ] {
        hash.update(name.as_bytes());
        match fs::symlink_metadata(folder.join(name)) {
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
    }
    Ok(hash.finalize().to_hex().to_string())
}
