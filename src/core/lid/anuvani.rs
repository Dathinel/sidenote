use super::*;
use crate::core::asr::Result;

pub(crate) trait Detector {
    fn scores(&mut self, samples: &[f32]) -> Result<Vec<f32>>;
    #[cfg(all(windows, feature = "asr-runtime"))]
    fn logits(&mut self, _samples: &[f32]) -> Result<Vec<f32>> {
        Err("Raw language logits unavailable".into())
    }
}

#[cfg(all(windows, feature = "asr-runtime"))]
struct Runtime {
    model: crate::core::asr::Anuvani,
    // Deny replacement from before hashing until the ONNX session is dropped.
    _verified: std::fs::File,
    _parents: Vec<std::fs::File>,
}
#[cfg(all(windows, feature = "asr-runtime"))]
impl Detector for Runtime {
    fn logits(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        self.model.logits(samples)
    }
    fn scores(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let logits = self.model.logits(samples)?;
        let peak = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut scores: Vec<_> = logits.iter().map(|v| (v - peak).exp()).collect();
        let mass: f32 = scores.iter().sum();
        for score in &mut scores {
            *score /= mass;
        }
        Ok(scores)
    }
}

#[cfg(all(windows, feature = "asr-runtime"))]
pub(crate) fn load(settings: &Settings) -> Result<Box<dyn Detector>> {
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        os::windows::fs::{MetadataExt, OpenOptionsExt},
    };
    if !settings.model.is_absolute() {
        return Err("Language model needs an absolute path".into());
    }
    let model = std::fs::canonicalize(&settings.model)?;
    let mut parents = Vec::new();
    // Pin resolved parents so a junction/symlink swap cannot change the verified graph.
    for parent in model
        .parent()
        .ok_or("Missing model parent")?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x02000000 | 0x00200000)
            .open(parent)?;
        let meta = file.metadata()?;
        if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
            return Err("Unsafe language model directory".into());
        }
        parents.push(file);
    }
    let mut verified = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(&model)?;
    let meta = verified.metadata()?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("Unsafe language model file".into());
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
    if actual != MODEL_SHA256 {
        return Err("Language model integrity check failed".into());
    }
    crate::core::asr::initialize(&settings.runtime_dll)?;
    Ok(Box::new(Runtime {
        model: crate::core::asr::Anuvani::load(&model)?,
        _verified: verified,
        _parents: parents,
    }))
}

#[cfg(not(all(windows, feature = "asr-runtime")))]
pub(crate) fn load(_settings: &Settings) -> Result<Box<dyn Detector>> {
    Err("Language model unavailable: requires Windows and the asr-runtime build".into())
}
