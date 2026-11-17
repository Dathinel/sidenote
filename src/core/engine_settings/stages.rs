//! Script defaults only; execution and helper authorization remain separate.
use super::{valid_model, valid_text};
use crate::core::{daily_check, notes::Helper};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StageSettings {
    pub translation_helper: Helper,
    pub translation_model: String,
    pub translation_deadline_seconds: u32,
    pub title_helper: Helper,
    pub title_model: String,
    pub title_deadline_seconds: u32,
    pub tasks_helper: Helper,
    pub tasks_model: String,
    pub tasks_request_deadline_seconds: u32,
    pub comparison_helper: Helper,
    pub comparison_model: String,
    pub comparison_request_deadline_seconds: u32,
    pub prepare_deadline_seconds: u32,
    pub routing_deadline_seconds: u32,
    pub notes_deadline_seconds: u32,
    pub render_deadline_seconds: u32,
    pub publish_deadline_seconds: u32,
    pub tasks_deadline_seconds: u32,
    pub comparison_deadline_seconds: u32,
    pub kb_refresh_deadline_seconds: u32,
    pub kb_read_deadline_seconds: u32,
    pub comparison_kb_deadline_seconds: u32,
    pub asr_request_deadline_seconds: u32,
    pub lid_request_deadline_seconds: u32,
    pub comparison_delay_seconds: u32,
    pub asr_seconds_per_audio_second_per_pass: f64,
    pub asr_deadline_headroom: f64,
    pub project_list_source: String,
    pub comparison_playbook_source: String,
    pub export_roots: Vec<String>,
    pub all_models: bool,
    pub routing_mode: String,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}
impl Default for StageSettings {
    fn default() -> Self {
        Self {
            translation_helper: Helper::OpenCode,
            translation_model: "openai/gpt-6.1-terra-fast".into(),
            translation_deadline_seconds: 240,
            title_helper: Helper::Codex,
            title_model: "gpt-6.1-sol".into(),
            title_deadline_seconds: 60,
            tasks_helper: Helper::Codex,
            tasks_model: "gpt-6.1-sol".into(),
            tasks_request_deadline_seconds: 300,
            comparison_helper: Helper::Codex,
            comparison_model: "gpt-6.1-sol".into(),
            comparison_request_deadline_seconds: 600,
            prepare_deadline_seconds: 1800,
            routing_deadline_seconds: 360,
            notes_deadline_seconds: 600,
            render_deadline_seconds: 120,
            publish_deadline_seconds: 120,
            tasks_deadline_seconds: 360,
            comparison_deadline_seconds: 1800,
            kb_refresh_deadline_seconds: 60,
            kb_read_deadline_seconds: 30,
            comparison_kb_deadline_seconds: 300,
            asr_request_deadline_seconds: 180,
            lid_request_deadline_seconds: 300,
            comparison_delay_seconds: 300,
            asr_seconds_per_audio_second_per_pass: 0.20,
            asr_deadline_headroom: 1.5,
            project_list_source: "07-Planning/tracking/projects.yaml".into(),
            comparison_playbook_source: "docs/reference/mom-playbook.md".into(),
            export_roots: Vec::new(),
            all_models: false,
            routing_mode: "shadow".into(),
            extra: BTreeMap::new(),
        }
    }
}
impl StageSettings {
    pub(super) const FIELDS: &'static [&'static str] = &[
        "stages.translation_helper",
        "stages.translation_model",
        "stages.translation_deadline_seconds",
        "stages.title_helper",
        "stages.title_model",
        "stages.title_deadline_seconds",
        "stages.tasks_helper",
        "stages.tasks_model",
        "stages.tasks_request_deadline_seconds",
        "stages.comparison_helper",
        "stages.comparison_model",
        "stages.comparison_request_deadline_seconds",
        "stages.prepare_deadline_seconds",
        "stages.routing_deadline_seconds",
        "stages.notes_deadline_seconds",
        "stages.render_deadline_seconds",
        "stages.publish_deadline_seconds",
        "stages.tasks_deadline_seconds",
        "stages.comparison_deadline_seconds",
        "stages.kb_refresh_deadline_seconds",
        "stages.kb_read_deadline_seconds",
        "stages.comparison_kb_deadline_seconds",
        "stages.asr_request_deadline_seconds",
        "stages.lid_request_deadline_seconds",
        "stages.comparison_delay_seconds",
        "stages.asr_seconds_per_audio_second_per_pass",
        "stages.asr_deadline_headroom",
        "stages.project_list_source",
        "stages.comparison_playbook_source",
        "stages.export_roots",
        "stages.all_models",
        "stages.routing_mode",
    ];
    pub(super) fn errors(&self) -> Vec<&'static str> {
        let value = serde_json::to_value(self).expect("serializable stage settings");
        Self::FIELDS
            .iter()
            .copied()
            .filter(|field| {
                let key = field.strip_prefix("stages.").unwrap();
                let current = &value[key];
                if key.ends_with("_model") {
                    !current.as_str().is_some_and(valid_model)
                } else if key.ends_with("_deadline_seconds") {
                    !current.as_u64().is_some_and(|v| (1..=3600).contains(&v))
                } else {
                    match key {
                        "comparison_delay_seconds" => self.comparison_delay_seconds > 86400,
                        "asr_seconds_per_audio_second_per_pass" => {
                            !(0.01..=10.0).contains(&self.asr_seconds_per_audio_second_per_pass)
                        }
                        "asr_deadline_headroom" => {
                            !(1.0..=10.0).contains(&self.asr_deadline_headroom)
                        }
                        "project_list_source" | "comparison_playbook_source" => {
                            !current.as_str().is_some_and(daily_check::safe_kb_path)
                        }
                        "export_roots" => self
                            .export_roots
                            .iter()
                            .any(|p| !valid_text(p) || !Path::new(p).is_absolute()),
                        "routing_mode" => {
                            !matches!(self.routing_mode.as_str(), "shadow" | "execute")
                        }
                        _ => false,
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_models_deadlines_and_script_ranges_round_trip() {
        let mut stages = StageSettings::default();
        assert!(stages.errors().is_empty());
        assert_eq!(stages.translation_helper, Helper::OpenCode);
        assert_eq!(stages.translation_deadline_seconds, 240);
        assert_eq!(stages.title_deadline_seconds, 60);
        assert_eq!(stages.notes_deadline_seconds, 600);
        assert_eq!(stages.tasks_deadline_seconds, 360);
        assert_eq!(stages.comparison_deadline_seconds, 1800);
        stages.translation_model = "synthetic-translation-model".into();
        stages.title_model = "synthetic-title-model".into();
        stages.translation_deadline_seconds = 321;
        stages.title_deadline_seconds = 42;
        stages.notes_deadline_seconds = 777;
        stages.asr_seconds_per_audio_second_per_pass = 0.4;
        stages.asr_deadline_headroom = 2.0;
        let bytes = serde_json::to_vec(&stages).unwrap();
        let loaded: StageSettings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&loaded).unwrap(), bytes);
        assert!(loaded.errors().is_empty());
        stages.translation_deadline_seconds = 0;
        stages.title_deadline_seconds = 3601;
        stages.asr_seconds_per_audio_second_per_pass = f64::NAN;
        stages.asr_deadline_headroom = 0.5;
        assert_eq!(
            stages.errors(),
            [
                "stages.translation_deadline_seconds",
                "stages.title_deadline_seconds",
                "stages.asr_seconds_per_audio_second_per_pass",
                "stages.asr_deadline_headroom"
            ]
        );
    }
}
