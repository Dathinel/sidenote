use super::super::{
    asr::Result,
    lid::LANGUAGES,
    model_catalog::{
        self as models,
        speech_packs::{PackRefusal, Registry, RouteSnapshot, code},
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) const DEFAULT_CPU_PASSES: bool = cfg!(all(windows, feature = "asr-runtime"));

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
            profile: if DEFAULT_CPU_PASSES {
                "experimental"
            } else {
                "compat"
            }
            .into(),
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
    /// The calibrated default set. It stays fixed until calibration is keyed by language set.
    #[cfg(any(windows, test))]
    pub(super) fn cpu_after_stop() -> Self {
        Self {
            primary: "en".into(),
            supporting: vec!["hi".into(), "te".into()],
            routes: [
                ("en", "hex", "parakeet_unified_en"),
                ("hi", "local", "indicconformer-hi-ctc"),
                ("te", "local", "indicconformer-te-ctc"),
            ]
            .into_iter()
            .map(|(language, provider, model)| {
                (
                    language.into(),
                    ProviderRoute {
                        provider: provider.into(),
                        model: model.into(),
                    },
                )
            })
            .collect(),
        }
    }

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
    /// Declarations only. Providers and models are checked against the speech packs when a
    /// request pins the routes it uses (`RoutingConfig::route_snapshot`).
    pub fn validate(&self) -> Result<()> {
        self.validate_declarations()?;
        for language in std::iter::once(&self.primary).chain(&self.supporting) {
            if !self.routes.contains_key(language) {
                return Err(format!("Missing provider route: {language}").into());
            }
        }
        if self.routes.len() != 1 + self.supporting.len() {
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
            "auto" | "off" | "forced:en" | "forced:hi" | "forced:te" | "forced:mixed"
        ) {
            return Err("Choose auto, off, forced:en, forced:hi, forced:te or forced:mixed".into());
        }
        Ok(())
    }
    /// Pins every language the after-stop planner can route in this mode: the forced
    /// language, the primary when only one is declared or detection is off, else all.
    pub fn route_snapshot(&self, registry: &Registry) -> RouteSnapshot {
        let Some(routes) = &self.after_stop_languages else {
            return RouteSnapshot::new();
        };
        let declared = std::iter::once(&routes.primary).chain(&routes.supporting);
        let languages: Vec<&str> = match self.language_mode.strip_prefix("forced:") {
            Some(forced) if forced != "mixed" => vec![forced],
            _ if routes.supporting.is_empty() || self.language_mode == "off" => {
                vec![routes.primary.as_str()]
            }
            _ => declared.map(String::as_str).collect(),
        };
        let pin = |language: &str| match routes.routes.get(language) {
            Some(route) => registry.resolve(language, &route.provider, &route.model),
            None => Err(PackRefusal::new(code::ROUTE, language)),
        };
        languages
            .into_iter()
            .map(|l| (l.to_owned(), pin(l)))
            .collect()
    }
    /// A pinned request runs only while every pin still matches the shipped packs.
    pub fn verify_routes(&self, pinned: &RouteSnapshot, registry: &Registry) -> Result<()> {
        let current = self.route_snapshot(registry);
        let mut languages = current.keys().chain(pinned.keys());
        match languages.find(|l| current.get(*l) != pinned.get(*l)) {
            Some(language) => Err(PackRefusal::new(code::REVISION, language.as_str()).into()),
            None => Ok(()),
        }
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
    use crate::core::model_catalog::speech_packs;
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
    fn after_stop_defaults_to_cpu_on_windows_asr_and_preserves_explicit_choices() {
        for labs in [
            json!({}),
            json!({
                "language_detection":true, "process_meeting_windows":true
            }),
        ] {
            let config: Config =
                serde_json::from_value(json!({"version":1, "features":{}, "labs":labs})).unwrap();
            assert_eq!(config.labs.after_stop_language_routing, DEFAULT_CPU_PASSES);
            assert_eq!(config.pipeline.enabled(), DEFAULT_CPU_PASSES);
            assert_eq!(config.pipeline.language_mode, "auto");
            assert_eq!(
                config.engine_settings.stages.translation_enabled_at_ms,
                None
            );
        }
        let labs: Labs =
            serde_json::from_value(json!({"after_stop_language_routing":true})).unwrap();
        assert!(labs.after_stop_language_routing);
        assert!(!labs.language_detection && !labs.process_meeting_windows);
        let old: Config = serde_json::from_value(json!({
            "version":1, "features":{}, "pipeline":{"profile":"compat"},
            "labs":{"language_detection":true, "after_stop_language_routing":false}
        }))
        .unwrap();
        assert!(!old.labs.after_stop_language_routing);
        assert!(!old.pipeline.enabled());
        assert!(old.labs.language_detection);
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
            profile: "compat".into(),
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
    fn route_pins_follow_the_mode_and_keep_the_existing_backend_and_model() {
        use speech_packs::{Backend::*, Decoder::*};
        let registry = speech_packs::registry().unwrap();
        let mut config = RoutingConfig {
            after_stop_languages: Some(LanguageRoutes::cpu_after_stop()),
            ..RoutingConfig::default()
        };
        let mut pins = config.route_snapshot(registry);
        for (language, expected) in [
            ("en", (Hex, "parakeet_unified_en", HexService)),
            ("hi", (Local, "indicconformer-hi-ctc", CtcTokens)),
            ("te", (Local, "indicconformer-te-ctc", CtcTokens)),
        ] {
            let pin = pins[language].as_ref().unwrap();
            assert_eq!((pin.backend, &*pin.model, pin.decoder), expected);
            assert_eq!(pin.revision, registry.pack(language).unwrap().revision);
        }
        // Only the languages the mode routes are pinned; a changed pin is refused, not remapped.
        for (mode, languages) in [
            ("forced:te", "te"),
            ("off", "en"),
            ("forced:mixed", "en,hi,te"),
        ] {
            config.language_mode = mode.into();
            let pinned = config.route_snapshot(registry);
            let keys: Vec<_> = pinned.keys().cloned().collect();
            assert_eq!(keys.join(","), languages);
            config.verify_routes(&pinned, registry).unwrap();
        }
        pins.get_mut("te").unwrap().as_mut().unwrap().revision = "0".repeat(64);
        let refusal = config.verify_routes(&pins, registry).unwrap_err();
        assert_eq!(refusal.to_string(), "speech_pack_revision_changed: te");
    }
    #[test]
    fn invalid_duplicate_and_unsupported_routes_explain_the_failure() {
        let registry = speech_packs::registry().unwrap();
        for (language, provider, model) in [
            ("en", "cloud", "parakeet_unified_en"),
            ("en", "local", "absent"),
            ("en", "local", "anuvani-lid-int8"),
            ("en", "local", "indicconformer-hi-ctc"),
            ("hi", "local", "whisper-hindi2hinglish-swift"),
            ("hi", "hex", "nemotron_3_5_asr"),
        ] {
            let (mut routes, mut config) = (routes(), RoutingConfig::default());
            let route = serde_json::from_value(json!({"provider":provider, "model":model}));
            routes.routes.insert(language.into(), route.unwrap());
            routes.validate().unwrap();
            config.after_stop_languages = Some(routes);
            let pins = config.route_snapshot(registry);
            assert_eq!(pins[language], Err(PackRefusal::new(code::ROUTE, language)));
            assert!(pins["te"].is_ok());
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
