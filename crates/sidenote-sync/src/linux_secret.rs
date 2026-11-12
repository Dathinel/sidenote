//! AEAD envelope; master key is held by Secret Service or provisioned systemd credentials.
use anyhow::{Context, Result, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
static CANCELLED: AtomicBool = AtomicBool::new(false);
const INIT_LIMIT: Duration = Duration::from_secs(5);
pub(crate) const UNAVAILABLE: &str = "SECRET_SERVICE_UNAVAILABLE: unlock Secret Service or provision systemd-creds (5 second initialization limit)";
pub(crate) fn cancel() {
    CANCELLED.store(true, Ordering::Release);
}
fn expired(deadline: Instant) -> bool {
    CANCELLED.load(Ordering::Acquire) || Instant::now() >= deadline
}

const HEADER: &[u8] = b"sidenote-secret-v1\0";

pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    let key = master(true)?;
    let mut nonce = [0; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = ChaCha20Poly1305::new(key.as_ref().into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: bytes,
                aad: HEADER,
            },
        )
        .map_err(|_| anyhow::anyhow!("SECRET_ENCRYPT_FAILED"))?;
    Ok([HEADER, &nonce, &encrypted].concat())
}

pub fn unprotect(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        bytes.starts_with(HEADER) && bytes.len() >= HEADER.len() + 28,
        "SECRET_ENVELOPE_INVALID"
    );
    let key = master(false)?;
    let (nonce, encrypted) = bytes[HEADER.len()..].split_at(12);
    ChaCha20Poly1305::new(key.as_ref().into())
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: encrypted,
                aad: HEADER,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| anyhow::anyhow!("SECRET_DECRYPT_FAILED"))
}

fn master(create: bool) -> Result<Zeroizing<[u8; 32]>> {
    if let Some(directory) = std::env::var_os("CREDENTIALS_DIRECTORY") {
        // systemd decrypts LoadCredentialEncrypted into a private, read-only runtime directory.
        let path = Path::new(&directory);
        ensure!(path.is_absolute(), "SECRET_CREDENTIAL_PATH_INVALID");
        let dir = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = dir.metadata()?;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
            "SECRET_CREDENTIAL_DIRECTORY_NOT_PRIVATE"
        );
        use std::os::fd::{AsRawFd, FromRawFd};
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                c"sidenote-key".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "SECRET_CREDENTIAL_UNAVAILABLE");
        let mut file = unsafe { fs::File::from_raw_fd(fd) };
        let meta = file.metadata()?;
        ensure!(
            meta.is_file()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.len() == 32,
            "SECRET_CREDENTIAL_NOT_PRIVATE_OR_INVALID"
        );
        let mut key = Zeroizing::new([0; 32]);
        file.read_exact(key.as_mut())?;
        return Ok(key);
    }
    ensure!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "SECRET_STORE_UNAVAILABLE: provision systemd-creds or unlock Secret Service"
    );
    let deadline = Instant::now() + INIT_LIMIT;
    // Serialize first-use key creation across processes. Never replace an existing key.
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| Path::new(&p).join(".config")))
        .context("SECRET_STORE_HOME_UNAVAILABLE")?
        .join("sidenote/secrets");
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&root)?;
    let root = crate::secure_store::ancestors(&root)?;
    let meta = fs::symlink_metadata(&root)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "SECRET_STORE_DIRECTORY_NOT_PRIVATE"
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("provider.lock"))?;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(error) => {
                if !matches!(error, fs::TryLockError::WouldBlock) {
                    return Err(error.into());
                }
                ensure!(!expired(deadline), UNAVAILABLE);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    use std::os::unix::ffi::OsStrExt;
    let install = CString::new(hex::encode(crate::identity::hash(
        root.as_os_str().as_bytes(),
    )))?;
    unsafe { keyring(create, &install, deadline) }.map_err(|_| anyhow::anyhow!(UNAVAILABLE))
}

// libsecret is optional at runtime; Cargo.lock has no Secret Service client crate.
struct Library(*mut c_void);
impl Drop for Library {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                libc::dlclose(self.0);
            }
        }
    }
}
impl Library {
    unsafe fn symbol(&self, name: &CStr) -> Result<*mut c_void> {
        let pointer = unsafe { libc::dlsym(self.0, name.as_ptr()) };
        ensure!(!pointer.is_null(), "SECRET_SERVICE_API_UNAVAILABLE");
        Ok(pointer)
    }
}

unsafe fn keyring(create: bool, install: &CStr, deadline: Instant) -> Result<Zeroizing<[u8; 32]>> {
    let library = Library(unsafe {
        libc::dlopen(
            c"libsecret-1.so.0".as_ptr(),
            libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_NODELETE,
        )
    });
    ensure!(
        !library.0.is_null(),
        "SECRET_SERVICE_UNAVAILABLE: install libsecret-1-0 or provision systemd-creds"
    );
    type SchemaNew = unsafe extern "C" fn(*const c_char, c_int, ...) -> *mut c_void;
    type Lookup =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void, ...) -> *mut c_char;
    type Store = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *const c_char,
        *const c_char,
        *mut c_void,
        *mut *mut c_void,
        ...
    ) -> c_int;
    type Free = unsafe extern "C" fn(*mut c_void);
    let new: SchemaNew = unsafe { std::mem::transmute(library.symbol(c"secret_schema_new")?) };
    let lookup: Lookup =
        unsafe { std::mem::transmute(library.symbol(c"secret_password_lookup_sync")?) };
    let store: Store =
        unsafe { std::mem::transmute(library.symbol(c"secret_password_store_sync")?) };
    let unref: Free = unsafe { std::mem::transmute(library.symbol(c"secret_schema_unref")?) };
    let free: Free = unsafe { std::mem::transmute(library.symbol(c"secret_password_free")?) };
    let error_free: Free = unsafe { std::mem::transmute(library.symbol(c"g_error_free")?) };
    let cancellable = unsafe { Cancel::new(&library, deadline)? };
    let schema = unsafe {
        new(
            c"org.sidenote.Identity".as_ptr(),
            0,
            c"application".as_ptr(),
            0i32,
            c"install".as_ptr(),
            0i32,
            std::ptr::null::<c_char>(),
        )
    };
    ensure!(!schema.is_null(), "SECRET_SERVICE_SCHEMA_FAILED");
    let mut error = std::ptr::null_mut();
    let password = unsafe {
        lookup(
            schema,
            cancellable.pointer,
            &mut error,
            c"application".as_ptr(),
            c"sidenote".as_ptr(),
            c"install".as_ptr(),
            install.as_ptr(),
            std::ptr::null::<c_char>(),
        )
    };
    let result = if !error.is_null() {
        unsafe {
            error_free(error);
        }
        Err(anyhow::anyhow!("SECRET_SERVICE_LOCKED_OR_UNAVAILABLE"))
    } else if !password.is_null() {
        let mut key = Zeroizing::new([0; 32]);
        let decoded =
            hex::decode_to_slice(unsafe { CStr::from_ptr(password) }.to_bytes(), key.as_mut());
        unsafe {
            free(password.cast());
        }
        decoded
            .map(|_| key)
            .map_err(|_| anyhow::anyhow!("SECRET_SERVICE_KEY_INVALID"))
    } else if !create {
        Err(anyhow::anyhow!("SECRET_SERVICE_KEY_MISSING"))
    } else {
        let key = Zeroizing::new(crate::identity::random32());
        let encoded = Zeroizing::new(hex::encode(key.as_ref()));
        let mut encoded = Zeroizing::new(CString::new(encoded.as_bytes())?.into_bytes_with_nul());
        let ok = unsafe {
            store(
                schema,
                std::ptr::null(),
                c"Sidenote identity encryption".as_ptr(),
                encoded.as_mut_ptr().cast(),
                cancellable.pointer,
                &mut error,
                c"application".as_ptr(),
                c"sidenote".as_ptr(),
                c"install".as_ptr(),
                install.as_ptr(),
                std::ptr::null::<c_char>(),
            )
        };
        if !error.is_null() {
            unsafe {
                error_free(error);
            }
        }
        if ok != 0 && error.is_null() && !expired(deadline) {
            let confirmed = unsafe {
                lookup(
                    schema,
                    cancellable.pointer,
                    &mut error,
                    c"application".as_ptr(),
                    c"sidenote".as_ptr(),
                    c"install".as_ptr(),
                    install.as_ptr(),
                    std::ptr::null::<c_char>(),
                )
            };
            let same = !confirmed.is_null()
                && error.is_null()
                && unsafe { CStr::from_ptr(confirmed) }.to_bytes()
                    == encoded.as_slice().strip_suffix(&[0]).unwrap();
            if !confirmed.is_null() {
                unsafe {
                    free(confirmed.cast());
                }
            }
            if !error.is_null() {
                unsafe {
                    error_free(error);
                }
            }
            if same {
                Ok(key)
            } else {
                Err(anyhow::anyhow!(UNAVAILABLE))
            }
        } else {
            Err(anyhow::anyhow!("SECRET_SERVICE_STORE_FAILED"))
        }
    };
    unsafe {
        unref(schema);
    }
    result
}

// The cancellation thread never calls libsecret, only the thread-safe GCancellable API.
struct Cancel {
    pointer: *mut c_void,
    stop: mpsc::Sender<()>,
    timer: Option<std::thread::JoinHandle<()>>,
    unref: unsafe extern "C" fn(*mut c_void),
}
impl Cancel {
    unsafe fn new(library: &Library, deadline: Instant) -> Result<Self> {
        type New = unsafe extern "C" fn() -> *mut c_void;
        type Call = unsafe extern "C" fn(*mut c_void);
        let new: New = unsafe { std::mem::transmute(library.symbol(c"g_cancellable_new")?) };
        let cancel: Call = unsafe { std::mem::transmute(library.symbol(c"g_cancellable_cancel")?) };
        let unref: Call = unsafe { std::mem::transmute(library.symbol(c"g_object_unref")?) };
        let pointer = unsafe { new() };
        ensure!(!pointer.is_null(), UNAVAILABLE);
        let address = pointer as usize;
        let (stop, rx) = mpsc::channel();
        let timer = std::thread::Builder::new()
            .name("secret-deadline".into())
            .spawn(move || {
                loop {
                    if expired(deadline) {
                        unsafe {
                            cancel(address as *mut c_void);
                        }
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            });
        match timer {
            Ok(timer) => Ok(Self {
                pointer,
                stop,
                timer: Some(timer),
                unref,
            }),
            Err(error) => {
                unsafe {
                    unref(pointer);
                }
                Err(error.into())
            }
        }
    }
}
impl Drop for Cancel {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
        unsafe {
            (self.unref)(self.pointer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelopes_authenticate_and_never_contain_plaintext() -> Result<()> {
        let plain = b"synthetic identity material";
        let mut encrypted = protect(plain)?;
        assert!(!encrypted.windows(plain.len()).any(|w| w == plain));
        assert_eq!(unprotect(&encrypted)?.as_slice(), plain);
        *encrypted.last_mut().unwrap() ^= 1;
        assert!(unprotect(&encrypted).is_err());
        assert!(unprotect(plain).is_err());
        Ok(())
    }
}
