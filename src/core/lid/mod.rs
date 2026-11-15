//! Experimental, advisory-only language stage for explicitly prepared PCM chunks.
pub(crate) mod anuvani;
mod calibration;
pub use calibration::{CALIBRATION_VERSION, CalibrationKey, Decision, Unsure, decide_logits};
mod contract;
#[cfg(test)]
mod tests;
mod worker;

pub use contract::{ChunkResult, Job, Request, Sidecar};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MODEL_SHA256: &str = "d787948f922bd699e1cbcbeeeb16bf830514ec138bbbb89b516dccbc7283bbe5";
pub const LANGUAGES: [&str; 23] = [
    "as", "bn", "brx", "doi", "en", "gu", "hi", "kn", "ks", "kok", "mai", "ml", "mni", "mr", "ne",
    "or", "pa", "sa", "sat", "sd", "ta", "te", "ur",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub model: PathBuf,
    pub runtime_dll: PathBuf,
    /// Uncalibrated; never use these guesses to route recognition in M5.
    pub confidence_threshold: f32,
    pub margin_threshold: f32,
    pub silence_mean_square: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            model: PathBuf::new(),
            runtime_dll: PathBuf::new(),
            confidence_threshold: 0.9,
            margin_threshold: 0.2,
            silence_mean_square: 0.00001,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> super::asr::Result<()> {
        for value in [
            self.confidence_threshold,
            self.margin_threshold,
            self.silence_mean_square,
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("Language thresholds must be finite values from 0 to 1".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Score {
    pub language: String,
    pub confidence: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Guess {
    pub chunk: String,
    pub language: String,
    pub confidence: Option<f32>,
    pub top_two: Vec<Score>,
    pub status: String,
    pub unrestricted_language: Option<String>,
    pub outside_meeting_languages: bool,
    pub forced_route: Option<String>,
    /// Observed sub-window winners; not a mixed-language classifier.
    pub observed_languages: Vec<String>,
}
impl Guess {
    pub fn fallback(chunk: &str, language: &str, status: &str) -> Self {
        Self {
            chunk: chunk.into(),
            language: language.into(),
            confidence: None,
            top_two: Vec::new(),
            status: status.into(),
            unrestricted_language: None,
            outside_meeting_languages: false,
            forced_route: None,
            observed_languages: Vec::new(),
        }
    }
}

pub fn decide(
    scores: &[f32],
    languages: &[String],
    settings: &Settings,
) -> super::asr::Result<Guess> {
    if scores.len() != LANGUAGES.len()
        || scores
            .iter()
            .any(|s| !s.is_finite() || *s < 0.0 || *s > 1.0)
        || (scores.iter().sum::<f32>() - 1.0).abs() > 0.001
    {
        return Err("Invalid language probabilities".into());
    }
    let mut top: Vec<_> = LANGUAGES
        .iter()
        .zip(scores)
        .filter(|(lang, _)| languages.iter().any(|s| s == **lang))
        .map(|(language, score)| Score {
            language: (*language).into(),
            confidence: *score,
        })
        .collect();
    top.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mass: f32 = top.iter().map(|s| s.confidence).sum();
    if top.is_empty() || mass <= f32::MIN_POSITIVE {
        return Err("No probability for declared meeting languages".into());
    }
    for score in &mut top {
        score.confidence /= mass;
    }
    top.truncate(2);
    let winner = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| LANGUAGES[i])
        .ok_or("Empty probabilities")?;
    let outside = !languages.iter().any(|s| s == winner);
    let margin = top[0].confidence - top.get(1).map_or(0.0, |s| s.confidence);
    let mut guess = Guess::fallback("", &top[0].language, "guess");
    guess.status = if outside {
        "outside_meeting_languages"
    } else if top[0].confidence < settings.confidence_threshold
        || margin < settings.margin_threshold
    {
        "uncertain"
    } else {
        "guess"
    }
    .into();
    guess.confidence = Some(top[0].confidence);
    guess.top_two = top;
    guess.unrestricted_language = Some(winner.into());
    guess.outside_meeting_languages = outside;
    Ok(guess)
}

pub use worker::{Worker, pcm_sha256, spawn as spawn_worker};

mod stage;
pub use stage::start;

mod cli;
pub use cli::run_cli;
