//! Device-local consent and credential for the speech host's status endpoint.
use super::{provider, store_io};
use serde::Deserialize;
use sidenote_sync::identity::{protect, unprotect};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(test)]
mod tests;

const INTERVAL: Duration = Duration::from_secs(30);
const STORE_ERROR: &str = "Could not save or read the protected speech host settings.";

pub fn settings_path() -> Result<PathBuf, String> {
    Ok(provider::secure_directory("speech-host")?.join("host.dpapi"))
}

pub struct Host {
    endpoint: url::Url,
    token: Zeroizing<Vec<u8>>,
}

fn endpoint(base: &str) -> Result<url::Url, String> {
    let mut url = provider::parse_base_url(base)?;
    if url.scheme() != "https" {
        return Err("The speech host URL must use HTTPS.".into());
    }
    if url.path() != "/" {
        return Err("Enter the speech host URL without a path.".into());
    }
    url.set_path("/v1/host");
    Ok(url)
}

fn valid_token(token: &[u8]) -> bool {
    !token.is_empty() && token.len() <= 8192 && token.iter().all(u8::is_ascii_graphic)
}

impl Host {
    pub fn recipient(&self) -> &str {
        &self.endpoint[..url::Position::BeforePath]
    }

    pub fn save_and_consent(path: &Path, base: &str, token: &[u8]) -> Result<Self, String> {
        let endpoint = endpoint(base)?;
        if !valid_token(token) {
            return Err("Enter a device token for the speech host.".into());
        }
        let mut record = Zeroizing::new(Vec::new());
        record.extend_from_slice(endpoint[..url::Position::BeforePath].as_bytes());
        record.push(b'\n');
        record.extend_from_slice(token);
        // Seal the recipient and token together so consent cannot carry over to an edited URL.
        let encrypted = protect(&record).map_err(|_| STORE_ERROR)?;
        store_io::atomic_write(path, &encrypted).map_err(|_| STORE_ERROR)?;
        Ok(Self {
            endpoint,
            token: Zeroizing::new(token.to_vec()),
        })
    }

    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let bytes = match store_io::read_bounded(path, 64 * 1024) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(STORE_ERROR.into()),
        };
        let record = unprotect(&bytes).map_err(|_| STORE_ERROR)?;
        let record = std::str::from_utf8(&record).map_err(|_| STORE_ERROR)?;
        let (base, token) = record.split_once('\n').ok_or(STORE_ERROR)?;
        if !valid_token(token.as_bytes()) {
            return Err(STORE_ERROR.into());
        }
        Ok(Some(Self {
            endpoint: endpoint(base)?,
            token: Zeroizing::new(token.as_bytes().to_vec()),
        }))
    }

    pub fn monitor(self, wake: impl Fn() + Send + 'static) -> Result<Monitor, String> {
        Monitor::start(INTERVAL, move || self.probe(), wake)
    }

    fn probe(&self) -> Option<Asr> {
        let body = request(&self.endpoint, &self.token)?;
        let host: Reply = serde_json::from_slice(&body).ok()?;
        (host.api == 1 && host.role == "server").then_some(host.asr.state)
    }
}

#[derive(Deserialize)]
struct Reply {
    api: u32,
    role: String,
    asr: AsrReply,
}

#[derive(Deserialize)]
struct AsrReply {
    state: Asr,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Asr {
    Ready,
    Loading,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Checking,
    Ready,
    Loading,
    Unavailable,
    Amber,
    Away,
}

impl Status {
    fn after(self, reply: Option<Asr>) -> Self {
        match reply {
            Some(Asr::Ready) => Self::Ready,
            Some(Asr::Loading) => Self::Loading,
            Some(Asr::Unavailable) => Self::Unavailable,
            None if matches!(self, Self::Amber | Self::Away) => Self::Away,
            None => Self::Amber,
        }
    }
}

pub struct Monitor {
    _stop: mpsc::Sender<()>,
    updates: mpsc::Receiver<Status>,
}

impl Monitor {
    fn start(
        interval: Duration,
        mut probe: impl FnMut() -> Option<Asr> + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let (stop, cancel) = mpsc::channel();
        let (send, updates) = mpsc::channel();
        std::thread::Builder::new()
            .name("speech-host-status".into())
            .spawn(move || {
                let mut status = Status::Checking;
                while cancel.try_recv() == Err(mpsc::TryRecvError::Empty) {
                    let started = Instant::now();
                    let next = status.after(probe());
                    if next != status {
                        status = next;
                        if send.send(status).is_err() {
                            break;
                        }
                        wake();
                    }
                    if cancel.recv_timeout(interval.saturating_sub(started.elapsed()))
                        != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
            })
            .map_err(|_| "Could not start the speech host status check.".to_owned())?;
        Ok(Self {
            _stop: stop,
            updates,
        })
    }

    pub fn take_status(&self) -> Option<Status> {
        self.updates.try_iter().last()
    }
}

pub fn clear(path: &Path, monitor: &mut Option<Monitor>) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not clear the speech host settings.".into()),
    }
    *monitor = None;
    Ok(())
}

fn request(url: &url::Url, token: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    use std::{ffi::c_void, ptr};
    use windows_sys::{Win32::Networking::WinHttp::*, core::w};

    struct Handle(ptr::NonNull<c_void>);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { WinHttpCloseHandle(self.0.as_ptr()) };
        }
    }
    let host: Vec<u16> = url
        .host_str()?
        .trim_matches(['[', ']'])
        .encode_utf16()
        .chain([0])
        .collect();
    let mut auth = Zeroizing::new(Vec::with_capacity(24 + token.len()));
    auth.extend("Authorization: Bearer ".encode_utf16());
    auth.extend(token.iter().map(|&b| u16::from(b)));
    auth.extend([13, 10]);
    let flags = if cfg!(test) && url.scheme() == "http" {
        0
    } else {
        WINHTTP_FLAG_SECURE
    };
    let start = Instant::now();
    // These buffers outlive the synchronous calls; each handle is closed in reverse order.
    unsafe {
        let handle = |raw| ptr::NonNull::new(raw).map(Handle);
        let session = handle(WinHttpOpen(
            w!("Sidenote"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        ))?;
        if WinHttpSetTimeouts(session.0.as_ptr(), 5000, 5000, 5000, 5000) == 0 {
            return None;
        }
        let connection = handle(WinHttpConnect(
            session.0.as_ptr(),
            host.as_ptr(),
            url.port_or_known_default()?,
            0,
        ))?;
        let request = handle(WinHttpOpenRequest(
            connection.0.as_ptr(),
            w!("GET"),
            w!("/v1/host"),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            flags,
        ))?;
        let raw = request.0.as_ptr();
        let disabled =
            WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        let option_bytes = std::mem::size_of::<u32>() as u32;
        if WinHttpSetOption(
            raw,
            WINHTTP_OPTION_DISABLE_FEATURE,
            ptr::from_ref(&disabled).cast(),
            option_bytes,
        ) == 0
            || WinHttpSendRequest(raw, auth.as_ptr(), auth.len() as u32, ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(raw, ptr::null_mut()) == 0
        {
            return None;
        }
        let mut status = 0u32;
        if WinHttpQueryHeaders(
            raw,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            ptr::from_mut(&mut status).cast(),
            &mut { option_bytes },
            ptr::null_mut(),
        ) == 0
            || status != 200
        {
            return None;
        }
        let mut bytes = Zeroizing::new(vec![0; 16 * 1024]);
        let mut len = 0;
        loop {
            let mut read = 0;
            if start.elapsed() >= Duration::from_secs(15)
                || len == bytes.len()
                || WinHttpReadData(
                    raw,
                    bytes[len..].as_mut_ptr().cast(),
                    (bytes.len() - len) as u32,
                    &mut read,
                ) == 0
            {
                return None;
            }
            if read == 0 {
                bytes.truncate(len);
                return Some(bytes);
            }
            len += read as usize;
        }
    }
}
