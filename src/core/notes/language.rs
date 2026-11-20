//! One status word saying whether a summary came from a transcript that had its language passes.
//! The word is computed from status fields only. Nothing here can see meeting text.
use crate::core::meeting_router::fallback::RoutingFallback;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Routed,
    EnglishOnly,
}

/// Routed: Hindi or Telugu text was read, or language ID heard only English. English only: the
/// passes did not run, or Hindi or Telugu may be hiding in captured English. `langs` are segment
/// `lang` codes and `degraded` comes from segment `status`.
pub(super) fn of<'a>(
    compatibility: bool,
    degraded: bool,
    reasons: &[RoutingFallback],
    langs: impl IntoIterator<Item = &'a str>,
) -> Language {
    if compatibility {
        return Language::EnglishOnly;
    }
    if langs.into_iter().any(|lang| lang != "en") {
        return Language::Routed;
    }
    if degraded || !reasons.is_empty() {
        Language::EnglishOnly
    } else {
        Language::Routed
    }
}

/// Adds the word, and the transcript revision it came from, to the summary's `made_by` record.
pub(super) fn stamp(made_by: &mut serde_json::Value, language: Language, revision: Option<&str>) {
    let Some(record) = made_by.as_object_mut() else {
        return;
    };
    record.insert("language_passes".into(), serde_json::json!(language));
    if let Some(revision) = revision {
        record.insert("transcript_revision".into(), revision.into());
    }
}
