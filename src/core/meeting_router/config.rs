use super::super::{asr::Result, lid::LANGUAGES, model_catalog as models};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_stop_languages: Option<LanguageRoutes>,
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
            after_stop_languages: None,
            model_state_dir: PathBuf::new(),
            runtime_dll: PathBuf::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// Declarations survive migration without implicitly selecting a provider or model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LanguageRoutes {
    pub primary: String,
    pub supporting: Vec<String>,
    #[serde(deserialize_with = "unique_routes")]
    pub routes: BTreeMap<String, ProviderRoute>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderRoute {
    pub provider: String,
    pub model: String,
}
fn unique_routes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, ProviderRoute>, D::Error> {
    struct Routes;
    impl<'de> serde::de::Visitor<'de> for Routes {
        type Value = BTreeMap<String, ProviderRoute>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a language-to-provider route map without duplicate languages")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut routes = BTreeMap::new();
            while let Some((language, route)) = map.next_entry::<String, ProviderRoute>()? {
                if routes.insert(language.clone(), route).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "Duplicate route: {language}"
                    )));
                }
            }
            Ok(routes)
        }
    }
    deserializer.deserialize_map(Routes)
}
impl LanguageRoutes {
    pub fn from_legacy(languages: Vec<String>) -> Result<Self> {
        let (primary, supporting) = languages
            .split_first()
            .ok_or("Declare a primary language")?;
        let config = Self {
            primary: primary.clone(),
            supporting: supporting.to_vec(),
            routes: BTreeMap::new(),
        };
        config.validate_declarations()?;
        Ok(config)
    }
    fn validate_declarations(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for language in std::iter::once(&self.primary).chain(&self.supporting) {
            if !LANGUAGES.contains(&language.as_str()) {
                return Err(format!("Unsupported declared language: {language}").into());
            }
            if !seen.insert(language) {
                return Err(format!("Duplicate declared language: {language}").into());
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_declarations()?;
        let declared: Vec<_> = std::iter::once(&self.primary)
            .chain(&self.supporting)
            .collect();
        for language in &declared {
            let route = self
                .routes
                .get(*language)
                .ok_or_else(|| format!("Missing provider route: {language}"))?;
            let supported = if route.provider == "hex" && route.model == "parakeet_unified_en" {
                language.as_str() == "en"
            } else {
                models::catalog().iter().any(|model| {
                    model.id == route.model
                        && model.role == models::Role::Asr
                        && model.languages.contains(&language.as_str())
                        && matches!(
                            (route.provider.as_str(), model.runtime),
                            ("hex", models::Runtime::Hex) | ("local", models::Runtime::SherpaOnnx)
                        )
                })
            };
            if !supported {
                return Err(format!(
                    "Unsupported provider/model route for {language}: {}/{}",
                    route.provider, route.model
                )
                .into());
            }
        }
        if self
            .routes
            .keys()
            .any(|language| !declared.contains(&language))
        {
            return Err("Provider route for undeclared language".into());
        }
        Ok(())
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
        if let Some(languages) = &self.after_stop_languages {
            languages.validate()?;
        }
        if self.meeting_languages.is_empty()
            || self.meeting_languages.iter().any(|s| {
                !matches!(s.as_str(), "en" | "mixed")
                    && !models::catalog()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{config::Config, meeting_watch::Labs};
    use serde_json::json;
    fn failure(config: &LanguageRoutes) -> String {
        config.validate().unwrap_err().to_string()
    }
    fn routes() -> LanguageRoutes {
        serde_json::from_value(json!({
            "primary":"te", "supporting":["en", "hi"], "routes":{
                "te":{"provider":"local", "model":"indicconformer-te-ctc"},
                "en":{"provider":"hex", "model":"parakeet_unified_en"},
                "hi":{"provider":"local", "model":"indicconformer-hi-ctc"}
            }
        }))
        .unwrap()
    }
    #[test]
    fn after_stop_flag_is_independent_and_default_false() {
        for labs in [
            json!({}),
            json!({
                "language_detection":true, "process_meeting_windows":true
            }),
        ] {
            let config: Config =
                serde_json::from_value(json!({"version":1, "features":{}, "labs":labs})).unwrap();
            assert!(!config.labs.after_stop_language_routing);
            assert_eq!(config.pipeline.profile, "compat");
            assert!(!config.pipeline.enabled());
        }
        let labs: Labs =
            serde_json::from_value(json!({"after_stop_language_routing":true})).unwrap();
        assert!(labs.after_stop_language_routing);
        assert!(!labs.language_detection && !labs.process_meeting_windows);
        let old: Config = serde_json::from_value(json!({
            "version":1, "features":{}, "pipeline":{"profile":"experimental"},
            "labs":{"language_detection":true}
        }))
        .unwrap();
        assert!(!old.labs.after_stop_language_routing);
    }
    #[test]
    fn legacy_declarations_and_explicit_routes_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let languages = ["te", "en", "hi", "ur"].map(str::to_owned).to_vec();
        models::save_meeting_languages(root.path(), &languages).unwrap();
        let migrated = models::load_language_routes(root.path()).unwrap();
        let migrated: LanguageRoutes =
            serde_json::from_value(serde_json::to_value(migrated).unwrap()).unwrap();
        assert_eq!(migrated.primary, "te");
        assert_eq!(migrated.supporting, ["en", "hi", "ur"]);
        assert!(migrated.routes.is_empty());
        assert_eq!(
            models::load_meeting_languages(root.path()).unwrap(),
            languages
        );
        let config = RoutingConfig {
            after_stop_languages: Some(routes()),
            ..Default::default()
        };
        let loaded: RoutingConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(loaded.after_stop_languages, config.after_stop_languages);
        loaded.validate().unwrap();
        loaded.after_stop_languages.unwrap().validate().unwrap();
        assert_eq!(loaded.profile, "compat");
    }
    #[test]
    fn invalid_duplicate_and_unsupported_routes_explain_the_failure() {
        for (provider, model) in [
            ("cloud", "parakeet_unified_en"),
            ("local", "absent"),
            ("local", "anuvani-lid-int8"),
            ("local", "indicconformer-hi-ctc"),
        ] {
            let mut config = routes();
            config.routes.insert(
                "en".into(),
                ProviderRoute {
                    provider: provider.into(),
                    model: model.into(),
                },
            );
            assert!(failure(&config).contains("Unsupported provider/model route for en"));
        }
        let mut config = routes();
        config.routes.remove("hi");
        assert_eq!(failure(&config), "Missing provider route: hi");
        let mut config = routes();
        config.supporting.push("te".into());
        assert_eq!(failure(&config), "Duplicate declared language: te");
        config.primary = "invalid".into();
        assert!(failure(&config).contains("Unsupported declared language"));
        let mut config = routes();
        config.supporting.pop();
        assert_eq!(failure(&config), "Provider route for undeclared language");
        let duplicate = r#"{"primary":"en","supporting":[],"routes":{"en":{"provider":"hex","model":"parakeet_unified_en"},"en":{"provider":"hex","model":"parakeet_unified_en"}}}"#;
        assert!(
            serde_json::from_str::<LanguageRoutes>(duplicate)
                .unwrap_err()
                .to_string()
                .contains("Duplicate route: en")
        );
    }
}
