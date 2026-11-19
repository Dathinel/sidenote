use super::super::fallback::{Interval, RoutingFallback};
use super::reconciliation::{Coverage, FallbackKind};
use super::*;
#[cfg(all(windows, feature = "asr-runtime"))]
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct NoSpeech {
    interval: Interval,
    reply_sha256: String,
    code: String,
}

#[cfg(all(windows, feature = "asr-runtime"))]
pub(crate) fn capture(transcript: &[u8]) -> Result<Vec<NoSpeech>> {
    let mut records = Vec::new();
    for line in std::str::from_utf8(transcript)?
        .lines()
        .filter(|s| !s.trim().is_empty())
    {
        let row: Value = serde_json::from_str(line)?;
        let reply = &row["baseline_reply"];
        if reply["code"] != "no-speech" {
            continue;
        }
        if reply["version"] != 4
            || reply["status"] != "error"
            || row["text"].as_str().is_none_or(|s| !s.trim().is_empty())
        {
            return Err("Invalid captured baseline no-speech reply".into());
        }
        let interval = Interval {
            track: row["source"].as_str().unwrap_or("system").into(),
            start: row["start_ms"].as_f64().ok_or("Missing no-speech start")? / 1000.0,
            end: row["end_ms"].as_f64().ok_or("Missing no-speech end")? / 1000.0,
        };
        if !interval.start.is_finite()
            || !interval.end.is_finite()
            || interval.start < 0.0
            || interval.start >= interval.end
        {
            return Err("Invalid baseline no-speech interval".into());
        }
        records.push(NoSpeech {
            interval,
            code: "no-speech".into(),
            reply_sha256: hex::encode(Sha256::digest(serde_json::to_vec(reply)?)),
        });
    }
    Ok(records)
}

fn intersects(c: &Chunk, track: &str, start: f64, end: f64) -> bool {
    c.track == track && c.start < end && start < c.end
}

fn uncovered(c: &Chunk, segments: &[Segment]) -> Vec<Chunk> {
    let mut covered: Vec<_> = segments
        .iter()
        .flat_map(|s| &s.sources)
        .map(|o| &o.chunk)
        .filter(|o| intersects(c, &o.track, o.start, o.end))
        .collect();
    covered.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut start = c.start;
    let mut missing = Vec::new();
    for (next, end) in covered
        .into_iter()
        .map(|o| (o.start, o.end))
        .chain(std::iter::once((c.end, c.end)))
    {
        if start < next {
            missing.push(Chunk {
                start,
                end: next,
                ..c.clone()
            });
        }
        start = start.max(end);
    }
    missing
}

fn attach(
    data: &mut Revision,
    coverage: &[Chunk],
    replies: &[NoSpeech],
    reconciliation: &Coverage,
) -> Result<()> {
    let routing = super::super::fallback::read(data)?;
    let manifest: super::super::pieces::PieceManifest =
        serde_json::from_value(if data.provenance["pieces"].is_object() {
            data.provenance["pieces"].clone()
        } else {
            data.provenance.clone()
        })?;
    let mut gaps = Vec::new();
    for c in coverage {
        if !c.start.is_finite()
            || !c.end.is_finite()
            || c.start < 0.0
            || c.start >= c.end
            || !c.english.is_empty()
            || !manifest.tracks.iter().any(|t| {
                t.track == c.track
                    && t.origin_sample as f64 / 16000.0 <= c.start
                    && c.end <= (t.origin_sample as f64 + t.sample_count as f64) / 16000.0
            })
        {
            return Err("Invalid speech coverage interval".into());
        }
        let missing = uncovered(c, &data.transcript.segments);
        if missing.is_empty() {
            continue;
        }
        let reasons: Vec<&RoutingFallback> = routing
            .iter()
            .filter(|r| {
                matches!(
                    r.reason,
                    super::super::fallback::Reason::GuardRejected
                        | super::super::fallback::Reason::Uncalibrated
                        | super::super::fallback::Reason::InvalidScores
                ) && r
                    .interval
                    .as_ref()
                    .is_some_and(|i| i.track == c.track && i.start == c.start && i.end == c.end)
            })
            .collect();
        let (evidence, disposition) = if !reasons.is_empty() {
            let labels = reasons
                .iter()
                .map(|r| serde_json::to_string(&r.reason))
                .collect::<std::result::Result<Vec<_>, _>>()?
                .join(", ");
            (serde_json::to_value(reasons)?, format!("router {labels}"))
        } else if let Some(record) = reconciliation.fallback_required.iter().find(|r| {
            matches!(
                r.reason,
                FallbackKind::NoSpeech | FallbackKind::HelperFailed
            ) && r.interval.track == c.track
                && r.interval.start == c.start
                && r.interval.end == c.end
        }) {
            let label = serde_json::to_string(&record.reason)?;
            (serde_json::json!([record]), format!("recognizer {label}"))
        } else {
            continue;
        };
        for gap in missing {
            if data
                .transcript
                .baseline
                .iter()
                .chain(
                    data.alternatives
                        .iter()
                        .flat_map(|s| &s.sources)
                        .map(|o| &o.chunk),
                )
                .any(|b| intersects(&gap, &b.track, b.start, b.end))
            {
                continue;
            }
            let reply = replies.iter().find(|r| {
                r.code == "no-speech"
                    && r.reply_sha256.len() == 64
                    && r.reply_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    && r.interval.track == gap.track
                    && r.interval.start <= gap.start
                    && gap.end <= r.interval.end
            });
            let (baseline, baseline_label) = match reply {
                Some(r) => (serde_json::to_value(r)?, "baseline no-speech"),
                None => (
                    serde_json::json!({"code":"no-overlapping-baseline-text"}),
                    "no captured words",
                ),
            };
            let text = format!(
                "[Speech unavailable: {:.3}-{:.3}s; {baseline_label}; {disposition}]",
                gap.start, gap.end
            );
            gaps.push(
                serde_json::json!({"id":gap.id,"track":gap.track,"start":gap.start,"end":gap.end,
                "reasons":evidence,"baseline":baseline}),
            );
            data.transcript.segments.push(Segment {
                start: gap.start,
                end: gap.end,
                track: gap.track.clone(),
                route: "en".into(),
                lang: "en".into(),
                lang_conf: None,
                text_original: text.clone(),
                text_en: Some(text),
                english_reading: String::new(),
                provider: "sidenote".into(),
                model: "none".into(),
                status: "marked-gap".into(),
                translate: false,
                sources: vec![Offset {
                    joined_start: 0.0,
                    joined_end: gap.end - gap.start,
                    chunk: gap,
                    scores: None,
                }],
            });
        }
    }
    data.provenance["speech_coverage"] = serde_json::to_value(coverage)?;
    data.provenance["baseline_no_speech"] = serde_json::to_value(replies)?;
    data.provenance["marked_gaps"] = serde_json::to_value(&gaps)?;
    if !gaps.is_empty() {
        data.transcript.warnings.push(format!(
            "{} speech intervals have no reading; visible gaps retained.",
            gaps.len()
        ));
    }
    data.transcript.segments.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.track.cmp(&b.track))
            .then(a.sources[0].chunk.id.cmp(&b.sources[0].chunk.id))
    });
    for c in coverage {
        if !super::super::baseline::source_covered(c, &data.transcript.segments) {
            return Err(format!("Speech lacks routed or explicit baseline coverage or marked gap: id={} track={} seconds=[{},{}]",
                c.id, c.track, c.start, c.end).into());
        }
    }
    Ok(())
}

pub(super) fn from_files(
    root: &Path,
    data: &mut Revision,
    reconciliation: &Coverage,
) -> Result<()> {
    let read = |name: &str| -> Result<Option<Vec<u8>>> {
        match super::super::inputs::Snapshot::read_file(&root.join(name), 16 << 20) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    let Some(coverage) = read("coverage.json")? else {
        return Ok(());
    };
    let coverage: Vec<Chunk> = serde_json::from_slice(&coverage)?;
    let replies: Vec<NoSpeech> = read("baseline-no-speech.json")?
        .map(|b| serde_json::from_slice(&b))
        .transpose()?
        .unwrap_or_default();
    attach(data, &coverage, &replies, reconciliation)
}

pub(super) fn validate(
    data: &Revision,
    expected: &mut Revision,
    reconciliation: &Coverage,
) -> Result<()> {
    if let Some(coverage) = data.provenance.get("speech_coverage") {
        let coverage: Vec<Chunk> = serde_json::from_value(coverage.clone())?;
        let replies: Vec<NoSpeech> =
            serde_json::from_value(data.provenance["baseline_no_speech"].clone())?;
        attach(expected, &coverage, &replies, reconciliation)?;
        if expected.provenance["marked_gaps"] != data.provenance["marked_gaps"] {
            return Err("Invalid marked gap evidence".into());
        }
    } else if data
        .transcript
        .segments
        .iter()
        .any(|s| s.status == "marked-gap")
    {
        return Err("Marked gap lacks speech coverage evidence".into());
    }
    Ok(())
}
