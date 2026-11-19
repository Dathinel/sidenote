//! Frozen S45 Silero boundaries. Padding/joining belong to the next pipeline stage.
use super::super::model_catalog;
#[cfg(feature = "asr-runtime")]
use super::super::{asr, store_io};
use serde::{Deserialize, Serialize};
use std::path::Path;
pub const MODEL_ID: &str = "silero-vad";
#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable;
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSampleRate(u32),
    Unavailable,
}
pub type Span = (u64, u64);
/// Persist with the piece receipt; create a fresh checkpoint for independent runs.
#[derive(Default, Serialize, Deserialize)]
pub struct Checkpoint {
    identity: (String, String),
    consumed: u64,
    frame: Vec<f32>,
    memory: Vec<f32>,
    context: Vec<f32>,
    speech: Option<u64>,
    silence: Option<u64>,
    finished: bool,
}
impl Checkpoint {
    fn boundary(&mut self, probability: f32, at: u64, spans: &mut Vec<Span>) {
        if probability >= 0.50 {
            self.silence = None;
            self.speech.get_or_insert(at);
        } else if probability < 0.35
            && let Some(start) = self.speech
        {
            let end = *self.silence.get_or_insert(at);
            if at - end >= 1600 {
                if end - start > 4000 {
                    spans.push((start, end));
                }
                self.speech = None;
                self.silence = None;
            }
        }
    }
    fn push(
        &mut self,
        identity: (&str, &str),
        start: u64,
        sample_rate: u32,
        mut audio: &[f32],
        last: bool,
        mut infer: impl FnMut(&mut Self, &[f32]) -> Result<f32, Unavailable>,
    ) -> Result<Vec<Span>, Error> {
        if sample_rate != 16000 {
            return Err(Error::InvalidSampleRate(sample_rate));
        }
        if self.identity != (identity.0.into(), identity.1.into()) {
            if start != 0 {
                return Err(Error::Unavailable);
            }
            *self = Self {
                identity: (identity.0.into(), identity.1.into()),
                ..Self::default()
            };
        }
        if start != self.consumed
            || self.finished
            || self.frame.len() >= 512
            || self.frame.len() as u64 > start
            || self
                .speech
                .is_some_and(|s| s > start - self.frame.len() as u64)
            || self.silence.is_some_and(|s| {
                s > start - self.frame.len() as u64 || self.speech.is_none_or(|b| s < b)
            })
            || audio.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Unavailable);
        }
        let end = start
            .checked_add(audio.len() as u64)
            .ok_or(Error::Unavailable)?;
        let mut spans = Vec::new();
        while !audio.is_empty() || (last && !self.frame.is_empty()) {
            let n = (512 - self.frame.len()).min(audio.len());
            self.frame.extend_from_slice(&audio[..n]);
            self.consumed += n as u64;
            audio = &audio[n..];
            if self.frame.len() < 512 && !last {
                break;
            }
            let at = self.consumed - self.frame.len() as u64;
            let mut frame = std::mem::take(&mut self.frame);
            frame.resize(512, 0.0);
            let probability = infer(self, &frame).map_err(|_| Error::Unavailable)?;
            if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                return Err(Error::Unavailable);
            }
            self.boundary(probability, at, &mut spans);
        }
        if last {
            if let Some(start) = self.speech.take()
                && end - start > 4000
            {
                spans.push((start, end));
            }
            self.finished = true;
        }
        Ok(spans)
    }
}
pub struct Silero {
    #[cfg(feature = "asr-runtime")]
    session: Option<ort::session::Session>,
}
impl Silero {
    /// Only reads an explicitly installed pack; failures leave baseline coverage to the caller.
    pub fn load(store: &model_catalog::ModelStore, dll: &Path) -> Result<Self, Unavailable> {
        #[cfg(feature = "asr-runtime")]
        {
            use sha2::{Digest, Sha256};
            #[cfg(all(test, windows))]
            if super::production::proof::active() {
                return Ok(Self { session: None });
            }
            let load = || -> asr::Result<Self> {
                let paths = store.installed_paths(MODEL_ID)?.ok_or("VAD unavailable")?;
                let expected = &model_catalog::model(MODEL_ID)?.files[0];
                let bytes = store_io::read_bounded(
                    paths.first().ok_or("VAD unavailable")?,
                    expected.bytes,
                )?;
                if bytes.len() as u64 != expected.bytes
                    || hex::encode(Sha256::digest(&bytes)) != expected.sha256
                {
                    return Err("VAD integrity check failed".into());
                }
                asr::initialize(dll)?;
                Ok(Self {
                    session: Some(
                        ort::session::Session::builder()?
                            .with_intra_threads(1)?
                            .with_inter_threads(1)?
                            .with_parallel_execution(false)?
                            .commit_from_memory(&bytes)?,
                    ),
                })
            };
            load().map_err(|_| Unavailable)
        }
        #[cfg(not(feature = "asr-runtime"))]
        {
            let _ = (store, dll);
            Err(Unavailable)
        }
    }
    /// On failure discard this checkpoint. Never commit a partial piece receipt.
    pub fn push(
        &mut self,
        state: &mut Checkpoint,
        identity: (&str, &str),
        start: u64,
        sample_rate: u32,
        audio: &[f32],
        last: bool,
    ) -> Result<Vec<Span>, Error> {
        #[cfg(all(test, windows, feature = "asr-runtime"))]
        if self.session.is_none() {
            return state.push(identity, start, sample_rate, audio, last, |_, _| Ok(1.0));
        }
        state.push(identity, start, sample_rate, audio, last, |state, frame| {
            #[cfg(target_os = "linux")]
            if crate::core::speech_jobs::STOP.load(std::sync::atomic::Ordering::Acquire) {
                return Err(Unavailable);
            }
            #[cfg(feature = "asr-runtime")]
            {
                use ort::value::Tensor;
                let mut run =
                    || -> asr::Result<f32> {
                        if state.memory.is_empty() {
                            state.memory = vec![0.0; 256];
                            state.context = vec![0.0; 64];
                        }
                        if state.memory.len() != 256 || state.context.len() != 64 {
                            return Err("Invalid VAD state".into());
                        }
                        let input: Vec<_> = state.context.iter().chain(frame).copied().collect();
                        let out = self.session.as_mut().ok_or("Missing VAD runtime")?.run(
                            ort::inputs![
                                "input" => Tensor::from_array(([1, 576], input))?,
                                "state" => Tensor::from_array(([2, 1, 128], state.memory.clone()))?,
                                "sr" => Tensor::from_array((Vec::<usize>::new(), vec![16000i64]))?
                            ],
                        )?;
                        let (_, probability) = out[0].try_extract_tensor::<f32>()?;
                        let (_, memory) = out[1].try_extract_tensor::<f32>()?;
                        if probability.len() != 1 || memory.len() != 256 {
                            return Err("Invalid VAD output".into());
                        }
                        state.memory = memory.to_vec();
                        state.context = frame[448..].to_vec();
                        Ok(probability[0])
                    };
                run().map_err(|_| Unavailable)
            }
            #[cfg(not(feature = "asr-runtime"))]
            {
                let _ = (state, frame);
                Err(Unavailable)
            }
        })
    }
}
#[cfg(all(test, target_os = "linux", feature = "asr-runtime"))]
mod linux_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_boundaries_and_resume() {
        let mut audio = vec![0.0; 512 * 30 + 71];
        audio[1024..6144].fill(0.8);
        let run = |state: &mut Checkpoint, start, audio: &[f32], last| {
            state.push(("job", "mic"), start, 16000, audio, last, |_, frame| {
                Ok(frame[0])
            })
        };
        let whole = run(&mut Checkpoint::default(), 0, &audio, true).unwrap();
        assert_eq!(whole, vec![(1024, 6144)]);
        for seam in [1000, 4096, 6201, audio.len() - 1] {
            let mut state = Checkpoint::default();
            let mut spans = run(&mut state, 0, &audio[..seam], false).unwrap();
            let mut restored =
                serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            spans.extend(run(&mut restored, seam as u64, &audio[seam..], true).unwrap());
            assert_eq!(spans, whole);
        }
    }
    #[test]
    fn sample_rate_rejected_before_inference() {
        let mut state = Checkpoint::default();
        let mut audio = vec![0.0; 512 * 30 + 71];
        audio[1024..6144].fill(0.8);
        for rate in [8000, 48000] {
            let before = serde_json::to_vec(&state).unwrap();
            let result = state.push(("job", "mic"), 0, rate, &audio, true, |_, _| {
                panic!("invalid sample rate reached inference")
            });
            assert_eq!(result, Err(Error::InvalidSampleRate(rate)));
            assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        }
        let spans = state.push(("job", "mic"), 0, 16000, &audio, true, |_, f| Ok(f[0]));
        assert_eq!(spans.unwrap(), vec![(1024, 6144)]);
    }
    #[test]
    fn absent_model_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let store = model_catalog::ModelStore::new(temp.path().into());
        assert!(Silero::load(&store, Path::new("absent.dll")).is_err());
    }
    #[test]
    #[cfg(all(windows, feature = "asr-runtime"))]
    #[ignore = "requires local S45 public audio/model and the existing pinned DLL"]
    fn s45_public_parity() {
        use std::fs;
        let root = std::path::PathBuf::from(std::env::var("VAD_S45_ROOT").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("models").join(MODEL_ID);
        fs::create_dir_all(&dir).unwrap();
        let model = dir.join("silero_vad.onnx");
        fs::copy(root.join("s45c/silero_vad.onnx"), &model).unwrap();
        let files = model_catalog::model(MODEL_ID).unwrap().files;
        let receipt: Vec<_> = files.iter().map(|f| (f.name, f.sha256, f.bytes)).collect();
        let receipt = serde_json::to_vec(&receipt).unwrap();
        fs::write(dir.join("installed.json"), &receipt).unwrap();
        let store = model_catalog::ModelStore::new(temp.path().into());
        let dll = std::path::PathBuf::from(std::env::var("VAD_RUNTIME_DLL").unwrap());
        // Raw sample boundaries undo S45c's 480-sample padding on these nonoverlapping spans.
        let expected = [
            [(15360, 107008), (146944, 187904), (216576, 257024)],
            [(15360, 107008), (136192, 214016), (220160, 270848)],
        ];
        for (id, expected) in ["en-hi-04", "en-te-04"].into_iter().zip(expected) {
            let audio = asr::read_wav(&root.join(format!("s45/audio/mixed/{id}.wav"))).unwrap();
            let mut vad = Silero::load(&store, &dll).unwrap();
            let whole = vad
                .push(
                    &mut Checkpoint::default(),
                    ("job", id),
                    0,
                    16000,
                    &audio,
                    true,
                )
                .unwrap();
            assert_eq!(whole, expected, "{id}");
            let seam = 80_123;
            let mut state = Checkpoint::default();
            let mut resumed = vad
                .push(&mut state, ("job", id), 0, 16000, &audio[..seam], false)
                .unwrap();
            let mut restored =
                serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            let mut cold = Silero::load(&store, &dll).unwrap();
            let tail = &audio[seam..];
            let next = cold.push(&mut restored, ("job", id), seam as u64, 16000, tail, true);
            resumed.extend(next.unwrap());
            assert_eq!(resumed, whole, "{id} cold seam");
            for key in [("new-job", id), ("new-job", "other-track")] {
                let reset = cold
                    .push(&mut restored, key, 0, 16000, &audio, true)
                    .unwrap();
                assert_eq!(reset, whole);
            }
        }
        fs::write(&model, vec![0; files[0].bytes as usize]).unwrap();
        fs::write(dir.join("installed.json"), &receipt).unwrap();
        assert!(matches!(Silero::load(&store, &dll), Err(Unavailable)));
    }
}
