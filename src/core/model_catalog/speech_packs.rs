//! Versioned speech pack registry. A pack names only what code does not already hold: its
//! language, script, catalogue model, decoder and readiness. File names and hashes, the
//! model's languages and runtime come from the model catalogue. The detector label is the
//! language code in Anuvani's pinned label order. The decoder contract says which backends
//! can run a pack; a route request names one of them.
use super::{Model, Role, Runtime, catalog};
use crate::core::lid::LANGUAGES;
use serde::{Deserialize, Serialize, de::IntoDeserializer};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::BTreeMap, sync::OnceLock};

const BUILTIN: &[u8] = include_bytes!("speech_packs.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Local,
    Hex,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decoder {
    CtcTokens,
    HexService,
}
impl Decoder {
    /// Model runtime, file roles in catalogue file order, and the backends implementing it.
    /// A new decoder or backend is one arm here plus its implementation.
    fn contract(self) -> (Runtime, &'static [&'static str], &'static [Backend]) {
        match self {
            Self::CtcTokens => (Runtime::SherpaOnnx, &["graph", "tokens"], &[Backend::Local]),
            Self::HexService => (Runtime::Hex, &[], &[Backend::Hex]),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Readiness {
    Qualified,
    Experimental,
}

/// Stable refusal codes. Records and tests match on these, never on prose.
pub mod code {
    pub const REGISTRY: &str = "speech_pack_registry_invalid";
    pub const DUPLICATE: &str = "speech_pack_duplicate_language";
    pub const LANGUAGE: &str = "speech_pack_undetectable_language";
    pub const SCRIPT: &str = "speech_pack_invalid_script";
    pub const DECODER: &str = "speech_pack_unsupported_decoder";
    pub const MODEL: &str = "speech_pack_model_mismatch";
    pub const HASH: &str = "speech_pack_invalid_hash";
    pub const UNKNOWN: &str = "speech_pack_unknown_language";
    pub const UNQUALIFIED: &str = "speech_pack_unqualified";
    pub const ROUTE: &str = "speech_pack_route_mismatch";
    pub const REVISION: &str = "speech_pack_revision_changed";
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackRefusal {
    pub code: Cow<'static, str>,
    pub subject: String,
}
impl PackRefusal {
    pub fn new(code: &'static str, subject: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            subject: subject.into(),
        }
    }
}
impl std::fmt::Display for PackRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.subject)
    }
}
impl std::error::Error for PackRefusal {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(rename = "schema")]
    _schema: u32,
    packs: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    language: String,
    script: String,
    model: String,
    decoder: String,
    readiness: Readiness,
}

pub struct Pack {
    pub script: String,
    pub model: &'static Model,
    pub decoder: Decoder,
    pub readiness: Readiness,
    /// Language, model, decoder and every catalogue file name and hash.
    pub revision: String,
}
impl Pack {
    /// The same rule for every backend: a catalogued ASR model of the decoder's runtime that
    /// serves the language, with one pinned file per decoder role.
    fn check(raw: RawPack, catalog: &'static [Model]) -> Result<Self, PackRefusal> {
        let refuse = |code| Err(PackRefusal::new(code, raw.language.as_str()));
        let script = raw.script.as_bytes();
        if !LANGUAGES.contains(&raw.language.as_str()) {
            return refuse(code::LANGUAGE);
        }
        if !(script.len() == 4
            && script[0].is_ascii_uppercase()
            && script[1..].iter().all(u8::is_ascii_lowercase))
        {
            return refuse(code::SCRIPT);
        }
        let Some(decoder) = variant::<Decoder>(&raw.decoder) else {
            return refuse(code::DECODER);
        };
        let (runtime, roles, _) = decoder.contract();
        let Some(model) = catalog.iter().find(|m| {
            m.id == raw.model
                && m.role == Role::Asr
                && m.runtime == runtime
                && m.languages.contains(&raw.language.as_str())
                && m.files.len() == roles.len()
        }) else {
            return refuse(code::MODEL);
        };
        let hex =
            |s: &str| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !model.files.iter().all(|f| hex(f.sha256)) {
            return refuse(code::HASH);
        }
        let (mut hash, decoder_id) = (Sha256::new(), format!("{decoder:?}"));
        let parts = [raw.language.as_str(), model.id, decoder_id.as_str()].into_iter();
        for part in parts.chain(model.files.iter().flat_map(|f| [f.name, f.sha256])) {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
        Ok(Self {
            revision: hash.finalize().iter().map(|b| format!("{b:02x}")).collect(),
            script: raw.script,
            model,
            decoder,
            readiness: raw.readiness,
        })
    }
}
fn variant<T: serde::de::DeserializeOwned>(id: &str) -> Option<T> {
    let id: serde::de::value::StrDeserializer<'_, serde::de::value::Error> = id.into_deserializer();
    T::deserialize(id).ok()
}

/// One route pinned to a pack revision. A request keeps it; it is never remapped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRoute {
    pub backend: Backend,
    pub model: String,
    pub decoder: Decoder,
    pub revision: String,
}
/// Per-language pins. A refusal stays with its language and blocks only routes that use it.
pub type RouteSnapshot = BTreeMap<String, Result<ResolvedRoute, PackRefusal>>;

/// A bad pack is held with its refusal; the other packs stay usable.
pub struct Registry {
    packs: BTreeMap<String, Result<Pack, PackRefusal>>,
}
impl Registry {
    pub fn parse(bytes: &[u8], catalog: &'static [Model]) -> Result<Self, PackRefusal> {
        let invalid = |e: serde_json::Error| PackRefusal::new(code::REGISTRY, e.to_string());
        // The version is read before the strict parse, so a newer schema is named as such.
        let document: serde_json::Value = serde_json::from_slice(bytes).map_err(invalid)?;
        if document["schema"] != 1 {
            let schema = document["schema"].to_string();
            return Err(PackRefusal::new(code::REGISTRY, format!("schema {schema}")));
        }
        let mut packs = BTreeMap::new();
        for value in Document::deserialize(document).map_err(invalid)?.packs {
            let Some(language) = value["language"].as_str().map(str::to_owned) else {
                return Err(PackRefusal::new(code::REGISTRY, "pack without a language"));
            };
            let mut pack = serde_json::from_value(value)
                .map_err(|e| PackRefusal::new(code::REGISTRY, format!("{language}: {e}")))
                .and_then(|raw| Pack::check(raw, catalog));
            if packs.contains_key(&language) {
                pack = Err(PackRefusal::new(code::DUPLICATE, &language));
            }
            packs.insert(language, pack);
        }
        Ok(Self { packs })
    }
    pub fn pack(&self, language: &str) -> Result<&Pack, PackRefusal> {
        match self.packs.get(language) {
            Some(pack) => pack.as_ref().map_err(Clone::clone),
            None => Err(PackRefusal::new(code::UNKNOWN, language)),
        }
    }
    pub fn resolve(
        &self,
        language: &str,
        backend: &str,
        model: &str,
    ) -> Result<ResolvedRoute, PackRefusal> {
        let pack = self.pack(language)?;
        if pack.readiness != Readiness::Qualified {
            return Err(PackRefusal::new(code::UNQUALIFIED, language));
        }
        let (_, _, backends) = pack.decoder.contract();
        match variant::<Backend>(backend).filter(|b| backends.contains(b)) {
            Some(backend) if model == pack.model.id => Ok(ResolvedRoute {
                backend,
                model: model.into(),
                decoder: pack.decoder,
                revision: pack.revision.clone(),
            }),
            _ => Err(PackRefusal::new(code::ROUTE, language)),
        }
    }
}

/// The shipped registry. It is compiled in, so it cannot change within one process.
pub fn registry() -> Result<&'static Registry, PackRefusal> {
    static SHIPPED: OnceLock<Result<Registry, PackRefusal>> = OnceLock::new();
    let shipped = SHIPPED.get_or_init(|| Registry::parse(BUILTIN, catalog()));
    shipped.as_ref().map_err(Clone::clone)
}
#[cfg(test)]
pub(crate) fn shipped_with(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut document: serde_json::Value = serde_json::from_slice(BUILTIN).unwrap();
    edit(&mut document);
    serde_json::to_vec(&document).unwrap()
}
/// The shipped catalogue with one file hash replaced.
#[cfg(test)]
pub(crate) fn catalog_with_hash(id: &str, file: usize, sha256: &'static str) -> &'static [Model] {
    let edit = |m: &Model| -> &'static [super::ModelFile] {
        let files = m.files.iter().enumerate().map(|(i, f)| super::ModelFile {
            sha256: if i == file { sha256 } else { f.sha256 },
            ..*f
        });
        Box::leak(files.collect::<Vec<_>>().into_boxed_slice())
    };
    let models = catalog().iter().map(|m| Model {
        files: if m.id == id { edit(m) } else { m.files },
        ..*m
    });
    Box::leak(models.collect::<Vec<_>>().into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::{code::*, *};
    use serde_json::json;
    fn parse(edit: impl FnOnce(&mut serde_json::Value)) -> Registry {
        Registry::parse(&shipped_with(edit), catalog()).unwrap()
    }
    fn assert_refused(registry: &Registry, language: &str, expected: &str) {
        let refusal = registry.pack(language).err().unwrap();
        let subject = refusal.subject.split(':').next();
        assert_eq!((&*refusal.code, subject), (expected, Some(language)));
    }
    #[test]
    fn registry_refuses_invalid_packs_with_typed_reasons() {
        let shipped = registry().unwrap();
        assert!(["en", "hi", "te"].iter().all(|l| shipped.pack(l).is_ok()));
        let te = |model: &str, decoder: &str| {
            json!({"language":"te", "script":"Telu", "model":model, "decoder":decoder,
                "readiness":"qualified"})
        };
        let mut extra = te("indicconformer-te-ctc", "ctc-tokens");
        extra["files"] = json!([]);
        // Every backend gets the same model rule: catalogued, right runtime, serves the language.
        for (value, expected) in [
            (te("indicconformer-te-ctc", "whisper-seq2seq"), DECODER),
            (te("indicconformer-te-ctc", "hex-service"), MODEL),
            (te("nemotron_3_5_asr", "hex-service"), MODEL),
            (te("parakeet_unified_en", "hex-service"), MODEL),
            (te("whisper-hindi2hinglish-swift", "ctc-tokens"), MODEL),
            (extra, REGISTRY),
            (json!({"language":"te"}), REGISTRY),
        ] {
            let registry = parse(|d| d["packs"][2] = value);
            assert_refused(&registry, "te", expected);
            assert!(registry.pack("en").is_ok() && registry.pack("hi").is_ok());
        }
        for (index, field, value, language, expected) in [
            (0, "model", "parakeet_unifed_en", "en", MODEL),
            (1, "script", "deva", "hi", SCRIPT),
            (1, "readiness", "ready", "hi", REGISTRY),
            (1, "language", "fr", "fr", LANGUAGE),
            (1, "language", "en", "en", DUPLICATE),
        ] {
            let registry = parse(|d| d["packs"][index][field] = json!(value));
            assert_refused(&registry, language, expected);
        }
        for (id, file, language) in [
            ("indicconformer-te-ctc", 0, "te"),
            ("indicconformer-hi-ctc", 1, "hi"),
        ] {
            let registry = Registry::parse(BUILTIN, catalog_with_hash(id, file, "")).unwrap();
            assert_refused(&registry, language, HASH);
        }
        let invalid = |bytes: &[u8]| Registry::parse(bytes, catalog()).err().unwrap();
        let schema = invalid(br#"{"schema":2,"packs":[],"calibration":{}}"#);
        assert_eq!((&*schema.code, &*schema.subject), (REGISTRY, "schema 2"));
        assert_eq!(invalid(b"{").code, REGISTRY);
    }
    #[test]
    fn a_pack_for_a_catalogued_model_is_added_with_data_alone() {
        let marathi = json!({"language":"mr", "script":"Deva",
            "model":"indicconformer-mr-ctc", "decoder":"ctc-tokens", "readiness":"qualified"});
        let registry = parse(|d| d["packs"].as_array_mut().unwrap().push(marathi));
        let route = registry.resolve("mr", "local", "indicconformer-mr-ctc");
        let route = route.unwrap();
        let pinned = (route.backend, route.decoder, &*route.revision);
        let pack = registry.pack("mr").unwrap();
        assert_eq!(
            pinned,
            (Backend::Local, Decoder::CtcTokens, &*pack.revision)
        );
        assert_ne!(route.revision, registry.pack("hi").unwrap().revision);
        for (language, backend, model, expected) in [
            ("mr", "hex", "indicconformer-mr-ctc", ROUTE),
            ("mr", "local", "indicconformer-hi-ctc", ROUTE),
            ("mr", "cloud", "indicconformer-mr-ctc", ROUTE),
            ("ta", "local", "indicconformer-ta-ctc", UNKNOWN),
        ] {
            let refused = registry.resolve(language, backend, model).unwrap_err();
            assert_eq!(refused, PackRefusal::new(expected, language));
        }
        let registry = parse(|d| d["packs"][1]["readiness"] = json!("experimental"));
        let refused = registry.resolve("hi", "local", "indicconformer-hi-ctc");
        assert_eq!(refused.unwrap_err().code, UNQUALIFIED);
    }
}
