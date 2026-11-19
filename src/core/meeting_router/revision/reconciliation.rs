//! Preserve owned readings that whole-sentence fallback cannot cover.
use super::super::fallback::{Interval, Reason as RouterReason, RoutingFallback};
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum FallbackKind {
    NoSpeech,
    LidUncertain,
    HelperFailed,
}

#[derive(Serialize, Deserialize)]
pub(super) struct RequiredFallback {
    reason: FallbackKind,
    interval: Interval,
    record: Value,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Coverage {
    pub fallback_required: Vec<RequiredFallback>,
    pub overlaps: Vec<Value>,
}

pub(super) fn from_router_records(
    plan: &Plan,
    manifest: &super::super::pieces::PieceManifest,
    markers: &BTreeMap<String, Value>,
    routing: &[RoutingFallback],
    transport: &Value,
) -> Result<Coverage> {
    let mut coverage = Coverage::default();
    for (id, record) in markers {
        let r = plan
            .batches
            .values()
            .find_map(|b| b.get(id))
            .ok_or("Missing NoSpeech range")?;
        if record
            != &serde_json::json!({"revision":plan.revision,"range":id,
            "reason":"no-speech","source":"captured-hex-baseline"})
        {
            return Err("Invalid NoSpeech record".into());
        }
        let origin = manifest
            .tracks
            .iter()
            .find(|t| t.track == r.track)
            .ok_or("Missing fallback track")?
            .origin_sample;
        coverage.fallback_required.push(RequiredFallback {
            reason: FallbackKind::NoSpeech,
            interval: Interval {
                track: r.track.clone(),
                start: meeting(origin, r.range.context.0)?,
                end: meeting(origin, r.range.context.1)?,
            },
            record: record.clone(),
        });
    }
    if let Some(ranges) = transport["ranges"].as_object() {
        for (id, record) in ranges.iter().filter(|(_, r)| r["transport"] == "failed") {
            let r = plan
                .batches
                .get("en")
                .and_then(|b| b.get(id))
                .ok_or("Missing failed helper range")?;
            if record["model"] != r.model_id
                || record["language"] != "en"
                || record["fallback"] != "captured-hex-baseline"
                || !record["failure_reason"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
            {
                return Err("Invalid failed helper record".into());
            }
            let origin = manifest
                .tracks
                .iter()
                .find(|t| t.track == r.track)
                .ok_or("Missing fallback track")?
                .origin_sample;
            for &(_, (start, end)) in &r.range.members {
                coverage.fallback_required.push(RequiredFallback {
                    reason: FallbackKind::HelperFailed,
                    interval: Interval {
                        track: r.track.clone(),
                        start: meeting(origin, start)?,
                        end: meeting(origin, end)?,
                    },
                    record: serde_json::json!({"revision":plan.revision,"range":id,"transport":record}),
                });
            }
        }
    }
    for recorded in routing {
        if !matches!(
            recorded.reason,
            RouterReason::GuardRejected | RouterReason::Uncalibrated | RouterReason::InvalidScores
        ) {
            continue;
        }
        let interval = recorded
            .interval
            .as_ref()
            .ok_or("Missing recorded fallback interval")?;
        let track = manifest
            .tracks
            .iter()
            .find(|t| t.track == interval.track)
            .ok_or("Missing fallback track")?;
        if !interval.start.is_finite()
            || !interval.end.is_finite()
            || interval.start >= interval.end
            || interval.start < meeting(track.origin_sample, 0)?
            || interval.end > meeting(track.origin_sample, track.sample_count)?
        {
            return Err("Invalid recorded fallback interval".into());
        }
        coverage.fallback_required.push(RequiredFallback {
            reason: FallbackKind::LidUncertain,
            interval: interval.clone(),
            record: serde_json::to_value(recorded)?,
        });
    }
    Ok(coverage)
}

pub(super) fn validate_records(data: &Revision, coverage: &Coverage) -> Result<()> {
    let plan: Plan = serde_json::from_value(data.provenance["plan"].clone())?;
    let manifest = serde_json::from_value(if data.provenance["pieces"].is_object() {
        data.provenance["pieces"].clone()
    } else {
        data.provenance.clone()
    })?;
    let markers: BTreeMap<String, Value> =
        serde_json::from_value(data.provenance["no_speech"].clone())?;
    if plan.revision != data.plan_revision
        || markers.keys().any(|id| data.evidence.contains_key(id))
        || data.provenance["english_transport"]["ranges"]
            .as_object()
            .is_some_and(|ranges| {
                ranges
                    .iter()
                    .any(|(id, r)| r["transport"] == "failed" && data.evidence.contains_key(id))
            })
    {
        return Err("Invalid fallback coverage evidence".into());
    }
    let expected = from_router_records(
        &plan,
        &manifest,
        &markers,
        &super::super::fallback::read(data)?,
        &data.provenance["english_transport"],
    )?;
    if serde_json::to_value(expected.fallback_required)?
        != serde_json::to_value(&coverage.fallback_required)?
    {
        return Err("Fallback reason was not recorded by the router".into());
    }
    Ok(())
}

fn overlaps(a: &Segment, b: &Segment) -> bool {
    a.sources.iter().any(|x| {
        b.sources.iter().any(|y| {
            x.chunk.track == y.chunk.track
                && x.chunk.start < y.chunk.end
                && y.chunk.start < x.chunk.end
        })
    })
}

fn baseline_of<'a>(s: &Segment, baseline: &'a [Chunk]) -> Option<&'a Chunk> {
    baseline.iter().find(|b| {
        s.route == "en"
            && s.sources.len() == 1
            && s.track == b.track
            && s.start == b.start
            && s.end == b.end
            && s.text_original == b.english
            && serde_json::to_value(&s.sources[0].chunk).ok() == serde_json::to_value(b).ok()
    })
}

fn fallback_intersection(required: &RequiredFallback, unit: &Chunk) -> Option<Interval> {
    let start = required.interval.start.max(unit.start);
    let end = required.interval.end.min(unit.end);
    (required.interval.track == unit.track && start < end).then(|| Interval {
        track: unit.track.clone(),
        start,
        end,
    })
}

pub(super) fn receipt(
    a: &Segment,
    b: &Segment,
    baseline: &[Chunk],
    coverage: &Coverage,
) -> Option<Value> {
    let (fallback, native, unit) = if let Some(unit) = baseline_of(a, baseline) {
        (a, b, unit)
    } else {
        (b, a, baseline_of(b, baseline)?)
    };
    if !matches!(native.route.as_str(), "hi" | "te")
        || native.provider != "local"
        || !matches!(
            native.status.as_str(),
            "segment-level" | "validated-samples"
        )
        || !native.sources.iter().any(|o| {
            o.chunk.track == unit.track && (o.chunk.start < unit.start || o.chunk.end > unit.end)
        })
        || !overlaps(fallback, native)
    {
        return None;
    }
    let required: Vec<_> = coverage
        .fallback_required
        .iter()
        .filter(|r| fallback_intersection(r, unit).is_some())
        .collect();
    if required.is_empty() {
        return None;
    }
    let intervals: Vec<_> = native
        .sources
        .iter()
        .filter(|o| o.chunk.track == unit.track)
        .map(|o| (o.chunk.start.max(unit.start), o.chunk.end.min(unit.end)))
        .filter(|(start, end)| start < end)
        .collect();
    let seconds: f64 = intervals.iter().map(|(start, end)| end - start).sum();
    Some(
        serde_json::json!({"baseline":unit.id,"native_source":native.sources.first()?.chunk.id,
        "track":unit.track,"intervals":intervals,"seconds":seconds,"fallback_required":required}),
    )
}

pub(super) fn reconcile(t: &mut Transcript, baseline: &[Chunk], coverage: &mut Coverage) {
    let candidates = t.segments.clone();
    t.segments.retain(|a| {
        a.status != "unresolved-context"
            && !candidates
                .iter()
                .any(|b| a.sources[0].chunk.id != b.sources[0].chunk.id && overlaps(a, b))
    });
    let eligible = t.segments.clone();
    super::super::baseline::reconcile_exact(t, baseline);
    // Restore whole, untimed readings; never trim their text or manufacture ownership.
    while let Some(candidate) = eligible.iter().find(|s| {
        s.sources
            .iter()
            .any(|o| !super::super::baseline::source_covered(&o.chunk, &t.segments))
    }) {
        t.segments.retain(|s| {
            let Some(unit) = baseline_of(s, baseline) else {
                return true;
            };
            !overlaps(s, candidate)
                || coverage
                    .fallback_required
                    .iter()
                    .any(|r| fallback_intersection(r, unit).is_some())
        });
        let mut selected = candidate.clone();
        selected.english_reading = baseline
            .iter()
            .filter(|b| {
                b.track == selected.track && b.start < selected.end && selected.start < b.end
            })
            .map(|b| b.english.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        t.segments.push(selected);
    }
    t.segments.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.track.cmp(&b.track))
            .then(a.sources[0].chunk.id.cmp(&b.sources[0].chunk.id))
    });
    coverage.overlaps.clear();
    for (i, a) in t.segments.iter().enumerate() {
        for b in &t.segments[i + 1..] {
            if let Some(value) = receipt(a, b, baseline, coverage) {
                coverage.overlaps.push(value);
            }
        }
    }
}
