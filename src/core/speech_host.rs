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

pub mod journal;
#[cfg(test)]
mod tests;

pub(super) mod upload;

const INTERVAL: Duration = Duration::from_secs(30);
const STORE_ERROR: &str = "Could not save or read the protected speech host settings.";

pub fn settings_path() -> Result<PathBuf, String> {
    Ok(provider::secure_directory("speech-host")?.join("host.dpapi"))
}

pub struct Host {
    endpoint: url::Url,
    token: Zeroizing<Vec<u8>>,
    upload_since: Option<i64>,
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
        Self::save(path, base, token, None)
    }

    pub fn save_with_upload_consent(path: &Path, base: &str, token: &[u8]) -> Result<Self, String> {
        Self::save(
            path,
            base,
            token,
            Some(chrono::Utc::now().timestamp_millis()),
        )
    }

    fn save(
        path: &Path,
        base: &str,
        token: &[u8],
        upload_since: Option<i64>,
    ) -> Result<Self, String> {
        let endpoint = endpoint(base)?;
        if !valid_token(token) {
            return Err("Enter a device token for the speech host.".into());
        }
        let mut record = Zeroizing::new(Vec::new());
        record.extend_from_slice(endpoint[..url::Position::BeforePath].as_bytes());
        record.push(b'\n');
        record.extend_from_slice(token);
        if let Some(since) = upload_since {
            record.push(b'\n');
            record.extend_from_slice(since.to_string().as_bytes());
        }
        // Seal the recipient and token together so consent cannot carry over to an edited URL.
        let encrypted = protect(&record).map_err(|_| STORE_ERROR)?;
        store_io::atomic_write(path, &encrypted).map_err(|_| STORE_ERROR)?;
        Ok(Self {
            endpoint,
            token: Zeroizing::new(token.to_vec()),
            upload_since,
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
        let (token, upload_since) = match token.split_once('\n') {
            Some((token, since)) => (token, Some(since.parse::<i64>().map_err(|_| STORE_ERROR)?)),
            None => (token, None),
        };
        if !valid_token(token.as_bytes()) {
            return Err(STORE_ERROR.into());
        }
        Ok(Some(Self {
            endpoint: endpoint(base)?,
            token: Zeroizing::new(token.as_bytes().to_vec()),
            upload_since,
        }))
    }

    pub fn upload_enabled(&self) -> bool {
        self.upload_since.is_some()
    }

    pub fn stop_audio_uploads(&self, path: &Path) -> Result<Self, String> {
        Self::save(path, self.recipient(), &self.token, None)
    }

    fn get(&self, path: &str) -> Option<(u32, Zeroizing<Vec<u8>>)> {
        self.bytes(
            Request {
                method: "GET",
                path,
                headers: "",
                body: &[],
            },
            16 * 1024,
        )
    }

    fn bytes(&self, call: Request<'_>, limit: usize) -> Option<(u32, Zeroizing<Vec<u8>>)> {
        let mut body = Zeroizing::new(Vec::new());
        let status = request(&self.endpoint, &self.token, call, |_, data| {
            if data.len() > limit - body.len() {
                return false;
            }
            body.extend_from_slice(data);
            true
        })?;
        Some((status, body))
    }

    pub fn monitor(self, wake: impl Fn() + Send + 'static) -> Result<Monitor, String> {
        Monitor::start(INTERVAL, move || self.probe(), wake)
    }

    fn probe(&self) -> Option<Status> {
        let (status, body) = self.get("/v1/host")?;
        match status {
            401 | 403 => return Some(Status::Rejected),
            200 => {}
            _ => return None,
        }
        let host: Reply = serde_json::from_slice(&body).ok()?;
        (host.api == 1 && host.role == "server").then_some(match host.asr.state {
            Asr::Ready => Status::Ready,
            Asr::Unavailable => Status::Unavailable,
        })
    }
}

#[derive(Deserialize)]
struct Reply {
    api: u32,
    role: String,
    asr: AsrReply,
    #[serde(default)]
    speech_jobs: bool,
}

#[derive(Deserialize)]
struct AsrReply {
    state: Asr,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    models: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Asr {
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Checking,
    Ready,
    Rejected,
    Unavailable,
    Amber,
    Away,
}

impl Status {
    fn after(self, reply: Option<Self>) -> Self {
        match reply {
            Some(status) => status,
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
        mut probe: impl FnMut() -> Option<Status> + Send + 'static,
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

struct Request<'a> {
    method: &'a str,
    path: &'a str,
    headers: &'a str,
    body: &'a [u8],
}

fn request(
    url: &url::Url,
    token: &[u8],
    call: Request<'_>,
    mut read_body: impl FnMut(u32, &[u8]) -> bool,
) -> Option<u32> {
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
    auth.extend(call.headers.encode_utf16());
    let method: Vec<u16> = call.method.encode_utf16().chain([0]).collect();
    let path: Vec<u16> = call.path.encode_utf16().chain([0]).collect();
    let len = u32::try_from(call.body.len()).ok()?;
    let flags = if cfg!(test) && url.host_str() == Some("127.0.0.1") {
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
            method.as_ptr(),
            path.as_ptr(),
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
            || WinHttpSendRequest(
                raw,
                auth.as_ptr(),
                auth.len() as u32,
                if len == 0 {
                    ptr::null()
                } else {
                    call.body.as_ptr().cast()
                },
                len,
                len,
                0,
            ) == 0
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
        {
            return None;
        }
        if !matches!(status, 200 | 201) {
            return Some(status);
        }
        let mut bytes = Zeroizing::new([0u8; 4096]);
        loop {
            let mut read = 0;
            if start.elapsed() >= Duration::from_secs(15)
                || WinHttpReadData(
                    raw,
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    &mut read,
                ) == 0
            {
                return None;
            }
            if read == 0 {
                return Some(status);
            }
            if !read_body(status, &bytes[..read as usize]) {
                return None;
            }
        }
    }
}
