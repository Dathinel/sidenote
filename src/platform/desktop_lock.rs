//! Desktop-only locks: private user namespace, canonical data identity, kernel lifetime.
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{self, File},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt},
        },
    },
    path::{Component, Path, PathBuf},
};

fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "UNSAFE_DESKTOP_LOCK")
}

// Pin every directory; only root/self may own ancestors. A root-owned sticky /tmp is safe.
fn directory(path: &Path, create: bool, private: bool) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(unsafe_path());
    }
    let uid = unsafe { libc::geteuid() };
    let mut dir = File::open("/")?;
    for part in path.components() {
        let name = match part {
            Component::RootDir => continue,
            Component::Normal(name) => CString::new(name.as_bytes()).map_err(|_| unsafe_path())?,
            _ => return Err(unsafe_path()),
        };
        if create
            && unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) } != 0
            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
        {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        dir = unsafe { File::from_raw_fd(fd) };
        let meta = dir.metadata()?;
        if (meta.uid() != uid && meta.uid() != 0)
            || (meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & libc::S_ISVTX != 0))
        {
            return Err(unsafe_path());
        }
    }
    let meta = dir.metadata()?;
    if private && (meta.uid() != uid || meta.mode() & 0o7777 != 0o700) {
        return Err(unsafe_path());
    }
    Ok(dir)
}

fn namespace() -> io::Result<PathBuf> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty()) {
        let runtime = PathBuf::from(runtime);
        directory(&runtime, false, true)?;
        return Ok(runtime.join("sidenote-desktop"));
    }
    let home = super::super::home();
    if directory(&home, false, false)?.metadata()?.uid() != unsafe { libc::geteuid() } {
        return Err(unsafe_path());
    }
    Ok(home.join(".local/state/sidenote/desktop-locks"))
}

pub(super) fn acquire(data: &Path) -> io::Result<Option<File>> {
    acquire_in(data, &namespace()?)
}

fn acquire_in(data: &Path, namespace: &Path) -> io::Result<Option<File>> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(data)?;
    let data = data.canonicalize()?;
    let directory = directory(namespace, true, true)?;
    let key = format!(
        "{}.lock",
        hex::encode(Sha256::digest(data.as_os_str().as_bytes()))
    );
    let name = CString::new(key).map_err(|_| unsafe_path())?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let lock = unsafe { File::from_raw_fd(fd) };
    let meta = lock.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o7777 != 0o600
        || meta.nlink() != 1
    {
        return Err(unsafe_path());
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        eprintln!("SIDENOTE_ALREADY_OPEN");
        return Ok(None);
    }
    // Never unlink: retaining the inode prevents a second name from bypassing an active lock.
    Ok(Some(lock))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn canonical_identity_and_fd_lifetime() -> io::Result<()> {
        let root = tempfile::tempdir_in(crate::test_home())?;
        let data = root.path().join("data");
        let locks = root.path().join("locks");
        let first = acquire_in(&data, &locks)?.unwrap();
        symlink(&data, root.path().join("alias"))?;
        assert!(acquire_in(&root.path().join("alias"), &locks)?.is_none());
        assert!(acquire_in(&root.path().join("different"), &locks)?.is_some());
        drop(first);
        assert!(acquire_in(&data, &locks)?.is_some());
        Ok(())
    }

    #[test]
    fn reject_unsafe_namespace_file_symlink_and_hardlink() -> io::Result<()> {
        let root = tempfile::tempdir_in(crate::test_home())?;
        let data = root.path().join("data");
        let locks = root.path().join("locks");
        drop(acquire_in(&data, &locks)?);
        let path = locks.join(format!(
            "{}.lock",
            hex::encode(Sha256::digest(data.canonicalize()?.as_os_str().as_bytes()))
        ));
        fs::set_permissions(&locks, fs::Permissions::from_mode(0o777))?;
        assert!(acquire_in(&data, &locks).is_err());
        fs::set_permissions(&locks, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o666))?;
        assert!(acquire_in(&data, &locks).is_err());
        fs::remove_file(&path)?;
        symlink(root.path().join("target"), &path)?;
        assert!(acquire_in(&data, &locks).is_err());
        fs::remove_file(&path)?;
        drop(acquire_in(&data, &locks)?);
        fs::hard_link(&path, root.path().join("alias-file"))?;
        assert!(acquire_in(&data, &locks).is_err());
        Ok(())
    }
}
