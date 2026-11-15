use super::{
    Helper,
    transcript::{Selection, Source},
};
use crate::core::{Config, engine_settings::Prompt};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub revision: String,
    pub helper: Helper,
    pub model: String,
    pub deadline_seconds: u32,
    pub batch_chars: u32,
    pub include_context: bool,
    pub prompt: String,
    pub prompt_version: u32,
    pub enabled_at_ms: i64,
    pub location: String,
    pub payment: String,
}
impl Snapshot {
    pub(super) fn authorized_by(&self, config: &Config) -> bool {
        let stages = &config.engine_settings.stages;
        config
            .notes
            .as_ref()
            .is_some_and(|n| n.engine() == super::Engine::App && n.enabled_at_ms.is_some())
            && config.engine_settings.steps.translation
            && stages.translation_enabled_at_ms == Some(self.enabled_at_ms)
            && stages.translation_helper == self.helper
            && stages.translation_location == self.location
            && stages.translation_payment == self.payment
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Disposition {
    pub reason: String,
    pub source: Source,
    pub segment_keys: Vec<String>,
    #[serde(default)]
    pub snapshot: Option<Snapshot>,
}
pub(super) fn prepare(
    root: &Path,
    selection: Option<&Selection>,
    previous: Option<&Disposition>,
    ended_ms: i64,
) -> io::Result<Disposition> {
    let mut result = Disposition {
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
        snapshot: previous.and_then(|p| p.snapshot.clone()),
    };
    let Some(selection) = selection.filter(|s| {
        s.source() != Source::HexBaseline && s.transcript.segments.iter().any(|s| s.translate)
    }) else {
        return Ok(result);
    };
    let config = Config::load_existing(root)?;
    let engine = &config.engine_settings;
    let stages = &engine.stages;
    let Some(enabled_at_ms) = stages
        .translation_enabled_at_ms
        .filter(|at| *at > 0 && *at <= ended_ms)
        .filter(|_| engine.steps.translation)
    else {
        return Ok(result);
    };
    if config
        .validate_engine_settings()
        .iter()
        .any(|f| f.starts_with("stages.translation_") || *f == "translation_prompt")
    {
        result.reason = "translation_settings_invalid".into();
        return Ok(result);
    }
    let snapshot = result.snapshot.get_or_insert_with(|| Snapshot {
        revision: selection.revision.clone(),
        helper: stages.translation_helper,
        model: stages.translation_model.clone(),
        deadline_seconds: stages.translation_deadline_seconds,
        batch_chars: stages.translation_batch_chars,
        include_context: stages.translation_include_context,
        prompt: engine.prompt(Prompt::Translation).into(),
        prompt_version: 1,
        enabled_at_ms,
        location: stages.translation_location.clone(),
        payment: stages.translation_payment.clone(),
    });
    result.reason = if snapshot.revision != selection.revision {
        "translation_source_changed"
    } else if !snapshot.authorized_by(&config) {
        "translation_consent_changed"
    } else if snapshot.location != "hosted" || snapshot.payment != "subscription" {
        "provider_policy_unavailable"
    } else {
        "helper_not_text_only"
    }
    .into();
    Ok(result)
}
