//! Capped regular-file reads; a refresh shares bytes instead of reading a file repeatedly.
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
pub const JOURNAL_MAX: u64 = 4 * 1024 * 1024;
pub const SUMMARY_MAX: u64 = 16 * 1024 * 1024;
pub const STORE_MAX: u64 = 256 * 1024 * 1024;
pub const ENTRIES_MAX: usize = 8192;
#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub reads: usize,
    pub bytes: usize,
    pub skipped: usize,
    pub entries: usize,
    pub temporary: bool,
}
#[derive(Default)]
struct Cache {
    files: HashMap<PathBuf, Result<Arc<[u8]>, io::ErrorKind>>,
    dirs: HashMap<PathBuf, Vec<Arc<fs::DirEntry>>>,
    stats: Stats,
    failed: Vec<PathBuf>,
}
thread_local! {static CACHE:RefCell<Option<Cache>>=const {RefCell::new(None)};}
pub struct Scope(std::marker::PhantomData<std::rc::Rc<()>>);
impl Scope {
    pub fn begin() -> Self {
        CACHE.with(|c| {
            assert!(c.borrow().is_none());
            *c.borrow_mut() = Some(Cache::default());
        });
        Self(Default::default())
    }
    pub fn failed(&self) -> Vec<PathBuf> {
        CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .map(|c| c.failed.clone())
                .unwrap_or_default()
        })
    }
    pub fn stats(&self) -> Stats {
        CACHE.with(|c| c.borrow().as_ref().map(|c| c.stats).unwrap_or_default())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CACHE.with(|c| *c.borrow_mut() = None);
    }
}
pub fn refreshing() -> bool {
    CACHE.with(|c| c.borrow().is_some())
}
pub fn not_refreshed(path: &Path) {
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut()
            && !c.failed.iter().any(|p| p == path)
        {
            c.failed.push(path.into());
        }
    });
}
pub fn error(path: &Path, error: &io::Error) {
    not_refreshed(path);
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.stats.skipped += 1;
            c.stats.temporary |= matches!(error.raw_os_error(), Some(32 | 33));
        }
    });
}
pub fn skipped() {
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.stats.skipped += 1;
        }
    });
}
pub fn regular(meta: &fs::Metadata) -> bool {
    meta.is_file() && !crate::platform::linked(meta)
}
pub fn directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !crate::platform::linked(&m))
}
pub fn checked_metadata(path: &Path) -> io::Result<fs::Metadata> {
    for parent in path.ancestors().skip(1) {
        if fs::symlink_metadata(parent).is_ok_and(|m| crate::platform::linked(&m)) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Reparse directory",
            ));
        }
    }
    let m = fs::symlink_metadata(path)?;
    if !regular(&m) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Not a regular file",
        ));
    }
    Ok(m)
}
pub fn read(path: &Path, cap: u64) -> io::Result<Arc<[u8]>> {
    if let Some(cached) =
        CACHE.with(|c| c.borrow().as_ref().and_then(|c| c.files.get(path).cloned()))
    {
        return cached
            .map_err(|kind| io::Error::new(kind, "File skipped"))
            .and_then(|b| {
                if b.len() as u64 <= cap {
                    Ok(b)
                } else {
                    Err(io::Error::new(io::ErrorKind::InvalidData, "File too large"))
                }
            });
    }
    let result = (|| {
        let m = checked_metadata(path)?;
        if m.len() > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Read budget exceeded",
            ));
        }
        let file = nofollow(path)?;
        let m = file.metadata()?;
        if !regular(&m) || m.len() > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File skipped"));
        }
        let mut bytes = Vec::new();
        file.take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > cap {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File grew past limit",
            ));
        }
        Ok(Arc::<[u8]>::from(bytes))
    })();
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            if let Ok(b) = &result {
                c.stats.reads += 1;
                c.stats.bytes += b.len();
            } else if result
                .as_ref()
                .err()
                .is_some_and(|e| e.kind() != io::ErrorKind::NotFound)
            {
                c.stats.skipped += 1;
                c.stats.temporary |= result
                    .as_ref()
                    .err()
                    .is_some_and(|e| matches!(e.raw_os_error(), Some(32 | 33)));
                c.failed.push(path.into());
            }
            if !result
                .as_ref()
                .err()
                .is_some_and(|e| e.kind() == io::ErrorKind::NotFound)
            {
                c.files.insert(
                    path.into(),
                    result.as_ref().map(Arc::clone).map_err(|e| e.kind()),
                );
            }
        }
    });
    result
}
pub fn text(path: &Path, cap: u64) -> io::Result<String> {
    String::from_utf8(read(path, cap)?.to_vec())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid text"))
}
pub fn entries(path: &Path) -> io::Result<Vec<Arc<fs::DirEntry>>> {
    if let Some(cached) =
        CACHE.with(|c| c.borrow().as_ref().and_then(|c| c.dirs.get(path).cloned()))
    {
        return Ok(cached);
    }
    let result = (|| {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
            Ok(m) if !m.is_dir() || crate::platform::linked(&m) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Unsafe folder"));
            }
            Ok(_) => {}
        }
        let mut out = Vec::new();
        for (n, e) in fs::read_dir(path)?.enumerate() {
            if n >= ENTRIES_MAX {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Folder entry limit exceeded",
                ));
            }
            let e = e?;
            let m = fs::symlink_metadata(e.path())?;
            if !crate::platform::linked(&m) && (m.is_file() || m.is_dir()) {
                out.push(Arc::new(e));
            } else {
                skipped();
            }
        }
        out.sort_by_key(|e| e.file_name());
        Ok(out)
    })();
    match &result {
        Ok(out) => CACHE.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.stats.entries += out.len();
                c.dirs.insert(path.into(), out.clone());
            }
        }),
        Err(e) => error(path, e),
    }
    result
}
pub fn dir_iter(path: &Path) -> io::Result<impl Iterator<Item = io::Result<Arc<fs::DirEntry>>>> {
    Ok(entries(path)?.into_iter().map(Ok))
}

pub(crate) fn nofollow(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}
