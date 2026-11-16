//! Sample-owned readings; immutable publication is separate from recognition.
use super::{
    Chunk, Offset, Segment, Transcript, baseline::reconcile_exact as reconcile, batches::Plan,
    english_fallback,
};
use crate::core::asr::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

mod publication;
pub use publication::{begin, commit, load, retain};

mod reconciliation;

#[derive(Clone, Serialize, Deserialize)]
pub struct Revision {
    pub generation: String,
    pub plan_revision: String,
    #[serde(flatten)]
    pub transcript: Transcript,
    pub alternatives: Vec<Segment>,
    pub evidence: BTreeMap<String, Value>,
    pub provenance: Value,
}
fn meeting(origin: i64, sample: u64) -> Result<f64> {
    let value = origin
        .checked_add(i64::try_from(sample)?)
        .ok_or("Offset overflow")?;
    if !(0..=9_007_199_254_740_992).contains(&value) {
        return Err("Unrepresentable meeting offset".into());
    }
    Ok(value as f64 / 16_000.0)
}
fn segment(id: &str, r: &super::batches::Range, origin: i64, value: &Value) -> Result<Segment> {
    let lang = r.range.language.as_deref().ok_or("Missing language")?;
    if !matches!(lang, "en" | "hi" | "te")
        || r.range.context.0 >= r.range.context.1
        || r.range.context.1 - r.range.context.0 > 480_000
    {
        return Err("Invalid range".into());
    }
    let mut previous = r.range.context.0;
    let mut sources = Vec::new();
    for &(_, (start, end)) in &r.range.members {
        if start < previous || start >= end || end > r.range.context.1 {
            return Err("Invalid ownership".into());
        }
        sources.push(Offset {
            chunk: Chunk {
                id: format!("{id}-{start}"),
                start: meeting(origin, start)?,
                end: meeting(origin, end)?,
                track: r.track.clone(),
                speaker: None,
                turn: None,
                english: String::new(),
                audio_start: None,
            },
            joined_start: (start - r.range.context.0) as f64 / 16_000.0,
            joined_end: (end - r.range.context.0) as f64 / 16_000.0,
            scores: None,
        });
        previous = end;
    }
    let first = sources.first().ok_or("Empty ownership")?;
    let mut text = value["text"]
        .as_str()
        .ok_or("Missing text")?
        .trim()
        .to_owned();
    let mut status = "segment-level";
    let mut owned_until = r.range.context.0;
    for &(_, (start, end)) in &r.range.members {
        if start != owned_until {
            status = "unresolved-context";
        }
        owned_until = end;
    }
    if owned_until != r.range.context.1 {
        status = "unresolved-context";
    }
    // Only a producer with qualified sample timing may trim context tokens.
    if value["timing"] == "validated-samples" {
        let mut owned = Vec::new();
        let mut previous = 0;
        for token in value["tokens"].as_array().ok_or("Missing timed tokens")? {
            let start = token["start_sample"]
                .as_u64()
                .ok_or("Invalid token start")?;
            let end = token["end_sample"].as_u64().ok_or("Invalid token end")?;
            if start < previous || start >= end || end > r.range.context.1 - r.range.context.0 {
                return Err("Invalid token timing".into());
            }
            previous = end;
            let a = start + r.range.context.0;
            let b = end + r.range.context.0;
            let inside = r.range.members.iter().any(|&(_, (s, e))| s <= a && b <= e);
            if !inside && r.range.members.iter().any(|&(_, (s, e))| a < e && b > s) {
                return Err("Ambiguous context token".into());
            }
            if inside {
                owned.push(token["text"].as_str().ok_or("Missing token text")?);
            }
        }
        text = owned.concat().replace('▁', " ").trim().to_owned();
        status = "validated-samples";
    }
    if text.is_empty() {
        return Err("Empty selected text".into());
    }
    Ok(Segment {
        start: first.chunk.start,
        end: sources.last().unwrap().chunk.end,
        track: r.track.clone(),
        route: lang.into(),
        lang: lang.into(),
        lang_conf: None,
        text_original: text.clone(),
        text_en: (lang == "en").then_some(text),
        english_reading: String::new(),
        provider: if lang == "en" { "hex" } else { "local" }.into(),
        model: r.model_id.clone(),
        status: status.into(),
        sources,
        translate: lang != "en",
    })
}
/// Failed receipts and unresolved competition preserve the complete baseline unit.
pub fn assemble(
    root: &Path,
    plan: &Plan,
    manifest: &super::pieces::PieceManifest,
    baseline: &[Chunk],
    generation: u128,
) -> Result<Revision> {
    if plan.revision.len() != 64 || !plan.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid plan identity".into());
    }
    if super::inputs::Snapshot::read_file(
        &root.join(format!("{}.plan.json", plan.revision)),
        16 << 20,
    )? != serde_json::to_vec(plan)?
    {
        return Err("Stale plan".into());
    }
    let mut transcript = english_fallback(
        baseline,
        &Default::default(),
        plan.revision.clone(),
        "routing fallback",
    );
    transcript.segments.clear();
    transcript.warnings.clear();
    transcript.input_checksum = manifest.source_revision.clone();
    transcript.config_checksum = manifest.config_checksum.clone();
    let mut evidence = BTreeMap::new();
    let mut no_speech = BTreeMap::new();
    let mut coverage = reconciliation::Coverage::default();
    for (id, range) in plan.batches.values().flat_map(|b| b.iter()) {
        let reading = super::batches::read_result(root, plan, id)?;
        if reading.is_none()
            && let Some(reason) = super::batches::read_no_speech(root, plan, id)
        {
            coverage.no_speech.insert(
                id.clone(),
                reconciliation::no_speech_chunk(id, range, manifest)?,
            );
            no_speech.insert(id.clone(), reason);
        }
        let track = manifest
            .tracks
            .iter()
            .find(|t| t.track == range.track)
            .ok_or("Missing track origin")?;
        if range.range.members.is_empty()
            || range.range.context.0 >= range.range.context.1
            || range.range.context.1 > track.sample_count
            || !track.pieces.iter().any(|p| {
                p.id == range.piece
                    && range
                        .range
                        .members
                        .iter()
                        .all(|&(_, (s, e))| p.start_sample <= s && s < e && e <= p.end_sample)
            })
        {
            return Err("Invalid piece ownership".into());
        }
        let mapped = reading
            .as_ref()
            .and_then(|v| segment(id, range, track.origin_sample, v).ok());
        match mapped {
            Some(s) => transcript.segments.push(s),
            None => transcript.warnings.push(format!("fallback: {id}")),
        }
        if let Some(value) = reading {
            evidence.insert(id.clone(), value);
        }
    }
    let candidates = transcript.segments.clone();
    reconciliation::reconcile(&mut transcript, baseline, &mut coverage);
    for range in plan.batches.values().flat_map(|b| b.values()) {
        let track = manifest
            .tracks
            .iter()
            .find(|t| t.track == range.track)
            .ok_or("Missing track origin")?;
        for &(_, (start, end)) in &range.range.members {
            let owned = Chunk {
                id: String::new(),
                start: meeting(track.origin_sample, start)?,
                end: meeting(track.origin_sample, end)?,
                track: range.track.clone(),
                speaker: None,
                turn: None,
                english: String::new(),
                audio_start: None,
            };
            if !super::baseline::source_covered(&owned, &transcript.segments) {
                return Err(format!("Owned interval lacks routed or baseline coverage: track={} language={:?} samples=[{start},{end}] seconds=[{},{}]", range.track, range.range.language, owned.start, owned.end).into());
            }
        }
    }
    transcript.segments.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.track.cmp(&b.track))
            .then(a.sources[0].chunk.id.cmp(&b.sources[0].chunk.id))
    });
    let mut provenance = serde_json::to_value(manifest)?;
    provenance["no_speech"] = serde_json::to_value(no_speech)?;
    provenance["plan"] = serde_json::to_value(plan)?;
    provenance["assembly_coverage"] = serde_json::to_value(coverage)?;
    Ok(Revision {
        generation: generation.to_string(),
        plan_revision: plan.revision.clone(),
        transcript,
        alternatives: candidates,
        evidence,
        provenance,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod captured_tests;

#[cfg(all(test, windows))]
mod receipts;
