//! Hex 7681742's loopback-only, ID-only completion stream. Never replay credentials.
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    OptedOut,
    Connected,
    Disabled,
    Busy,
    Offline,
    Unauthorized,
    InvalidInput,
    Stopped,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ControlState {
    Unsupported,
}

pub struct Connection {
    stream: TcpStream,
    pending: Vec<u8>,
    last_read: Instant,
}
impl Connection {
    /// The caller supplies the platform-resolved local-api.json path. Only connect reads it.
    pub fn connect(discovery: &Path) -> Result<Self, State> {
        #[derive(Deserialize)]
        struct Document {
            port: u16,
            token: String,
        }
        let mut bytes = Zeroizing::new(Vec::new());
        File::open(discovery)
            .and_then(|file| file.take(4097).read_to_end(&mut bytes))
            .map_err(|_| State::Offline)?;
        if bytes.len() > 4096 {
            return Err(State::InvalidInput);
        }
        let mut doc: Document = serde_json::from_slice(&bytes).map_err(|_| State::InvalidInput)?;
        let token = Zeroizing::new(std::mem::take(&mut doc.token));
        if doc.port == 0
            || token.is_empty()
            || token.len() > 256
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(State::InvalidInput);
        }
        let deadline = Duration::from_secs(2);
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, doc.port));
        let mut stream =
            TcpStream::connect_timeout(&address, deadline).map_err(|_| State::Offline)?;
        stream
            .set_read_timeout(Some(deadline))
            .map_err(|_| State::Offline)?;
        stream
            .set_write_timeout(Some(deadline))
            .map_err(|_| State::Offline)?;
        let request = Zeroizing::new(format!(
            "GET /meetings/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\n\r\n",
            *token
        ));
        stream
            .write_all(request.as_bytes())
            .map_err(|_| State::Offline)?;
        let mut headers = Zeroizing::new(Vec::new());
        let started = Instant::now();
        while !headers.ends_with(b"\r\n\r\n") {
            if headers.len() >= 4096 || started.elapsed() >= deadline {
                return Err(State::InvalidInput);
            }
            let mut byte = [0];
            stream.read_exact(&mut byte).map_err(|_| State::Offline)?;
            headers.push(byte[0]);
        }
        let headers = std::str::from_utf8(&headers).map_err(|_| State::InvalidInput)?;
        match headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        {
            Some("200") => {}
            Some("404") => return Err(State::Disabled),
            Some("409") => return Err(State::Busy),
            Some("401" | "403") => return Err(State::Unauthorized),
            _ => return Err(State::InvalidInput),
        }
        if !headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("Content-Type: text/event-stream"))
        {
            return Err(State::InvalidInput);
        }
        stream.set_nonblocking(true).map_err(|_| State::Offline)?;
        Ok(Self {
            stream,
            pending: Vec::new(),
            last_read: Instant::now(),
        })
    }

    /// Bounded work per caller tick; IDs only wake the authoritative folder scanner.
    pub fn poll(&mut self) -> Result<(), State> {
        for _ in 0..4096 {
            let mut byte = [0];
            match self.stream.read(&mut byte) {
                Ok(0) => return Err(State::Offline),
                Ok(_) => {
                    self.last_read = Instant::now();
                    self.pending.push(byte[0]);
                    if self.pending.len() > 1024 {
                        return Err(State::InvalidInput);
                    }
                    if byte[0] == b'\n' {
                        let result = Self::line(&self.pending);
                        self.pending.zeroize();
                        self.pending.clear();
                        result?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return Err(State::Offline),
            }
        }
        if self.last_read.elapsed() > Duration::from_secs(45) {
            return Err(State::Offline);
        }
        Ok(())
    }

    fn line(line: &[u8]) -> Result<(), State> {
        let line = std::str::from_utf8(line)
            .map_err(|_| State::InvalidInput)?
            .trim_end();
        if line.is_empty() || line.starts_with(':') {
            return Ok(());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Event {
            r#type: String,
            id: String,
        }
        let data = line.strip_prefix("data: ").ok_or(State::InvalidInput)?;
        let event: Event = serde_json::from_str(data).map_err(|_| State::InvalidInput)?;
        if event.r#type != "meeting.finished"
            || event.id.is_empty()
            || event.id.len() > 100
            || !event.id.bytes().any(|b| b.is_ascii_digit())
            || !event.id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        {
            return Err(State::InvalidInput);
        }
        Ok(())
    }
}

/// Opt-in SSE subscription with folder catch-up on every tick, including outages.
/// Use inside Session so reconnects and restarts share its durable delivery receipts.
pub struct Events {
    folders: super::folders::Discovery,
    discovery: std::path::PathBuf,
    connection: Option<Connection>,
    state: State,
    retry_at: Instant,
    backoff: Duration,
}
impl Events {
    /// Resolve the discovery path at the platform edge; constructing this reads nothing.
    pub fn new(
        folders: super::folders::Discovery,
        discovery: std::path::PathBuf,
        opt_in: bool,
    ) -> Result<Self, super::Error> {
        if !discovery.is_absolute()
            || discovery
                .components()
                .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(super::Error::InvalidInput);
        }
        Ok(Self {
            folders,
            discovery,
            connection: None,
            state: if opt_in {
                State::Offline
            } else {
                State::OptedOut
            },
            retry_at: Instant::now(),
            backoff: Duration::from_secs(1),
        })
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// No control request is attempted: pinned Windows/Linux control is unqualified.
    pub fn control_state(&self) -> ControlState {
        ControlState::Unsupported
    }

    fn failed(&mut self, state: State) {
        self.connection = None;
        self.state = state;
        self.retry_at = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(Duration::from_secs(30));
    }

    fn tick(&mut self) {
        if matches!(self.state, State::OptedOut | State::Stopped) {
            return;
        }
        if let Some(connection) = &mut self.connection {
            if let Err(state) = connection.poll() {
                self.failed(state);
            }
        } else if Instant::now() >= self.retry_at {
            match Connection::connect(&self.discovery) {
                Ok(connection) => {
                    self.connection = Some(connection);
                    self.state = State::Connected;
                    self.backoff = Duration::from_secs(1);
                }
                Err(state) => self.failed(state),
            }
        }
    }
}
impl super::Source for Events {
    fn start(&mut self) -> Result<(), super::Error> {
        self.tick(); // Subscribe before the first catch-up; failure still allows folder fallback.
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<super::CompletedMeeting>, super::Error> {
        super::Source::catch_up(self).map(|(completed, _)| completed)
    }

    fn catch_up(&mut self) -> Result<super::CatchUp, super::Error> {
        if self.state == State::Stopped {
            return Err(super::Error::Stopped);
        }
        if self.state == State::OptedOut {
            return Ok((Vec::new(), None));
        }
        self.tick();
        super::Source::catch_up(&mut self.folders)
    }

    fn stop(&mut self) {
        self.connection = None;
        self.state = State::Stopped;
    }
}

#[cfg(test)]
mod tests;
