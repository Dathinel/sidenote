//! Local status contains no group locators, protocol payloads or keys.
use super::*;
use sidenote_sync::capabilities::Capabilities;
pub const UPDATE_REQUIRED: &str =
    "Another device runs a newer Sidenote. Update this one to keep syncing.";
pub fn build_capabilities() -> Capabilities {
    Capabilities {
        schema: Capabilities::CURRENT.schema,
        features: if cfg!(feature = "relay-capability-v2") {
            2
        } else {
            Capabilities::CURRENT.features
        },
    }
}
pub(super) fn supported(_root: &Path) -> Capabilities {
    #[cfg(any(debug_assertions, feature = "relay-qualification"))]
    let root = _root;
    #[cfg(any(debug_assertions, feature = "relay-qualification"))]
    // Only the isolated headless proof can impersonate a future application build.
    if super::super::is_proof_path(root, "sidenote-relay-preview")
        && std::env::args_os().any(|a| a == "--relay-preview-headless")
        && root.join("local/relay/test-capabilities.json").is_file()
        && let Ok(c) =
            store_io::read::<Capabilities>(&root.join("local/relay/test-capabilities.json"))
    {
        return c;
    }
    build_capabilities()
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Operational {
    pub read_only: bool,
    pub message: String,
    pub last_contact: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Contact {
    address: String,
    at: Option<String>,
}
struct Health {
    address: String,
    checked: std::time::Instant,
    generation: u64,
    message: String,
    contact: Option<String>,
}
static HEALTH: OnceLock<Mutex<BTreeMap<PathBuf, Health>>> = OnceLock::new();
fn health() -> &'static Mutex<BTreeMap<PathBuf, Health>> {
    HEALTH.get_or_init(Mutex::default)
}
#[derive(Debug, PartialEq)]
enum ProbeError {
    Invalid,
    Unreachable,
    WrongService,
}
fn probe(address: &str) -> Result<(), ProbeError> {
    endpoint(address).map_err(|_| ProbeError::Invalid)?;
    let mut url = url::Url::parse(address).map_err(|_| ProbeError::Invalid)?;
    let scheme = if url.scheme() == "ws" {
        "http"
    } else {
        "https"
    };
    url.set_scheme(scheme).map_err(|_| ProbeError::Invalid)?;
    url.set_path("/health");
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url.as_str())
        .call()
        .map_err(|_| ProbeError::Unreachable)?;
    if response.status().as_u16() != 200 {
        return Err(ProbeError::WrongService);
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(65)
        .read_to_end(&mut bytes)
        .map_err(|_| ProbeError::Unreachable)?;
    if bytes != b"sidenote-relay-v1" {
        return Err(ProbeError::WrongService);
    }
    Ok(())
}
fn begin(root: &Path, address: &str, force: bool) -> io::Result<Option<u64>> {
    let mut entries = health().lock().map_err(failed)?;
    if let Some(h) = entries.get(root)
        && h.address == address
        && !force
        && h.checked.elapsed() < std::time::Duration::from_secs(15)
    {
        return Ok(None);
    }
    let saved: Contact = store_io::read(&root.join("local/relay/contact.json"))?;
    let generation = entries.get(root).map_or(1, |h| h.generation + 1);
    entries.insert(
        root.into(),
        Health {
            address: address.into(),
            checked: std::time::Instant::now(),
            generation,
            message: "Checking relay...".into(),
            contact: (saved.address == address).then_some(saved.at).flatten(),
        },
    );
    Ok(Some(generation))
}
fn finish(root: &Path, address: &str, generation: u64) {
    let result = probe(address);
    let Ok(mut entries) = health().lock() else {
        return;
    };
    let Some(h) = entries
        .get_mut(root)
        .filter(|h| h.generation == generation && h.address == address)
    else {
        return;
    };
    if !Settings::load(root).is_ok_and(|s| s.enabled && s.address == address) {
        return;
    }
    h.message = match result {
        Ok(()) => {
            let at = chrono::Utc::now().to_rfc3339();
            if store_io::write(
                &root.join("local/relay/contact.json"),
                &Contact {
                    address: address.into(),
                    at: Some(at.clone()),
                },
            )
            .is_ok()
            {
                h.contact = Some(at);
            }
            "Relay reachable. Both devices must be online to exchange changes.".into()
        }
        Err(ProbeError::Invalid) => "Enter a valid relay address in Advanced.".into(),
        Err(ProbeError::Unreachable) => {
            "Relay unavailable. Check the address and your connection. Edits stay on this PC."
                .into()
        }
        Err(ProbeError::WrongService) => {
            "This address did not answer as a Sidenote relay. Check the address in Advanced.".into()
        }
    };
}
pub fn check_now(root: &Path) -> io::Result<Operational> {
    #[cfg(target_os = "linux")]
    if cfg!(target_os = "linux") {
        return Ok(Operational {
            read_only: true,
            message: sidenote_sync::RELAY_UNAVAILABLE.into(),
            last_contact: None,
        });
    }
    let settings = Settings::load(root)?;
    if let Some(generation) = begin(root, &settings.address, true)? {
        finish(root, &settings.address, generation);
    }
    status(root)
}
impl Preview {
    pub fn operational(&self) -> io::Result<Operational> {
        let read_only = self.device.store(|s| s.check_compatible()).is_err();
        let settings = Settings::load(&self.root)?;
        if !settings.address.is_empty()
            && let Some(generation) = begin(&self.root, &settings.address, false)?
        {
            let (root, address) = (self.root.clone(), settings.address.clone());
            std::thread::spawn(move || finish(&root, &address, generation));
        }
        let entries = health().lock().map_err(failed)?;
        let entry = entries
            .get(&self.root)
            .filter(|h| h.address == settings.address);
        Ok(Operational {
            read_only,
            message: if read_only {
                UPDATE_REQUIRED.into()
            } else if settings.address.is_empty() {
                "Add a relay address in Advanced to connect.".into()
            } else {
                entry.map(|h| h.message.clone()).unwrap_or_default()
            },
            last_contact: entry.and_then(|h| h.contact.clone()),
        })
    }
}
pub fn status(root: &Path) -> io::Result<Operational> {
    #[cfg(target_os = "linux")]
    if cfg!(target_os = "linux") {
        return Ok(Operational {
            read_only: true,
            message: sidenote_sync::RELAY_UNAVAILABLE.into(),
            last_contact: None,
        });
    }
    get(root)?
        .map(|p| p.operational())
        .unwrap_or_else(|| Ok(Operational::default()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn health_server(body: &'static [u8]) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..16 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                // Closing with unread request bytes can reset TCP and discard the response.
                let mut header = Vec::new();
                while header.len() < 8192 && !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                assert!(header.ends_with(b"\r\n\r\n"));
                // Box port discovery sends HEAD / before the test's health request.
                if !header.starts_with(b"GET /health HTTP/1.1\r\n") {
                    socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                }
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(body).unwrap();
                return;
            }
            panic!("health fixture received too many unrelated requests");
        });
        (address, worker)
    }
    #[test]
    fn health_fixture_keeps_listening_after_an_unrelated_port_probe() {
        use std::io::{Read, Write};
        let (address, worker) = health_server(b"sidenote-relay-v1");
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        socket
            .write_all(b"HEAD / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        assert!(response.starts_with(b"HTTP/1.1 404"));
        assert_eq!(probe(&format!("ws://{address}")), Ok(()));
        worker.join().unwrap();
    }
    #[test]
    fn health_fixture_waits_for_fragmented_request_headers() {
        use std::io::{Read, Write};
        let (address, worker) = health_server(b"sidenote-relay-v1");
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        socket.write_all(b"GET /health HTTP/1.1\r\nHost: ").unwrap();
        let error = socket.read(&mut [0]).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        socket.write_all(b"localhost\r\n\r\n").unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        assert!(response.ends_with(b"sidenote-relay-v1"));
        worker.join().unwrap();
    }
    #[test]
    fn invalid_unreachable_and_wrong_service_are_distinct() {
        assert_eq!(probe("ws://localhost:8787"), Err(ProbeError::Invalid));
        let (address, worker) = health_server(b"wrong");
        assert_eq!(
            probe(&format!("ws://{address}")),
            Err(ProbeError::WrongService)
        );
        worker.join().unwrap();
        assert_eq!(
            probe(&format!("ws://{address}")),
            Err(ProbeError::Unreachable)
        );
    }
    #[test]
    #[cfg(windows)]
    fn successful_contact_survives_outage_and_a_fresh_status_cache() {
        let parent = std::env::temp_dir().join("sidenote-relay-preview");
        std::fs::create_dir_all(&parent).unwrap();
        let root = tempfile::tempdir_in(parent).unwrap();
        store_io::write(
            &root.path().join("device.json"),
            &super::super::super::DeviceSettings {
                name: "Synthetic health PC".into(),
                id: "e".repeat(32),
                daily_check: false,
                role: Default::default(),
            },
        )
        .unwrap();
        let (address, worker) = health_server(b"sidenote-relay-v1");
        Settings {
            enabled: true,
            address: format!("ws://{address}"),
            ..Default::default()
        }
        .save(root.path())
        .unwrap();
        let first = check_now(root.path()).unwrap();
        worker.join().unwrap();
        assert!(first.last_contact.is_some(), "{}", first.message);
        let failed = check_now(root.path()).unwrap();
        assert!(failed.message.starts_with("Relay unavailable."));
        assert_eq!(failed.last_contact, first.last_contact);
        health().lock().unwrap().remove(root.path());
        let reloaded = status(root.path()).unwrap();
        assert_eq!(reloaded.last_contact, first.last_contact);
        disconnect(root.path()).unwrap();
    }
}
