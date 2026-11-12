//! StrictModes trust boundary: root/current uid ancestors cannot be replaced by another uid.
use anyhow::{Result, ensure};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

pub(crate) fn ancestors(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "UNSAFE_STATE_ANCESTOR: choose an absolute private directory"
    );
    let uid = unsafe { libc::geteuid() };
    let mut current = PathBuf::from("/");
    let mut sticky = false;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => current.push(name),
            _ => anyhow::bail!("UNSAFE_STATE_ANCESTOR: traversal is not allowed"),
        }
        let meta = fs::symlink_metadata(&current)?;
        ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && (meta.uid() == 0 || meta.uid() == uid),
            "UNSAFE_STATE_ANCESTOR: untrusted directory owner or symlink"
        );
        if sticky {
            ensure!(
                meta.uid() == uid && meta.mode() & 0o022 == 0,
                "UNSAFE_STATE_ANCESTOR: sticky directory must lead directly to a private user-owned directory"
            );
        }
        sticky = meta.mode() & 0o022 != 0;
        ensure!(
            !sticky || (meta.mode() & libc::S_ISVTX != 0 && meta.uid() == 0),
            "UNSAFE_STATE_ANCESTOR: group/other-writable directory"
        );
    }
    ensure!(
        !sticky,
        "UNSAFE_STATE_ANCESTOR: state directory cannot be shared"
    );
    Ok(current)
}

pub(crate) fn private_file(file: &fs::File) -> Result<()> {
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o600,
        "STATE_FILE_NOT_PRIVATE: require owned regular files with mode 0600"
    );
    Ok(())
}

pub(crate) fn sqlite_descriptors(path: &Path, include_sidecars: bool) -> Result<()> {
    use std::os::fd::FromRawFd;
    let mut names = vec![path.to_owned()];
    for suffix in if include_sidecars {
        &["-wal", "-shm"][..]
    } else {
        &[][..]
    } {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let name = PathBuf::from(name);
        match fs::symlink_metadata(&name) {
            Ok(meta) => {
                ensure!(!meta.file_type().is_symlink(), "STATE_SIDECAR_SYMLINK");
                names.push(name);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    // Inspect the descriptors actually held by SQLite, not a second pathname open.
    // procfs is mandatory on this Linux backend; no private-VFS ABI assumptions.
    let mut found = vec![false; names.len()];
    for entry in fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        let Some(index) = names.iter().position(|name| name == &target) else {
            continue;
        };
        let Ok(fd) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if duplicate < 0 {
            continue;
        }
        let file = unsafe { fs::File::from_raw_fd(duplicate) };
        private_file(&file)?;
        let actual = file.metadata()?;
        let expected = fs::symlink_metadata(&names[index])?;
        ensure!(
            actual.dev() == expected.dev() && actual.ino() == expected.ino(),
            "STATE_DATABASE_CHANGED"
        );
        found[index] = true;
    }
    ensure!(
        found.iter().all(|found| *found),
        "STATE_DATABASE_DESCRIPTOR_UNAVAILABLE"
    );
    Ok(())
}

pub(crate) fn sidecars(path: &Path) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(Path::new(&name))
        {
            Ok(file) => private_file(&file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn writable_ancestor_is_rejected_without_repairing_it() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let shared = temp.path().join("shared");
        let state = shared.join("state");
        fs::create_dir_all(&state)?;
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o777))?;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
        assert!(
            ancestors(&state)
                .unwrap_err()
                .to_string()
                .contains("UNSAFE_STATE_ANCESTOR")
        );
        assert_eq!(fs::metadata(&shared)?.mode() & 0o777, 0o777);
        Ok(())
    }
}
