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
    if !directory && m.nlink() != 1 {
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
                if m.nlink() != 1 {
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
pub(crate) struct MeetingScan {
    directory: Directory,
    pub names: Vec<std::ffi::OsString>,
    pub metadata: std::collections::BTreeMap<std::ffi::OsString, fs::Metadata>,
}
impl MeetingScan {
    pub(crate) fn root_matches(&self, path: &Path) -> io::Result<bool> {
        let current = Directory::open(path)?;
        #[cfg(windows)]
        {
            Ok(directory_id(&current.base)? == directory_id(&self.directory.base)?)
        }
        #[cfg(unix)]
        {
            Ok(stamp(&current.base, true)? == stamp(&self.directory.base, true)?)
        }
    }
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let directory = Directory::open(path)?;
        // Linux enumeration follows the pinned descriptor, not a replaceable root pathname.
        #[cfg(target_os = "linux")]
        let enumeration =
            std::path::PathBuf::from(format!("/proc/self/fd/{}", directory.base.as_raw_fd()));
        #[cfg(windows)]
        let enumeration = &directory.path;
        #[cfg(all(unix, not(target_os = "linux")))]
        let enumeration = path;
        let mut names = Vec::new();
        let mut metadata = std::collections::BTreeMap::new();
        for entry in fs::read_dir(enumeration)? {
            let entry = entry?;
            let name = entry.file_name();
            if let Ok(meta) = entry.metadata() {
                metadata.insert(name.clone(), meta);
            }
            names.push(name);
        }
        Ok(Self {
            directory,
            names,
            metadata,
        })
    }
    pub(crate) fn capture(&self, id: &str) -> io::Result<Snapshot> {
        let name = std::ffi::OsStr::new(id);
        if Path::new(id).file_name() != Some(name) {
            return Err(invalid());
        }
        #[cfg(unix)]
        let directory = Directory {
            base: open(&self.directory.base, name, true)?,
            _pins: Vec::new(),
        };
        #[cfg(windows)]
        let directory = {
            let root = Directory::open(&self.directory.path)?;
            if directory_id(&root.base)? != directory_id(&self.directory.base)? {
                return Err(invalid());
            }
            // Both root walks retain handles without delete sharing through the child open.
            Directory::open(&self.directory.path.join(id))?
        };
        Snapshot::capture_directory(directory)
    }
}
#[cfg(windows)]
fn directory_id(file: &fs::File) -> io::Result<(u64, [u8; 16])> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            &mut info as *mut _ as _,
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}
pub(crate) struct Snapshot {
    directory: Directory,
    files: Vec<Input>,
    pub manifest: Vec<u8>,
    pub transcript: Vec<u8>,
    pub windows: serde_json::Value,
}
impl Snapshot {
    pub(crate) fn open_file(path: &Path) -> io::Result<fs::File> {
        let mut directory = Directory::open(path.parent().ok_or_else(invalid)?)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?;
        directory
            .file(name)?
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    pub(crate) fn read_file(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
        let mut file = Self::open_file(path)?;
        if file.metadata()?.len() > cap {
            return Err(invalid());
        }
        let before = stamp(&file, false)?;
        let mut bytes = Vec::new();
        (&mut file).take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > cap || stamp(&file, false)? != before {
            return Err(invalid());
        }
        Ok(bytes)
    }
    pub fn capture(folder: &Path) -> io::Result<Self> {
        Self::capture_directory(Directory::open(folder)?)
    }
    fn capture_directory(directory: Directory) -> io::Result<Self> {
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
    pub(crate) fn wav_header(
        &mut self,
        track: &str,
    ) -> io::Result<Option<crate::core::meeting_watch::WavHeader>> {
        let name = match track {
            "system" => "system.wav",
            "microphone" => "microphone.wav",
            _ => return Err(invalid()),
        };
        let Some(input) = self.files.iter_mut().find(|f| f.name == name) else {
            return Ok(None);
        };
        input.file.seek(SeekFrom::Start(0))?;
        let length = input.file.metadata()?.len();
        let header = crate::core::meeting_watch::wav_header(&mut input.file, length)?;
        if stamp(&input.file, false)? != input.stamp {
            return Err(invalid());
        }
        Ok(Some(header))
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("meetings/123-4");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("manifest.json"),
            br#"{"status":"complete","duration_ms":30000}"#,
        )
        .unwrap();
        fs::write(
            folder.join("transcript.ndjson"),
            serde_json::json!({"text":vec!["word";25].join(" ")}).to_string(),
        )
        .unwrap();
        (root, folder)
    }
    #[test]
    fn refuses_hardlinked_outside_inputs_for_every_role() {
        for name in [
            "manifest.json",
            "transcript.ndjson",
            "audio-evidence.json",
            "system.wav",
            "microphone.wav",
            "hex-original/transcript.ndjson",
        ] {
            let (root, folder) = fixture();
            let outside = root.path().join("outside");
            fs::write(&outside, b"{}").unwrap();
            let target = folder.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if target.exists() {
                fs::remove_file(&target).unwrap();
            }
            fs::hard_link(&outside, target).unwrap();
            assert!(Snapshot::capture(&folder).is_err(), "{name}");
        }
    }
    #[test]
    fn discovery_cannot_authorize_a_later_transcript_or_ancestor_symlink() {
        for ancestor in [false, true] {
            let (root, folder) = fixture();
            let meetings = folder.parent().unwrap();
            let discovered = fs::read_dir(meetings)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let outside = root.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(
                outside.join("manifest.json"),
                br#"{"status":"complete","duration_ms":29999}"#,
            )
            .unwrap();
            fs::write(
                outside.join("transcript.ndjson"),
                br#"{"text":"outside synthetic data"}"#,
            )
            .unwrap();
            if ancestor {
                fs::rename(meetings, root.path().join("saved-meetings")).unwrap();
                let replacement = root.path().join("replacement");
                fs::create_dir(&replacement).unwrap();
                fs::rename(&outside, replacement.join("123-4")).unwrap();
                symlink(replacement, meetings).unwrap();
            } else {
                fs::remove_file(discovered.join("transcript.ndjson")).unwrap();
                symlink(
                    outside.join("transcript.ndjson"),
                    discovered.join("transcript.ndjson"),
                )
                .unwrap();
            }
            assert!(Snapshot::capture(&discovered).is_err());
            assert_eq!(
                crate::core::recording_guard::eligibility(root.path(), meetings, "123-4").unwrap(),
                crate::core::recording_guard::Eligibility::Unknown
            );
            assert!(
                !root
                    .path()
                    .join("local/recording-guard/123-4.json")
                    .exists(),
                "unsafe input persisted a short-recording decision"
            );
        }
    }
    #[test]
    fn refuses_symlinks_at_every_input_and_directory_component() {
        for name in [
            "manifest.json",
            "transcript.ndjson",
            "audio-evidence.json",
            "system.wav",
            "microphone.wav",
            "hex-original/transcript.ndjson",
        ] {
            let (root, folder) = fixture();
            let source = root.path().join("outside");
            fs::write(&source, b"[]").unwrap();
            let target = folder.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if target.exists() {
                fs::remove_file(&target).unwrap();
            }
            symlink(&source, target).unwrap();
            assert!(Snapshot::capture(&folder).is_err(), "{name}");
        }
        let (root, folder) = fixture();
        symlink(&folder, root.path().join("linked")).unwrap();
        assert!(Snapshot::capture(&root.path().join("linked")).is_err());
        fs::create_dir(root.path().join("outside")).unwrap();
        symlink(root.path().join("outside"), folder.join("hex-original")).unwrap();
        assert!(Snapshot::capture(&folder).is_err());
    }
    #[test]
    fn component_swaps_cannot_change_the_pinned_snapshot() {
        let (root, folder) = fixture();
        fs::write(folder.join("system.wav"), b"pinned audio").unwrap();
        let mut captured = Snapshot::capture(&folder).unwrap();
        let saved = folder.with_extension("saved");
        fs::rename(&folder, &saved).unwrap();
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("system.wav"), b"replacement audio").unwrap();
        let work = root.path().join("work");
        fs::create_dir(&work).unwrap();
        let staged = captured.stage(&work).unwrap();
        assert_eq!(
            fs::read(staged.path().join("system.wav")).unwrap(),
            b"pinned audio"
        );
        let mut directory = Directory::open(&saved).unwrap();
        fs::create_dir(saved.join("hex-original")).unwrap();
        fs::rename(saved.join("hex-original"), saved.join("old-original")).unwrap();
        symlink(&folder, saved.join("hex-original")).unwrap();
        assert!(directory.file("hex-original/transcript.ndjson").is_err());
    }
    #[test]
    fn rejects_special_files_without_blocking() {
        for kind in ["fifo", "socket", "directory", "device"] {
            let (_root, folder) = fixture();
            let path = folder.join("system.wav");
            let _socket;
            match kind {
                "fifo" => {
                    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
                }
                "socket" => {
                    _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
                }
                "directory" => fs::create_dir(&path).unwrap(),
                _ => {
                    symlink("/dev/zero", &path).unwrap();
                }
            }
            assert!(Snapshot::capture(&folder).is_err(), "{kind}");
        }
    }
    #[test]
    fn caps_each_input_before_allocating_or_reading() {
        for (name, cap) in [
            ("manifest.json", 1024 * 1024),
            ("transcript.ndjson", JSON_MAX),
            ("audio-evidence.json", JSON_MAX),
            ("system.wav", AUDIO_MAX),
            ("microphone.wav", AUDIO_MAX),
        ] {
            let (_root, folder) = fixture();
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(folder.join(name))
                .unwrap()
                .set_len(cap + 1)
                .unwrap();
            assert!(Snapshot::capture(&folder).is_err(), "{name}");
        }
    }
    #[test]
    fn eligibility_and_processing_share_captured_bytes() {
        let (root, folder) = fixture();
        let mut captured = Snapshot::capture(&folder).unwrap();
        let eligible = crate::core::recording_guard::eligibility_captured(
            root.path(),
            "123-4",
            &captured.manifest,
            &captured.transcript,
        )
        .unwrap();
        assert_eq!(
            eligible,
            crate::core::recording_guard::Eligibility::Eligible
        );
        let original = captured.transcript.clone();
        let work = root.path().join("work");
        fs::create_dir(&work).unwrap();
        let staged = captured.stage(&work).unwrap();
        fs::rename(folder.join("transcript.ndjson"), folder.join("old")).unwrap();
        fs::write(folder.join("transcript.ndjson"), b"untrusted replacement").unwrap();
        assert_eq!(
            fs::read(staged.path().join("transcript.ndjson")).unwrap(),
            original
        );
    }
}
