//! Operational provenance, separate from recognizer text and helper instructions.
use super::revision::Revision;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    GuardRejected,
    Uncalibrated,
    InvalidScores,
    PinnedRevisionUnavailable,
    SelectedRevisionUnavailable,
    PendingRecordInvalid,
    RoutingFailed,
    CpuLanguageAssetsUnavailable,
    CpuLanguageModelsDownloading { ready: u8 },
    DeadlineExpired,
}
impl Reason {
    pub fn message(self) -> &'static str {
        match self {
            Self::GuardRejected => {
                "Language confidence was too low. Notes use the captured Hex transcript for that speech."
            }
            Self::Uncalibrated => {
                "Language routing was not calibrated for this language set. Notes use captured Hex text."
            }
            Self::InvalidScores => {
                "Language detection returned invalid scores. Notes use captured Hex text."
            }
            Self::PinnedRevisionUnavailable | Self::SelectedRevisionUnavailable => {
                "The routed transcript was missing or damaged. Notes use the captured Hex transcript."
            }
            Self::PendingRecordInvalid => {
                "The language-routing request record was damaged. Notes use the captured Hex transcript."
            }
            Self::RoutingFailed => "A language pass failed. Notes use the captured Hex transcript.",
            Self::CpuLanguageAssetsUnavailable => {
                "CPU language passes are unavailable because the ONNX runtime or a verified model pack is missing. Notes use the captured Hex transcript."
            }
            Self::CpuLanguageModelsDownloading { ready } => [
                "Language models are still being set up (0 of 4 ready). Notes use the captured Hex transcript.",
                "Language models are still being set up (1 of 4 ready). Notes use the captured Hex transcript.",
                "Language models are still being set up (2 of 4 ready). Notes use the captured Hex transcript.",
                "Language models are still being set up (3 of 4 ready). Notes use the captured Hex transcript.",
            ].get(usize::from(ready)).copied().unwrap_or(Self::CpuLanguageAssetsUnavailable.message()),
            Self::DeadlineExpired => {
                "Language routing missed the Stop deadline. Notes use the captured Hex transcript."
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub track: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingFallback {
    pub reason: Reason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<Interval>,
}
impl RoutingFallback {
    pub fn new(reason: Reason) -> Self {
        Self {
            reason,
            interval: None,
        }
    }
}
pub fn read(data: &Revision) -> serde_json::Result<Vec<RoutingFallback>> {
    data.provenance
        .get("routing_fallbacks")
        .cloned()
        .map(serde_json::from_value)
        .unwrap_or_else(|| Ok(Vec::new()))
}
pub fn message(reasons: &[RoutingFallback]) -> String {
    let mut messages = Vec::new();
    for r in reasons {
        let text = r.reason.message();
        if !messages.contains(&text) {
            messages.push(text);
        }
    }
    messages.join(" ")
}
pub(crate) fn attach(data: &mut Revision, reasons: &[RoutingFallback]) -> serde_json::Result<()> {
    if reasons.is_empty() {
        return Ok(());
    }
    if data.provenance.is_null() {
        data.provenance = serde_json::json!({});
    }
    data.provenance["routing_fallbacks"] = serde_json::to_value(reasons)?;
    for r in reasons {
        let text = r.reason.message().to_owned();
        if !data.transcript.warnings.contains(&text) {
            data.transcript.warnings.push(text.clone());
        }
        for s in &mut data.transcript.segments {
            if s.sources.iter().any(|o| !o.chunk.english.is_empty())
                && r.interval
                    .as_ref()
                    .is_none_or(|i| i.track == s.track && i.start < s.end && s.start < i.end)
            {
                s.status = format!("captured Hex retained: {text}");
            }
        }
    }
    Ok(())
}
