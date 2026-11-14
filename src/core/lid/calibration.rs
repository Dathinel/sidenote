use super::{LANGUAGES, MODEL_SHA256, Settings};

pub const CALIBRATION_VERSION: &str = "s45b-20261113";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalibrationKey {
    pub model_sha256: String,
    pub declared_set: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Accepted {
        calibration: CalibrationKey,
        language: String,
        confidence: f64,
        margin: f64,
    },
    /// No replacement language: the caller must retain baseline coverage.
    Unsure(Unsure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsure {
    Uncalibrated,
    InvalidScores,
    GuardRejected,
}

/// Raw 23-label logits only. S45b calibrates biases, not the separate Labs guards.
pub fn decide_logits(
    logits: &[f32],
    languages: &[String],
    model_sha256: &str,
    version: &str,
    guards: &Settings,
) -> Decision {
    let mut declared = languages.to_vec();
    declared.sort();
    let declared_set = declared.join("+");
    if model_sha256 != MODEL_SHA256
        || languages.iter().any(|l| !LANGUAGES.contains(&l.as_str()))
        || version != CALIBRATION_VERSION
        || !matches!(declared_set.as_str(), "en+hi" | "en+te" | "en+hi+te")
    {
        return Decision::Unsure(Unsure::Uncalibrated);
    }
    if logits.len() != LANGUAGES.len() || logits.iter().any(|v| !v.is_finite()) {
        return Decision::Unsure(Unsure::InvalidScores);
    }
    let mut top: Vec<_> = LANGUAGES
        .iter()
        .zip(logits)
        .filter(|(language, _)| declared.iter().any(|s| s == **language))
        .map(|(language, logit)| {
            (
                *language,
                f64::from(*logit) + if *language == "te" { -4.0 } else { 0.0 },
            )
        })
        .collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    let peak = top[0].1;
    let mass: f64 = top.iter().map(|(_, v)| (v - peak).exp()).sum();
    let confidence = 1.0 / mass;
    let margin = confidence - (top[1].1 - peak).exp() / mass;
    if guards.validate().is_err()
        || confidence < f64::from(guards.confidence_threshold)
        || margin < f64::from(guards.margin_threshold)
        || margin == 0.0
    {
        return Decision::Unsure(Unsure::GuardRejected);
    }
    Decision::Accepted {
        calibration: CalibrationKey {
            model_sha256: model_sha256.into(),
            declared_set,
            version: version.into(),
        },
        language: top[0].0.into(),
        confidence,
        margin,
    }
}

#[cfg(test)]
mod tests;
