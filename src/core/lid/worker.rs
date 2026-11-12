use super::{
    anuvani::{self, Detector},
    *,
};
use crate::core::{asr::Result, store_io};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    thread::{self, JoinHandle},
};

pub struct Worker {
    settings: Settings,
    cache: PathBuf,
    detector: Option<std::result::Result<Box<dyn Detector>, String>>,
    benchmark: bool,
}

/// The callback owns the only access to this non-Send model session.
pub fn spawn<T: Send + 'static>(
    settings: Settings,
    cache: PathBuf,
    benchmark: bool,
    run: impl FnOnce(&mut Worker) -> Result<T> + Send + 'static,
) -> std::io::Result<JoinHandle<Result<T>>> {
    thread::Builder::new()
        .name("language-detect".into())
        .spawn(move || {
            below_normal()?;
            settings.validate()?;
            run(&mut Worker {
                settings,
                cache,
                detector: None,
                benchmark,
            })
        })
}
#[cfg(windows)]
fn below_normal() -> Result<()> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn below_normal() -> Result<()> {
    Ok(())
}

impl Worker {
    pub fn identify(&mut self, samples: &[f32], languages: &[String]) -> Result<Guess> {
        if !(2 * 16000..=24 * 16000).contains(&samples.len())
            || samples.iter().any(|s| !s.is_finite())
            || languages.len() < 2
            || languages.iter().any(|s| !LANGUAGES.contains(&s.as_str()))
        {
            return Err(
                "Detection requires 2 to 24 seconds of finite PCM and supported declared languages"
                    .into(),
            );
        }
        if self.detector.is_none() {
            self.detector = Some(
                anuvani::load(&self.settings)
                    .map_err(|e| format!("Language model unavailable: {e}")),
            );
            if self.benchmark && self.detector.as_ref().is_some_and(|m| m.is_ok()) {
                use std::io::{self, Write};
                println!("{{\"event\":\"model_loaded\"}}");
                io::stdout().flush()?;
                let mut line = String::new();
                io::stdin().read_line(&mut line)?;
                if line.trim() != "continue" {
                    let error = "Benchmark needs post-load sampler acknowledgement";
                    self.detector = Some(Err(error.into()));
                    return Err(error.into());
                }
            }
        }
        let detector = self
            .detector
            .as_mut()
            .ok_or("Missing detector")?
            .as_mut()
            .map_err(|e| e.clone())?;
        let key = cache_key(samples, languages, &self.settings)?;
        let probabilities = match read_cache(&self.cache, &key)
            .filter(|s| aggregate(s, languages, &self.settings).is_ok())
        {
            Some(scores) => scores,
            None => {
                let scores = detect(detector.as_mut(), samples)?;
                aggregate(&scores, languages, &self.settings)?;
                write_cache(&self.cache, &key, &scores)?;
                scores
            }
        };
        aggregate(&probabilities, languages, &self.settings)
    }
}

pub fn pcm_sha256(samples: &[f32]) -> String {
    let mut hash = Sha256::new();
    for sample in samples {
        hash.update(sample.to_le_bytes());
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn detect(detector: &mut dyn Detector, samples: &[f32]) -> Result<Vec<Vec<f32>>> {
    // Five-second windows reuse the public probe; merge a <2s tail into its predecessor.
    let mut scores = Vec::new();
    let mut offset = 0;
    while offset < samples.len() {
        let mut end = (offset + 5 * 16000).min(samples.len());
        if samples.len() - end < 2 * 16000 {
            end = samples.len();
        }
        scores.push(detector.scores(&samples[offset..end])?);
        offset = end;
    }
    Ok(scores)
}
pub(super) fn aggregate(
    scores: &[Vec<f32>],
    languages: &[String],
    settings: &Settings,
) -> Result<Guess> {
    if scores.is_empty() || scores.len() > 5 {
        return Err("Invalid cached window count".into());
    }
    let mut mean = vec![0.0; 23];
    let mut observed = BTreeSet::new();
    for score in scores {
        let guess = decide(score, languages, settings)?;
        if guess.status == "guess" {
            observed.insert(guess.language);
        }
        for (m, s) in mean.iter_mut().zip(score) {
            *m += s / scores.len() as f32;
        }
    }
    let mut guess = decide(&mean, languages, settings)?;
    guess.observed_languages = observed.into_iter().collect();
    Ok(guess)
}
fn cache_key(samples: &[f32], languages: &[String], settings: &Settings) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&(
        MODEL_SHA256,
        "lid-stage-v1",
        languages,
        settings,
    ))?);
    for s in samples {
        hash.update(s.to_le_bytes());
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn read_cache(root: &Path, key: &str) -> Option<Vec<Vec<f32>>> {
    let path = root.join(format!("{key}.json"));
    let bytes = store_io::read_bounded(&path, 65536).ok()?;
    let scores = serde_json::from_slice(&bytes).ok()?;
    // Advance mtime for existing device-local LRU eviction.
    let _ = store_io::write(&path, &scores);
    Some(scores)
}
fn write_cache(root: &Path, key: &str, scores: &[Vec<f32>]) -> Result<()> {
    store_io::write(&root.join(format!("{key}.json")), &scores)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() && entry.path().extension().is_some_and(|e| e == "json") {
            entries.push((meta.modified()?, meta.len(), entry.path()));
        }
    }
    entries.sort_by_key(|e| e.0);
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    for (_, size, path) in entries {
        if total <= 500 * 1024 * 1024 {
            break;
        }
        fs::remove_file(path)?;
        total -= size;
    }
    Ok(())
}
