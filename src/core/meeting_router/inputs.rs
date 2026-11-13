//! One pinned, bounded view of the synced files; helpers only see its private copy.
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(unix)]
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::{ffi::OsStrExt, fs::MetadataExt},
};
#[cfg(windows)]
use std::path::PathBuf;
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path},
};

const JSON_MAX: u64 = 16 * 1024 * 1024;
const AUDIO_MAX: u64 = 2 * 1024 * 1024 * 1024;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "UNSAFE_MEETING_INPUT")
}
fn stamp(file: &fs::File, directory: bool) -> io::Result<String> {
    let m = file.metadata()?;
    #[cfg(unix)]
    if !directory && m.nlink() > 1 {
        return Err(invalid());
    }
    #[cfg(unix)]
    return Ok(if directory {
        format!("{}:{}", m.dev(), m.ino())
    } else {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec()
        )
    });
    #[cfg(windows)]
    Ok(if directory {
        m.creation_time().to_string()
    } else {
        format!(
            "{}:{}:{}",
            m.creation_time(),
            m.last_write_time(),
            m.file_size()
        )
    })
}
#[cfg(unix)]
fn open(parent: &fs::File, name: &std::ffi::OsStr, directory: bool) -> io::Result<fs::File> {
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| invalid())?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}
struct Directory {
    #[cfg(windows)]
    path: PathBuf,
    base: fs::File,
    _pins: Vec<fs::File>,
}
impl Directory {
    fn open(path: &Path) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let mut pins = Vec::new();
        #[cfg(unix)]
        {
            pins.push(fs::File::open("/")?);
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => pins.push(open(pins.last().unwrap(), name, true)?),
                    _ => return Err(invalid()),
                }
            }
        }
        #[cfg(windows)]
        {
            let mut current = PathBuf::new();
            for component in path.components() {
                if matches!(component, Component::ParentDir | Component::CurDir) {
                    return Err(invalid());
                }
                current.push(component.as_os_str());
                if matches!(component, Component::Prefix(_)) {
                    continue;
                }
                let file = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x02200000)
                    .open(&current)?;
                let m = file.metadata()?;
                if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
                pins.push(file);
            }
        }
        Ok(Self {
            #[cfg(windows)]
            path,
            base: pins.last().ok_or_else(invalid)?.try_clone()?,
            _pins: pins,
        })
    }
    fn file(&mut self, relative: &str) -> io::Result<Option<fs::File>> {
        let parts: Vec<_> = Path::new(relative).components().collect();
        if parts.is_empty() || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
            return Err(invalid());
        }
        #[cfg(unix)]
        let result = (|| {
            let mut parent = self.base.try_clone()?;
            for component in &parts[..parts.len() - 1] {
                parent = open(&parent, component.as_os_str(), true)?;
            }
            open(&parent, parts.last().unwrap().as_os_str(), false)
        })();
        #[cfg(windows)]
        let result = (|| {
            let mut parent = self.path.clone();
            for component in &parts[..parts.len() - 1] {
                parent.push(component.as_os_str());
                let f = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(3)
                    .custom_flags(0x02200000)
                    .open(&parent)?;
                if !f.metadata()?.is_dir() || f.metadata()?.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
                self._pins.push(f);
            }
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(self.path.join(relative))
        })();
        match result {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
            Ok(f) => {
                let m = f.metadata()?;
                #[cfg(unix)]
                if m.nlink() > 1 {
                    return Err(invalid());
                }
                #[cfg(windows)]
                if m.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
                if !m.is_file() {
                    return Err(invalid());
                }
                Ok(Some(f))
            }
        }
    }
}
struct Input {
    name: &'static str,
    file: fs::File,
    stamp: String,
    cap: u64,
}
pub(super) struct Snapshot {
    directory: Directory,
    files: Vec<Input>,
    pub manifest: Vec<u8>,
    pub transcript: Vec<u8>,
    pub windows: serde_json::Value,
}
impl Snapshot {
    pub fn capture(folder: &Path) -> io::Result<Self> {
        let directory = Directory::open(folder)?;
        let mut snapshot = Self {
            directory,
            files: Vec::new(),
            manifest: Vec::new(),
            transcript: Vec::new(),
            windows: serde_json::Value::Null,
        };
        snapshot
            .add("manifest.json", 1024 * 1024)?
            .ok_or_else(invalid)?;
        snapshot.manifest = snapshot.bytes(0)?;
        if snapshot
            .add("hex-original/transcript.ndjson", JSON_MAX)?
            .is_none()
        {
            snapshot
                .add("transcript.ndjson", JSON_MAX)?
                .ok_or_else(invalid)?;
        }
        snapshot.transcript = snapshot.bytes(1)?;
        if let Some(index) = snapshot.add("audio-evidence.json", JSON_MAX)? {
            snapshot.windows =
                serde_json::from_slice(&snapshot.bytes(index)?).map_err(|_| invalid())?;
        }
        for name in ["system.wav", "microphone.wav"] {
            let _ = snapshot.add(name, AUDIO_MAX)?;
        }
        Ok(snapshot)
    }
    fn add(&mut self, name: &'static str, cap: u64) -> io::Result<Option<usize>> {
        let Some(file) = self.directory.file(name)? else {
            return Ok(None);
        };
        if file.metadata()?.len() > cap {
            return Err(invalid());
        }
        let stamp = stamp(&file, false)?;
        self.files.push(Input {
            name,
            file,
            stamp,
            cap,
        });
        Ok(Some(self.files.len() - 1))
    }
    fn bytes(&mut self, index: usize) -> io::Result<Vec<u8>> {
        let input = &mut self.files[index];
        let mut bytes = Vec::new();
        (&mut input.file)
            .take(input.cap + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > input.cap || stamp(&input.file, false)? != input.stamp {
            return Err(invalid());
        }
        Ok(bytes)
    }
    pub fn revision(&self) -> io::Result<String> {
        Ok(format!(
            "{}:{:?}",
            stamp(&self.directory.base, true)?,
            self.files
                .iter()
                .map(|f| (f.name, &f.stamp))
                .collect::<Vec<_>>()
        ))
    }
    pub fn stage(&mut self, work: &Path) -> io::Result<tempfile::TempDir> {
        let copy = tempfile::Builder::new()
            .prefix("inputs-")
            .tempdir_in(work)?;
        for input in &mut self.files {
            let path = copy.path().join(input.name);
            let parent = path.parent().ok_or_else(invalid)?;
            fs::create_dir_all(parent)?;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let mut output = options.open(path)?;
            let n = match input.name {
                "manifest.json" => {
                    output.write_all(&self.manifest)?;
                    self.manifest.len() as u64
                }
                "transcript.ndjson" | "hex-original/transcript.ndjson" => {
                    output.write_all(&self.transcript)?;
                    self.transcript.len() as u64
                }
                "audio-evidence.json" => {
                    let bytes = serde_json::to_vec(&self.windows)?;
                    output.write_all(&bytes)?;
                    bytes.len() as u64
                }
                _ => {
                    input.file.seek(SeekFrom::Start(0))?;
                    io::copy(&mut (&mut input.file).take(input.cap + 1), &mut output)?
                }
            };
            output.flush()?;
            if n > input.cap || stamp(&input.file, false)? != input.stamp {
                return Err(invalid());
            }
        }
        Ok(copy)
    }
}
