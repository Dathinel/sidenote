//! Immutable sequential language batches with restart receipts.
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
    read_result(root, plan, id).ok().flatten().is_some()
}
pub trait Recognizer {
    fn recognize(&mut self, range: &Range) -> Result<Value>;
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
    fn limit(&mut self, _remaining: std::time::Duration) {}
}
/// Validate the receipt before exposing a reading to reconciliation; no audio is opened.
pub fn read_result(root: &Path, plan: &Plan, id: &str) -> Result<Option<Value>> {
    let range = plan
        .batches
        .values()
        .find_map(|b| b.get(id))
        .ok_or("Range is not in plan")?;
    if id.len() != 64
        || !id.bytes().all(|b| b.is_ascii_hexdigit())
        || key(range)? != id
        || plan.revision.len() != 64
        || !plan.revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid plan/range identity".into());
    }
    #[cfg(windows)]
    {
        let check = || -> Result<Option<Value>> {
            let bytes = Snapshot::read_file(&path(root, plan, id, "result"), 1 << 20)?;
            let receipt: Receipt = serde_json::from_slice(&Snapshot::read_file(
                &path(root, plan, id, "receipt"),
                4096,
            )?)?;
            let value: Value = serde_json::from_slice(&bytes)?;
            Ok((receipt.revision == plan.revision
                && receipt.range == id
                && receipt.sha256 == hash(&bytes)
                && value["text"].as_str().is_some_and(|s| !s.trim().is_empty()))
            .then_some(value))
        };
        Ok(check().unwrap_or(None))
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(None)
    }
}
#[cfg(windows)]
fn no_speech(plan: &Plan, id: &str) -> Value {
    serde_json::json!({"revision":plan.revision,"range":id,"reason":"no-speech",
        "source":"captured-hex-baseline"})
}
pub(super) fn read_no_speech(root: &Path, plan: &Plan, id: &str) -> Option<Value> {
    #[cfg(windows)]
    {
        let value: Value = serde_json::from_slice(
            &Snapshot::read_file(&path(root, plan, id, "fallback"), 4096).ok()?,
        )
        .ok()?;
        (value == no_speech(plan, id)).then_some(value)
    }
    #[cfg(not(windows))]
    {
        let _ = (root, plan, id);
        None
    }
}
pub trait EnglishBridge {
    fn transcribe(&mut self, range: &Range) -> Result<Value>;
    fn limit(&mut self, _remaining: std::time::Duration) {}
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
        run_windows(root, plan, native, bridge, None)
    }
}
#[cfg(windows)]
pub fn run_until(
    root: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
    bridge: &mut impl EnglishBridge,
    timeout: std::time::Duration,
) -> Result<()> {
    run_windows(
        root,
        plan,
        native,
        bridge,
        Some(std::time::Instant::now() + timeout),
    )
}
#[cfg(windows)]
fn remaining(deadline: Option<std::time::Instant>) -> Result<Option<std::time::Duration>> {
    deadline
        .map(|d| {
            d.checked_duration_since(std::time::Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| super::hex_bridge::HexFailure::Timeout.into())
        })
        .transpose()
}
#[cfg(windows)]
fn run_windows(
    root: &Path,
    plan: &Plan,
    native: &mut impl NativeLoader,
    bridge: &mut impl EnglishBridge,
    deadline: Option<std::time::Instant>,
) -> Result<()> {
    let _lock = store_io::lock(&root.join("job.lock"))?;
    if revision(&plan.batches)? != plan.revision
        || Snapshot::read_file(&root.join(format!("{}.plan.json", plan.revision)), 16 << 20)?
            != serde_json::to_vec(plan)?
    {
        return Err("Stale plan revision".into());
    }
    for (lang, ranges) in &plan.batches {
        let started = std::time::Instant::now();
        let pending: Vec<_> = ranges
            .iter()
            .filter(|(id, _)| !cached(root, plan, id))
            .collect();
        let Some((_, first)) = pending.first() else {
            continue;
        };
        remaining(deadline)?;
        let mut session = match lang.as_str() {
            "en" => None,
            "hi" => Some(native.load(NativeLanguage::Hindi, first)?),
            "te" => Some(native.load(NativeLanguage::Telugu, first)?),
            _ => return Err("Unknown language".into()),
        };
        for (id, r) in pending {
            if let Some(left) = remaining(deadline)? {
                match &mut session {
                    Some(s) => s.limit(left),
                    None => bridge.limit(left),
                }
            }
            let result = match &mut session {
                Some(s) => s.recognize(r),
                None => bridge.transcribe(r),
            }
            .and_then(|value| {
                let text = value["text"].as_str().ok_or("Empty result")?;
                if text.trim().is_empty() {
                    Err(super::hex_bridge::HexFailure::NoSpeech.into())
                } else {
                    Ok(value)
                }
            });
            let v = match result {
                Ok(value) => value,
                Err(error)
                    if error
                        .downcast_ref::<super::hex_bridge::HexFailure>()
                        .is_some_and(|e| e.kind() == &super::hex_bridge::HexFailure::NoSpeech) =>
                {
                    remaining(deadline)?;
                    store_io::write(&path(root, plan, id, "fallback"), &no_speech(plan, id))?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            remaining(deadline)?;
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
        if let Some(s) = &mut session {
            s.finish()?;
        }
        drop(session);
        store_io::write(
            &root.join(format!("{lang}.clock.json")),
            &serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"revision":plan.revision}),
        )?;
    }
    Ok(())
}
#[cfg(all(test, windows))]
mod tests;
#[cfg(windows)]
pub mod worker;
