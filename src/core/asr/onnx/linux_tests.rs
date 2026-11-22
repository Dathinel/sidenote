//! Qualification of the pinned Linux ONNX Runtime through `initialize`, the production entry point.
//! Public release assets are fetched and hash-checked; a mismatch fails the test, it never skips.
use super::*;
use crate::core::{
    meeting_router::vad::{Checkpoint, Silero},
    model_catalog::{self, ModelStore},
};
use std::{io::Write, os::fd::AsRawFd, path::PathBuf, process::Command};

const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";
const TARBALL: (&str, &str) = (
    "v2.0.1/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts.tar.bz2",
    "b3b095f619059d54914027ff38f8a0a132ea3bbc5df32a212a9743a8bd1467b2",
);
const LIBRARY_IN_TARBALL: &str = "sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib/libonnxruntime.so";
/// Public 60 s mono PCM16 test clip; sha256 is the digest GitHub lists for the release asset.
const WAV: (&str, &str) = (
    "asr-models/test_silero_vad.wav",
    "89f17d9c94c4b31eb320f424628bcbc920abaddbee6e2760fd868bfb1d9a2e47",
);

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/l1-fixtures");
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

pub(crate) fn fetch(name: &str, url: &str, sha: &str) -> Vec<u8> {
    let cached = fixtures().join(name);
    if let Ok(bytes) = fs::read(&cached)
        && sha256(&bytes) == sha
    {
        return bytes;
    }
    let bytes = ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("download of {name} failed: {e}"))
        .into_body()
        .with_config()
        .limit(256 << 20)
        .read_to_vec()
        .unwrap();
    assert_eq!(
        sha256(&bytes),
        sha,
        "{name} does not match its pinned sha256"
    );
    crate::core::store_io::atomic_write(&cached, &bytes).unwrap();
    bytes
}

/// The pinned library, extracted from the release tarball whose own hash is also pinned.
pub(crate) fn library() -> PathBuf {
    fetch(
        "runtime.tar.bz2",
        &format!("{RELEASE}/{}", TARBALL.0),
        TARBALL.1,
    );
    let out = fixtures().join("libonnxruntime.so");
    let status = Command::new("tar")
        .args(["-xjOf"])
        .arg(fixtures().join("runtime.tar.bz2"))
        .arg(LIBRARY_IN_TARBALL)
        .output()
        .unwrap();
    assert!(status.status.success(), "tar could not read the library");
    crate::core::store_io::atomic_write(&out, &status.stdout).unwrap();
    out
}

/// The production Silero pack (catalogue url and hash), installed with a receipt in a temporary store.
pub(crate) fn silero_store() -> (tempfile::TempDir, ModelStore) {
    let file = &model_catalog::model("silero-vad").unwrap().files[0];
    let bytes = fetch("silero_vad_catalogue.onnx", file.url, file.sha256);
    assert_eq!(bytes.len() as u64, file.bytes);
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("models/silero-vad");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(file.name), &bytes).unwrap();
    let receipt = serde_json::to_vec(&[(file.name, file.sha256, file.bytes)]).unwrap();
    fs::write(dir.join("installed.json"), receipt).unwrap();
    let store = ModelStore::new(temp.path().into());
    (temp, store)
}

pub(crate) fn test_wav() -> Vec<f32> {
    fetch(
        "test_silero_vad.wav",
        &format!("{RELEASE}/{}", WAV.0),
        WAV.1,
    );
    read_wav(&fixtures().join("test_silero_vad.wav")).unwrap()
}

#[test]
fn qualified_runtime_loads_and_runs_one_silero_frame() {
    let library = library();
    initialize(&library).expect("pinned Linux runtime must load");
    let version = runtime_version().expect("version comes from the loaded library");
    eprintln!("loaded ONNX Runtime {version} from sha256 {RUNTIME_SHA256}");
    assert!(version.starts_with("1."));

    // The sherpa asr-models silero_vad.onnx is an older (x, h, c) graph; the app pins the v5 file.
    let (_temp, store) = silero_store();
    let mut vad = Silero::load(&store, &library).expect("Silero::load on Linux");
    // One 512 sample frame runs through the real session; any inference failure is an error here.
    let spans = vad.push(
        &mut Checkpoint::default(),
        ("l1", "probe"),
        0,
        16000,
        &[0.0; 512],
        false,
    );
    assert_eq!(spans, Ok(vec![]));
}

#[test]
fn a_flipped_byte_in_the_library_is_refused() {
    let mut bytes = fs::read(library()).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 1;
    let copy = fixtures().join("flipped-libonnxruntime.so");
    fs::write(&copy, &bytes).unwrap();
    let error = initialize(&copy).unwrap_err().to_string();
    assert!(error.contains("integrity check failed"), "{error}");
    // Same length is not enough; a truncated copy is refused too.
    fs::write(&copy, &bytes[..bytes.len() - 1]).unwrap();
    assert!(initialize(&copy).is_err());
}

#[test]
fn relative_and_missing_paths_are_refused() {
    assert!(initialize(Path::new("libonnxruntime.so")).is_err());
    assert!(initialize(Path::new("/missing/libonnxruntime.so")).is_err());
}

#[test]
fn memfd_copy_is_sealed_against_rewrites() {
    let fd = sealed_memfd(b"verified bytes").unwrap();
    let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
    let all = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    assert_eq!(seals & all, all);
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .open(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .unwrap();
    assert!(writer.write_all(b"x").is_err());
    assert!(fs::File::from(fd).set_len(1).is_err());
}
