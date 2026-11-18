//! Qualification of the pinned Linux ONNX Runtime through `initialize`, the production entry point.
//! Public release assets are fetched and hash-checked; a mismatch fails the test, it never skips.
use super::*;
use std::{io::Write, os::fd::AsRawFd, path::PathBuf, process::Command};

const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";
const TARBALL: (&str, &str) = (
    "v2.0.1/sherpa-onnx-v2.0.1-linux-x64-shared-no-tts.tar.bz2",
    "b3b095f619059d54914027ff38f8a0a132ea3bbc5df32a212a9743a8bd1467b2",
);
const LIBRARY_IN_TARBALL: &str = "sherpa-onnx-v2.0.1-linux-x64-shared-no-tts/lib/libonnxruntime.so";
const SILERO: (&str, &str, usize) = (
    "asr-models/silero_vad.onnx",
    "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
    643_854,
);

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/l1-fixtures");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fetch(name: &str, url: &str, sha: &str) -> Vec<u8> {
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
        .limit(64 << 20)
        .read_to_vec()
        .unwrap();
    assert_eq!(
        sha256(&bytes),
        sha,
        "{name} does not match its pinned sha256"
    );
    fs::write(&cached, &bytes).unwrap();
    bytes
}

/// The pinned library, extracted from the release tarball whose own hash is also pinned.
fn library() -> PathBuf {
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
    fs::write(&out, status.stdout).unwrap();
    out
}

#[test]
fn qualified_runtime_loads_and_runs_one_silero_frame() {
    let library = library();
    initialize(&library).expect("pinned Linux runtime must load");
    let version = runtime_version().expect("version comes from the loaded library");
    eprintln!("loaded ONNX Runtime {version} from sha256 {RUNTIME_SHA256}");
    assert!(version.starts_with("1."));

    let model = fetch(
        "silero_vad.onnx",
        &format!("{RELEASE}/{}", SILERO.0),
        SILERO.1,
    );
    assert_eq!(model.len(), SILERO.2);
    let path = fixtures().join("silero_vad.onnx");
    let mut session = session(&path).expect("session through the production builder");
    let names: Vec<_> = session.inputs.iter().map(|i| i.name.clone()).collect();
    eprintln!("silero inputs {names:?}");
    // The sherpa asr-models file is the three-input graph (x, h, c): one 512 sample frame, zero state.
    assert_eq!(names, ["x", "h", "c"]);
    let out = session
        .run(ort::inputs![
            "x" => Tensor::from_array(([1, 512], vec![0.0f32; 512])).unwrap(),
            "h" => Tensor::from_array(([2, 1, 64], vec![0.0f32; 128])).unwrap(),
            "c" => Tensor::from_array(([2, 1, 64], vec![0.0f32; 128])).unwrap()
        ])
        .unwrap();
    let (_, probability) = out[0].try_extract_tensor::<f32>().unwrap();
    assert_eq!(probability.len(), 1);
    assert!(
        (0.0..=1.0).contains(&probability[0]),
        "probability {probability:?}"
    );
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
