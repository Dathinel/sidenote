//! Unwired recognition spool; process ownership and real adapters belong to layer 8b.
use super::ranges::AsrRange;
use crate::core::{asr::Result, store_io};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
pub struct Range {
    pub track: String,
    pub piece: String,
    /// Immutable cut of exactly range.context, mono 16 kHz PCM16.
    pub wav: PathBuf,
    pub range: AsrRange,
}
#[derive(Serialize, Deserialize)]
pub struct Plan {
    /// Caller supplies source/config/calibration/model identities.
    pub revision: String,
    pub batches: BTreeMap<String, Vec<Range>>,
}
fn key(value: &impl Serialize) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(value)?)
        .to_hex()
        .to_string())
}
pub fn spool(root: &Path, revision: String, ranges: Vec<Range>) -> Result<Plan> {
    if revision.is_empty() {
        return Err("Missing revision".into());
    }
    let mut batches: BTreeMap<String, Vec<Range>> = BTreeMap::new();
    let mut ids = HashSet::new();
    for r in ranges {
        let lang = r.range.language.as_deref().ok_or("Baseline range")?;
        if !matches!(lang, "en" | "hi" | "te")
            || !ids.insert(key(&r)?)
            || !(1..=480_000).contains(&r.range.context.1.saturating_sub(r.range.context.0))
        {
            return Err("Invalid or duplicate range".into());
        }
        batches.entry(lang.into()).or_default().push(r);
    }
    let plan = Plan { revision, batches };
    let _lock = store_io::lock(&root.join("job.lock"))?;
    let path = root.join("plan.json");
    let bytes = serde_json::to_vec(&plan)?;
    if path.exists() {
        if store_io::read_bounded(&path, 16 << 20)? != bytes {
            return Err("Stale plan revision".into());
        }
    } else {
        store_io::publish(&path, &bytes)?;
    }
    Ok(plan)
}
fn cached(root: &Path, id: &str) -> bool {
    store_io::read_bounded(&root.join(id), 1 << 20)
        .ok()
        .and_then(|b| serde_json::from_slice::<(String, Value)>(&b).ok())
        .is_some_and(|(receipt, v)| {
            receipt == id && v["text"].as_str().is_some_and(|s| !s.trim().is_empty())
        })
}
/// One owned session per language; layer-7 request/result types stay in its adapter.
pub trait Recognizer {
    fn recognize(&mut self, range: &Range) -> Result<Value>;
}
pub fn run(
    root: &Path,
    plan: &Plan,
    mut load: impl FnMut(&str) -> Result<Box<dyn Recognizer>>,
) -> Result<()> {
    let _lock = store_io::lock(&root.join("job.lock"))?;
    if store_io::read_bounded(&root.join("plan.json"), 16 << 20)? != serde_json::to_vec(plan)? {
        return Err("Stale plan revision".into());
    }
    for (lang, ranges) in &plan.batches {
        let ids = ranges
            .iter()
            .map(|r| key(&(plan, r)))
            .collect::<Result<Vec<_>>>()?;
        let pending: Vec<_> = ranges
            .iter()
            .zip(ids)
            .filter(|(_, id)| !cached(root, id))
            .collect();
        if pending.is_empty() {
            continue;
        }
        let mut session = load(lang)?;
        for (r, id) in pending {
            let v = session.recognize(r)?;
            if v["text"].as_str().is_none_or(|s| s.trim().is_empty()) {
                return Err("Empty result".into());
            }
            store_io::write(&root.join(&id), &(id, v))?;
        }
        drop(session);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
