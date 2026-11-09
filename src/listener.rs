use serde::Deserialize;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    time::Duration,
};

const HIDDEN: u32 = 0x08000000;
#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}
enum Problem {
    Http(u16),
    Io(io::ErrorKind),
    Value,
}
impl From<io::Error> for Problem {
    fn from(e: io::Error) -> Self {
        Self::Io(e.kind())
    }
}
impl Problem {
    fn message(&self) -> String {
        match self {
            Self::Http(404) => "Labs > Meeting finished event is off".into(),
            Self::Http(409) => "another app is already listening".into(),
            Self::Http(code) => format!("HTTP {code}"),
            Self::Value => "Hex not reachable (JSONDecodeError)".into(),
            Self::Io(kind) => format!(
                "Hex not reachable ({})",
                match kind {
                    io::ErrorKind::NotFound => "FileNotFoundError",
                    io::ErrorKind::PermissionDenied => "PermissionError",
                    io::ErrorKind::ConnectionRefused => "URLError",
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "TimeoutError",
                    _ => "OSError",
                }
            ),
        }
    }
}
fn log(message: &str) {
    let path = crate::core::log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "{} listener: {message}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
    }
}
fn pipeline(meeting_id: Option<&str>) -> io::Result<()> {
    let activity = crate::updates::activity()?;
    if let Some(id) = meeting_id
        && crate::core::meeting_router::jobs::configured()
    {
        if crate::core::meeting_router::jobs::enqueue(id).is_err() {
            log("could not queue meeting routing; existing pipeline continues");
        } else if crate::core::meeting_router::jobs::start_worker().is_err() {
            log("routing remains queued for the next drain");
        }
    }
    let script = crate::core::live::notes_pipeline();
    if !script.is_file() {
        log(crate::core::live::NOTES_UNAVAILABLE);
        return Ok(());
    }
    let root = crate::core::meetings_dir();
    let folders: Vec<_> = fs::read_dir(&root)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|folder| {
            !folder.join("english-summary.json").is_file()
                && (meeting_id.is_some_and(|id| folder.file_name().is_some_and(|n| n == id))
                    || fs::read(folder.join("manifest.json"))
                        .ok()
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                        .is_some_and(|m| m["status"] == "complete"))
        })
        .collect();
    let child = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .creation_flags(HIDDEN)
        .spawn();
    std::thread::spawn(move || {
        let _activity = activity;
        let succeeded = child
            .and_then(|mut child| child.wait())
            .is_ok_and(|status| status.success());
        for folder in folders {
            if !succeeded {
                let _ = fs::write(folder.join("notes-failed"), b"failed");
            } else {
                let _ = fs::remove_file(folder.join("notes-failed"));
            }
        }
        if !succeeded {
            log("Notes pipeline failed; the recording and personal notes are saved");
        }
    });
    Ok(())
}
fn catch_up() -> io::Result<()> {
    let _activity = crate::updates::activity()?;
    if crate::core::meeting_router::jobs::configured() {
        for entry in fs::read_dir(crate::core::meetings_dir())? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                continue;
            }
            let complete = fs::read(entry.path().join("manifest.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .is_some_and(|manifest| manifest["status"] == "complete");
            if complete {
                crate::core::meeting_router::jobs::enqueue(&id).map_err(io::Error::other)?;
            }
        }
        crate::core::meeting_router::jobs::start_worker()?;
    }
    pipeline(None)
}
// A streaming HTTP body, with no TLS or proxy needed for the loopback API.
struct Body {
    stream: BufReader<TcpStream>,
    chunked: bool,
    remaining: usize,
    ended: bool,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.ended {
            return Ok(0);
        }
        if !self.chunked {
            return self.stream.read(out);
        }
        if self.remaining == 0 {
            let mut line = String::new();
            self.stream.read_line(&mut line)?;
            self.remaining = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            if self.remaining == 0 {
                self.ended = true;
                return Ok(0);
            }
        }
        let len = out.len().min(self.remaining);
        let n = self.stream.read(&mut out[..len])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= n;
        if self.remaining == 0 {
            let mut end = [0; 2];
            self.stream.read_exact(&mut end)?;
            if end != *b"\r\n" {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(n)
    }
}
fn listen() -> Result<(), Problem> {
    let path =
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("hex/local-api.json");
    let config: Discovery = serde_json::from_slice(&fs::read(path)?).map_err(|_| Problem::Value)?;
    // Reject header injection without ever including the credential in an error.
    if config.token.contains(['\r', '\n']) {
        return Err(Problem::Value);
    }
    let addr: SocketAddr = ([127, 0, 0, 1], config.port).into();
    let mut socket = TcpStream::connect_timeout(&addr, Duration::from_secs(60))?;
    let (watch, stopped) = std::sync::mpsc::channel::<()>();
    let _watch = watch;
    if let Ok(control) = socket.try_clone() {
        std::thread::spawn(move || {
            while !crate::updates::listener_stopping() {
                if matches!(
                    stopped.recv_timeout(Duration::from_millis(250)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ) {
                    return;
                }
            }
            let _ = control.shutdown(std::net::Shutdown::Both);
        });
    }
    socket.set_read_timeout(Some(Duration::from_secs(60)))?;
    socket.set_write_timeout(Some(Duration::from_secs(60)))?;
    write!(
        socket,
        "GET /meetings/events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        config.port, config.token
    )?;
    let mut stream = BufReader::new(socket);
    let mut header = String::new();
    stream.read_line(&mut header)?;
    let status = header
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(Problem::Value)?;
    if !(200..300).contains(&status) {
        return Err(Problem::Http(status));
    }
    let mut chunked = false;
    loop {
        header.clear();
        let n = stream.read_line(&mut header)?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if header
            .to_ascii_lowercase()
            .starts_with("transfer-encoding:")
            && header.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    log("connected");
    crate::hex_engine::meeting_event();
    pipeline(None)?;
    let mut body = BufReader::new(Body {
        stream,
        chunked,
        remaining: 0,
        ended: false,
    });
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if body.read_until(b'\n', &mut raw)? == 0 {
            break;
        }
        let decoded = String::from_utf8_lossy(&raw);
        let Some(data) = decoded.trim().strip_prefix("data:") else {
            continue;
        };
        let event: serde_json::Value = serde_json::from_str(data).map_err(|_| Problem::Value)?;
        crate::hex_engine::meeting_event();
        if event.get("type").and_then(|v| v.as_str()) == Some("meeting.finished") {
            let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("None");
            // Only meeting ids belong in the shared log, never server-provided error bodies.
            if id.chars().all(|c| c.is_ascii_digit() || c == '-') {
                log(&format!("meeting finished {id}"));
            } else {
                log("meeting finished None");
            }
            let valid = !id.is_empty()
                && id.bytes().any(|c| c.is_ascii_digit())
                && id.bytes().all(|c| c.is_ascii_digit() || c == b'-');
            pipeline(valid.then_some(id))?;
        }
    }
    Ok(())
}
pub fn run() -> ! {
    let _instance = match crate::updates::listener_lock() {
        Ok(lock) if !crate::updates::listener_stopping() => lock,
        _ => std::process::exit(0),
    };
    let isolated = std::env::args().any(|a| a == "--listener-isolated")
        && std::env::args().any(|a| a == "--data-dir");
    if isolated {
        while !crate::updates::listener_stopping() {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::process::exit(0);
    }
    // The watcher scans completed meetings even when the event API is unavailable.
    if catch_up().is_err() {
        log("startup catch-up could not start; retrying on the next connection");
    }
    std::thread::spawn(|| {
        while !crate::updates::listener_stopping() {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::process::exit(0);
    });
    std::thread::spawn(|| {
        loop {
            let _ = crate::core::meeting_router::jobs::start_worker();
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    let mut last = String::new();
    loop {
        let problem = match listen() {
            Ok(()) => "stream ended".into(),
            Err(e) => e.message(),
        };
        if problem != last {
            log(&format!("{problem}; retrying every 30 s"));
            last = problem;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}
