use super::transcript::{Selection, Source};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Disposition {
    pub reason: String,
    pub source: Source,
    pub segment_keys: Vec<String>,
}
pub(super) fn prepare(selection: Option<&Selection>) -> Disposition {
    Disposition {
        reason: match selection {
            None => "language_passes_not_available",
            Some(s) if s.source() == Source::HexBaseline => "language_passes_not_available",
            Some(s) if !s.transcript.segments.iter().any(|s| s.translate) => "no_non_english",
            Some(_) => "translation_off",
        }
        .into(),
        source: selection.map_or(Source::Compatibility, Selection::source),
        segment_keys: selection
            .map(|s| {
                (0..s.transcript.segments.len())
                    .map(|i| s.segment_key(i))
                    .collect()
            })
            .unwrap_or_default(),
    }
}
