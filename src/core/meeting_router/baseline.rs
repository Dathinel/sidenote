use super::*;

pub fn english_fallback(
    baseline: &[Chunk],
    config: &RoutingConfig,
    input_checksum: String,
    reason: &str,
) -> Transcript {
    Transcript {
        schema: "transcript.segments.v1".into(),
        experimental: true,
        input_checksum,
        config_checksum: configuration_key(config).unwrap_or_default(),
        baseline: baseline.to_vec(),
        segments: baseline
            .iter()
            .map(|b| {
                english(
                    b,
                    None,
                    &format!("language not checked: {reason}; English retained"),
                )
            })
            .collect(),
        english_reading: baseline
            .iter()
            .map(|b| b.english.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        wall_seconds: 0.0,
        lid_load_seconds: None,
        hi_load_seconds: None,
        mixed_calls: 0,
        mixed_seconds: 0.0,
        warnings: vec![reason.into()],
    }
}

fn overlaps(segment: &Segment, baseline: &Chunk) -> bool {
    segment.sources.iter().any(|s| {
        s.chunk.track == baseline.track
            && s.chunk.start < baseline.end
            && s.chunk.end > baseline.start
    })
}
fn covered(baseline: &Chunk, segments: &[Segment], valid: &[bool], tolerance: f64) -> bool {
    let mut spans: Vec<_> = segments
        .iter()
        .zip(valid)
        .filter(|(_, v)| **v)
        .flat_map(|(s, _)| &s.sources)
        .filter(|s| {
            s.chunk.track == baseline.track
                && s.chunk.end > baseline.start
                && s.chunk.start < baseline.end
        })
        .map(|s| (s.chunk.start, s.chunk.end))
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut until = baseline.start;
    for (start, end) in spans {
        if start > until + tolerance {
            #[cfg(test)]
            if std::env::var_os("SIDENOTE_ASSEMBLY_TRACE").is_some() {
                eprintln!(
                    "baseline incomplete: id={} interval=[{},{}] first_gap=[{},{}] tolerance={tolerance}",
                    baseline.id, baseline.start, baseline.end, until, start
                );
            }
            return false;
        }
        until = until.max(end);
        if until + tolerance >= baseline.end {
            return true;
        }
    }
    false
}
pub(super) fn source_covered(baseline: &Chunk, segments: &[Segment]) -> bool {
    covered(baseline, segments, &vec![true; segments.len()], 0.0)
}

/// A partial reading cannot replace any part of a baseline sentence. Keep its own interval.
pub fn reconcile(transcript: &mut Transcript, baseline: &[Chunk]) {
    reconcile_with_tolerance(transcript, baseline, 0.0001, false);
}
pub(super) fn reconcile_exact(transcript: &mut Transcript, baseline: &[Chunk]) {
    reconcile_with_tolerance(transcript, baseline, 0.0, true);
}
fn reconcile_with_tolerance(
    transcript: &mut Transcript,
    baseline: &[Chunk],
    tolerance: f64,
    include_english: bool,
) {
    let original = std::mem::take(&mut transcript.segments);
    let mut valid: Vec<_> = original
        .iter()
        .map(|s| (include_english || s.route != "en") && !s.text_original.trim().is_empty())
        .collect();
    loop {
        let complete: Vec<_> = baseline
            .iter()
            .map(|b| covered(b, &original, &valid, tolerance))
            .collect();
        let mut changed = false;
        for (i, s) in original.iter().enumerate() {
            if valid[i]
                && baseline
                    .iter()
                    .zip(&complete)
                    .any(|(b, done)| overlaps(s, b) && !done)
            {
                #[cfg(test)]
                if std::env::var_os("SIDENOTE_ASSEMBLY_TRACE").is_some() {
                    for (b, done) in baseline.iter().zip(&complete) {
                        if overlaps(s, b) && !done {
                            eprintln!(
                                "reconcile drop: source={} route={} interval=[{},{}] baseline={} interval=[{},{}] complete=false",
                                s.sources[0].chunk.id,
                                s.route,
                                s.start,
                                s.end,
                                b.id,
                                b.start,
                                b.end
                            );
                        }
                    }
                }
                valid[i] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (s, valid) in original.iter().zip(&valid) {
        if *valid {
            let mut s = s.clone();
            let reading = baseline
                .iter()
                .filter(|b| overlaps(&s, b))
                .map(|b| b.english.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if !reading.is_empty() && (s.route != "en" || s.english_reading.is_empty()) {
                s.english_reading = reading;
            }
            transcript.segments.push(s);
        }
    }
    for b in baseline {
        if covered(b, &original, &valid, tolerance) {
            continue;
        }
        let matching: Vec<_> = original.iter().filter(|s| overlaps(s, b)).collect();
        let status = if matching.iter().any(|s| s.route != "en") {
            "routed text did not fully cover English interval; English retained"
        } else if let Some(s) = matching.iter().find(|s| s.status != "checked") {
            s.status.as_str()
        } else if matching.is_empty() {
            "language not checked: no speech window; English retained"
        } else {
            "checked"
        };
        let scores = matching
            .iter()
            .find_map(|s| s.sources.first().and_then(|o| o.scores.clone()));
        transcript.segments.push(english(b, scores, status));
    }
    transcript.baseline = baseline.to_vec();
    transcript.english_reading = baseline
        .iter()
        .map(|b| b.english.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    transcript.segments.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then_with(|| a.track.cmp(&b.track))
    });
}
