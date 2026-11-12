//! Filesystem-private socket, peer credentials, bounded frames and socket deadlines.
use super::*;
use std::{
    fs,
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
};
fn directory(root: &Path) -> io::Result<PathBuf> {
    let directory = root.join("local/sync");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    sidenote_sync::validate_private_directory(&directory)
        .map_err(|_| io::Error::other("SYNC_CONTROL_UNSAFE_DIRECTORY"))?;
    Ok(directory)
}
fn authenticate(stream: &UnixStream) -> io::Result<()> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&cred) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&cred)
        || cred.uid != unsafe { libc::geteuid() }
    {
        return Err(io::Error::other("SYNC_CONTROL_PEER_REJECTED"));
    }
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))
}
fn socket(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
    {
        return Err(io::Error::other("SYNC_CONTROL_UNSAFE_SOCKET"));
    }
    Ok(())
}
fn read_frame(stream: &mut UnixStream, maximum: usize) -> io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > maximum {
        return Err(invalid());
    }
    let mut bytes = zeroize::Zeroizing::new(vec![0; size]);
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn write_frame(stream: &mut UnixStream, bytes: &[u8], maximum: usize) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(invalid());
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(bytes)
}
pub(super) struct Endpoint {
    listener: UnixListener,
    path: PathBuf,
}
impl Endpoint {
    pub fn bind(root: &Path) -> io::Result<Self> {
        let path = directory(root)?.join("control.sock");
        if fs::symlink_metadata(&path).is_ok() {
            // The caller holds service.lock. A live or foreign endpoint is never removed.
            socket(&path)?;
            match UnixStream::connect(&path) {
                Ok(_) => return Err(io::Error::other("SYNC_CONTROL_ALREADY_OWNED")),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(&path)?
                }
                Err(error) => return Err(error),
            }
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }
    pub fn serve(self, controller: Arc<Mutex<Controller>>, stop: Arc<AtomicBool>) {
        let slots = Arc::new(tokio::sync::Semaphore::new(16));
        while !stop.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        continue;
                    };
                    let controller = controller.clone();
                    thread::spawn(move || {
                        let _slot = slot;
                        let _ = (|| -> io::Result<()> {
                            authenticate(&stream)?;
                            let bytes = read_frame(&mut stream, MAX_REQUEST)?;
                            let reply =
                                zeroize::Zeroizing::new(commands::respond(&controller, &bytes)?);
                            write_frame(&mut stream, &reply, MAX_REPLY)
                        })();
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25))
                }
                Err(_) => return,
            }
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
pub(super) fn request(root: &Path, bytes: &[u8]) -> io::Result<Vec<u8>> {
    let path = directory(root)?.join("control.sock");
    socket(&path)?;
    let mut stream = UnixStream::connect(path)?;
    authenticate(&stream)?;
    write_frame(&mut stream, bytes, MAX_REQUEST)?;
    Ok(read_frame(&mut stream, MAX_REPLY)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_connection_does_not_serialize_status() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let endpoint = Endpoint::bind(root.path())?;
        let controller = Arc::new(Mutex::new(Controller {
            root: root.path().into(),
            pair: None,
            state: Default::default(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let held = controller.lock().unwrap();
        let mut blocked = UnixStream::connect(&endpoint.path)?;
        authenticate(&blocked)?;
        write_frame(&mut blocked, br#"{"op":"pair-state"}"#, MAX_REQUEST)?;
        let done = stop.clone();
        let control = controller.clone();
        let server = thread::spawn(move || endpoint.serve(control, done));
        let response = request(root.path(), br#"{"op":"status"}"#);
        drop(held);
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response?)?["ok"],
            true
        );
        Ok(())
    }
    #[test]
    fn oversized_frames_are_rejected_before_payload_allocation() -> io::Result<()> {
        let (mut send, mut receive) = UnixStream::pair()?;
        send.write_all(&((MAX_REQUEST + 1) as u32).to_be_bytes())?;
        assert!(read_frame(&mut receive, MAX_REQUEST).is_err());
        Ok(())
    }
}
