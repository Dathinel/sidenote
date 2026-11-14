//! Optional CPU runtime. Construct once per meeting; model loading never happens per chunk.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(feature = "asr-runtime")]
mod onnx;
#[cfg(feature = "asr-runtime")]
pub use onnx::{Anuvani, IndicConformer, IndicError, initialize};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const SAMPLE_RATE: u32 = 16000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguageScores {
    pub english: f32,
    pub hindi: f32,
    pub unrestricted: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Token {
    pub text: String,
    pub seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reading {
    pub text: String,
    pub tokens: Vec<Token>,
    pub confidence: Option<f32>,
}

pub trait LanguageId {
    fn identify(&mut self, samples: &[f32], declared: &[String]) -> Result<LanguageScores>;
}

pub trait SpeechEngine {
    fn transcribe(&mut self, samples: &[f32], sample_rate: u32, language: &str) -> Result<Reading>;
}

/// Catalogue paths are snapshots. Constructors must still handle every file-open failure.
/// `root` is an isolated Sidenote state directory, or empty for this device's default store.
pub fn model_path(root: &Path, id: &str) -> Result<PathBuf> {
    let paths = if root.as_os_str().is_empty() {
        super::model_catalog::installed_model_paths(id)?
    } else {
        super::model_catalog::ModelStore::new(root.to_owned()).installed_paths(id)?
    }
    .ok_or("Meeting model is unavailable")?;
    paths
        .into_iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "onnx"))
        .ok_or_else(|| "Installed pack has no ONNX graph".into())
}

pub fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err("Audio must be mono 16 kHz PCM16 WAV; normalize before routing".into());
    }
    Ok(reader
        .samples::<i16>()
        .map(|v| v.map(|s| s as f32 / 32768.0))
        .collect::<std::result::Result<_, _>>()?)
}

pub fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 0.9999695) * 32768.0) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
