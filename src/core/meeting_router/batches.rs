//! Unwired spool. Real adapters belong to layer 8c.
use super::{inputs::Snapshot, ranges::AsrRange};
use crate::core::{asr::Result, store_io};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
pub struct Range {
    pub track: String,
    pub piece: String,
    /// Immutable mono 16 kHz PCM16 cut of range.context, at most 30 seconds.
    pub wav: PathBuf,
    pub range: AsrRange,
    pub model_id: String,
    /// Pinned model/pack checksum from the explicit route, not an installed-file timestamp.
    pub model_sha256: String,
}
#[derive(Serialize, Deserialize)]
pub struct Plan {
    pub revision: String,
    pub batches: BTreeMap<String, BTreeMap<String, Range>>,
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn key(value: &impl Serialize) -> Result<String> {
    Ok(hash(&serde_json::to_vec(value)?))
}
fn revision(batches: &BTreeMap<String, BTreeMap<String, Range>>) -> Result<String> {
    let mut audio = BTreeMap::new();
    for r in batches.values().flat_map(|b| b.values()) {
        if !audio.contains_key(&r.wav) {
            audio.insert(&r.wav, hash(&Snapshot::read_file(&r.wav, 1 << 20)?));
        }
    }
    key(&(batches, audio))
}
pub fn spool(root: &Path, ranges: Vec<Range>) -> Result<Plan> {
    let mut batches: BTreeMap<String, BTreeMap<String, Range>> = BTreeMap::new();
    for r in ranges {
        let lang = r.range.language.as_deref().ok_or("Baseline range")?;
        if !matches!(lang, "en" | "hi" | "te")
            || r.model_id.is_empty()
            || r.model_sha256.len() != 64
            || !r.model_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !(1..=480_000).contains(&r.range.context.1.saturating_sub(r.range.context.0))
        {
            return Err("Invalid range/model pin".into());
        }
        let batch = batches.entry(lang.into()).or_default();
        if batch
            .values()
            .any(|v| v.model_id != r.model_id || v.model_sha256 != r.model_sha256)
            || batch.insert(key(&r)?, r).is_some()
        {
            return Err("Duplicate range or conflicting model".into());
        }
    }
    let plan = Plan {
        revision: revision(&batches)?,
        batches,
    };
    let _lock = store_io::lock(&root.join("job.lock"))?;
    let path = root.join(format!("{}.plan.json", plan.revision));
    let bytes = serde_json::to_vec(&plan)?;
    if path.exists() {
        if Snapshot::read_file(&path, 16 << 20)? != bytes {
            return Err("Stale plan".into());
        }
    } else {
        store_io::publish(&path, &bytes)?;
    }
    Ok(plan)
}
#[cfg(windows)]
fn path(root: &Path, plan: &Plan, id: &str, kind: &str) -> PathBuf {
    root.join(format!("{}-{id}.{kind}", plan.revision))
}
#[cfg(windows)]
#[derive(Serialize, Deserialize)]
struct Receipt {
    revision: String,
    range: String,
    sha256: String,
}
#[cfg(windows)]
fn cached(root: &Path, plan: &Plan, id: &str) -> bool {
    let check = || -> Result<bool> {
        let bytes = Snapshot::read_file(&path(root, plan, id, "result"), 1 << 20)?;
        let receipt: Receipt = serde_json::from_slice(&Snapshot::read_file(
            &path(root, plan, id, "receipt"),
            4096,
        )?)?;
        let value: Value = serde_json::from_slice(&bytes)?;
        Ok(receipt.revision == plan.revision
            && receipt.range == id
            && receipt.sha256 == hash(&bytes)
            && value["text"].as_str().is_some_and(|s| !s.trim().is_empty()))
    };
    check().unwrap_or(false)
}
pub trait Recognizer {
    fn recognize(&mut self, range: &Range) -> Result<Value>;
}
pub trait EnglishBridge {
    fn transcribe(&mut self, range: &Range) -> Result<Value>;
}
#[derive(Clone, Copy, Debug)]
pub enum NativeLanguage {
    Hindi,
    Telugu,
}
pub trait NativeLoader {
    fn load(&mut self, language: NativeLanguage, range: &Range) -> Result<Box<dyn Recognizer>>;
}
pub fn run(
    root: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
    bridge: &mut impl EnglishBridge,
) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = (root, plan, native, bridge);
        Err(super::hex_bridge::HexFailure::Unsupported.into())
    }
    #[cfg(windows)]
    {
        run_windows(root, plan, native, bridge)
    }
}
#[cfg(windows)]
fn run_windows(
    root: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
    bridge: &mut impl EnglishBridge,
) -> Result<()> {
    let _lock = store_io::lock(&root.join("job.lock"))?;
    if revision(&plan.batches)? != plan.revision
        || Snapshot::read_file(&root.join(format!("{}.plan.json", plan.revision)), 16 << 20)?
            != serde_json::to_vec(plan)?
    {
        return Err("Stale plan revision".into());
    }
    for (lang, ranges) in &plan.batches {
        let pending: Vec<_> = ranges
            .iter()
            .filter(|(id, _)| !cached(root, plan, id))
            .collect();
        let Some((_, first)) = pending.first() else {
            continue;
        };
        let mut session = match lang.as_str() {
            "en" => None,
            "hi" => Some(native.load(NativeLanguage::Hindi, first)?),
            "te" => Some(native.load(NativeLanguage::Telugu, first)?),
            _ => return Err("Unknown language".into()),
        };
        for (id, r) in pending {
            let v = match &mut session {
                Some(s) => s.recognize(r)?,
                None => bridge.transcribe(r)?,
            };
            if v["text"].as_str().is_none_or(|s| s.trim().is_empty()) {
                return Err("Empty result".into());
            }
            let bytes = serde_json::to_vec(&v)?;
            store_io::atomic_write(&path(root, plan, id, "result"), &bytes)?;
            store_io::write(
                &path(root, plan, id, "receipt"),
                &Receipt {
                    revision: plan.revision.clone(),
                    range: id.clone(),
                    sha256: hash(&bytes),
                },
            )?;
        }
        drop(session);
    }
    Ok(())
}
#[cfg(all(test, windows))]
mod tests;
#[cfg(windows)]
pub mod worker;
