//! Explicit ownership for local meeting recycling.
use super::{file_reads, store_io};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub relative: PathBuf,
    identity: [u64; 2],
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HexInput {
    pub relative: PathBuf,
    pub bytes: u64,
    pub hash: String,
    pub directory: bool,
}

pub struct Inventory {
    pub ids: BTreeSet<String>,
    pub members: Vec<Member>,
    pub hex_inputs: Vec<HexInput>,
}

pub struct Storage {
    meetings: PathBuf,
    state: PathBuf,
}

fn invalid() -> io::Error {
    store_io::invalid("Unsafe or ambiguous meeting storage")
}

fn valid_id(id: &str) -> bool {
    id.len() <= 100
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    store_io::reject_reparse(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() || m.is_dir() => Ok(Some(m)),
        Ok(_) => Err(invalid()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn identity(_path: &Path, m: &fs::Metadata) -> io::Result<[u64; 2]> {
    use std::os::unix::fs::MetadataExt;
    if m.is_file() && m.nlink() != 1 {
        return Err(invalid());
    }
    Ok([m.dev(), m.ino()])
}

#[cfg(windows)]
fn identity(path: &Path, m: &fs::Metadata) -> io::Result<[u64; 2]> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        GetFileInformationByHandle,
    };
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & 0x400 != 0 || (m.is_file() && info.nNumberOfLinks != 1) {
        return Err(invalid());
    }
    Ok([
        u64::from(info.dwVolumeSerialNumber),
        u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
    ])
}

#[cfg(target_os = "linux")]
fn volume(path: &Path, _: [u64; 2]) -> io::Result<u64> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid())?;
    let mut info: libc::statx = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            &mut info,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(invalid());
    }
    Ok(info.stx_mnt_id)
}

#[cfg(not(target_os = "linux"))]
fn volume(_: &Path, identity: [u64; 2]) -> io::Result<u64> {
    Ok(identity[0])
}

fn same_volume(expected: u64, actual: u64) -> io::Result<()> {
    if actual != expected {
        return Err(io::ErrorKind::CrossesDevices.into());
    }
    Ok(())
}

fn inspect(path: &Path, target_volume: u64) -> io::Result<Option<[u64; 2]>> {
    let Some(m) = metadata(path)? else {
        return Ok(None);
    };
    let key = identity(path, &m)?;
    let mut pending = vec![(path.to_owned(), m, key)];
    while let Some((path, m, key)) = pending.pop() {
        same_volume(target_volume, volume(&path, key)?)?;
        if m.is_dir() {
            for entry in fs::read_dir(path)? {
                let path = entry?.path();
                let m = metadata(&path)?.ok_or_else(invalid)?;
                let key = identity(&path, &m)?;
                pending.push((path, m, key));
            }
        }
    }
    Ok(Some(key))
}

fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    Ok(serde_json::from_slice(&store_io::read_bounded(
        path,
        file_reads::SUMMARY_MAX,
    )?)?)
}

impl Storage {
    pub fn new(meetings: &Path, state: &Path) -> io::Result<Self> {
        for path in [meetings, state] {
            if path.components().any(|c| c == Component::ParentDir)
                || !metadata(path)?.is_some_and(|m| m.is_dir())
            {
                return Err(invalid());
            }
        }
        let meetings = fs::canonicalize(meetings)?;
        let state = fs::canonicalize(state)?;
        if meetings.starts_with(&state) || state.starts_with(&meetings) {
            return Err(invalid());
        }
        Ok(Self { meetings, state })
    }

    fn directory_volume(&self, path: &Path) -> io::Result<u64> {
        let m = metadata(path)?
            .filter(fs::Metadata::is_dir)
            .ok_or_else(invalid)?;
        volume(path, identity(path, &m)?)
    }

    fn volume(&self) -> io::Result<u64> {
        self.directory_volume(&self.state)
    }

    fn state_path(&self, relative: &Path) -> io::Result<PathBuf> {
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(invalid());
        }
        Ok(self.state.join(relative))
    }

    pub fn inventory(&self, id: &str) -> io::Result<Inventory> {
        if !valid_id(id) {
            return Err(invalid());
        }
        let mut inventory = Inventory {
            ids: BTreeSet::from([id.to_owned()]),
            members: Vec::new(),
            hex_inputs: Vec::new(),
        };
        self.physical(id, &mut inventory)?;
        self.state_members(id, &mut inventory.members)?;
        Ok(inventory)
    }

    fn physical(&self, id: &str, inventory: &mut Inventory) -> io::Result<()> {
        let root = self.meetings.join(id);
        let _: serde_json::Map<String, serde_json::Value> = read(&root.join("manifest.json"))?;
        self.fingerprint_hex(Path::new(id), &mut inventory.hex_inputs)?;
        Ok(())
    }

    fn fingerprint_hex(
        &self,
        relative: &Path,
        entries: &mut Vec<HexInput>,
    ) -> io::Result<(u64, String)> {
        let path = self.meetings.join(relative);
        let m = metadata(&path)?.ok_or_else(invalid)?;
        identity(&path, &m)?;
        let mut hash = blake3::Hasher::new();
        let mut bytes = 0_u64;
        if m.is_dir() {
            let mut children = fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
            children.sort_by_key(fs::DirEntry::file_name);
            for child in children {
                let name = child.file_name();
                let (size, digest) = self.fingerprint_hex(&relative.join(&name), entries)?;
                bytes = bytes.checked_add(size).ok_or_else(invalid)?;
                hash.update(&serde_json::to_vec(&(name, size, digest))?);
            }
        } else {
            let mut file = super::meeting_router::inputs::Snapshot::open_file(&path)?;
            bytes = io::copy(&mut file, &mut hash)?;
            if bytes != m.len() {
                return Err(invalid());
            }
        }
        let hash = hash.finalize().to_hex().to_string();
        entries.push(HexInput {
            relative: relative.to_owned(),
            bytes,
            hash: hash.clone(),
            directory: m.is_dir(),
        });
        Ok((bytes, hash))
    }

    fn state_members(&self, id: &str, members: &mut Vec<Member>) -> io::Result<()> {
        for directory in [
            "transcripts",
            "router-work",
            "evidence",
            "joined-transcripts",
        ] {
            let relative = PathBuf::from("local").join(directory).join(id);
            self.member(relative, false, members)?;
        }
        for (directory, ext) in [
            ("router-queue", "json"),
            ("router-queue", "lock"),
            ("router-queue", "completed"),
            ("notes-jobs", "json"),
            ("notes-jobs", "lock"),
            ("draft-summaries", "json"),
            ("draft-outcomes", "json"),
            ("recording-guard", "json"),
        ] {
            let relative = PathBuf::from("local")
                .join(directory)
                .join(format!("{id}.{ext}"));
            self.member(relative, false, members)?;
        }
        Ok(())
    }

    fn member(
        &self,
        relative: PathBuf,
        required: bool,
        members: &mut Vec<Member>,
    ) -> io::Result<()> {
        let path = self.state_path(&relative)?;
        let volume = self.volume()?;
        if let Some(identity) = inspect(&path, volume)? {
            if metadata(&path)?.is_some_and(|m| m.is_file() != relative.extension().is_some()) {
                return Err(invalid());
            }
            same_volume(
                volume,
                self.directory_volume(path.parent().ok_or_else(invalid)?)?,
            )?;
            members.push(Member { relative, identity });
        } else if required {
            return Err(invalid());
        }
        Ok(())
    }
}
