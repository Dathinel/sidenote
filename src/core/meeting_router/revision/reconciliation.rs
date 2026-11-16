//! Preserve owned readings that whole-sentence fallback cannot cover.
use super::*;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Coverage {
    pub no_speech: BTreeMap<String, Chunk>,
    pub overlaps: Vec<Value>,
}

pub(super) fn no_speech_chunk(
    id: &str,
    r: &super::super::batches::Range,
    manifest: &super::super::pieces::PieceManifest,
) -> Result<Chunk> {
    let origin = manifest
        .tracks
        .iter()
        .find(|t| t.track == r.track)
        .ok_or("Missing NoSpeech track")?
        .origin_sample;
    Ok(Chunk {
        id: id.into(),
        track: r.track.clone(),
        start: meeting(origin, r.range.context.0)?,
        end: meeting(origin, r.range.context.1)?,
        speaker: None,
        turn: None,
        english: String::new(),
        audio_start: None,
    })
}

pub(super) fn validate_markers(data: &Revision, coverage: &Coverage) -> Result<()> {
    let plan: Plan = serde_json::from_value(data.provenance["plan"].clone())?;
    let manifest = serde_json::from_value(if data.provenance["pieces"].is_object() {
        data.provenance["pieces"].clone()
    } else {
        data.provenance.clone()
    })?;
    let markers = data.provenance["no_speech"]
        .as_object()
        .ok_or("Missing NoSpeech evidence")?;
    if plan.revision != data.plan_revision || markers.len() != coverage.no_speech.len() {
        return Err("Invalid NoSpeech coverage evidence".into());
    }
    for (id, chunk) in &coverage.no_speech {
        let range = plan
            .batches
            .values()
            .find_map(|b| b.get(id))
            .ok_or("Missing NoSpeech range")?;
        let expected = no_speech_chunk(id, range, &manifest)?;
        if data.evidence.contains_key(id)
            || serde_json::to_value(chunk)? != serde_json::to_value(expected)?
            || markers.get(id)
                != Some(&serde_json::json!({"revision":plan.revision,"range":id,
                "reason":"no-speech","source":"captured-hex-baseline"}))
        {
            return Err("Invalid NoSpeech coverage evidence".into());
        }
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
    let markers: Vec<_> = coverage
        .no_speech
        .iter()
        .filter(|(_, c)| super::super::baseline::source_covered(c, std::slice::from_ref(fallback)))
        .map(|(id, _)| id)
        .collect();
    if markers.is_empty() {
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
        "track":unit.track,"intervals":intervals,"seconds":seconds,"no_speech":markers}),
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
    loop {
        let Some(candidate) = eligible.iter().find(|s| {
            s.sources
                .iter()
                .any(|o| !super::super::baseline::source_covered(&o.chunk, &t.segments))
        }) else {
            break;
        };
        t.segments.retain(|s| {
            let Some(unit) = baseline_of(s, baseline) else {
                return true;
            };
            !overlaps(s, candidate)
                || coverage
                    .no_speech
                    .values()
                    .any(|c| c.track == unit.track && c.start < unit.end && unit.start < c.end)
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
