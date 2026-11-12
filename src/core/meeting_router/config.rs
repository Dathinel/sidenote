use super::super::asr::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingConfig {
    pub schema: u32,
    pub profile: String,
    #[serde(skip)]
    pub meeting_languages: Vec<String>,
    pub language_mode: String,
    pub language_detection: super::super::lid::Settings,
    pub model_state_dir: PathBuf,
    pub runtime_dll: PathBuf,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            profile: "compat".into(),
            meeting_languages: vec!["en".into()],
            language_mode: "auto".into(),
            language_detection: super::super::lid::Settings::default(),
            model_state_dir: PathBuf::new(),
            runtime_dll: PathBuf::new(),
            extra: BTreeMap::new(),
        }
    }
}
impl RoutingConfig {
    pub fn enabled(&self) -> bool {
        self.profile == "experimental"
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 || !matches!(self.profile.as_str(), "compat" | "experimental") {
            return Err("Unsupported routing profile".into());
        }
        if self.meeting_languages.is_empty()
            || self.meeting_languages.iter().any(|s| {
                !matches!(s.as_str(), "en" | "mixed")
                    && !super::super::model_catalog::catalog()
                        .iter()
                        .any(|m| m.languages.contains(&s.as_str()))
            })
        {
            return Err("Choose known meeting languages in Settings".into());
        }
        if !matches!(
            self.language_mode.as_str(),
            "auto" | "off" | "forced:en" | "forced:hi" | "forced:mixed"
        ) {
            return Err("Choose auto, off, forced:en, forced:hi or forced:mixed".into());
        }
        Ok(())
    }
    pub fn non_english_route(&self) -> &'static str {
        if self.meeting_languages.iter().any(|s| s == "mixed") {
            "mixed"
        } else {
            "hi"
        }
    }
}
