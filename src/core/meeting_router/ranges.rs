//! Pure, per-track/piece S45d planning; no live router calls this module.
use super::vad::Span;
use crate::core::{asr::Result, lid::Decision};
const PAD: u64 = 8_000;
const CAP: u64 = 480_000;
pub struct Speech<'a> {
    pub interval: Span,
    pub decision: &'a Decision,
    pub speaker_turn: Option<(&'a str, &'a str)>,
}
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AsrRange {
    pub context: Span,
    pub members: Vec<(usize, Span)>,
    /// None explicitly retains the captured baseline route.
    pub language: Option<String>,
}
/// Sorted half-open speech inside one piece; call separately for each track/piece.
/// Silero splits short silence equally (floor on both sides, leaving an odd sample).
/// Speech is partitioned to ownership first; context can cross a piece seam.
pub fn padded(track: Span, ownership: Span, speech: &[Span]) -> Result<Vec<Span>> {
    if track.0 > ownership.0 || ownership.0 > ownership.1 || ownership.1 > track.1 {
        return Err("Ownership must be inside the track".into());
    }
    let mut previous = ownership.0;
    for &(start, end) in speech {
        if start < previous || start >= end || end > ownership.1 {
            return Err("Speech must be sorted, disjoint and inside the piece".into());
        }
        previous = end;
    }
    Ok(speech
        .iter()
        .enumerate()
        .map(|(i, &(start, end))| {
            let left = if i == 0 {
                PAD
            } else {
                ((start - speech[i - 1].1) / 2).min(PAD)
            };
            let right = speech
                .get(i + 1)
                .map_or(PAD, |next| ((next.0 - end) / 2).min(PAD));
            (
                start.saturating_sub(left).max(track.0),
                end.saturating_add(right).min(track.1),
            )
        })
        .collect())
}
/// Joins only adjacent accepted spans in one known speaker/turn, including silence.
/// A single oversized span is split at the cap with disjoint context/ownership,
/// matching S45d (no second padding pass at the artificial split).
pub fn plan(track: Span, ownership: Span, speech: &[Speech<'_>]) -> Result<Vec<AsrRange>> {
    let intervals: Vec<_> = speech.iter().map(|s| s.interval).collect();
    let context = padded(track, ownership, &intervals)?;
    let mut groups: Vec<(Span, Option<&str>, Vec<usize>)> = Vec::new();
    for (i, s) in speech.iter().enumerate() {
        let language = match s.decision {
            Decision::Accepted { language, .. } => Some(language.as_str()),
            Decision::Unsure(_) => None,
        };
        if let Some(last) = groups.last_mut()
            && language.is_some()
            && last.1 == language
            && s.speaker_turn.is_some()
            && s.speaker_turn == speech[i - 1].speaker_turn
            && context[i].1 - last.0.0 <= CAP
        {
            last.0.1 = context[i].1;
            last.2.push(i);
        } else {
            groups.push((context[i], language, vec![i]));
        }
    }
    let mut ranges = Vec::new();
    for ((mut start, end), language, members) in groups {
        while start < end {
            let stop = start.saturating_add(CAP).min(end);
            let owned = members
                .iter()
                .filter_map(|&i| {
                    let a = intervals[i].0.max(start);
                    let b = intervals[i].1.min(stop);
                    (a < b).then_some((i, (a, b)))
                })
                .collect();
            ranges.push(AsrRange {
                context: (start, stop),
                members: owned,
                language: language.map(str::to_owned),
            });
            start = stop;
        }
    }
    Ok(ranges)
}
#[cfg(test)]
mod tests;
