//! Meeting-only model files. This module never invokes Hex or loads model code.
use super::{model_catalog_data::MODELS, state_dir, store_io};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Hex,
    SherpaOnnx,
}
impl Runtime {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::SherpaOnnx => "sherpa-onnx",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Asr,
    LanguageId,
    Diarization,
    VoiceActivity,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeTier {
    Default,
    OptIn,
}

pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub languages: &'static [&'static str],
    pub runtime: Runtime,
    pub role: Role,
    pub files: &'static [ModelFile],
    pub licence: &'static str,
    pub minimum_ram_mb: Option<u32>,
    pub source: &'static str,
    pub runtime_note: &'static str,
}

/// Slowest sustained download accepted after the first minute (16 KB/s, about 128 kbit/s).
const MIN_BYTES_PER_SEC: u64 = 16 * 1024;

// Small defaults. Hindi IndicConformer and Anuvani are measured (ADR 0009 (b), 2026-11-09:
// 12.8 WER on Hindi; 1% of Hindi and 5% of mixed clips missed). The other IndicConformer
// languages are the only small option found and are unmeasured. Anuvani is recommended only
// when more than one language is declared; the Settings screen applies that rule.
pub const PROVISIONAL_DEFAULT_IDS: &[&str] = &[
    "anuvani-lid-int8",
    "indicconformer-hi-ctc",
    "indicconformer-bn-ctc",
    "indicconformer-ta-ctc",
    "indicconformer-te-ctc",
    "indicconformer-mr-ctc",
];
impl Model {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
    pub fn size_tier(&self) -> SizeTier {
        if PROVISIONAL_DEFAULT_IDS.contains(&self.id) {
            SizeTier::Default
        } else {
            SizeTier::OptIn
        }
    }
}
pub fn catalog() -> &'static [Model] {
    MODELS
}
pub fn model(id: &str) -> io::Result<&'static Model> {
    catalog()
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| store_io::invalid("Unknown model id"))
}

#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Downloading,
    Verifying,
}
#[derive(Clone, Debug)]
pub struct Progress {
    pub downloaded: u64,
    pub total: u64,
    pub phase: Phase,
}
#[derive(Clone)]
pub struct ModelStore {
    root: PathBuf,
}
impl Default for ModelStore {
    fn default() -> Self {
        Self::new(state_dir())
    }
}
impl ModelStore {
    pub fn new(state: PathBuf) -> Self {
        Self {
            root: state.join("models"),
        }
    }
    fn dir(&self, m: &Model) -> PathBuf {
        self.root.join(m.id)
    }
    fn lock(&self, m: &Model) -> io::Result<File> {
        store_io::lock(&self.root.join(format!("{}.lock", m.id)))
    }
    /// Only complete, verified installations have a receipt. Never returns Hex's paths.
    pub fn installed_paths(&self, id: &str) -> io::Result<Option<Vec<PathBuf>>> {
        let m = model(id)?;
        if m.runtime == Runtime::Hex {
            return Ok(None);
        }
        reject_link(&self.root)?;
        let dir = self.dir(m);
        reject_link(&dir)?;
        let receipt = dir.join("installed.json");
        reject_link(&receipt)?;
        let expected: Vec<_> = m
            .files
            .iter()
            .map(|f| (f.name, f.sha256, f.bytes))
            .collect();
        let Ok(bytes) = fs::read(&receipt) else {
            return Ok(None);
        };
        if bytes != serde_json::to_vec(&expected)? {
            return Ok(None);
        }
        // The receipt is written after every file is verified and renamed, so a file changed
        // later (same size or not) is newer than the receipt and no longer counts. This does not
        // catch silent bit rot; a full re-hash is too slow for every lookup.
        let Ok(receipt_time) = fs::metadata(&receipt).and_then(|m| m.modified()) else {
            return Ok(None);
        };
        let paths: Vec<_> = m.files.iter().map(|f| dir.join(f.name)).collect();
        if paths.iter().zip(m.files).all(|(p, f)| {
            fs::symlink_metadata(p).is_ok_and(|meta| {
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.len() == f.bytes
                    && meta.modified().is_ok_and(|t| t <= receipt_time)
            })
        }) {
            Ok(Some(paths))
        } else {
            Ok(None)
        }
    }
    /// Explicit administration re-hashes every file; readiness lookups remain inexpensive.
    pub fn verify_installed(&self, id: &str) -> io::Result<bool> {
        let m = model(id)?;
        if m.runtime == Runtime::Hex {
            return Err(store_io::invalid("Hex manages this model"));
        }
        let _lock = self.lock(m)?;
        let Some(paths) = self.installed_paths(id)? else {
            return Ok(false);
        };
        let cancel = AtomicBool::new(false);
        for (path, file) in paths.iter().zip(m.files) {
            if !verify(path, file, &cancel)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Includes partial files so Settings can offer resume or cleanup after a restart.
    pub fn stored_bytes(&self, id: &str) -> io::Result<u64> {
        let m = model(id)?;
        if m.runtime == Runtime::Hex {
            return Ok(0);
        }
        reject_link(&self.root)?;
        let dir = self.dir(m);
        reject_link(&dir)?;
        let mut bytes = 0;
        for f in m.files {
            for name in [f.name.to_owned(), format!("{}.part", f.name)] {
                let path = dir.join(name);
                reject_link(&path)?;
                match fs::metadata(path) {
                    Ok(meta) if meta.is_file() => bytes += meta.len(),
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
        }
        Ok(bytes)
    }
    /// Removes only this catalogue id, serialized against installs in every app process.
    pub fn remove(&self, id: &str) -> io::Result<()> {
        let m = model(id)?;
        if m.runtime == Runtime::Hex {
            return Err(store_io::invalid("Hex manages this model"));
        }
        reject_link(&self.root)?;
        let _lock = self.lock(m)?;
        let dir = self.dir(m);
        reject_link(&dir)?;
        match fs::remove_dir_all(dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
    pub fn install(
        &self,
        id: &str,
        cancel: &AtomicBool,
        mut progress: impl FnMut(Progress),
    ) -> io::Result<()> {
        let m = model(id)?;
        if m.runtime == Runtime::Hex {
            return Err(store_io::invalid("Hex manages this model"));
        }
        reject_link(&self.root)?;
        let _lock = self.lock(m)?;
        let dir = self.dir(m);
        reject_link(&dir)?;
        fs::create_dir_all(&dir)?;
        let receipt = dir.join("installed.json");
        if receipt.exists() {
            fs::remove_file(&receipt)?;
        }
        // No proxy from the environment: an HTTPS_PROXY with a user and password would be sent
        // as Proxy-Authorization, possibly over plain HTTP. Nothing secret leaves this request.
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .https_only(true)
            .max_redirects(0)
            .timeout_resolve(Some(Duration::from_secs(3)))
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_per_read(Some(Duration::from_secs(3)))
            .timeout_recv_response(Some(Duration::from_secs(20)))
            .build()
            .new_agent();
        let mut completed = 0;
        for f in m.files {
            check_cancel(cancel)?;
            let dest = dir.join(f.name);
            reject_link(&dest)?;
            if dest.exists() {
                if verify(&dest, f, cancel)? {
                    completed += f.bytes;
                    continue;
                }
                fs::remove_file(&dest)?;
            }
            let part = dir.join(format!("{}.part", f.name));
            reject_link(&part)?;
            let mut offset = fs::metadata(&part).map(|v| v.len()).unwrap_or(0);
            if offset > f.bytes {
                fs::remove_file(&part)?;
                offset = 0;
            }
            if offset < f.bytes {
                let mut response = request(&agent, f.url, offset, cancel)?;
                if response.status() == 200 {
                    offset = 0;
                } else if response.status() == 206 {
                    let expected = format!("bytes {offset}-");
                    let range = response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if !range.starts_with(&expected) || !range.ends_with(&format!("/{}", f.bytes)) {
                        return Err(store_io::invalid("Server returned an invalid resume range"));
                    }
                } else {
                    return Err(io::Error::other(format!(
                        "Download failed (HTTP {})",
                        response.status().as_u16()
                    )));
                }
                if response
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_some_and(|len| len > f.bytes - offset)
                {
                    return Err(store_io::invalid("Download exceeds catalogue size"));
                }
                let mut output = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(offset == 0)
                    .append(offset > 0)
                    .open(&part)?;
                let mut reader = response.body_mut().as_reader();
                let mut buffer = [0; 64 * 1024];
                let mut last = Instant::now();
                // The per-read timeout restarts on every byte, so a server trickling one byte at a
                // time never trips it. After a minute, require a floor rate; the .part resumes.
                let started = Instant::now();
                let start_offset = offset;
                progress(Progress {
                    downloaded: completed + offset,
                    total: m.bytes(),
                    phase: Phase::Downloading,
                });
                loop {
                    check_cancel(cancel)?;
                    let n = reader.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    if n as u64 > f.bytes - offset {
                        drop(output);
                        fs::remove_file(&part)?;
                        return Err(store_io::invalid("Download exceeds catalogue size"));
                    }
                    output.write_all(&buffer[..n])?;
                    offset += n as u64;
                    let secs = started.elapsed().as_secs();
                    if secs >= 60 && offset - start_offset < secs * MIN_BYTES_PER_SEC {
                        output.sync_all()?;
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "The download is too slow. Try again later; it will resume.",
                        ));
                    }
                    if last.elapsed() >= Duration::from_millis(150) {
                        progress(Progress {
                            downloaded: completed + offset,
                            total: m.bytes(),
                            phase: Phase::Downloading,
                        });
                        last = Instant::now();
                    }
                }
                output.sync_all()?;
            }
            progress(Progress {
                downloaded: completed + offset,
                total: m.bytes(),
                phase: Phase::Verifying,
            });
            if !verify(&part, f, cancel)? {
                fs::remove_file(&part)?;
                return Err(store_io::invalid(
                    "Checksum verification failed. The downloaded file was deleted. Try again.",
                ));
            }
            check_cancel(cancel)?;
            fs::rename(&part, &dest)?;
            completed += f.bytes;
        }
        check_cancel(cancel)?;
        let expected: Vec<_> = m
            .files
            .iter()
            .map(|f| (f.name, f.sha256, f.bytes))
            .collect();
        store_io::atomic_write(&receipt, &serde_json::to_vec(&expected)?)?;
        progress(Progress {
            downloaded: m.bytes(),
            total: m.bytes(),
            phase: Phase::Verifying,
        });
        Ok(())
    }
}
/// Integration seam for the pipeline lane, using the existing legacy-aware state_dir().
pub fn installed_model_paths(id: &str) -> io::Result<Option<Vec<PathBuf>>> {
    ModelStore::default().installed_paths(id)
}
fn reject_link(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| crate::platform::linked(&m)) {
        return Err(store_io::invalid(
            "Model paths cannot be links or junctions",
        ));
    }
    Ok(())
}
fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Download cancelled. Choose Download to resume.",
        ))
    } else {
        Ok(())
    }
}
fn verify(path: &Path, f: &ModelFile, cancel: &AtomicBool) -> io::Result<bool> {
    let mut input = File::open(path)?;
    if input.metadata()?.len() != f.bytes {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let mut actual = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut actual, "{byte:02x}").map_err(io::Error::other)?;
    }
    Ok(actual == f.sha256)
}
fn allowed(url: &url::Url) -> bool {
    if model("silero-vad").is_ok_and(|m| m.files[0].url == url.as_str()) {
        return true;
    }
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "huggingface.co"
                    | "cdn-lfs.huggingface.co"
                    | "cdn-lfs-us-1.huggingface.co"
                    | "cdn-lfs-eu-1.huggingface.co"
                    | "cas-bridge.xethub.hf.co"
                    | "us.aws.cdn.hf.co"
                    | "us.gcp.cdn.hf.co"
            )
        )
}
fn request(
    agent: &ureq::Agent,
    source: &str,
    offset: u64,
    cancel: &AtomicBool,
) -> io::Result<ureq::http::Response<ureq::Body>> {
    let mut url = url::Url::parse(source).map_err(|_| store_io::invalid("Invalid model URL"))?;
    for _ in 0..8 {
        check_cancel(cancel)?;
        if !allowed(&url) {
            return Err(store_io::invalid("Model download host is not allowed"));
        }
        let mut req = agent
            .get(url.as_str())
            .header("Accept-Encoding", "identity");
        if offset > 0 {
            req = req.header("Range", &format!("bytes={offset}-"));
        }
        // Do not surface HTTP errors containing signed CDN URLs in the UI or logs.
        let response = req.call().map_err(|_| {
            io::Error::other("Could not download model. Check your connection and retry.")
        })?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| store_io::invalid("Missing download redirect"))?;
            url = url
                .join(location)
                .map_err(|_| store_io::invalid("Invalid download redirect"))?;
        } else {
            return Ok(response);
        }
    }
    Err(store_io::invalid("Too many download redirects"))
}

/// Device-local language choices, separate from the pipeline configuration lane.
pub fn load_language_routes(root: &Path) -> io::Result<super::meeting_router::LanguageRoutes> {
    super::meeting_router::LanguageRoutes::from_legacy(load_meeting_languages(root)?)
        .map_err(|error| store_io::invalid(&error.to_string()))
}

pub fn load_meeting_languages(root: &Path) -> io::Result<Vec<String>> {
    let _lock = store_io::lock(&root.join("local/meeting-languages.lock"))?;
    let path = root.join("meeting-languages.json");
    match fs::read(path) {
        Ok(bytes) => {
            let languages: Vec<String> = serde_json::from_slice(&bytes)?;
            validate_languages(&languages)?;
            Ok(languages)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec!["en".into()]),
        Err(e) => Err(e),
    }
}
pub fn save_meeting_languages(root: &Path, languages: &[String]) -> io::Result<()> {
    validate_languages(languages)?;
    let _lock = store_io::lock(&root.join("local/meeting-languages.lock"))?;
    store_io::write(&root.join("meeting-languages.json"), &languages)
}
fn validate_languages(languages: &[String]) -> io::Result<()> {
    if languages.is_empty()
        || languages.len() > 23
        || languages
            .iter()
            .any(|l| l != "en" && !catalog().iter().any(|m| m.languages.contains(&l.as_str())))
    {
        return Err(store_io::invalid("Choose known meeting languages"));
    }
    Ok(())
}

#[cfg(test)]
mod integrity_tests {
    use super::*;
    const FILE: ModelFile = ModelFile {
        name: "synthetic.bin",
        url: "https://huggingface.co/synthetic",
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };
    #[test]
    fn documented_gcp_edge_preserves_https_exact_host_and_no_credentials_policy() {
        // https://huggingface.co/docs/hub/models-downloading lists this CDN edge.
        assert!(allowed(
            &url::Url::parse("https://us.gcp.cdn.hf.co/model").unwrap()
        ));
        for value in [
            "http://us.gcp.cdn.hf.co/model",
            "https://us.gcp.cdn.hf.co:444/model",
            "https://untrusted@us.gcp.cdn.hf.co/model",
            "https://us.gcp.cdn.hf.co.example.invalid/model",
            "https://unlisted.hf.co/model",
            "https://127.0.0.1/model",
        ] {
            assert!(!allowed(&url::Url::parse(value).unwrap()));
        }
    }
    #[test]
    fn verification_rejects_same_size_corruption_truncation_and_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(FILE.name);
        let cancel = AtomicBool::new(false);
        for (bytes, accepted) in [
            (b"abc".as_slice(), true),
            (b"abd", false),
            (b"ab", false),
            (b"abcd", false),
        ] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(verify(&path, &FILE, &cancel).unwrap(), accepted);
        }
        fs::write(&path, b"abc").unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            verify(&path, &FILE, &cancel).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(
            !ModelStore::new(root.path().into())
                .verify_installed("indicconformer-hi-ctc")
                .unwrap()
        );
    }
}
