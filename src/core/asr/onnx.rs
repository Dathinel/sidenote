use super::*;
use kaldi_fbank::{FbankOptions, OnlineFbank, WindowType};
use ort::{session::Session, value::Tensor};
use sha2::{Digest, Sha256};
#[cfg(any(windows, target_os = "linux"))]
use std::io::Read;
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::{fs, sync::OnceLock, time::Instant};
#[cfg(windows)]
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LoadLibraryExW,
};

static RUNTIME: OnceLock<std::result::Result<(), String>> = OnceLock::new();
/// ONNX Runtime from sherpa-onnx v2.0.1 win-x64 shared MD no-TTS, API 22.
#[cfg(windows)]
pub const RUNTIME_SHA256: &str = "422d776ab0e3218260f7f628fcb84606aaa5c21116f720c8619a6da5e0b2e0f9";
/// ONNX Runtime from sherpa-onnx v2.0.1 linux-x64 shared no-TTS, `lib/libonnxruntime.so`, API 22.
#[cfg(target_os = "linux")]
pub const RUNTIME_SHA256: &str = "4b3607aebd1784b26b6f9b20e4bd974c7ab8287043e4d095cb7d2cb40b5e566e";
#[cfg(target_os = "linux")]
pub const RUNTIME_BYTES: u64 = 27_026_609;
#[cfg(target_os = "linux")]
static VERSION: OnceLock<String> = OnceLock::new();

/// Version string of the loaded runtime, read from the library itself; `None` until `initialize` succeeds.
#[cfg(target_os = "linux")]
pub fn runtime_version() -> Option<&'static str> {
    VERSION.get().map(String::as_str)
}

#[cfg(windows)]
pub(crate) fn verify_runtime(dll: &Path) -> Result<fs::File> {
    if !dll.is_absolute() || !dll.is_file() {
        return Err("Choose an absolute path to the ONNX Runtime DLL".into());
    }
    // Deny writes/deletion between hashing and LoadLibrary; never load arbitrary DLL code.
    let mut verified = fs::OpenOptions::new().read(true).share_mode(1).open(dll)?;
    if verified.metadata()?.len() != 17_136_128 {
        return Err("ONNX Runtime integrity check failed; model unavailable".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = verified.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let actual: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual != RUNTIME_SHA256 {
        return Err("ONNX Runtime integrity check failed; model unavailable".into());
    }
    Ok(verified)
}

#[cfg(windows)]
pub fn initialize(dll: &Path) -> Result<()> {
    let _verified = verify_runtime(dll)?;
    RUNTIME
        .get_or_init(|| {
            preflight(dll)
                .map_err(|_| "ONNX Runtime DLL or its dependencies are unavailable".to_owned())?;
            std::panic::catch_unwind(|| ort::init_from(dll.to_string_lossy()).commit())
                .map_err(|_| "ONNX Runtime DLL is incompatible".to_string())?
                .map(|_| ())
                .map_err(|_| "ONNX Runtime initialization failed".to_string())
        })
        .clone()
        .map_err(Into::into)
}

#[cfg(windows)]
fn preflight(dll: &Path) -> Result<()> {
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(Some(0)).collect();
    // Pin for process lifetime, as ort keeps a global API pointer. Probe before its panic-based loader.
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        )
    };
    if module.is_null() {
        return Err("ONNX Runtime could not load".into());
    }
    let get = unsafe { GetProcAddress(module, c"OrtGetApiBase".as_ptr().cast()) }
        .ok_or("ONNX Runtime API is missing")?;
    let get: unsafe extern "system" fn() -> *const ort::sys::OrtApiBase =
        unsafe { std::mem::transmute(get) };
    let base = unsafe { get().as_ref() }.ok_or("ONNX Runtime API is missing")?;
    let api = unsafe { (base.GetApi)(ort::sys::ORT_API_VERSION) };
    if api.is_null() {
        return Err("ONNX Runtime API version is unsupported".into());
    }
    Ok(())
}

fn builder() -> Result<ort::session::builder::SessionBuilder> {
    if RUNTIME.get().is_none_or(|r| r.is_err()) {
        return Err("Initialize the local ONNX runtime first".into());
    }
    Ok(Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .with_memory_pattern(false)?)
}

fn session(path: &Path) -> Result<Session> {
    Ok(builder()?.commit_from_file(path)?)
}

pub struct Anuvani {
    session: Session,
    pub load_seconds: f64,
}
impl Anuvani {
    pub fn load(model: &Path) -> Result<Self> {
        let start = Instant::now();
        Self::checked(session(model)?, start)
    }

    /// Loads bytes the caller already hashed, so no later file edit can change the graph.
    #[cfg(target_os = "linux")]
    pub fn load_verified(model: &[u8]) -> Result<Self> {
        let start = Instant::now();
        Self::checked(builder()?.commit_from_memory(model)?, start)
    }

    fn checked(session: Session, start: Instant) -> Result<Self> {
        let meta = session.metadata()?;
        if meta.custom("model_name")?.as_deref() != Some("CIIL Anuvani-LID")
            || meta.custom("labels")?.is_none_or(|s| {
                s.split(',').nth(4) != Some("english") || s.split(',').nth(6) != Some("hindi")
            })
        {
            return Err("Expected the measured Anuvani INT8 pack with English/Hindi labels".into());
        }
        drop(meta);
        Ok(Self {
            session,
            load_seconds: start.elapsed().as_secs_f64(),
        })
    }

    /// Pinned 23-label inference before any declared-set decision.
    pub fn logits(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        if samples.len() < 1600
            || samples.len() > SAMPLE_RATE as usize * 24
            || samples.iter().any(|s| !s.is_finite())
        {
            return Err("Language detection needs 0.1 to 24 seconds of finite audio".into());
        }
        let out = self.session.run(ort::inputs![
            "audio" => Tensor::from_array(([1, samples.len()], samples.to_vec()))?,
            "audio_length" => Tensor::from_array(([1], vec![samples.len() as i64]))?
        ])?;
        let (_, logits) = out[0].try_extract_tensor::<f32>()?;
        if logits.len() != 23 || logits.iter().any(|s| !s.is_finite()) {
            return Err("Invalid Anuvani scores".into());
        }
        Ok(logits.to_vec())
    }
}
impl LanguageId for Anuvani {
    fn identify(&mut self, samples: &[f32], declared: &[String]) -> Result<LanguageScores> {
        if declared.iter().any(|s| !matches!(s.as_str(), "en" | "hi"))
            || !declared.iter().any(|s| s == "en")
            || !declared.iter().any(|s| s == "hi")
        {
            return Err(
                "Anuvani routing currently supports the declared English/Hindi pair".into(),
            );
        }
        let logits = self.logits(samples)?;
        let peak = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut unrestricted: Vec<_> = logits.iter().map(|v| (v - peak).exp()).collect();
        let sum: f32 = unrestricted.iter().sum();
        for v in &mut unrestricted {
            *v /= sum;
        }
        let pair_peak = logits[4].max(logits[6]);
        let en = (logits[4] - pair_peak).exp();
        let hi = (logits[6] - pair_peak).exp();
        Ok(LanguageScores {
            english: en / (en + hi),
            hindi: hi / (en + hi),
            unrestricted,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum IndicError {
    ModelHash,
    Tokens,
    Metadata,
    Language,
    Audio,
    InvalidSampleRate(u32),
    OutputShape,
    RuntimeUnavailable,
}
impl std::fmt::Display for IndicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IndicConformer unavailable: {self:?}")
    }
}
impl std::error::Error for IndicError {}

fn indic_hash(bytes: &[u8], language: &str) -> Result<()> {
    let expected = match language {
        "hi" => "915c71e04dd7e5378a4057fdebb252b3a587188e4e99db6d7ce0909ad5ad05fa",
        "te" => "710f176e33080f1be84b4f38bb5f33be3f522dcf4f191bf4d5cc89f7a85924f3",
        _ => return Err(IndicError::Language.into()),
    };
    if hex::encode(Sha256::digest(bytes)) != expected {
        return Err(IndicError::ModelHash.into());
    }
    Ok(())
}
fn indic_tokens(bytes: &[u8]) -> Result<&str> {
    if hex::encode(Sha256::digest(bytes))
        != "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2"
    {
        return Err(IndicError::Tokens.into());
    }
    std::str::from_utf8(bytes).map_err(|_| IndicError::Tokens.into())
}
fn indic_metadata(values: &[Option<String>]) -> Result<()> {
    let expected = ["ai4bharat", "per_feature", "4", "5633", "EncDecCTCModelBPE"];
    if values.iter().map(|s| s.as_deref()).ne(expected.map(Some)) {
        return Err(IndicError::Metadata.into());
    }
    Ok(())
}
fn indic_audio(samples: &[f32], sample_rate: u32, requested: &str, loaded: &str) -> Result<()> {
    if sample_rate != SAMPLE_RATE {
        return Err(IndicError::InvalidSampleRate(sample_rate).into());
    }
    if requested != loaded {
        return Err(IndicError::Language.into());
    }
    if samples.len() < 1600
        || samples.len() > SAMPLE_RATE as usize * 30
        || samples.iter().any(|s| !s.is_finite())
    {
        return Err(IndicError::Audio.into());
    }
    Ok(())
}

const INDIC_SUBSAMPLING: usize = 4;
fn indic_output(shape: &[i64], input_frames: usize, vocabulary: usize) -> Result<()> {
    // Both pinned graphs use two padded stride-2 convolutions: ceil(feature frames / 4).
    let frames = input_frames.div_ceil(INDIC_SUBSAMPLING);
    if frames == 0 || vocabulary != 5633 || shape != [1, frames as i64, vocabulary as i64] {
        return Err(IndicError::OutputShape.into());
    }
    Ok(())
}

pub struct IndicConformer {
    session: Session,
    tokens: Vec<String>,
    blank: usize,
    language: String,
    pub load_seconds: f64,
}
impl IndicConformer {
    pub fn load(model: &Path, tokens: &Path) -> Result<Self> {
        Self::load_for_language(model, tokens, "hi")
    }

    /// Explicit pack identity; never load a different language as fallback.
    pub fn load_for_language(model: &Path, tokens: &Path, language: &str) -> Result<Self> {
        Self::load_with_threads(model, tokens, language, 1)
    }

    pub fn load_with_threads(
        model: &Path,
        tokens: &Path,
        language: &str,
        threads: usize,
    ) -> Result<Self> {
        if !(1..=256).contains(&threads) {
            return Err("Invalid ASR thread count".into());
        }
        let start = Instant::now();
        if RUNTIME.get().is_none_or(|r| r.is_err()) {
            return Err(IndicError::RuntimeUnavailable.into());
        }
        let graph = fs::read(model)?;
        indic_hash(&graph, language)?;
        let token_bytes = fs::read(tokens)?;
        let token_text = indic_tokens(&token_bytes)?;
        // Infer from the exact verified bytes, even if the source path changes.
        let session = Session::builder()?
            .with_intra_threads(threads)?
            .with_inter_threads(1)?
            .with_memory_pattern(false)?
            .commit_from_memory(&graph)?;
        let meta = session.metadata()?;
        let values = [
            "model_author",
            "normalize_type",
            "subsampling_factor",
            "vocab_size",
            "model_type",
        ]
        .map(|key| meta.custom(key))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
        indic_metadata(&values)?;
        let mut table = vec![String::new(); 5633];
        let mut seen = vec![false; table.len()];
        for line in token_text.lines() {
            let (word, id) = line.rsplit_once(' ').ok_or("Invalid token table")?;
            let id: usize = id.parse()?;
            if id >= table.len() || seen[id] {
                return Err("Invalid or repeated token id".into());
            }
            seen[id] = true;
            table[id] = word.replace('\u{2581}', " ");
        }
        if seen.iter().any(|v| !v) {
            return Err("Incomplete token table".into());
        }
        let blank = table
            .iter()
            .position(|s| s == "<blk>")
            .ok_or("Missing CTC blank")?;
        drop(meta);
        Ok(Self {
            session,
            tokens: table,
            blank,
            language: language.to_owned(),
            load_seconds: start.elapsed().as_secs_f64(),
        })
    }
}
impl SpeechEngine for IndicConformer {
    fn transcribe(&mut self, samples: &[f32], sample_rate: u32, language: &str) -> Result<Reading> {
        indic_audio(samples, sample_rate, language, &self.language)?;
        let mut opts = FbankOptions::default();
        opts.frame_opts.dither = 0.0;
        opts.frame_opts.snip_edges = false;
        opts.frame_opts.remove_dc_offset = false;
        opts.frame_opts.window_type = WindowType::Hann;
        opts.mel_opts.num_bins = 80;
        opts.mel_opts.low_freq = 0.0;
        opts.mel_opts.high_freq = 0.0;
        opts.mel_opts.is_librosa = true;
        opts.use_log_fbank = false;
        let mut fbank = OnlineFbank::new(opts);
        fbank.accept_waveform(SAMPLE_RATE as f32, samples);
        fbank.input_finished();
        let n = fbank.num_frames_ready() as usize;
        if n < 2 {
            return Err("Not enough feature frames".into());
        }
        let frames: Vec<Vec<f32>> = (0..n)
            .map(|t| {
                fbank
                    .get_frame(t as i32)
                    .iter()
                    .map(|v| (v + f32::EPSILON / 2.0).ln())
                    .collect()
            })
            .collect();
        let mut features = vec![0.0f32; n * 80];
        for c in 0..80 {
            let mean = frames.iter().map(|v| v[c]).sum::<f32>() / n as f32;
            let variance = frames.iter().map(|v| (v[c] - mean).powi(2)).sum::<f32>() / n as f32;
            let scale = variance.sqrt() + 1e-5;
            for t in 0..n {
                features[c * n + t] = (frames[t][c] - mean) / scale;
            }
        }
        let out = self.session.run(ort::inputs![
            "audio_signal" => Tensor::from_array(([1, 80, n], features))?,
            "length" => Tensor::from_array(([1], vec![n as i64]))?
        ])?;
        let (shape, probs) = out[0].try_extract_tensor::<f32>()?;
        indic_output(shape, n, self.tokens.len())?;
        let mut previous = self.blank;
        let mut tokens = Vec::new();
        for (frame, row) in probs
            .chunks_exact(self.tokens.len())
            .take(n / INDIC_SUBSAMPLING)
            .enumerate()
        {
            let id = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .ok_or("Empty CTC output")?
                .0;
            if id != self.blank && id != previous {
                tokens.push(Token {
                    text: self.tokens[id].clone(),
                    seconds: frame as f64 * 0.04,
                });
            }
            previous = id;
        }
        let text = tokens
            .iter()
            .map(|t| t.text.as_str())
            .collect::<String>()
            .trim()
            .to_owned();
        Ok(Reading {
            text,
            tokens,
            confidence: None,
        })
    }
}

#[cfg(target_os = "linux")]
pub fn initialize(library: &Path) -> Result<()> {
    if !library.is_absolute() || !library.is_file() {
        return Err("Choose an absolute path to libonnxruntime.so".into());
    }
    // One bounded read feeds both the hash and the sealed copy, so later edits of the file change nothing.
    let mut bytes = Vec::new();
    fs::File::open(library)?
        .take(RUNTIME_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if bytes.len() as u64 != RUNTIME_BYTES || actual != RUNTIME_SHA256 {
        return Err("ONNX Runtime integrity check failed; model unavailable".into());
    }
    RUNTIME
        .get_or_init(|| {
            let path = load_sealed(&bytes).map_err(|_| {
                "ONNX Runtime library or its dependencies are unavailable".to_owned()
            })?;
            std::panic::catch_unwind(|| ort::init_from(&path).commit())
                .map_err(|_| "ONNX Runtime library is incompatible".to_string())?
                .map(|_| ())
                .map_err(|_| "ONNX Runtime initialization failed".to_string())
        })
        .clone()
        .map_err(Into::into)
}

/// Sealed, anonymous copy of the verified bytes: nothing can rewrite it between the hash and `dlopen`.
#[cfg(target_os = "linux")]
fn sealed_memfd(bytes: &[u8]) -> Result<std::os::fd::OwnedFd> {
    use std::{io::Write, os::fd::FromRawFd};
    let fd = unsafe {
        libc::memfd_create(
            c"onnxruntime".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    fs::File::from(owned.try_clone()?).write_all(bytes)?;
    let seals = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, seals) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(owned)
}

/// Loads the sealed copy, probes API 22 before ort's panic-based loader, and returns the path ort must open.
#[cfg(target_os = "linux")]
fn load_sealed(bytes: &[u8]) -> Result<String> {
    use std::os::fd::IntoRawFd;
    // The descriptor stays open for the process lifetime: ort opens the same path again and glibc
    // matches it to this mapping by inode, so only the verified bytes can run.
    let fd = sealed_memfd(bytes)?.into_raw_fd();
    let path = format!("/proc/self/fd/{fd}");
    let c_path = std::ffi::CString::new(path.as_str())?;
    let module = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if module.is_null() {
        return Err("ONNX Runtime could not load".into());
    }
    let get = unsafe { libc::dlsym(module, c"OrtGetApiBase".as_ptr()) };
    if get.is_null() {
        return Err("ONNX Runtime API is missing".into());
    }
    let get: unsafe extern "C" fn() -> *const ort::sys::OrtApiBase =
        unsafe { std::mem::transmute(get) };
    let base = unsafe { get().as_ref() }.ok_or("ONNX Runtime API is missing")?;
    if unsafe { (base.GetApi)(ort::sys::ORT_API_VERSION) }.is_null() {
        return Err("ONNX Runtime API version is unsupported".into());
    }
    let version = unsafe { (base.GetVersionString)() };
    if !version.is_null() {
        let _ = VERSION.set(
            unsafe { std::ffi::CStr::from_ptr(version) }
                .to_string_lossy()
                .into_owned(),
        );
    }
    Ok(path)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn initialize(_library: &Path) -> Result<()> {
    Err("ONNX_RUNTIME_UNAVAILABLE: no qualified pinned runtime for this platform".into())
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) mod linux_tests;

#[cfg(test)]
mod indic_tests;
