//! Capped regular-file reads; a refresh shares bytes instead of reading a file repeatedly.
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{self, Read},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
pub const JOURNAL_MAX: u64 = 4 * 1024 * 1024;
pub const SUMMARY_MAX: u64 = 16 * 1024 * 1024;
const CACHE_MAX: usize = 64 * 1024 * 1024;
const FILES_MAX: usize = 4096;
pub const ENTRIES_MAX: usize = 8192;
#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub reads: usize,
    pub bytes: usize,
    pub skipped: usize,
    pub entries: usize,
}
#[derive(Default)]
struct Cache {
    files: HashMap<PathBuf, Result<Arc<[u8]>, io::ErrorKind>>,
    dirs: HashMap<PathBuf, Vec<Arc<fs::DirEntry>>>,
    stats: Stats,
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
    pub fn stats(&self) -> Stats {
        CACHE.with(|c| c.borrow().as_ref().map(|c| c.stats).unwrap_or_default())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CACHE.with(|c| *c.borrow_mut() = None);
    }
}
pub fn skipped() {
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.stats.skipped += 1;
        }
    });
}
pub fn regular(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.file_attributes() & 0x400 == 0
}
pub fn directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.file_attributes() & 0x400 == 0)
}
pub fn checked_metadata(path: &Path) -> io::Result<fs::Metadata> {
    for parent in path.ancestors().skip(1) {
        if fs::symlink_metadata(parent).is_ok_and(|m| m.file_attributes() & 0x400 != 0) {
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
    let remaining = CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .map(|c| CACHE_MAX.saturating_sub(c.stats.bytes))
            .unwrap_or(CACHE_MAX)
    });
    let budget = CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .is_none_or(|c| c.files.len() < FILES_MAX)
    });
    let result = (|| {
        let m = checked_metadata(path)?;
        if !budget || m.len() > cap || m.len() > remaining as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Read budget exceeded",
            ));
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x00200000)
            .open(path)?;
        let m = file.metadata()?;
        if !regular(&m) || m.len() > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File skipped"));
        }
        let mut bytes = Vec::new();
        file.take(cap.min(remaining as u64) + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > cap || bytes.len() > remaining {
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
            }
            if budget
                && !result
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
    if !directory(path) {
        if !path.try_exists()? {
            return Ok(vec![]);
        }
        skipped();
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for e in fs::read_dir(path)?.take(ENTRIES_MAX + 1) {
        let full = CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .is_some_and(|c| c.stats.entries >= ENTRIES_MAX)
        });
        if full || out.len() >= ENTRIES_MAX {
            skipped();
            break;
        }
        CACHE.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.stats.entries += 1;
            }
        });
        match e {
            Ok(e) => match fs::symlink_metadata(e.path()) {
                Ok(m) if m.file_attributes() & 0x400 == 0 && (m.is_file() || m.is_dir()) => {
                    out.push(Arc::new(e))
                }
                _ => skipped(),
            },
            Err(_) => skipped(),
        }
    }
    out.sort_by_key(|e| e.file_name());
    CACHE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut()
            && c.dirs.len() < FILES_MAX
        {
            c.dirs.insert(path.into(), out.clone());
        }
    });
    Ok(out)
}
pub fn dir_iter(path: &Path) -> io::Result<impl Iterator<Item = io::Result<Arc<fs::DirEntry>>>> {
    Ok(entries(path)?.into_iter().map(Ok))
}
